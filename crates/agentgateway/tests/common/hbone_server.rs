// Test helper for running an HBONE server that echoes data with a waypoint prefix
// Based on ztunnel's test server implementation:
// https://github.com/istio/ztunnel/blob/master/src/test_helpers/tcp.rs

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::Full;
use hyper::Response;
use hyper::server::conn::http2;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use rand::Rng;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{debug, error, info};

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum Mode {
	ReadDoubleWrite,
	ReadWrite,
	Forward(SocketAddr), // Forward connections to another HBONE server
}

static BUFFER_SIZE: usize = 2 * 1024 * 1024;

/// HBONE test server that accepts mTLS connections and echoes data with a prefix
pub struct HboneTestServer {
	listener: TcpListener,
	mode: Mode,
	name: String,
	waypoint_message: Vec<u8>, // Prefix to write before echoing data
	port: u16,                 // The actual bound port
}

impl HboneTestServer {
	/// Creates a new HBONE test server. If port is 0, the OS will assign an available port.
	pub async fn new(mode: Mode, name: &str, waypoint_message: Vec<u8>, port: u16) -> Self {
		let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

		let addr = SocketAddr::from(([127, 0, 0, 1], port));
		let listener = TcpListener::bind(addr).await.unwrap();
		let actual_port = listener.local_addr().unwrap().port();

		Self {
			listener,
			mode,
			name: name.to_string(),
			waypoint_message,
			port: actual_port,
		}
	}

	/// Returns the port this server is bound to
	pub fn port(&self) -> u16 {
		self.port
	}

	pub async fn run(self) {
		let certs = generate_test_certs(&self.name);
		let acceptor = create_tls_acceptor(certs);

		// Track consecutive TLS errors to detect persistent configuration issues
		const MAX_CONSECUTIVE_ERRORS: usize = 10;
		let mut consecutive_errors = 0;

		loop {
			let (tcp_stream, _) = self.listener.accept().await.unwrap();
			let tls_stream = match acceptor.accept(tcp_stream).await {
				Ok(stream) => {
					// Reset error counter on successful connection
					consecutive_errors = 0;
					stream
				},
				Err(e) => {
					consecutive_errors += 1;
					// Log as debug since transient TLS errors are expected during test startup
					// when the client is still fetching certificates
					debug!(
						"TLS accept error (likely transient during startup): {:?}",
						e
					);

					if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
						panic!(
							"Test server '{}' failed with {} consecutive TLS errors. \
							This indicates a persistent TLS configuration issue, not transient startup errors.",
							self.name, consecutive_errors
						);
					}
					continue;
				},
			};

			let mode = self.mode.clone();
			let waypoint_message = self.waypoint_message.clone();
			let name = self.name.clone();

			tokio::spawn(async move {
				if let Err(err) = http2::Builder::new(hyper_util::rt::TokioExecutor::new())
					.serve_connection(
						TokioIo::new(tls_stream),
						service_fn(move |req| {
							let waypoint_message = waypoint_message.clone();
							let mode = mode.clone();
							let name = name.clone();
							async move {
								info!("{}: received request", name);
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
		Mode::Forward(forward_addr) => {
			// Connect to the target HBONE server
			let mut target_stream = tokio::net::TcpStream::connect(forward_addr)
				.await
				.expect("Failed to connect to forward target");

			// Bidirectionally copy data between the two streams
			// Use tokio::io::copy_bidirectional for proper full-duplex forwarding
			if let Err(e) = tokio::io::copy_bidirectional(rw, &mut target_stream).await {
				// Connection closed errors are expected during test cleanup
				debug!("Connection closed during forward: {:?}", e);
			}
		},
	}
}

fn generate_test_certs(name: &str) -> rustls::ServerConfig {
	// Generate certificates using rcgen with static test keys
	use std::time::{Duration, SystemTime};

	use rcgen::{
		CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, Issuer, KeyPair,
		KeyUsagePurpose, SanType, SerialNumber,
	};
	use rustls::pki_types::CertificateDer;

	// Generate random serial number (159 bits)
	let serial_number = {
		let mut data = [0u8; 20];
		rand::rng().fill_bytes(&mut data);
		data[0] &= 0x7f;
		data
	};

	// Set up certificate parameters
	let mut params = CertificateParams::default();
	params.not_before = SystemTime::now().into();
	params.not_after = (SystemTime::now() + Duration::from_secs(365 * 24 * 60 * 60)).into();
	params.serial_number = Some(SerialNumber::from_slice(&serial_number));

	let mut dn = DistinguishedName::new();
	dn.push(DnType::OrganizationName, "cluster.local");
	params.distinguished_name = dn;

	params.key_usages = vec![
		KeyUsagePurpose::DigitalSignature,
		KeyUsagePurpose::KeyEncipherment,
	];
	params.extended_key_usages = vec![
		ExtendedKeyUsagePurpose::ServerAuth,
		ExtendedKeyUsagePurpose::ClientAuth,
	];

	// Set SPIFFE ID as SAN
	let spiffe_id = format!("spiffe://cluster.local/ns/default/sa/{}", name);
	params.subject_alt_names = vec![SanType::URI(spiffe_id.try_into().unwrap())];

	// Use static test key for consistency
	let kp = KeyPair::from_pem(std::str::from_utf8(super::shared_ca::TEST_PKEY).unwrap()).unwrap();
	let key_pem = kp.serialize_pem();

	// Load CA key
	let ca_kp =
		KeyPair::from_pem(std::str::from_utf8(super::shared_ca::TEST_ROOT_KEY).unwrap()).unwrap();

	// Sign certificate with CA
	let issuer = Issuer::from_params(&params, &ca_kp);
	let server_cert = params.signed_by(&kp, &issuer).unwrap();

	// Convert to DER for rustls
	let cert_der = CertificateDer::from(server_cert.der().to_vec());
	let key_der = rustls_pemfile::private_key(&mut key_pem.as_bytes())
		.unwrap()
		.unwrap();

	// Load CA cert for trust store
	let mut root_cursor = std::io::Cursor::new(super::shared_ca::TEST_ROOT);
	let ca_der = rustls_pemfile::certs(&mut root_cursor)
		.next()
		.unwrap()
		.unwrap();

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
