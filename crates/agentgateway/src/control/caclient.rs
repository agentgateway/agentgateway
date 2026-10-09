use std::cmp;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::RwLock;
use rustls::client::Resumption;
use rustls::server::VerifierBuilderError;
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use rustls_pki_types::pem::{PemObject, SectionKind};
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::sync::watch;
use tonic::IntoRequest;
use tracing::{error, info, warn};
use x509_parser::certificate::X509Certificate;

use crate::types::discovery::Identity;
use crate::*;

/// Cache key for server TLS configs: (ALPN protocols, require client cert).
type ServerConfigCacheKey = (Vec<Vec<u8>>, bool);

// Generated from proto/citadel.proto
pub mod istio {
	pub mod ca {
		pub use protos::istio::v1::auth::*;
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
	#[serde(serialize_with = "crate::serdes::ser_sensitive_header_vec")]
	pub ca_headers: Vec<(String, String)>,
	pub allowed_trust_domains: Arc<[Strng]>,
	pub skip_validate_trust_domain: bool,
}

#[derive(Clone, Debug)]
pub struct Expiration {
	pub not_before: SystemTime,
	pub not_after: SystemTime,
}

#[derive(Debug)]
pub struct WorkloadCertificate {
	roots: Arc<RootCertStore>,
	chain: Vec<Certificate>,
	private_key: PrivateKeyDer<'static>,
	expiry: Expiration,
	identity: Identity,
	allowed_trust_domains: Arc<[Strng]>,
	skip_validate_trust_domain: bool,
	// Cache TLS configs to avoid expensive per-request rebuilds.
	// Keyed by the variable parameters (identity list, ALPNs, etc.).
	legacy_mtls_cache: RwLock<HashMap<Vec<Identity>, Arc<ClientConfig>>>,
	hbone_mtls_cache: RwLock<HashMap<Vec<Identity>, Arc<ClientConfig>>>,
	server_config_cache: RwLock<HashMap<ServerConfigCacheKey, Arc<ServerConfig>>>,
}

impl WorkloadCertificate {
	fn new(
		key: &[u8],
		cert: &[u8],
		chain: Vec<&[u8]>,
		allowed_trust_domains: Arc<[Strng]>,
		skip_validate_trust_domain: bool,
	) -> Result<WorkloadCertificate, Error> {
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
			allowed_trust_domains,
			skip_validate_trust_domain,
			legacy_mtls_cache: RwLock::new(HashMap::new()),
			hbone_mtls_cache: RwLock::new(HashMap::new()),
			server_config_cache: RwLock::new(HashMap::new()),
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
		// Check cache first
		if let Some(cached) = self.legacy_mtls_cache.read().get(&identity) {
			return Ok(VersionedBackendTLS {
				hostname_override: None,
				config: cached.clone(),
				peer_identity_mode: transport::tls::PeerIdentityMode::Istio,
			});
		}

		let cc = self.build_client_config(&identity, vec![b"istio".into()])?;
		let arc_cc = Arc::new(cc);
		self
			.legacy_mtls_cache
			.write()
			.insert(identity, arc_cc.clone());
		Ok(VersionedBackendTLS {
			hostname_override: None,
			config: arc_cc,
			peer_identity_mode: transport::tls::PeerIdentityMode::Istio,
		})
	}

	pub fn hbone_mtls(&self, identity: Vec<Identity>) -> Result<VersionedBackendTLS, Error> {
		// Check cache first
		if let Some(cached) = self.hbone_mtls_cache.read().get(&identity) {
			return Ok(VersionedBackendTLS {
				hostname_override: None,
				config: cached.clone(),
				peer_identity_mode: transport::tls::PeerIdentityMode::Istio,
			});
		}

		let mut cc = self.build_client_config(&identity, vec![b"h2".into()])?;
		cc.enable_sni = false;
		let arc_cc = Arc::new(cc);
		self
			.hbone_mtls_cache
			.write()
			.insert(identity, arc_cc.clone());
		Ok(VersionedBackendTLS {
			hostname_override: None,
			config: arc_cc,
			peer_identity_mode: transport::tls::PeerIdentityMode::Istio,
		})
	}

