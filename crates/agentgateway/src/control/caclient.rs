use std::cmp;
use std::collections::HashMap;
use std::io::Cursor;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rustls::client::Resumption;
use rustls::server::VerifierBuilderError;
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use rustls_pemfile::Item;
use rustls_pki_types::PrivateKeyDer;
use tokio::sync::watch;
use tonic::IntoRequest;
use tracing::{error, info, warn};
use x509_parser::certificate::X509Certificate;

use crate::types::discovery::Identity;
use crate::*;

// Generated from proto/citadel.proto
pub mod istio {
	pub mod ca {
		tonic::include_proto!("istio.v1.auth");
	}
}

use istio::ca::IstioCertificateRequest;
use istio::ca::istio_certificate_service_client::IstioCertificateServiceClient;

use crate::control::{AuthSource, RootCert};
use crate::http::backendtls::VersionedBackendTLS;

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

pub struct WorkloadCertificate {
	roots: Arc<RootCertStore>,
	chain: Vec<Certificate>,
	private_key: PrivateKeyDer<'static>,
	expiry: Expiration,
	identity: Identity,
	/// Cache for outbound mTLS client configs, keyed by sorted destination identities.
	/// This ensures connection pooling works correctly (pool keys use Arc pointer equality).
	legacy_mtls_cache: RwLock<HashMap<Vec<Identity>, VersionedBackendTLS>>,
	hbone_mtls_cache: RwLock<HashMap<Vec<Identity>, VersionedBackendTLS>>,
}

