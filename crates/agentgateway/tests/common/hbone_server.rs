// Test helper for running an HBONE server that echoes data with a waypoint prefix
// Based on ztunnel's test server implementation:
// https://github.com/istio/ztunnel/blob/master/src/test_helpers/tcp.rs

use bytes::Bytes;
use http_body_util::Full;
use hyper::Response;
use hyper::server::conn::http2;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{debug, error, info};

#[allow(dead_code)]
#[derive(Copy, Clone, Debug)]
pub enum Mode {
	ReadDoubleWrite,
	ReadWrite,
}

static BUFFER_SIZE: usize = 2 * 1024 * 1024;

/// HBONE test server that accepts mTLS connections on port 15008 and echoes data with a prefix
pub struct HboneTestServer {
	listener: TcpListener,
	mode: Mode,
	name: String,
	waypoint_message: Vec<u8>, // Prefix to write before echoing data
}

impl HboneTestServer {
	pub async fn new(mode: Mode, name: &str, waypoint_message: Vec<u8>, port: u16) -> Self {
		let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

		let addr = SocketAddr::from(([127, 0, 0, 1], port));
		let listener = TcpListener::bind(addr).await.unwrap();
		Self {
			listener,
			mode,
			name: name.to_string(),
			waypoint_message,
		}
	}

	pub async fn run(self) {
		let certs = generate_test_certs(&self.name);
		let acceptor = create_tls_acceptor(certs);

		loop {
			let (tcp_stream, _) = self.listener.accept().await.unwrap();
			let tls_stream = match acceptor.accept(tcp_stream).await {
				Ok(stream) => stream,
				Err(e) => {
					// Log as debug since transient TLS errors are expected during test startup
					// when the client is still fetching certificates
					debug!(
						"TLS accept error (likely transient during startup): {:?}",
						e
					);
					continue;
				},
			};

			let mode = self.mode;
			let waypoint_message = self.waypoint_message.clone();

			tokio::spawn(async move {
				if let Err(err) = http2::Builder::new(hyper_util::rt::TokioExecutor::new())
					.serve_connection(
						TokioIo::new(tls_stream),
						service_fn(move |req| {
							let waypoint_message = waypoint_message.clone();
							async move {
								info!("waypoint: received request");
								tokio::task::spawn(async move {
									match hyper::upgrade::on(req).await {
										Ok(upgraded) => {
											let mut io = TokioIo::new(upgraded);
											io.write_all(&waypoint_message[..]).await.unwrap();
											handle_stream(mode, &mut io).await;
										},
										Err(e) => error!("No upgrade {e}"),
									}
								});
								Ok::<_, Infallible>(Response::new(Full::<Bytes>::from("streaming...")))
							}
						}),
					)
					.await
				{
					error!("Error serving connection: {:?}", err);
				}
			});
		}
	}
}

async fn handle_stream<IO>(mode: Mode, rw: &mut IO)
where
	IO: AsyncRead + AsyncWrite + Unpin,
{
	match mode {
		Mode::ReadWrite => {
			let (r, mut w) = tokio::io::split(rw);
			let mut r = tokio::io::BufReader::with_capacity(BUFFER_SIZE, r);
			tokio::io::copy_buf(&mut r, &mut w).await.expect("tcp copy");
		},
		Mode::ReadDoubleWrite => {
			let (mut r, mut w) = tokio::io::split(rw);
			let mut buffer = vec![0; BUFFER_SIZE];
			loop {
				let read = r.read(&mut buffer).await.expect("tcp ready");
				if read == 0 {
					break;
				}
				let wrote = w.write(&buffer[..read]).await.expect("tcp ready");
				if wrote == 0 {
					break;
				}
				let wrote = w.write(&buffer[..read]).await.expect("tcp ready");
				if wrote == 0 {
					break;
				}
			}
		},
	}
}

fn generate_test_certs(name: &str) -> rustls::ServerConfig {
	use openssl::asn1::Asn1Time;
	use openssl::bn::BigNum;
	use openssl::hash::MessageDigest;
	use openssl::nid::Nid;
	use openssl::pkey::PKey;
	use openssl::rsa::Rsa;
	use openssl::x509::X509Builder;
	use openssl::x509::extension::{KeyUsage, SubjectAlternativeName};
	use rustls::pki_types::{CertificateDer, PrivateKeyDer};

	let shared_ca = super::shared_ca::get_shared_ca();
	let ca_key = &shared_ca.ca_key;
	let ca_cert = &shared_ca.ca_cert;
	let server_key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
	let mut cert_builder = X509Builder::new().unwrap();
	cert_builder.set_version(2).unwrap();

	let serial = {
		let mut s = BigNum::new().unwrap();
		s.rand(159, openssl::bn::MsbOption::MAYBE_ZERO, false)
			.unwrap();
		s.to_asn1_integer().unwrap()
	};
	cert_builder.set_serial_number(&serial).unwrap();

	let mut subject = openssl::x509::X509NameBuilder::new().unwrap();
	subject.append_entry_by_nid(Nid::COMMONNAME, name).unwrap();
	subject
		.append_entry_by_nid(Nid::ORGANIZATIONNAME, "cluster.local")
		.unwrap();
	let subject = subject.build();

	cert_builder.set_subject_name(&subject).unwrap();
	cert_builder
		.set_issuer_name(ca_cert.subject_name())
		.unwrap();
	cert_builder
		.set_not_before(&Asn1Time::days_from_now(0).unwrap())
		.unwrap();
	cert_builder
		.set_not_after(&Asn1Time::days_from_now(365).unwrap())
		.unwrap();
	cert_builder.set_pubkey(&server_key).unwrap();

	let spiffe_id = format!("spiffe://cluster.local/ns/default/sa/{}", name);
	let san = SubjectAlternativeName::new()
		.uri(&spiffe_id)
		.build(&cert_builder.x509v3_context(Some(ca_cert), None))
		.unwrap();
	cert_builder.append_extension(san).unwrap();

	let key_usage = KeyUsage::new()
		.digital_signature()
		.key_encipherment()
		.build()
		.unwrap();
	cert_builder.append_extension(key_usage).unwrap();

	cert_builder.sign(ca_key, MessageDigest::sha256()).unwrap();
	let server_cert = cert_builder.build();

	let cert_der = CertificateDer::from(server_cert.to_der().unwrap());
	let key_der = PrivateKeyDer::try_from(server_key.private_key_to_der().unwrap()).unwrap();
	let ca_der = CertificateDer::from(ca_cert.to_der().unwrap());
	let mut root_store = rustls::RootCertStore::empty();
	root_store.add(ca_der).unwrap();

	let client_verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(root_store))
		.build()
		.unwrap();

	let mut config = rustls::ServerConfig::builder()
		.with_client_cert_verifier(client_verifier)
		.with_single_cert(vec![cert_der], key_der)
		.unwrap();

	config.alpn_protocols = vec![b"h2".to_vec()];
	config
}

fn create_tls_acceptor(config: rustls::ServerConfig) -> tokio_rustls::TlsAcceptor {
	tokio_rustls::TlsAcceptor::from(Arc::new(config))
}