	/// Build a rustls ClientConfig for mTLS. Extracted to avoid duplication between
	/// cached methods. Callers should set any post-build fields (e.g. enable_sni) before use.
	fn build_client_config(
		&self,
		identity: &[Identity],
		alpn: Vec<Vec<u8>>,
	) -> Result<ClientConfig, Error> {
		let roots = self.roots.clone();
		let verifier = transport::tls::identity::IdentityVerifier {
			roots,
			identity: identity.to_vec(),
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
		cc.key_log = transport::tls::key_log();
		cc.alpn_protocols = alpn;
		cc.resumption = Resumption::disabled();
		Ok(cc)
	}
	pub fn hbone_termination(&self) -> Result<Arc<ServerConfig>, Error> {
		self.server_config(vec![b"h2".into()], true)
	}

	pub fn server_config(
		&self,
		alpns: Vec<Vec<u8>>,
		require_client_cert: bool,
	) -> Result<Arc<ServerConfig>, Error> {
		let cache_key = (alpns.clone(), require_client_cert);
		// Check cache first
		if let Some(cached) = self.server_config_cache.read().get(&cache_key) {
			return Ok(cached.clone());
		}

		let sc = self.build_server_config(alpns, require_client_cert)?;
		let arc_sc = Arc::new(sc);
		self
			.server_config_cache
			.write()
			.insert(cache_key, arc_sc.clone());
		Ok(arc_sc)
	}

	fn build_server_config(
		&self,
		alpns: Vec<Vec<u8>>,
		require_client_cert: bool,
	) -> Result<ServerConfig, Error> {
		let roots = self.roots.clone();
		let scb = ServerConfig::builder_with_provider(transport::tls::provider())
			.with_protocol_versions(transport::tls::ALL_TLS_VERSIONS)
			.expect("server config must be valid");
		let scb = if require_client_cert {
			let raw_client_cert_verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
				roots,
				transport::tls::provider(),
			)
			.build()?;
			// Verify the client's SPIFFE trust domain is in the allowed set, unless explicitly
			// disabled via skip_validate_trust_domain. CA-level certificate validation still applies.
			let client_cert_verifier: Arc<dyn rustls::server::danger::ClientCertVerifier> =
				if self.skip_validate_trust_domain {
					raw_client_cert_verifier
				} else {
					transport::tls::trustdomain::TrustDomainVerifier::new(
						raw_client_cert_verifier,
						self.allowed_trust_domains.clone(),
					)
				};
			scb.with_client_cert_verifier(client_cert_verifier)
		} else {
			scb.with_no_client_auth()
		};
		let mut sc = scb.with_single_cert(
			self.chain.iter().map(|c| c.der.clone()).collect(),
			self.private_key.clone_key(),
		)?;
		sc.key_log = transport::tls::key_log();
		sc.alpn_protocols = alpns;
		Ok(sc)
	}
}

#[derive(Clone, Debug)]
struct Certificate {
	expiry: Expiration,
	identity: Option<Identity>,
	der: rustls_pki_types::CertificateDer<'static>,
}

fn parse_key(key: &[u8]) -> Result<PrivateKeyDer<'static>, Error> {
	let (kind, der) = <(SectionKind, Vec<u8>)>::from_pem_slice(key).map_err(|e| match e {
		rustls_pki_types::pem::Error::NoItemsFound => Error::CertificateParse("no key".to_string()),
		_ => Error::CertificateParse(e.to_string()),
	})?;

	match kind {
		SectionKind::PrivateKey => Ok(PrivateKeyDer::Pkcs8(der.into())),
		SectionKind::RsaPrivateKey => Ok(PrivateKeyDer::Pkcs1(der.into())),
		SectionKind::EcPrivateKey => Ok(PrivateKeyDer::Sec1(der.into())),
		_ => Err(Error::CertificateParse("no key".to_string())),
	}
}

