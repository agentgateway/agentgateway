use std::cmp;
use std::io::Cursor;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ::http::Uri;
use rustls::client::Resumption;
use rustls::server::VerifierBuilderError;
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use rustls_pemfile::Item;
use rustls_pki_types::PrivateKeyDer;
use tokio::sync::{Mutex, RwLock, watch};
use tokio::time::sleep_until;
use tonic::IntoRequest;
use tonic::body::Body;
use tonic::metadata::{AsciiMetadataKey, AsciiMetadataValue};
use tracing::{debug, error, info, warn};
use x509_parser::certificate::X509Certificate;

use crate::types::discovery::Identity;
use crate::*;

// Generated from proto/citadel.proto
pub mod istio {
	pub mod ca {
		tonic::include_proto!("istio.v1.auth");
	}
}

use istio::ca::istio_certificate_service_client::IstioCertificateServiceClient;
use istio::ca::{IstioCertificateRequest, IstioCertificateResponse};

use crate::control::{AuthSource, RootCert};
use crate::http::backendtls::BackendTLS;

#[derive(Clone, Debug, thiserror::Error)]
pub enum Error {
	#[error("CA client error: {0}")]
	CaClient(#[from] Box<tonic::Status>),
	#[error("CA client creation: {0}")]
	CaClientCreation(Arc<anyhow::Error>),
	#[error("Empty certificate response")]
	EmptyResponse,
	#[error("invalid csr: {0}")]
	Csr(Arc<anyhow::Error>),
	#[error("invalid root certificate: {0}")]
	InvalidRootCert(String),
	#[error("certificate: {0}")]
	CertificateParse(String),
	#[error("rustls: {0}")]
	Rustls(#[from] rustls::Error),
	#[error("rustls verifier: {0}")]
	Verifier(#[from] VerifierBuilderError),

	#[error("Certificate SAN mismatch: expected {expected}, got {actual}")]
	SanMismatch { expected: String, actual: String },
	#[error("Certificate expired")]
	Expired,
	#[error("Certificate not ready")]
	NotReady,
}

#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Config {
	pub address: String,
	#[serde(with = "serde_dur")]
	pub secret_ttl: Duration,
	pub identity: Identity,
	pub auth: AuthSource,
	pub ca_cert: RootCert,
}

#[derive(Clone, Debug)]
pub struct Expiration {
	pub not_before: SystemTime,
	pub not_after: SystemTime,
}

#[derive(Debug)]
pub struct WorkloadCertificate {
	// server_config: Arc<ServerConfig>,
	// client_config: Arc<ClientConfig>,
	roots: Arc<RootCertStore>,
	chain: Vec<Certificate>,
	private_key: PrivateKeyDer<'static>,
	expiry: Expiration,
	identity: Identity,
}

impl WorkloadCertificate {
	fn new(key: &[u8], cert: &[u8], chain: Vec<&[u8]>) -> Result<WorkloadCertificate, Error> {
		let cert = parse_cert(cert.to_vec())?;
		let mut roots_store = RootCertStore::empty();
		let identity = cert
			.identity
			.clone()
			.ok_or_else(|| Error::CertificateParse("to identity found".into()))?;
		let expiry = cert.expiry.clone();

		// The Istio API does something pretty unhelpful, by providing a single chain of certs.
		// The last one is the root. However, there may be multiple roots concatenated in that last cert,
		// so we will need to split them.
		let Some(raw_root) = chain.last() else {
			return Err(Error::InvalidRootCert(
				"no root certificate present".to_string(),
			));
		};
		let key: PrivateKeyDer = parse_key(key)?;
		let roots = parse_cert_multi(raw_root)?;
		let (_valid, invalid) =
			roots_store.add_parsable_certificates(roots.iter().map(|c| c.der.clone()));
		if invalid > 0 {
			tracing::warn!("warning: found {invalid} invalid root certs");
		}
		let mut cert_and_chain = vec![cert];
		let chains = chain[..cmp::max(0, chain.len() - 1)]
			.iter()
			.map(|x| x.to_vec())
			.map(parse_cert)
			.collect::<Result<Vec<_>, _>>()?;
		for c in chains {
			cert_and_chain.push(c);
		}

		Ok(WorkloadCertificate {
			roots: Arc::new(roots_store),
			expiry,
			private_key: key,
			chain: cert_and_chain,
			identity,
		})
	}
	pub fn is_expired(&self) -> bool {
		SystemTime::now() > self.expiry.not_after
	}

	pub fn refresh_at(&self) -> SystemTime {
		let expiry = &self.expiry;
		match expiry.not_after.duration_since(expiry.not_before) {
			Ok(valid_for) => expiry.not_before + valid_for / 2,
			Err(_) => expiry.not_after,
		}
	}

	pub fn legacy_mtls(&self, identity: Vec<Identity>) -> Result<BackendTLS, Error> {
		// TODO: this is (way) too expensive to build per request
		let roots = self.roots.clone();
		let verifier = transport::tls::identity::IdentityVerifier { roots, identity };
		let mut cc = ClientConfig::builder_with_provider(transport::tls::provider())
			.with_protocol_versions(transport::tls::ALL_TLS_VERSIONS)
			.expect("client config must be valid")
			.dangerous() // Customer verifier is requires "dangerous" opt-in
			.with_custom_certificate_verifier(Arc::new(verifier))
			.with_client_auth_cert(
				self.chain.iter().map(|c| c.der.clone()).collect(),
				self.private_key.clone_key(),
			)?;
		cc.alpn_protocols = vec![b"istio".into()];
		cc.resumption = Resumption::disabled();
		// cc.enable_sni = false;
		Ok(BackendTLS {
			config: Arc::new(cc),
		})
	}
	pub fn hbone_mtls(&self, identity: Vec<Identity>) -> Result<BackendTLS, Error> {
		// TODO: this is (way) too expensive to build per request
		let roots = self.roots.clone();
		let verifier = transport::tls::identity::IdentityVerifier { roots, identity };
		let mut cc = ClientConfig::builder_with_provider(transport::tls::provider())
			.with_protocol_versions(transport::tls::ALL_TLS_VERSIONS)
			.expect("client config must be valid")
			.dangerous() // Customer verifier is requires "dangerous" opt-in
			.with_custom_certificate_verifier(Arc::new(verifier))
			.with_client_auth_cert(
				self.chain.iter().map(|c| c.der.clone()).collect(),
				self.private_key.clone_key(),
			)?;
		cc.alpn_protocols = vec![b"h2".into()];
		cc.resumption = Resumption::disabled();
		cc.enable_sni = false;
		Ok(BackendTLS {
			config: Arc::new(cc),
		})
	}
	pub fn hbone_termination(&self) -> Result<ServerConfig, Error> {
		let Identity::Spiffe { trust_domain, .. } = &self.identity;

		// TODO: this istoo expensive to build per request
		let roots = self.roots.clone();
		let raw_client_cert_verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
			roots.clone(),
			transport::tls::provider(),
		)
		.build()?;
		let client_cert_verifier = transport::tls::trustdomain::TrustDomainVerifier::new(
			raw_client_cert_verifier,
			Some(trust_domain.clone()),
		);
		let mut sc = ServerConfig::builder_with_provider(transport::tls::provider())
			.with_protocol_versions(transport::tls::ALL_TLS_VERSIONS)
			.expect("server config must be valid")
			.with_client_cert_verifier(client_cert_verifier)
			.with_single_cert(
				self.chain.iter().map(|c| c.der.clone()).collect(),
				self.private_key.clone_key(),
			)?;
		Ok(sc)
	}
}

#[derive(Clone, Debug)]
struct Certificate {
	expiry: Expiration,
	identity: Option<Identity>,
	der: rustls_pki_types::CertificateDer<'static>,
}

fn parse_key(mut key: &[u8]) -> Result<PrivateKeyDer<'static>, Error> {
	let mut reader = std::io::BufReader::new(Cursor::new(&mut key));
	let parsed = rustls_pemfile::read_one(&mut reader)
		.map_err(|e| Error::CertificateParse(e.to_string()))?
		.ok_or_else(|| Error::CertificateParse("no key".to_string()))?;
	match parsed {
		Item::Pkcs8Key(c) => Ok(PrivateKeyDer::Pkcs8(c)),
		Item::Sec1Key(c) => Ok(PrivateKeyDer::Sec1(c)),
		_ => Err(Error::CertificateParse("no key".to_string())),
	}
}