impl std::fmt::Debug for WorkloadCertificate {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("WorkloadCertificate")
			.field("identity", &self.identity)
			.field("expiry", &self.expiry)
			.finish_non_exhaustive()
	}
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
			legacy_mtls_cache: RwLock::new(HashMap::new()),
			hbone_mtls_cache: RwLock::new(HashMap::new()),
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

	pub fn legacy_mtls(&self, identity: Vec<Identity>) -> Result<VersionedBackendTLS, Error> {
		// Normalize identity order for consistent cache keys
		let mut cache_key = identity;
		cache_key.sort();

		// Check cache with read lock first (fast path)
		{
			let reader = self.legacy_mtls_cache.read().unwrap();
			if let Some(cached) = reader.get(&cache_key) {
				return Ok(cached.clone());
			}
		}

		// Acquire write lock and double-check (another thread may have inserted)
		let mut writer = self.legacy_mtls_cache.write().unwrap();
		if let Some(cached) = writer.get(&cache_key) {
			return Ok(cached.clone());
		}

		// Build new config while holding write lock to prevent duplicate builds
		let roots = self.roots.clone();
		let verifier = transport::tls::identity::IdentityVerifier {
			roots,
			identity: cache_key.clone(),
		};
		let mut cc = ClientConfig::builder_with_provider(transport::tls::provider())
			.with_protocol_versions(transport::tls::ALL_TLS_VERSIONS)
			.expect("client config must be valid")
			.dangerous() // Custom verifier requires "dangerous" opt-in
			.with_custom_certificate_verifier(Arc::new(verifier))
			.with_client_auth_cert(
				self.chain.iter().map(|c| c.der.clone()).collect(),
				self.private_key.clone_key(),
			)?;
		cc.alpn_protocols = vec![b"istio".into()];
		cc.resumption = Resumption::disabled();

		let result = VersionedBackendTLS {
			hostname_override: None,
			config: Arc::new(cc),
		};
		writer.insert(cache_key, result.clone());

		Ok(result)
	}
	pub fn hbone_mtls(&self, identity: Vec<Identity>) -> Result<VersionedBackendTLS, Error> {
		// Normalize identity order for consistent cache keys
		let mut cache_key = identity;
		cache_key.sort();

		// Check cache with read lock first (fast path)
		{
			let reader = self.hbone_mtls_cache.read().unwrap();
			if let Some(cached) = reader.get(&cache_key) {
				return Ok(cached.clone());
			}
		}

		// Acquire write lock and double-check (another thread may have inserted)
		let mut writer = self.hbone_mtls_cache.write().unwrap();
		if let Some(cached) = writer.get(&cache_key) {
			return Ok(cached.clone());
		}

		// Build new config while holding write lock to prevent duplicate builds
		let roots = self.roots.clone();
		let verifier = transport::tls::identity::IdentityVerifier {
			roots,
			identity: cache_key.clone(),
		};
		let mut cc = ClientConfig::builder_with_provider(transport::tls::provider())
			.with_protocol_versions(transport::tls::ALL_TLS_VERSIONS)
			.expect("client config must be valid")
			.dangerous() // Custom verifier requires "dangerous" opt-in
			.with_custom_certificate_verifier(Arc::new(verifier))
			.with_client_auth_cert(
				self.chain.iter().map(|c| c.der.clone()).collect(),
				self.private_key.clone_key(),
			)?;
		cc.alpn_protocols = vec![b"h2".into()];
		cc.resumption = Resumption::disabled();
		cc.enable_sni = false;

		let result = VersionedBackendTLS {
			hostname_override: None,
			config: Arc::new(cc),
		};
		writer.insert(cache_key, result.clone());

		Ok(result)
	}
	/// Create a TLS ServerConfig for terminating HBONE (ambient mesh) connections.
	pub fn hbone_termination(&self) -> Result<ServerConfig, Error> {
		self.mtls_server_config(None)
	}

	/// Create a TLS ServerConfig for HTTPS listeners using workload identity mTLS.
	///
	/// ALPN protocols:
	/// - `istio`: For clients with Istio sidecars (mesh mTLS)
	/// - `h2`, `http/1.1`: For direct HTTP clients without sidecars
	pub fn https_mtls_termination(&self) -> Result<ServerConfig, Error> {
		const HTTPS_ALPN: &[&[u8]] = &[b"istio", b"h2", b"http/1.1"];
		self.mtls_server_config(Some(HTTPS_ALPN))
	}

	fn mtls_server_config(&self, alpn: Option<&[&[u8]]>) -> Result<ServerConfig, Error> {
		let Identity::Spiffe { trust_domain, .. } = &self.identity;

		let raw_verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
			self.roots.clone(),
			transport::tls::provider(),
		)
		.build()?;

		let verifier = transport::tls::trustdomain::TrustDomainVerifier::new(
			raw_verifier,
			Some(trust_domain.clone()),
		);

		let mut config = ServerConfig::builder_with_provider(transport::tls::provider())
			.with_protocol_versions(transport::tls::ALL_TLS_VERSIONS)
			.expect("server config must be valid")
			.with_client_cert_verifier(verifier)
			.with_single_cert(
				self.chain.iter().map(|c| c.der.clone()).collect(),
				self.private_key.clone_key(),
			)?;

		if let Some(protocols) = alpn {
			config.alpn_protocols = protocols.iter().map(|p| p.to_vec()).collect();
		}

		Ok(config)
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

/// Initial backoff delay after a failed certificate fetch.
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
/// Maximum backoff delay between retry attempts.
const MAX_BACKOFF: Duration = Duration::from_secs(120);
/// How often to check if refresh is needed when we have a valid certificate.
const CHECK_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Default)]
enum CertificateState {
	#[default]
	NotReady,
	Available(Arc<WorkloadCertificate>),
	Error(Error),
}

#[derive(Debug)]
pub struct CaClient {
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
		let mut backoff = INITIAL_BACKOFF;
		let mut next_attempt = Instant::now();