fn parse_cert(cert: Vec<u8>) -> Result<Certificate, Error> {
	let (kind, der) = <(SectionKind, Vec<u8>)>::from_pem_slice(&cert).map_err(|e| match e {
		rustls_pki_types::pem::Error::NoItemsFound => {
			Error::CertificateParse("no certificate".to_string())
		},
		_ => Error::CertificateParse(e.to_string()),
	})?;
	if kind != SectionKind::Certificate {
		return Err(Error::CertificateParse("no certificate".to_string()));
	}
	let der = CertificateDer::from(der);

	let (_, cert) = x509_parser::parse_x509_certificate(der.as_ref())
		.map_err(|e| Error::CertificateParse(e.to_string()))?;
	Ok(Certificate {
		der: der.clone(),
		expiry: expiration(cert.clone()),
		identity: identity(cert),
	})
}

fn parse_cert_multi(cert: &[u8]) -> Result<Vec<Certificate>, Error> {
	let parsed: Result<Vec<_>, _> = <(SectionKind, Vec<u8>)>::pem_slice_iter(cert).collect();
	parsed
		.map_err(|e| Error::CertificateParse(e.to_string()))?
		.into_iter()
		.map(|(kind, der)| {
			if kind != SectionKind::Certificate {
				return Err(Error::CertificateParse("no certificate".to_string()));
			}
			let der = CertificateDer::from(der);
			let (_, cert) = x509_parser::parse_x509_certificate(der.as_ref())
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
	state: watch::Receiver<CertificateState>,
	_fetcher_handle: tokio::task::JoinHandle<()>,
}

impl CaClient {
	pub fn new(client: client::Client, config: Config) -> Result<Self, Error> {
		let (state_tx, state_rx) = watch::channel(CertificateState::NotReady);

		let headers: Vec<(http::header::HeaderName, http::HeaderValue)> = config
			.ca_headers
			.iter()
			.map(|(k, v)| {
				Ok((
					http::header::HeaderName::from_str(k)
						.map_err(|e| Error::CaClientCreation(Arc::new(anyhow::Error::new(e))))?,
					http::HeaderValue::from_str(v)
						.map_err(|e| Error::CaClientCreation(Arc::new(anyhow::Error::new(e))))?,
				))
			})
			.collect::<Result<_, Error>>()?;

		// Start the fetcher task
		let fetcher_handle = tokio::spawn({
			let config = config.clone();
			let state_tx = state_tx.clone();
			let headers = headers.clone();

			async move {
				Self::run_fetcher(client, config, state_tx, headers).await;
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
		headers: Vec<(http::header::HeaderName, http::HeaderValue)>,
	) {
		let mut interval = tokio::time::interval(Duration::from_secs(30)); // Check every 30 seconds

		// Start with an immediate fetch
		if let Err(e) =
			Self::fetch_and_update_certificate(client.clone(), &config, &state_tx, headers.clone()).await
		{
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

				match Self::fetch_and_update_certificate(
					client.clone(),
					&config,
					&state_tx,
					headers.clone(),
				)
				.await
				{
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
		headers: Vec<(http::header::HeaderName, http::HeaderValue)>,
	) -> Result<(), Error> {
		info!("Fetching certificate for identity: {}", config.identity);

		let svc = control::grpc_connector(
			client,
			config.address.clone(),
			config.auth.clone(),
			config.ca_cert.clone(),
			headers.clone(),
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
			config.allowed_trust_domains.clone(),
			config.skip_validate_trust_domain,
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

	/// Helper: generate a self-signed CA + leaf cert for testing TLS config caching.
	fn test_workload_certificate() -> WorkloadCertificate {
		use std::time::{Duration, SystemTime};

		use rcgen::{
			BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, Issuer, KeyPair,
			KeyUsagePurpose, SanType,
		};

		// Generate CA key + self-signed CA cert
		let ca_kp = KeyPair::generate().unwrap();
		let mut ca_params = CertificateParams::default();
		ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
		ca_params.not_before = SystemTime::now().into();
		ca_params.not_after = (SystemTime::now() + Duration::from_secs(3600)).into();
		let mut ca_dn = DistinguishedName::new();
		ca_dn.push(DnType::OrganizationName, "test-ca");
		ca_params.distinguished_name = ca_dn;
		let ca_cert = ca_params.self_signed(&ca_kp).unwrap();

		// Generate leaf key + cert signed by CA with SPIFFE SAN
		let leaf_kp = KeyPair::generate().unwrap();
		let mut leaf_params = CertificateParams::default();
		leaf_params.not_before = SystemTime::now().into();
		leaf_params.not_after = (SystemTime::now() + Duration::from_secs(3600)).into();
		leaf_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
		leaf_params.extended_key_usages = vec![
			rcgen::ExtendedKeyUsagePurpose::ClientAuth,
			rcgen::ExtendedKeyUsagePurpose::ServerAuth,
		];
		leaf_params.subject_alt_names = vec![SanType::URI(
			"spiffe://cluster.local/ns/default/sa/test"
				.try_into()
				.unwrap(),
		)];
		let issuer = Issuer::from_params(&ca_params, &ca_kp);
		let leaf_cert = leaf_params.signed_by(&leaf_kp, &issuer).unwrap();

		let leaf_pem = leaf_cert.pem();
		let ca_pem = ca_cert.pem();
		let key_pem = leaf_kp.serialize_pem();

		WorkloadCertificate::new(
			key_pem.as_bytes(),
			leaf_pem.as_bytes(),
			vec![ca_pem.as_bytes()],
			Arc::from([Strng::from("cluster.local")]),
			false,
		)
		.expect("test cert should be valid")
	}

	#[test]
	fn test_legacy_mtls_caching() {
		let cert = test_workload_certificate();
		let identity =
			vec![Identity::from_str("spiffe://cluster.local/ns/default/sa/backend").unwrap()];

		// First call builds the config
		let result1 = cert.legacy_mtls(identity.clone()).unwrap();
		// Second call should return the cached config (same Arc pointer)
		let result2 = cert.legacy_mtls(identity.clone()).unwrap();

		assert!(Arc::ptr_eq(&result1.config, &result2.config));
	}

	#[test]
	fn test_hbone_mtls_caching() {
		let cert = test_workload_certificate();
		let identity =
			vec![Identity::from_str("spiffe://cluster.local/ns/default/sa/backend").unwrap()];

		let result1 = cert.hbone_mtls(identity.clone()).unwrap();
		let result2 = cert.hbone_mtls(identity.clone()).unwrap();

		assert!(Arc::ptr_eq(&result1.config, &result2.config));
	}

	#[test]
	fn test_server_config_caching() {
		let cert = test_workload_certificate();

		let result1 = cert.server_config(vec![b"h2".into()], true).unwrap();
		let result2 = cert.server_config(vec![b"h2".into()], true).unwrap();

		assert!(Arc::ptr_eq(&result1, &result2));
	}

	#[test]
	fn test_server_config_different_keys() {
		let cert = test_workload_certificate();

		// Different ALPNs → different cache entries
		let r1 = cert.server_config(vec![b"h2".into()], true).unwrap();
		let r2 = cert.server_config(vec![b"http/1.1".into()], true).unwrap();
		// Different require_client_cert → different cache entries
		let r3 = cert.server_config(vec![b"h2".into()], false).unwrap();

		assert!(!Arc::ptr_eq(&r1, &r2));
		assert!(!Arc::ptr_eq(&r1, &r3));
	}

	#[test]
	fn test_mtls_different_identities_not_cached() {
		let cert = test_workload_certificate();
		let id1 = vec![Identity::from_str("spiffe://cluster.local/ns/default/sa/backend").unwrap()];
		let id2 = vec![Identity::from_str("spiffe://cluster.local/ns/default/sa/frontend").unwrap()];

		let r1 = cert.hbone_mtls(id1).unwrap();
		let r2 = cert.hbone_mtls(id2).unwrap();

		// Different identities → different configs
		assert!(!Arc::ptr_eq(&r1.config, &r2.config));
	}
}