fn parse_cert(mut cert: Vec<u8>) -> Result<Certificate, Error> {
	let mut reader = std::io::BufReader::new(Cursor::new(&mut cert));
	let parsed = rustls_pemfile::read_one(&mut reader)
		.map_err(|e| Error::CertificateParse(e.to_string()))?
		.ok_or_else(|| Error::CertificateParse("no certificate".to_string()))?;
	let Item::X509Certificate(der) = parsed else {
		return Err(Error::CertificateParse("no certificate".to_string()));
	};

	let (_, cert) = x509_parser::parse_x509_certificate(&der)
		.map_err(|e| Error::CertificateParse(e.to_string()))?;
	Ok(Certificate {
		der: der.clone(),
		expiry: expiration(cert.clone()),
		identity: identity(cert),
	})
}

fn parse_cert_multi(mut cert: &[u8]) -> Result<Vec<Certificate>, Error> {
	let mut reader = std::io::BufReader::new(Cursor::new(&mut cert));
	let parsed: Result<Vec<_>, _> = rustls_pemfile::read_all(&mut reader).collect();
	parsed
		.map_err(|e| Error::CertificateParse(e.to_string()))?
		.into_iter()
		.map(|p| {
			let Item::X509Certificate(der) = p else {
				return Err(Error::CertificateParse("no certificate".to_string()));
			};
			let (_, cert) = x509_parser::parse_x509_certificate(&der)
				.map_err(|e| Error::CertificateParse(e.to_string()))?;
			Ok(Certificate {
				der: der.clone(),
				expiry: expiration(cert),
				identity: None,
			})
		})
		.collect()
}