		loop {
			// Sleep until next attempt time
			tokio::time::sleep_until(next_attempt.into()).await;

			// Check current state to determine what to do
			let (should_fetch, valid_cert_expiry) = {
				let state = state_tx.borrow();
				match &*state {
					CertificateState::Available(cert) => {
						let needs_refresh = SystemTime::now() >= cert.refresh_at();
						let expiry = if cert.is_expired() {
							None
						} else {
							Some(cert.expiry.not_after)
						};
						(needs_refresh, expiry)
					},
					CertificateState::Error(_) | CertificateState::NotReady => (true, None),
				}
			};

			if !should_fetch {
				// Certificate is valid and doesn't need refresh yet, check again later
				next_attempt = Instant::now() + CHECK_INTERVAL;
				continue;
			}

			info!("Fetching certificate for identity: {}", config.identity);

			match Self::fetch_and_update_certificate(client.clone(), &config, &state_tx).await {
				Ok(_) => {
					info!(
						"Successfully fetched certificate for identity: {}",
						config.identity
					);
					// Reset backoff on success
					backoff = INITIAL_BACKOFF;
					// Schedule next check based on normal interval
					next_attempt = Instant::now() + CHECK_INTERVAL;
				},
				Err(e) => {
					// Calculate retry delay, capping at cert expiry if we have a valid cert
					let retry_delay = match valid_cert_expiry {
						Some(expiry) => {
							// Cap retry at cert expiry to maximize renewal attempts
							let until_expiry = expiry
								.duration_since(SystemTime::now())
								.unwrap_or(Duration::ZERO);
							cmp::min(backoff, until_expiry)
						},
						None => backoff,
					};

					if valid_cert_expiry.is_some() {
						// We still have a valid certificate - keep using it, retry with backoff
						warn!(
							"Certificate refresh failed for {}, retaining valid certificate: {}. Retrying in {:?}",
							config.identity, e, retry_delay
						);
						// Don't update state - keep the valid certificate
					} else {
						// No valid certificate available - set error state
						error!(
							"Certificate fetch failed for {} with no valid fallback: {}. Retrying in {:?}",
							config.identity, e, retry_delay
						);
						let _ = state_tx.send(CertificateState::Error(e));
					}
					// Schedule retry
					next_attempt = Instant::now() + retry_delay;
					backoff = cmp::min(MAX_BACKOFF, backoff * 2);
				},
			}
		}
	}

	async fn fetch_and_update_certificate(
		client: client::Client,
		config: &Config,
		state_tx: &watch::Sender<CertificateState>,
	) -> Result<(), Error> {
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
		let request = tonic::Request::new(IstioCertificateRequest {
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
		#[cfg(feature = "testing")]
		let mut cert_chain = response.cert_chain;
		#[cfg(not(feature = "testing"))]
		let cert_chain = response.cert_chain;

		if cert_chain.is_empty() {
			return Err(Error::EmptyResponse);
		}

		// TEST ONLY: Mock CA returns the private key in the cert chain with a special marker
		// because rcgen doesn't support CSR parsing.
		// Detect the test marker and use the provided key. Real CAs never return private keys.
		// Only enabled when the "testing" feature is active (used by integration tests).
		#[cfg(feature = "testing")]
		let actual_private_key = {
			const TEST_CERT_MARKER: &str = "X-Test-Certificate-Key";
			if cert_chain.len() >= 2 && cert_chain[1].contains(TEST_CERT_MARKER) {
				// Extract the test private key (strip the marker comment line)
				let test_key_with_marker = cert_chain.remove(1);
				let key_pem = test_key_with_marker
					.lines()
					.skip(1) // Skip the marker line
					.collect::<Vec<_>>()
					.join("\n");
				key_pem.as_bytes().to_vec()
			} else {
				private_key
			}
		};

		#[cfg(not(feature = "testing"))]
		let actual_private_key = private_key;

		let leaf_cert = cert_chain[0].as_bytes();
		let chain_certs = if cert_chain.len() > 1 {
			cert_chain[1..].iter().map(|s| s.as_bytes()).collect()
		} else {
			warn!("No chain certificates for: {}", config.identity);
			vec![]
		};

		// Create the workload certificate
		let cert = Arc::new(WorkloadCertificate::new(
			&actual_private_key,
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