fn identity(cert: X509Certificate) -> Option<Identity> {
	cert
		.subject_alternative_name()
		.ok()
		.flatten()
		.and_then(|ext| {
			ext
				.value
				.general_names
				.iter()
				.filter_map(|n| match n {
					x509_parser::extensions::GeneralName::URI(uri) => Some(uri),
					_ => None,
				})
				.next()
		})
		.and_then(|san| Identity::from_str(san).ok())
}

fn expiration(cert: X509Certificate) -> Expiration {
	Expiration {
		not_before: UNIX_EPOCH
			+ Duration::from_secs(
				cert
					.validity
					.not_before
					.timestamp()
					.try_into()
					.unwrap_or_default(),
			),
		not_after: UNIX_EPOCH
			+ Duration::from_secs(
				cert
					.validity
					.not_after
					.timestamp()
					.try_into()
					.unwrap_or_default(),
			),
	}
}

#[derive(Debug, Clone, Default)]
enum CertificateState {
	#[default]
	NotReady,
	Available(Arc<WorkloadCertificate>),
	Error(Error),
}

#[derive(Debug)]
pub struct CaClient {
	config: Config,
	state: watch::Receiver<CertificateState>,
	_fetcher_handle: tokio::task::JoinHandle<()>,
}

impl CaClient {
	pub fn new(client: client::Client, config: Config) -> Result<Self, Error> {
		let (state_tx, state_rx) = watch::channel(CertificateState::NotReady);

		// Start the fetcher task
		let fetcher_handle = tokio::spawn({
			let config = config.clone();
			let state_tx = state_tx.clone();

			async move {
				Self::run_fetcher(client, config, state_tx).await;
			}
		});

		Ok(Self {
			config,
			state: state_rx,
			_fetcher_handle: fetcher_handle,
		})
	}

	/// Get the latest certificate. If no certificate is available, one will be requested.
	/// After the first call, this will return the cached certificate without blocking.
	pub async fn get_identity(&self) -> Result<Arc<WorkloadCertificate>, Error> {
		loop {
			let mut rx = self.state.clone();
			let state = rx.borrow_and_update().clone();
			match state {
				CertificateState::Available(cert) => {
					if !cert.is_expired() {
						return Ok(cert);
					} else {
						return Err(Error::Expired);
					}
				},
				CertificateState::Error(err) => {
					return Err(err);
				},
				CertificateState::NotReady => {
					// Wait for the state to change
					if rx.changed().await.is_err() {
						return Err(Error::NotReady);
					}
				},
			}
		}
	}

	async fn run_fetcher(
		client: client::Client,
		config: Config,
		state_tx: watch::Sender<CertificateState>,
	) {
		let mut interval = tokio::time::interval(Duration::from_secs(30)); // Check every 30 seconds

		// Start with an immediate fetch
		if let Err(e) = Self::fetch_and_update_certificate(client.clone(), &config, &state_tx).await {
			error!("Initial certificate fetch failed: {:?}", e);
			let _ = state_tx.send(CertificateState::Error(e));
		}

		loop {
			interval.tick().await;

			// Check if we need to renew
			let should_renew = {
				let state = state_tx.borrow();
				match &*state {
					CertificateState::Available(cert) => {
						let refresh_at = cert.refresh_at();
						SystemTime::now() >= refresh_at
					},
					CertificateState::Error(_) | CertificateState::NotReady => true,
				}
			};

			if should_renew {
				info!("Renewing certificate for identity: {}", config.identity);

				match Self::fetch_and_update_certificate(client.clone(), &config, &state_tx).await {
					Ok(_) => {
						info!(
							"Successfully renewed certificate for identity: {}",
							config.identity
						);
					},
					Err(e) => {
						error!(
							"Failed to renew certificate for identity {}: {}",
							config.identity, e
						);
						let _ = state_tx.send(CertificateState::Error(e));
					},
				}
			}
		}
	}

	async fn fetch_and_update_certificate(
		client: client::Client,
		config: &Config,
		state_tx: &watch::Sender<CertificateState>,
	) -> Result<(), Error> {
		info!("Fetching certificate for identity: {}", config.identity);

		let svc = control::grpc_connector(
			client,
			config.address.clone(),
			config.auth.clone(),
			config.ca_cert.clone(),
		)
		.await
		.map_err(|e| Error::CaClientCreation(Arc::new(e)))?;
		let mut client = IstioCertificateServiceClient::new(svc);

		// Generate CSR
		let csr_options = csr::CsrOptions {
			san: config.identity.to_string(),
		};
		let csr = csr_options
			.generate()
			.map_err(|e| Error::Csr(Arc::new(e)))?;
		let private_key = csr.private_key;

		// Create request
		let mut request = tonic::Request::new(IstioCertificateRequest {
			csr: csr.csr,
			validity_duration: config.secret_ttl.as_secs() as i64,
			metadata: None, // We don't need impersonation for single cert
		});

		// Make the request
		let response = client
			.create_certificate(request.into_request())
			.await
			.map_err(|e| Error::CaClient(Box::new(e)))?;

		let response = response.into_inner();

		// Parse the certificate chain
		let cert_chain = response.cert_chain;
		if cert_chain.is_empty() {
			return Err(Error::EmptyResponse);
		}

		let leaf_cert = cert_chain[0].as_bytes();
		let chain_certs = if cert_chain.len() > 1 {
			cert_chain[1..].iter().map(|s| s.as_bytes()).collect()
		} else {
			warn!("No chain certificates for: {}", config.identity);
			vec![]
		};

		// Create the workload certificate
		let cert = Arc::new(WorkloadCertificate::new(
			&private_key,
			leaf_cert,
			chain_certs,
		)?);

		// Verify the certificate matches our identity
		if cert.identity != config.identity {
			return Err(Error::SanMismatch {
				expected: config.identity.to_string(),
				actual: cert.identity.to_string(),
			});
		}

		// Update state
		let _ = state_tx.send(CertificateState::Available(cert));

		info!(
			"Successfully fetched certificate for identity: {}",
			config.identity
		);
		Ok(())
	}
}

impl Drop for CaClient {
	fn drop(&mut self) {
		self._fetcher_handle.abort()
	}
}

mod csr {

	pub struct CertSign {
		pub csr: String,
		pub private_key: Vec<u8>,
	}

	pub struct CsrOptions {
		pub san: String,
	}

	impl CsrOptions {
		pub fn generate(&self) -> anyhow::Result<CertSign> {
			use rcgen::{CertificateParams, DistinguishedName, SanType};
			let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?;
			let private_key = kp.serialize_pem();
			let mut params = CertificateParams::default();
			params.subject_alt_names = vec![SanType::URI(self.san.clone().try_into()?)];
			params.key_identifier_method = rcgen::KeyIdMethod::Sha256;
			// Avoid setting CN. rcgen defaults it to "rcgen self signed cert" which we don't want
			params.distinguished_name = DistinguishedName::new();
			let csr = params.serialize_request(&kp)?.pem()?;

			Ok(CertSign {
				csr,
				private_key: private_key.into(),
			})
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_parse_key_ec_private() {
		let ec_key = b"-----BEGIN EC PRIVATE KEY-----
MHcCAQEEIGfhD3tZlZOmw7LfyyERnPCyOnzmqiy1VcwiK36ro1H5oAoGCCqGSM49
AwEHoUQDQgAEwWSdCtU7tQGYtpNpJXSB5VN4yT1lRXzHh8UOgWWqiYXX1WYHk8vf
63XQuFFo4YbnXLIPdRxfxk9HzwyPw8jW8Q==
-----END EC PRIVATE KEY-----";

		let result = parse_key(ec_key);
		assert!(result.is_ok());

		let key = result.unwrap();
		match key {
			PrivateKeyDer::Sec1(_) => {}, // Expected for EC private keys
			_ => panic!("Expected SEC1 (EC) private key format"),
		}
	}

	#[test]
	fn test_parse_key_pkcs8_ec() {
		// PKCS8 wrapped EC key should also work
		let pkcs8_ec_key = b"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg7oRJ3/tWjzNRdSXj
k2kj5FhI/GKfGpvAJbDe6A4VlzuhRANCAASTGTFE0FdYwKqcaUEZ3VhqKlpZLjY/
SGjfUH8wjCgRLFmKGfZSFZFh1xN9M5Bq6v1P6kNqW7nM7oA4VJWqKp5W
-----END PRIVATE KEY-----";

		let result = parse_key(pkcs8_ec_key);
		assert!(result.is_ok());

		let key = result.unwrap();
		match key {
			PrivateKeyDer::Pkcs8(_) => {}, // Expected for PKCS8 format
			_ => panic!("Expected PKCS8 private key format"),
		}
	}

	#[test]
	fn test_parse_key_unsupported() {
		let unsupported_key = b"-----BEGIN CERTIFICATE-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA4f6wg4PvmdHJzX...
-----END CERTIFICATE-----";

		let result = parse_key(unsupported_key);
		assert!(result.is_err());
		// Just verify it fails - the actual error message depends on the input format
		let _error = result.unwrap_err();
	}
}
