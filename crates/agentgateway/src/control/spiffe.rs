//! SPIFFE Workload API integration.
//!
//! [`SpiffeClient`] wraps a [`X509Source`], which connects to the local SPIFFE Workload API
//! endpoint and keeps the gateway's X.509-SVID and trust bundles up to date in the background,
//! rotating them automatically.
//!
//! From the current SVID (the cert chain + private key) it builds, on demand, both a
//! [`ServerConfig`] for terminating TLS on listeners and a [`ClientConfig`] for outbound mTLS to
//! upstream backends.
use ::spiffe::{TrustDomain, X509Context, X509Source, X509SourceUpdates, X509Svid};
use rustls::client::danger::ServerCertVerifier;
use rustls::server::danger::ClientCertVerifier;
use rustls::{ClientConfig, ServerConfig};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

use crate::*;

/// Configuration for the shared connection to the local SPIFFE Workload API.
#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Config {
	/// Endpoint of the SPIFFE Workload API (e.g. `unix:///run/spire/agent.sock`)
	pub endpoint: String,
	/// Federated trust domains this gateway may accept, on top of its own (local) trust domain.
	/// This is an advisory allow-list used to validate the per-listener/backend accepted lists; it
	/// does NOT control which bundles are delivered (SPIRE decides that via `federatesWith`). When
	/// empty, the allow-list guard is disabled and only runtime bundle availability is enforced.
	#[serde(default)]
	pub federated_trust_domains: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("spiffe workload api: {0}")]
	Source(#[from] ::spiffe::x509_source::X509SourceError),
	#[error("rustls: {0}")]
	Rustls(#[from] rustls::Error),
	#[error("rustls verifier: {0}")]
	Verifier(#[from] rustls::server::VerifierBuilderError),
	#[error(
		"timed out after {0:?} connecting to the SPIFFE Workload API at {1}; is the endpoint reachable?"
	)]
	Timeout(Duration, String),
	#[error("no root certificates in SPIFFE trust bundle")]
	EmptyBundle,
	#[error("connected to the SPIFFE Workload API but no X.509-SVID is available")]
	NoSvid,
	#[error("invalid SPIFFE verification SAN: {0}")]
	InvalidSan(String),
	// Fail closed: SPIRE has not delivered a bundle for this trust domain (not federated with the
	// gateway's own trust domain, or not yet synced).
	#[error(
		"no SPIFFE trust bundle available for trust domain {0:?}; is it federated with this gateway?"
	)]
	MissingBundle(String),
	#[error("SPIFFE trust domain {0:?} is not declared in spiffe.federatedTrustDomains")]
	TrustDomainNotFederated(String),
	#[error("invalid SPIFFE trust domain {0:?}: {1}")]
	InvalidTrustDomain(String, String),
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ServerConfigKey {
	alpns: Vec<Vec<u8>>,
	// Canonical (sorted, lowercased) accepted federated trust domain names.
	accepted_trust_domains: Vec<String>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ClientConfigCacheKey {
	alpns: Vec<Vec<u8>>,
	verify_sans: Vec<String>,
	// Canonical (sorted, lowercased) accepted federated trust domain names.
	accepted_trust_domains: Vec<String>,
}

/// A rotation-aware map with 2 methods, Get & Insert, both having a sequence number parameter.
/// `Get` returns None if the sequence number does not match, regardless of whether the key exists.
/// `Insert` clears the cache if the sequence number passed in does not match the stored value, and then stores the new value and sequence
/// The sequence number is provided by the `X509Source`.
struct RotatingCache<K, V> {
	seq: u64,
	entries: HashMap<K, Arc<V>>,
}

impl<K, V> Default for RotatingCache<K, V> {
	fn default() -> Self {
		Self {
			seq: 0,
			entries: HashMap::new(),
		}
	}
}

impl<K: Eq + std::hash::Hash, V> RotatingCache<K, V> {
	fn get(&self, seq: u64, key: &K) -> Option<Arc<V>> {
		if self.seq == seq {
			self.entries.get(key).cloned()
		} else {
			None
		}
	}

	/// Stores `value` under `key` and returns the cached value, which is the one already
	/// cached if another caller won the race.
	fn insert(&mut self, seq: u64, key: K, value: Arc<V>) -> Arc<V> {
		if seq < self.seq {
			// stale sequence number; return it without touching the cache
			return value;
		}
		if self.seq != seq {
			// new sequence number; clear the cache of stale entries
			self.entries.clear();
			self.seq = seq;
		}
		self.entries.entry(key).or_insert(value).clone()
	}
}

#[derive(Clone)]
pub struct SpiffeClient {
	source: Arc<X509Source>,
	updates: X509SourceUpdates,
	server_cache: Arc<Mutex<RotatingCache<ServerConfigKey, ServerConfig>>>,
	client_cache: Arc<Mutex<RotatingCache<ClientConfigCacheKey, ClientConfig>>>,
	/// Allow-list of federated trust domains (parsed once); see [`Config::federated_trust_domains`].
	federated_trust_domains: Arc<[TrustDomain]>,
}

impl std::fmt::Debug for SpiffeClient {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("SpiffeClient").finish_non_exhaustive()
	}
}

impl SpiffeClient {
	/// Connects to the SPIFFE Workload API and performs the initial SVID/bundle sync.
	pub async fn new(endpoint: String, federated_trust_domains: Vec<String>) -> Result<Self, Error> {
		info!(endpoint = %endpoint, "connecting to SPIFFE workload API");
		// Validate the declared federated trust domains up front so a typo fails at startup.
		let federated_trust_domains: Arc<[TrustDomain]> =
			normalize_trust_domains(&federated_trust_domains)?.into();
		let source: X509Source = match X509Source::builder()
			.endpoint(endpoint.clone())
			.build()
			.await
		{
			Ok(source) => source,
			Err(e) => {
				warn!(endpoint = %endpoint, error = %e, "failed to connect to SPIFFE workload API");
				return Err(Error::Source(e));
			},
		};
		let updates = source.updated();
		let client = Self {
			source: Arc::new(source),
			updates,
			server_cache: Arc::new(Mutex::new(RotatingCache::default())),
			client_cache: Arc::new(Mutex::new(RotatingCache::default())),
			federated_trust_domains,
		};
		match client.spiffe_id() {
			Some(id) => {
				debug!(spiffe_id = %id, "connected to SPIFFE workload API; initial SVID received")
			},
			None => {
				warn!(endpoint = %endpoint, "connected to SPIFFE workload API, but no SVID available");
				return Err(Error::NoSvid);
			},
		}
		Ok(client)
	}

	fn spiffe_id(&self) -> Option<String> {
		self.source.try_svid().map(|s| s.spiffe_id().to_string())
	}

	/// Builds (or returns a cached) `rustls::ServerConfig` from the current SVID and trust bundles.
	///
	/// Incoming connections must present a client SVID (mutual TLS is always required). The SVID is
	/// verified against the bundle for its own trust domain: the gateway's local trust domain (always
	/// implicit) plus any `accepted_trust_domains` (federated). Each SVID is checked against only its
	/// own trust domain's bundle (SPIFFE Federation spec §7.3). Use the `source.spiffeId` CEL field to
	/// apply further restrictions.
	pub fn server_config(
		&self,
		alpns: Vec<Vec<u8>>,
		accepted_trust_domains: Vec<String>,
	) -> Result<Arc<ServerConfig>, Error> {
		let accepted = normalize_trust_domains(&accepted_trust_domains)?;
		let seq = self.updates.last();
		let key = ServerConfigKey {
			alpns: alpns.clone(),
			accepted_trust_domains: trust_domain_names(&accepted),
		};

		if let Some(cfg) = self.server_cache.lock().unwrap().get(seq, &key) {
			return Ok(cfg);
		}

		let cfg = Arc::new(self.build_server_config(alpns, &accepted)?);
		Ok(self.server_cache.lock().unwrap().insert(seq, key, cfg))
	}

	fn build_server_config(
		&self,
		alpns: Vec<Vec<u8>>,
		accepted: &[TrustDomain],
	) -> Result<ServerConfig, Error> {
		let ctx = self.source.x509_context()?;
		let provider = transport::tls::provider();
		// Verify inbound client SVIDs against the accepted trust domains' bundles (local + federated).
		let verifier = build_client_verifier(
			&ctx,
			accepted,
			&self.federated_trust_domains,
			provider.clone(),
		)?;
		let (chain, key, spiffe_id) = svid_identity(&ctx)?;

		let mut config = ServerConfig::builder_with_provider(provider)
			.with_protocol_versions(transport::tls::ALL_TLS_VERSIONS)
			.expect("server config must be valid")
			.with_client_cert_verifier(verifier)
			.with_single_cert(chain, key)?;
		config.key_log = transport::tls::key_log();
		config.alpn_protocols = alpns;
		// Disable session resumption (TLS 1.2 cache + TLS 1.3 tickets): a resumed session skips
		// certificate re-validation, which would let a peer keep a session alive past the expiry of
		// the short-lived SVID that authenticated it.
		config.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
		config.send_tls13_tickets = 0;
		debug!(spiffe_id = %spiffe_id,alpn_count = config.alpn_protocols.len(),"built SPIFFE-sourced rustls ServerConfig");
		Ok(config)
	}

	/// Builds (or returns a cached) `rustls::ClientConfig` for outbound mTLS to a SPIFFE-backed
	/// upstream. The gateway presents its current SVID as the client certificate. The upstream's
	/// certificate is verified against the bundle for its own trust domain (local, always implicit,
	/// plus any `accepted_trust_domains`); when `verify_sans` is empty any SVID chaining to that
	/// bundle is accepted (DNS hostname checks do not apply to SPIFFE SVIDs), otherwise the upstream's
	/// SPIFFE ID must match one of the provided `spiffe://` URIs.
	pub fn client_config(
		&self,
		alpns: Vec<Vec<u8>>,
		verify_sans: Vec<String>,
		accepted_trust_domains: Vec<String>,
	) -> Result<Arc<ClientConfig>, Error> {
		let accepted = normalize_trust_domains(&accepted_trust_domains)?;
		// Sequence sampled before the SVID read; see server_config.
		let seq = self.updates.last();
		let key = ClientConfigCacheKey {
			alpns: alpns.clone(),
			verify_sans: verify_sans.clone(),
			accepted_trust_domains: trust_domain_names(&accepted),
		};

		if let Some(cfg) = self.client_cache.lock().unwrap().get(seq, &key) {
			return Ok(cfg);
		}

		let cfg = Arc::new(self.build_client_config(alpns, verify_sans, &accepted)?);
		Ok(self.client_cache.lock().unwrap().insert(seq, key, cfg))
	}

	fn build_client_config(
		&self,
		alpns: Vec<Vec<u8>>,
		verify_sans: Vec<String>,
		accepted: &[TrustDomain],
	) -> Result<ClientConfig, Error> {
		let ctx = self.source.x509_context()?;
		let provider = transport::tls::provider();
		let sans_count = verify_sans.len();
		let verifier =
			build_server_verifier(&ctx, verify_sans, accepted, &self.federated_trust_domains)?;
		let (chain, key, spiffe_id) = svid_identity(&ctx)?;
		let mut config = ClientConfig::builder_with_provider(provider)
			.with_protocol_versions(transport::tls::ALL_TLS_VERSIONS)
			.expect("client config must be valid")
			.dangerous()
			.with_custom_certificate_verifier(verifier)
			.with_client_auth_cert(chain, key)?;

		config.key_log = transport::tls::key_log();
		config.alpn_protocols = alpns;
		// Disable session resumption so an upstream's short-lived SVID is re-validated on every
		// handshake rather than being skipped by a resumed session (see build_server_config).
		config.resumption = rustls::client::Resumption::disabled();
		debug!(
			spiffe_id = %spiffe_id,
			sans_count,
			alpn_count = config.alpn_protocols.len(),
			"built SPIFFE-sourced rustls ClientConfig"
		);
		Ok(config)
	}
}

fn snapshot_svid(ctx: &X509Context) -> Result<&Arc<X509Svid>, Error> {
	ctx.default_svid().ok_or(Error::NoSvid)
}

/// Parse, validate, de-duplicate and canonically order trust domain names. `TrustDomain` lowercases
/// names, so the result is a stable cache key regardless of input case or order.
fn normalize_trust_domains(names: &[String]) -> Result<Vec<TrustDomain>, Error> {
	let mut tds: Vec<TrustDomain> = Vec::with_capacity(names.len());
	for name in names {
		let td =
			TrustDomain::new(name).map_err(|e| Error::InvalidTrustDomain(name.clone(), e.to_string()))?;
		if !tds.contains(&td) {
			tds.push(td);
		}
	}
	tds.sort();
	Ok(tds)
}

fn trust_domain_names(tds: &[TrustDomain]) -> Vec<String> {
	tds.iter().map(|td| td.to_string()).collect()
}

/// Per-trust-domain root stores used to verify peer SVIDs. Always includes the gateway's own
/// (local) trust domain; each federated trust domain named in `accepted` must be present in the
/// bundle set delivered by the Workload API, otherwise we fail closed.
///
/// Bundles are kept per trust domain and never pooled: SPIFFE Federation spec §7.3 requires an SVID
/// to be validated against only the bundle for the trust domain named in its own SPIFFE ID; pooling
/// would let one accepted trust domain mint SVIDs impersonating another.
fn roots_by_trust_domain(
	ctx: &X509Context,
	accepted: &[TrustDomain],
	federated: &[TrustDomain],
	purpose: &str,
) -> Result<HashMap<TrustDomain, Arc<rustls::RootCertStore>>, Error> {
	let svid = snapshot_svid(ctx)?;
	let local_td = svid.spiffe_id().trust_domain().clone();

	// The local trust domain is always accepted implicitly.
	let mut wanted: Vec<TrustDomain> = vec![local_td.clone()];
	for td in accepted {
		if *td == local_td || wanted.contains(td) {
			continue;
		}
		// When federatedTrustDomains is configured, an accepted domain must be declared in it.
		// TODO(jaellio): also enforce this subset relationship in the controller once the translator
		// has access to the gateway's AgentgatewayParameters SpiffeSpec.
		if !federated.is_empty() && !federated.contains(td) {
			return Err(Error::TrustDomainNotFederated(td.to_string()));
		}
		wanted.push(td.clone());
	}

	let bundles = ctx.bundle_set();
	let mut by_td = HashMap::with_capacity(wanted.len());
	for td in wanted {
		// Fail closed: without a delivered bundle we cannot verify this trust domain's SVIDs.
		let bundle = bundles
			.get(&td)
			.ok_or_else(|| Error::MissingBundle(td.to_string()))?;
		let mut roots = rustls::RootCertStore::empty();
		for authority in bundle.authorities() {
			roots
				.add(CertificateDer::from(authority.as_bytes().to_vec()))
				.map_err(Error::Rustls)?;
		}
		if roots.is_empty() {
			return Err(Error::EmptyBundle);
		}
		by_td.insert(td, Arc::new(roots));
	}
	debug!(
		purpose,
		trust_domains = by_td.len(),
		"loaded SPIFFE trust bundles"
	);
	Ok(by_td)
}

/// Inbound: one `WebPkiClientVerifier` per accepted trust domain, wrapped in a dispatcher that
/// selects the verifier by the peer's own trust domain before validating the chain.
fn build_client_verifier(
	ctx: &X509Context,
	accepted: &[TrustDomain],
	federated: &[TrustDomain],
	provider: Arc<rustls::crypto::CryptoProvider>,
) -> Result<Arc<dyn ClientCertVerifier>, Error> {
	let roots = roots_by_trust_domain(ctx, accepted, federated, "client certificate verification")?;
	let mut by_td: HashMap<TrustDomain, Arc<dyn ClientCertVerifier>> =
		HashMap::with_capacity(roots.len());
	for (td, store) in roots {
		let verifier =
			rustls::server::WebPkiClientVerifier::builder_with_provider(store, provider.clone())
				.build()?;
		by_td.insert(td, verifier);
	}
	Ok(Arc::new(verify::SpiffeClientCertVerifier::new(by_td)))
}

/// Outbound: one server verifier per accepted trust domain (each applying the optional SPIFFE-ID
/// pin), wrapped in a dispatcher that selects by the upstream's own trust domain. SPIFFE SVIDs carry
/// a `spiffe://` URI SAN and no DNS SAN, so standard WebPKI hostname verification does not apply.
fn build_server_verifier(
	ctx: &X509Context,
	verify_sans: Vec<String>,
	accepted: &[TrustDomain],
	federated: &[TrustDomain],
) -> Result<Arc<dyn ServerCertVerifier>, Error> {
	let roots = roots_by_trust_domain(ctx, accepted, federated, "upstream server verification")?;
	let provider = transport::tls::provider();
	let mut by_td: HashMap<TrustDomain, Arc<dyn ServerCertVerifier>> =
		HashMap::with_capacity(roots.len());
	for (td, store) in roots {
		let verifier: Arc<dyn ServerCertVerifier> = if verify_sans.is_empty() {
			// No SPIFFE ID pinned: accept any SVID that chains to this trust domain's bundle.
			let inner =
				rustls::client::WebPkiServerVerifier::builder_with_provider(store, provider.clone())
					.build()?;
			Arc::new(transport::tls::insecure::NoServerNameVerification::new(
				inner,
			))
		} else {
			// Rebuilt per trust domain because `ExtendedServerName` is not `Clone`.
			let alt_names = verify_sans
				.iter()
				.cloned()
				.map(transport::tls::ExtendedServerName::try_from)
				.collect::<Result<Box<[_]>, _>>()
				.map_err(|e| Error::InvalidSan(e.to_string()))?;
			Arc::new(transport::tls::insecure::AltHostnameVerifier::new(
				store, alt_names,
			))
		};
		by_td.insert(td, verifier);
	}
	Ok(Arc::new(verify::SpiffeServerCertVerifier::new(by_td)))
}

/// Custom rustls verifiers that preserve the SPIFFE `<trust domain, bundle>` binding (SPIFFE
/// Federation spec §7.3): each peer SVID is validated against only the bundle for the trust domain
/// named in its own SPIFFE ID. We dispatch to the matching per-trust-domain verifier built above.
mod verify {
	use std::collections::HashMap;
	use std::sync::Arc;

	use ::spiffe::TrustDomain;
	use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
	use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
	use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
	use rustls::{
		CertificateError, DigitallySignedStruct, DistinguishedName, OtherError, SignatureScheme,
	};

	#[derive(Debug, thiserror::Error)]
	enum VerifyError {
		#[error("peer certificate has no valid SPIFFE ID: {0}")]
		PeerSpiffeId(String),
		#[error("peer trust domain {0:?} is not accepted by this listener/backend")]
		NotAccepted(String),
	}

	fn other(err: VerifyError) -> rustls::Error {
		rustls::Error::InvalidCertificate(CertificateError::Other(OtherError(Arc::new(err))))
	}

	/// The peer certificate's trust domain, taken from its single `spiffe://` URI SAN.
	fn peer_trust_domain(cert: &CertificateDer<'_>) -> Result<TrustDomain, rustls::Error> {
		let id = ::spiffe::cert::spiffe_id_from_der(cert.as_ref())
			.map_err(|e| other(VerifyError::PeerSpiffeId(e.to_string())))?;
		Ok(id.trust_domain().clone())
	}

	/// Inbound: dispatch a client SVID to the verifier owning its trust domain's bundle.
	#[derive(Debug)]
	pub(super) struct SpiffeClientCertVerifier {
		by_td: HashMap<TrustDomain, Arc<dyn ClientCertVerifier>>,
		root_hint_subjects: Vec<DistinguishedName>,
	}

	impl SpiffeClientCertVerifier {
		pub(super) fn new(by_td: HashMap<TrustDomain, Arc<dyn ClientCertVerifier>>) -> Self {
			// Advertise the union of accepted CAs so clients can select a client certificate.
			let root_hint_subjects = by_td
				.values()
				.flat_map(|v| v.root_hint_subjects().to_vec())
				.collect();
			Self {
				by_td,
				root_hint_subjects,
			}
		}
	}

	impl ClientCertVerifier for SpiffeClientCertVerifier {
		fn offer_client_auth(&self) -> bool {
			true
		}

		fn client_auth_mandatory(&self) -> bool {
			true
		}

		fn root_hint_subjects(&self) -> &[DistinguishedName] {
			&self.root_hint_subjects
		}

		fn verify_client_cert(
			&self,
			end_entity: &CertificateDer<'_>,
			intermediates: &[CertificateDer<'_>],
			now: UnixTime,
		) -> Result<ClientCertVerified, rustls::Error> {
			let td = peer_trust_domain(end_entity)?;
			let inner = self
				.by_td
				.get(&td)
				.ok_or_else(|| other(VerifyError::NotAccepted(td.to_string())))?;
			inner.verify_client_cert(end_entity, intermediates, now)
		}

		fn verify_tls12_signature(
			&self,
			message: &[u8],
			cert: &CertificateDer<'_>,
			dss: &DigitallySignedStruct,
		) -> Result<HandshakeSignatureValid, rustls::Error> {
			rustls::crypto::verify_tls12_signature(
				message,
				cert,
				dss,
				&crate::crypto::tls::signature_verification_algorithms(),
			)
		}

		fn verify_tls13_signature(
			&self,
			message: &[u8],
			cert: &CertificateDer<'_>,
			dss: &DigitallySignedStruct,
		) -> Result<HandshakeSignatureValid, rustls::Error> {
			rustls::crypto::verify_tls13_signature(
				message,
				cert,
				dss,
				&crate::crypto::tls::signature_verification_algorithms(),
			)
		}

		fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
			crate::crypto::tls::signature_verification_algorithms().supported_schemes()
		}
	}

	/// Outbound: dispatch an upstream SVID to the verifier owning its trust domain's bundle.
	#[derive(Debug)]
	pub(super) struct SpiffeServerCertVerifier {
		by_td: HashMap<TrustDomain, Arc<dyn ServerCertVerifier>>,
	}

	impl SpiffeServerCertVerifier {
		pub(super) fn new(by_td: HashMap<TrustDomain, Arc<dyn ServerCertVerifier>>) -> Self {
			Self { by_td }
		}
	}

	impl ServerCertVerifier for SpiffeServerCertVerifier {
		fn verify_server_cert(
			&self,
			end_entity: &CertificateDer<'_>,
			intermediates: &[CertificateDer<'_>],
			server_name: &ServerName<'_>,
			ocsp_response: &[u8],
			now: UnixTime,
		) -> Result<ServerCertVerified, rustls::Error> {
			let td = peer_trust_domain(end_entity)?;
			let inner = self
				.by_td
				.get(&td)
				.ok_or_else(|| other(VerifyError::NotAccepted(td.to_string())))?;
			inner.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
		}

		fn verify_tls12_signature(
			&self,
			message: &[u8],
			cert: &CertificateDer<'_>,
			dss: &DigitallySignedStruct,
		) -> Result<HandshakeSignatureValid, rustls::Error> {
			rustls::crypto::verify_tls12_signature(
				message,
				cert,
				dss,
				&crate::crypto::tls::signature_verification_algorithms(),
			)
		}

		fn verify_tls13_signature(
			&self,
			message: &[u8],
			cert: &CertificateDer<'_>,
			dss: &DigitallySignedStruct,
		) -> Result<HandshakeSignatureValid, rustls::Error> {
			rustls::crypto::verify_tls13_signature(
				message,
				cert,
				dss,
				&crate::crypto::tls::signature_verification_algorithms(),
			)
		}

		fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
			crate::crypto::tls::signature_verification_algorithms().supported_schemes()
		}
	}
}

/// Extracts the certificate chain, private key, and SPIFFE ID string from the X%09Context
fn svid_identity(
	ctx: &X509Context,
) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>, String), Error> {
	let svid = snapshot_svid(ctx)?;
	let chain: Vec<CertificateDer<'static>> = svid
		.cert_chain()
		.iter()
		.map(|c| CertificateDer::from(c.as_bytes().to_vec()))
		.collect();
	// The SPIFFE Workload API always returns the key as PKCS#8 DER.
	let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
		svid.private_key().as_bytes().to_vec(),
	));
	Ok((chain, key, svid.spiffe_id().to_string()))
}

// The SPIFFE Workload API is a Unix-domain-socket protocol, limited to unix.
#[cfg(all(test, target_family = "unix"))]
mod tests {
	use std::sync::atomic::{AtomicU32, Ordering};

	use futures::StreamExt;
	use protos::spiffe_workload_api::spiffe_workload_api_server::{
		SpiffeWorkloadApi, SpiffeWorkloadApiServer,
	};
	use protos::spiffe_workload_api::*;
	use rustls_pki_types::{ServerName, UnixTime};
	use tokio::sync::mpsc;
	use tonic::{Request, Response, Status};

	use super::*;

	#[test]
	fn rotating_cache_hits_within_same_generation() {
		let mut cache: RotatingCache<&str, u32> = RotatingCache::default();
		cache.insert(0, "a", Arc::new(1));
		assert_eq!(cache.get(0, &"a").as_deref(), Some(&1));
		// Unknown key in the same generation misses.
		assert!(cache.get(0, &"b").is_none());
	}

	#[test]
	fn rotating_cache_misses_on_stale_generation() {
		let mut cache: RotatingCache<&str, u32> = RotatingCache::default();
		cache.insert(0, "a", Arc::new(1));
		// A later rotation sequence invalidates the cached value even for the same key.
		assert!(cache.get(1, &"a").is_none());
	}

	#[test]
	fn rotating_cache_insert_clears_previous_generation() {
		let mut cache: RotatingCache<&str, u32> = RotatingCache::default();
		cache.insert(0, "a", Arc::new(1));
		// Inserting at a newer sequence drops the whole stale generation.
		cache.insert(1, "b", Arc::new(2));
		assert!(cache.get(1, &"a").is_none());
		assert_eq!(cache.get(1, &"b").as_deref(), Some(&2));
		// The old generation is gone regardless of sequence queried.
		assert!(cache.get(0, &"a").is_none());
	}

	/// Two callers missing on the same key concurrently both build a value, but must end up sharing
	/// the one that landed first — distinct `Arc`s for the same config would split the connection pool.
	#[test]
	fn rotating_cache_insert_returns_the_winning_value() {
		let mut cache: RotatingCache<&str, u32> = RotatingCache::default();
		let first = Arc::new(1);
		let winner = cache.insert(0, "a", first.clone());
		assert!(Arc::ptr_eq(&winner, &first));

		// The loser's value is discarded and it receives the already-cached one instead.
		let loser = cache.insert(0, "a", Arc::new(2));
		assert!(Arc::ptr_eq(&loser, &first));
		assert_eq!(cache.get(0, &"a").as_deref(), Some(&1));
	}

	/// A slow builder can finish after a rotation has already been cached. Its value is returned to
	/// its own caller but must not clear or overwrite the newer generation.
	#[test]
	fn rotating_cache_insert_ignores_superseded_generation() {
		let mut cache: RotatingCache<&str, u32> = RotatingCache::default();
		cache.insert(1, "current", Arc::new(2));

		let stale = Arc::new(1);
		let returned = cache.insert(0, "stale", stale.clone());
		assert!(Arc::ptr_eq(&returned, &stale));

		// The newer generation survives intact and the stale value was not stored.
		assert_eq!(cache.get(1, &"current").as_deref(), Some(&2));
		assert!(cache.get(1, &"stale").is_none());
	}

	/// A throwaway CA for minting SPIFFE SVIDs in tests. Every SVID it issues chains to this CA, so
	/// they validate against `cert_der`/`cert_pem` when used as the trust bundle.
	struct TestCa {
		kp: rcgen::KeyPair,
		params: rcgen::CertificateParams,
		/// The CA certificate (the trust bundle), DER- and PEM-encoded.
		cert_der: Vec<u8>,
		cert_pem: String,
	}

	/// A minted leaf SVID in the encodings tests need: DER for the Workload API response, PEM for an
	/// mTLS client config.
	struct IssuedSvid {
		leaf_der: Vec<u8>,
		key_der: Vec<u8>,
		leaf_pem: String,
		key_pem: String,
	}

	impl TestCa {
		fn new() -> Self {
			use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};

			let now = std::time::SystemTime::now();
			let not_after = now + Duration::from_secs(3600);
			let kp = KeyPair::generate().unwrap();
			let mut params = CertificateParams::default();
			params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
			params.not_before = now.into();
			params.not_after = not_after.into();
			let cert = params.self_signed(&kp).unwrap();
			Self {
				cert_der: cert.der().to_vec(),
				cert_pem: cert.pem(),
				kp,
				params,
			}
		}

		/// Issue a leaf SVID for `spiffe_id`, signed by this CA.
		fn issue(&self, spiffe_id: &str) -> IssuedSvid {
			use rcgen::{
				CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose,
				SanType, SerialNumber,
			};

			let now = std::time::SystemTime::now();
			let not_after = now + Duration::from_secs(3600);
			let leaf_kp = KeyPair::generate().unwrap();
			let mut params = CertificateParams::default();
			// SPIFFE SVIDs must carry an explicit basicConstraints (CA:FALSE); the spiffe crate
			// rejects leaves that omit it (OID 2.5.29.19).
			params.is_ca = IsCa::ExplicitNoCa;
			params.not_before = now.into();
			params.not_after = not_after.into();
			params.serial_number = Some(SerialNumber::from_slice(&[1]));
			params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
			params.extended_key_usages = vec![
				ExtendedKeyUsagePurpose::ServerAuth,
				ExtendedKeyUsagePurpose::ClientAuth,
			];
			params.subject_alt_names = vec![SanType::URI(spiffe_id.try_into().unwrap())];
			let issuer = Issuer::from_params(&self.params, &self.kp);
			let leaf = params.signed_by(&leaf_kp, &issuer).unwrap();
			IssuedSvid {
				leaf_der: leaf.der().to_vec(),
				key_der: leaf_kp.serialize_der(),
				leaf_pem: leaf.pem(),
				key_pem: leaf_kp.serialize_pem(),
			}
		}
	}

	type RespStream<T> = futures::stream::BoxStream<'static, Result<T, Status>>;

	/// A fake SPIFFE Workload API server. Only `FetchX509SVID` is implemented (the rest return
	/// `unimplemented`); it streams the initial response, then any responses pushed through
	/// `rotations` (for the rotation test), and finally holds the stream open so the `X509Source`
	/// treats the SVID as live rather than reconnecting.
	///
	/// The spiffe crate opens two separate streams: the first for initial sync (reads one item and
	/// drops the stream) and the second for ongoing supervisor updates. The `rotations` receiver is
	/// therefore given to the **second** connection so the supervisor can receive rotations.
	struct FakeWorkloadApi {
		resp: X509svidResponse,
		rotations: Mutex<Option<mpsc::Receiver<X509svidResponse>>>,
		connection_count: AtomicU32,
	}

	#[tonic::async_trait]
	impl SpiffeWorkloadApi for FakeWorkloadApi {
		type FetchX509SVIDStream = RespStream<X509svidResponse>;
		async fn fetch_x509svid(
			&self,
			_request: Request<X509svidRequest>,
		) -> Result<Response<Self::FetchX509SVIDStream>, Status> {
			let resp = self.resp.clone();
			// The spiffe crate opens two streams: the first for initial sync (reads one item and
			// closes), the second for the ongoing supervisor. Only give rotations to the second
			// connection so they reach the supervisor rather than the short-lived initial-sync stream.
			let conn = self.connection_count.fetch_add(1, Ordering::SeqCst);
			let tail: RespStream<X509svidResponse> = if conn >= 1 {
				match self.rotations.lock().unwrap().take() {
					Some(rx) => tokio_stream::wrappers::ReceiverStream::new(rx)
						.map(Ok::<_, Status>)
						.chain(futures::stream::pending())
						.boxed(),
					None => futures::stream::pending().boxed(),
				}
			} else {
				futures::stream::pending().boxed()
			};
			let stream = futures::stream::once(async move { Ok::<_, Status>(resp) }).chain(tail);
			Ok(Response::new(stream.boxed()))
		}
		type FetchX509BundlesStream = RespStream<X509BundlesResponse>;
		async fn fetch_x509_bundles(
			&self,
			_request: Request<X509BundlesRequest>,
		) -> Result<Response<Self::FetchX509BundlesStream>, Status> {
			Err(Status::unimplemented("not used in test"))
		}
		async fn fetch_jwtsvid(
			&self,
			_request: Request<JwtsvidRequest>,
		) -> Result<Response<JwtsvidResponse>, Status> {
			Err(Status::unimplemented("not used in test"))
		}
		type FetchJWTBundlesStream = RespStream<JwtBundlesResponse>;
		async fn fetch_jwt_bundles(
			&self,
			_request: Request<JwtBundlesRequest>,
		) -> Result<Response<Self::FetchJWTBundlesStream>, Status> {
			Err(Status::unimplemented("not used in test"))
		}
		async fn validate_jwtsvid(
			&self,
			_request: Request<ValidateJwtsvidRequest>,
		) -> Result<Response<ValidateJwtsvidResponse>, Status> {
			Err(Status::unimplemented("not used in test"))
		}
	}

	/// Spawn the fake Workload API on a fresh unix socket. Returns the temp dir (keep it alive for
	/// the socket's lifetime), the `unix://` endpoint, and the server task handle. Pass `rotations`
	/// to stream further SVIDs after the initial one (drive rotation); `None` just holds the stream
	/// open.
	async fn spawn_fake_workload_api(
		initial_response: X509svidResponse,
		rotations: Option<mpsc::Receiver<X509svidResponse>>,
	) -> (
		tempfile::TempDir,
		String,
		tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
	) {
		let dir = tempfile::tempdir().unwrap();
		let sock = dir.path().join("agent.sock");
		let listener = tokio::net::UnixListener::bind(&sock).unwrap();
		let incoming = tokio_stream::wrappers::UnixListenerStream::new(listener);
		let server = tonic::transport::Server::builder()
			.add_service(SpiffeWorkloadApiServer::new(FakeWorkloadApi {
				resp: initial_response,
				rotations: Mutex::new(rotations),
				connection_count: AtomicU32::new(0),
			}))
			.serve_with_incoming(incoming);
		let endpoint = format!("unix://{}", sock.display());
		let handle = tokio::spawn(server);
		(dir, endpoint, handle)
	}

	/// Build a single-SVID `X509SVIDResponse` (leaf + key + bundle, all DER).
	fn x509_svid_response(
		spiffe_id: &str,
		leaf: Vec<u8>,
		key: Vec<u8>,
		bundle: Vec<u8>,
	) -> X509svidResponse {
		X509svidResponse {
			svids: vec![X509svid {
				spiffe_id: spiffe_id.to_string(),
				x509_svid: leaf,
				x509_svid_key: key,
				bundle,
				hint: String::new(),
			}],
			crl: vec![],
			federated_bundles: Default::default(),
		}
	}

	/// Like `x509_svid_response`, but also advertises federated trust bundles (trust-domain name →
	/// bundle DER). The spiffe crate folds these into `X509Context::bundle_set()`, which is what a
	/// federated trust domain needs before it can be accepted.
	fn x509_svid_response_with_federated(
		spiffe_id: &str,
		leaf: Vec<u8>,
		key: Vec<u8>,
		bundle: Vec<u8>,
		federated: std::collections::HashMap<String, Vec<u8>>,
	) -> X509svidResponse {
		let mut resp = x509_svid_response(spiffe_id, leaf, key, bundle);
		resp.federated_bundles = federated;
		resp
	}

	/// Connect a `SpiffeClient` whose gateway SVID is issued by `local`, delivering the given
	/// `federated_bundles` and declaring `declared_federated` as the federation allow-list. Keep the
	/// returned `TempDir`/handle alive for the socket's lifetime.
	async fn connect_with_federated(
		spiffe_id: &str,
		local: &TestCa,
		federated_bundles: std::collections::HashMap<String, Vec<u8>>,
		declared_federated: Vec<String>,
	) -> (
		SpiffeClient,
		tempfile::TempDir,
		tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
	) {
		let gw = local.issue(spiffe_id);
		let resp = x509_svid_response_with_federated(
			spiffe_id,
			gw.leaf_der,
			gw.key_der,
			local.cert_der.clone(),
			federated_bundles,
		);
		let (dir, endpoint, handle) = spawn_fake_workload_api(resp, None).await;
		let client = SpiffeClient::new(endpoint, declared_federated)
			.await
			.expect("SpiffeClient should connect to the fake Workload API");
		(client, dir, handle)
	}

	/// Federation, inbound: a client SVID from an accepted+delivered federated trust domain is
	/// accepted, the local trust domain stays implicitly accepted, a federated CA cannot impersonate
	/// the local trust domain (SPIFFE Federation spec §7.3 — bundles are never pooled), and declaring
	/// a federated domain without accepting it does not accept its SVIDs.
	#[tokio::test]
	async fn spiffe_federation_ingress_accepts_and_isolates() {
		let local = TestCa::new(); // example.org — the gateway's own trust domain
		let fed = TestCa::new(); // federated.example
		let (client, _dir, handle) = connect_with_federated(
			"spiffe://example.org/ns/default/sa/gateway",
			&local,
			std::collections::HashMap::from([("federated.example".to_string(), fed.cert_der.clone())]),
			vec!["federated.example".to_string()],
		)
		.await;
		let ctx = client.source.x509_context().unwrap();
		let now = UnixTime::now();
		let accepted = normalize_trust_domains(&["federated.example".to_string()]).unwrap();
		let federated = accepted.clone();

		let fed_peer = CertificateDer::from(
			fed
				.issue("spiffe://federated.example/ns/default/sa/peer")
				.leaf_der,
		);
		let local_peer = CertificateDer::from(
			local
				.issue("spiffe://example.org/ns/default/sa/peer")
				.leaf_der,
		);

		let v = build_client_verifier(&ctx, &accepted, &federated, transport::tls::provider()).unwrap();
		assert!(
			v.verify_client_cert(&fed_peer, &[], now).is_ok(),
			"an accepted federated trust domain's SVID is accepted"
		);
		assert!(
			v.verify_client_cert(&local_peer, &[], now).is_ok(),
			"the local trust domain is always implicitly accepted"
		);
		// §7.3: dispatched to the local bundle by its claimed trust domain, a foreign-CA cert cannot chain.
		let imposter = CertificateDer::from(
			fed
				.issue("spiffe://example.org/ns/default/sa/victim")
				.leaf_der,
		);
		assert!(
			v.verify_client_cert(&imposter, &[], now).is_err(),
			"a federated CA must not be able to impersonate the local trust domain"
		);

		// Declared (allow-listed) but not accepted on this flow ⇒ still rejected.
		let local_only =
			build_client_verifier(&ctx, &[], &federated, transport::tls::provider()).unwrap();
		assert!(
			local_only.verify_client_cert(&fed_peer, &[], now).is_err(),
			"a federated SVID is rejected unless the trust domain is in the accepted list"
		);
		handle.abort();
	}

	/// A per-flow accepted trust domain that is not declared in `federatedTrustDomains` fails closed
	/// at config-build time rather than being silently ignored.
	#[tokio::test]
	async fn spiffe_federation_rejects_undeclared_accepted_domain() {
		let local = TestCa::new();
		let fed = TestCa::new();
		let (client, _dir, handle) = connect_with_federated(
			"spiffe://example.org/ns/default/sa/gateway",
			&local,
			std::collections::HashMap::from([("federated.example".to_string(), fed.cert_der.clone())]),
			vec!["federated.example".to_string()], // allow-list
		)
		.await;
		let ctx = client.source.x509_context().unwrap();
		// Accept "other.example", which is NOT in the declared allow-list.
		let accepted = normalize_trust_domains(&["other.example".to_string()]).unwrap();
		let federated = normalize_trust_domains(&["federated.example".to_string()]).unwrap();
		let err = build_client_verifier(&ctx, &accepted, &federated, transport::tls::provider())
			.expect_err("an undeclared accepted trust domain must fail");
		assert!(
			matches!(err, Error::TrustDomainNotFederated(_)),
			"got {err:?}"
		);
		handle.abort();
	}

	/// An accepted (and declared) federated trust domain whose bundle the Workload API has not
	/// delivered fails closed rather than serving without a way to verify it.
	#[tokio::test]
	async fn spiffe_federation_fails_closed_on_undelivered_bundle() {
		let local = TestCa::new();
		let (client, _dir, handle) = connect_with_federated(
			"spiffe://example.org/ns/default/sa/gateway",
			&local,
			std::collections::HashMap::new(), // nothing delivered
			vec!["federated.example".to_string()],
		)
		.await;
		let ctx = client.source.x509_context().unwrap();
		let accepted = normalize_trust_domains(&["federated.example".to_string()]).unwrap();
		let federated = accepted.clone();
		let err = build_client_verifier(&ctx, &accepted, &federated, transport::tls::provider())
			.expect_err("an accepted domain with no delivered bundle must fail closed");
		assert!(matches!(err, Error::MissingBundle(_)), "got {err:?}");
		handle.abort();
	}

	/// Federation, outbound: an upstream SVID from an accepted federated trust domain is accepted, a
	/// federated CA cannot impersonate the local trust domain, and SPIFFE-ID pinning still applies
	/// across trust domains.
	#[tokio::test]
	async fn spiffe_federation_egress_accepts_isolates_and_pins() {
		let local = TestCa::new();
		let fed = TestCa::new();
		let (client, _dir, handle) = connect_with_federated(
			"spiffe://example.org/ns/default/sa/gateway",
			&local,
			std::collections::HashMap::from([("federated.example".to_string(), fed.cert_der.clone())]),
			vec!["federated.example".to_string()],
		)
		.await;
		let ctx = client.source.x509_context().unwrap();
		let now = UnixTime::now();
		let sni = ServerName::try_from("federated.example").unwrap();
		let accepted = normalize_trust_domains(&["federated.example".to_string()]).unwrap();
		let federated = accepted.clone();

		let upstream_id = "spiffe://federated.example/ns/default/sa/upstream";
		let upstream = CertificateDer::from(fed.issue(upstream_id).leaf_der);

		// No pin: any SVID chaining to the federated bundle is accepted.
		let v = build_server_verifier(&ctx, vec![], &accepted, &federated).unwrap();
		assert!(v.verify_server_cert(&upstream, &[], &sni, &[], now).is_ok());
		// §7.3: a federated CA claiming the local trust domain is rejected.
		let imposter = CertificateDer::from(
			fed
				.issue("spiffe://example.org/ns/default/sa/upstream")
				.leaf_der,
		);
		assert!(
			v.verify_server_cert(&imposter, &[], &sni, &[], now)
				.is_err()
		);

		// Pinning a federated SPIFFE ID accepts the match and rejects a different federated ID.
		let pinned =
			build_server_verifier(&ctx, vec![upstream_id.to_string()], &accepted, &federated).unwrap();
		assert!(
			pinned
				.verify_server_cert(&upstream, &[], &sni, &[], now)
				.is_ok()
		);
		let other = CertificateDer::from(
			fed
				.issue("spiffe://federated.example/ns/default/sa/other")
				.leaf_der,
		);
		assert!(
			pinned
				.verify_server_cert(&other, &[], &sni, &[], now)
				.is_err()
		);
		handle.abort();
	}

	/// The high-level `server_config`/`client_config` builders accept a delivered+declared+accepted
	/// federated trust domain, and fail closed when it is declared+accepted but not delivered.
	#[tokio::test]
	async fn spiffe_federation_config_builders() {
		let local = TestCa::new();
		let fed = TestCa::new();
		let (client, _dir, handle) = connect_with_federated(
			"spiffe://example.org/ns/default/sa/gateway",
			&local,
			std::collections::HashMap::from([("federated.example".to_string(), fed.cert_der.clone())]),
			vec!["federated.example".to_string()],
		)
		.await;
		let alpns = vec![b"h2".to_vec()];
		client
			.server_config(alpns.clone(), vec!["federated.example".to_string()])
			.expect("server config should build for a delivered federated domain");
		client
			.client_config(alpns.clone(), vec![], vec!["federated.example".to_string()])
			.expect("client config should build for a delivered federated domain");

		// A gateway that declares+accepts a domain with no delivered bundle fails closed.
		let (client2, _dir2, handle2) = connect_with_federated(
			"spiffe://example.org/ns/default/sa/gateway",
			&local,
			std::collections::HashMap::new(),
			vec!["federated.example".to_string()],
		)
		.await;
		let err = client2
			.server_config(alpns, vec!["federated.example".to_string()])
			.expect_err("an undelivered federated bundle must fail closed");
		assert!(matches!(err, Error::MissingBundle(_)), "got {err:?}");
		handle.abort();
		handle2.abort();
	}

	/// End-to-end check of the dataplane SPIFFE path without a real SPIFFE Workload API provider: stand up a fake
	/// SPIFFE Workload API server over a unix socket (compiled only under `protos/spiffe-test-server`,
	/// enabled for tests), connect `SpiffeClient` to it, and confirm it reads the SPIFFE ID and builds
	/// both server and client rustls configs from the streamed SVID + bundle.
	#[tokio::test]
	async fn spiffe_client_builds_configs_from_fake_workload_api() {
		let spiffe_id = "spiffe://example.org/ns/default/sa/test";
		let ca = TestCa::new();
		let svid = ca.issue(spiffe_id);
		let (_dir, endpoint, handle) = spawn_fake_workload_api(
			x509_svid_response(spiffe_id, svid.leaf_der, svid.key_der, ca.cert_der),
			None,
		)
		.await;

		let client = SpiffeClient::new(endpoint, vec![])
			.await
			.expect("SpiffeClient should connect to the fake Workload API");

		assert_eq!(client.spiffe_id().as_deref(), Some(spiffe_id));
		let alpns = vec![b"h2".to_vec()];
		client
			.server_config(alpns.clone(), vec![])
			.expect("server config should build from the streamed SVID");
		client
			.client_config(alpns, vec![spiffe_id.to_string()], vec![])
			.expect("client config should build from the streamed SVID");

		handle.abort();
	}

	/// The verifier accepts identities that chain to the gateway's local trust domain bundle,
	/// rejects any cert signed by a foreign CA (in both directions), and enforces SPIFFE-ID pinning
	/// on the outbound path (empty pin list ⇒ accept any chaining SVID; non-empty ⇒ require a match).
	#[tokio::test]
	async fn spiffe_verifies_against_local_trust_domain_bundle() {
		let ca = TestCa::new(); // example.org — the gateway's own trust domain
		let own_id = "spiffe://example.org/ns/default/sa/gateway";
		let own = ca.issue(own_id);
		let (_dir, endpoint, _handle) = spawn_fake_workload_api(
			x509_svid_response(own_id, own.leaf_der, own.key_der, ca.cert_der.clone()),
			None,
		)
		.await;
		let client = SpiffeClient::new(endpoint, vec![])
			.await
			.expect("SpiffeClient should connect to the fake Workload API");

		let provider = transport::tls::provider();
		let ctx = client.source.x509_context().unwrap();
		let client_verifier = build_client_verifier(&ctx, &[], &[], provider).unwrap();
		let server_verifier = build_server_verifier(&ctx, vec![], &[], &[]).unwrap();
		let now = UnixTime::now();
		let sni = ServerName::try_from("example.org").unwrap();

		// An SVID signed by the local CA is accepted in both directions.
		let legit = CertificateDer::from(ca.issue("spiffe://example.org/ns/default/sa/peer").leaf_der);
		assert!(client_verifier.verify_client_cert(&legit, &[], now).is_ok());
		assert!(
			server_verifier
				.verify_server_cert(&legit, &[], &sni, &[], now)
				.is_ok()
		);

		// A cert signed by any other CA does not chain to the local bundle and is rejected.
		let foreign = CertificateDer::from(
			TestCa::new()
				.issue("spiffe://example.org/ns/default/sa/victim")
				.leaf_der,
		);
		assert!(
			client_verifier
				.verify_client_cert(&foreign, &[], now)
				.is_err(),
			"inbound: SVID signed by a foreign CA must be rejected"
		);
		assert!(
			server_verifier
				.verify_server_cert(&foreign, &[], &sni, &[], now)
				.is_err(),
			"outbound: SVID signed by a foreign CA must be rejected"
		);

		// SPIFFE-ID pinning: the matching ID is accepted, a valid but unpinned ID is rejected.
		let pinned = build_server_verifier(
			&ctx,
			vec!["spiffe://example.org/ns/default/sa/peer".to_string()],
			&[],
			&[],
		)
		.unwrap();
		assert!(
			pinned
				.verify_server_cert(&legit, &[], &sni, &[], now)
				.is_ok(),
			"the pinned SPIFFE ID is accepted"
		);
		let other = CertificateDer::from(
			ca.issue("spiffe://example.org/ns/default/sa/other")
				.leaf_der,
		);
		assert!(
			pinned
				.verify_server_cert(&other, &[], &sni, &[], now)
				.is_err(),
			"a valid SVID that is not in the pin list is rejected"
		);
	}

	/// A `tls: spiffe` HTTPS listener bound to `*.example.com`, mirroring `proxymock::simple_bind`
	/// but serving HTTPS and sourcing its serving identity from SPIFFE.
	fn spiffe_https_bind() -> types::agent::BindSnapshot {
		use crate::test_helpers::proxymock::{BIND_KEY, LISTENER_KEY};
		use crate::types::agent::{
			Bind, BindProtocol, BindSnapshot, Listener, ListenerProtocol, ListenerSet,
		};

		BindSnapshot {
			bind: Arc::new(Bind {
				key: BIND_KEY,
				address: "127.0.0.1:0".parse().unwrap(),
				protocol: BindProtocol::tls,
				tunnel_protocol: Default::default(),
				mode: Default::default(),
			}),
			listeners: Arc::new(ListenerSet::from_list([Listener {
				key: LISTENER_KEY,
				name: Default::default(),
				hostname: strng::new("*.example.com"),
				protocol: ListenerProtocol::HTTPS(crate::types::agent::ServerTLSConfig::spiffe(
					vec![b"h2".to_vec(), b"http/1.1".to_vec()],
					vec![],
				)),
			}])),
		}
	}

	/// A SPIFFE-sourced HTTPS listener always requires and verifies a client SVID (mutual TLS): a
	/// client presenting a valid SVID succeeds end-to-end, while a client presenting no certificate
	/// is rejected at the handshake.
	#[tokio::test]
	async fn spiffe_listener_requires_and_accepts_client_svid() {
		use crate::proxy::request_builder::RequestBuilder;
		use crate::test_helpers::proxymock::{
			BIND_KEY, basic_route, setup_proxy_test_with_spiffe, simple_mock,
		};

		// The harness parses a config (which reads env vars); hold the env lock so config::tests
		// that set env cannot race it.
		let _env = crate::config::lock_env_for_tests_async().await;

		let ca = TestCa::new();
		let gateway = ca.issue("spiffe://example.org/ns/default/sa/gateway");
		let client_svid = ca.issue("spiffe://example.org/ns/default/sa/client");

		let (_dir, endpoint, handle) = spawn_fake_workload_api(
			x509_svid_response(
				"spiffe://example.org/ns/default/sa/gateway",
				gateway.leaf_der,
				gateway.key_der,
				ca.cert_der.clone(),
			),
			None,
		)
		.await;
		let spiffe = Arc::new(
			SpiffeClient::new(endpoint, vec![])
				.await
				.expect("SpiffeClient should connect to the fake Workload API"),
		);

		let mock = simple_mock().await;
		let t = setup_proxy_test_with_spiffe("{}", Some(spiffe))
			.unwrap()
			.with_backend(*mock.address())
			.with_bind(spiffe_https_bind())
			.with_route(basic_route(*mock.address()));

		let root = ca.cert_pem.clone().into_bytes();

		// A client presenting a valid SVID (chains to the same CA) completes mutual TLS and routes.
		let io = t.serve_https_client_auth(
			BIND_KEY,
			Some("a.example.com"),
			root.clone(),
			Some((
				client_svid.leaf_pem.into_bytes(),
				client_svid.key_pem.into_bytes(),
			)),
		);
		let res = RequestBuilder::new(http::Method::GET, "http://a.example.com")
			.send(io)
			.await
			.expect("request presenting a valid client SVID should succeed");
		assert_eq!(res.status(), 200);

		// No client certificate: the SPIFFE listener always requires a client SVID, so it's rejected.
		let io = t.serve_https_client_auth(BIND_KEY, Some("a.example.com"), root, None);
		let res = RequestBuilder::new(http::Method::GET, "http://a.example.com")
			.send(io)
			.await;
		assert!(
			res.is_err(),
			"request without a client SVID must be rejected by the SPIFFE listener"
		);

		handle.abort();
	}

	/// A SPIFFE listener rejects a client that presents a certificate signed by a CA outside the
	/// gateway's trust domain bundle, even though a client certificate is offered.
	#[tokio::test]
	async fn spiffe_listener_rejects_foreign_client_cert() {
		use crate::proxy::request_builder::RequestBuilder;
		use crate::test_helpers::proxymock::{
			BIND_KEY, basic_route, setup_proxy_test_with_spiffe, simple_mock,
		};

		// See spiffe_listener_requires_and_accepts_client_svid: hold the env lock across the
		// harness's config parse.
		let _env = crate::config::lock_env_for_tests_async().await;

		let ca = TestCa::new();
		let gateway = ca.issue("spiffe://example.org/ns/default/sa/gateway");
		// A client SVID signed by a *different* CA that the gateway's bundle does not trust.
		let foreign_ca = TestCa::new();
		let foreign_client = foreign_ca.issue("spiffe://example.org/ns/default/sa/client");

		let (_dir, endpoint, handle) = spawn_fake_workload_api(
			x509_svid_response(
				"spiffe://example.org/ns/default/sa/gateway",
				gateway.leaf_der,
				gateway.key_der,
				ca.cert_der.clone(),
			),
			None,
		)
		.await;
		let spiffe = Arc::new(
			SpiffeClient::new(endpoint, vec![])
				.await
				.expect("SpiffeClient should connect to the fake Workload API"),
		);

		let mock = simple_mock().await;
		let t = setup_proxy_test_with_spiffe("{}", Some(spiffe))
			.unwrap()
			.with_backend(*mock.address())
			.with_bind(spiffe_https_bind())
			.with_route(basic_route(*mock.address()));

		// The client still trusts the gateway's CA (so the server side of the handshake succeeds),
		// but presents a cert signed by a foreign CA — the listener's verifier must reject it.
		let io = t.serve_https_client_auth(
			BIND_KEY,
			Some("a.example.com"),
			ca.cert_pem.clone().into_bytes(),
			Some((
				foreign_client.leaf_pem.into_bytes(),
				foreign_client.key_pem.into_bytes(),
			)),
		);
		let res = RequestBuilder::new(http::Method::GET, "http://a.example.com")
			.send(io)
			.await;
		assert!(
			res.is_err(),
			"a client cert signed by a CA outside the trust domain bundle must be rejected"
		);

		handle.abort();
	}

	/// End-to-end rotation: when the Workload API streams a fresh SVID, the source's sequence
	/// advances and `server_config` rebuilds from the new SVID rather than serving the stale cache.
	#[tokio::test]
	async fn spiffe_server_config_rebuilds_on_svid_rotation() {
		let spiffe_id = "spiffe://example.org/ns/default/sa/gateway";
		let ca = TestCa::new();
		let initial = ca.issue(spiffe_id);
		// This test drives rotation, so it wires up the rotation channel itself.
		let (tx, rx) = mpsc::channel(4);
		let (_dir, endpoint, handle) = spawn_fake_workload_api(
			x509_svid_response(
				spiffe_id,
				initial.leaf_der,
				initial.key_der,
				ca.cert_der.clone(),
			),
			Some(rx),
		)
		.await;

		let client = SpiffeClient::new(endpoint, vec![])
			.await
			.expect("SpiffeClient should connect to the fake Workload API");

		let alpns = vec![b"h2".to_vec()];
		let seq_before = client.source.updated().last();
		let cfg_before = client.server_config(alpns.clone(), vec![]).unwrap();
		// While the SVID is unchanged, the same key returns the cached config (same allocation).
		assert!(
			Arc::ptr_eq(
				&cfg_before,
				&client.server_config(alpns.clone(), vec![]).unwrap()
			),
			"an unchanged SVID should serve the cached config"
		);

		// Subscribe before triggering the rotation so the notification cannot be missed, then stream
		// a fresh SVID (new leaf) for the same identity, signed by the same CA.
		let mut updates = client.source.updated();
		let rotated = ca.issue(spiffe_id);
		tx.send(x509_svid_response(
			spiffe_id,
			rotated.leaf_der,
			rotated.key_der,
			ca.cert_der.clone(),
		))
		.await
		.expect("the rotation response should be accepted by the stream");

		tokio::time::timeout(Duration::from_secs(5), async {
			while client.source.updated().last() == seq_before {
				updates
					.changed()
					.await
					.expect("rotation update should not error");
			}
		})
		.await
		.expect("the source should observe the rotation within the timeout");

		let cfg_after = client.server_config(alpns, vec![]).unwrap();
		assert!(
			!Arc::ptr_eq(&cfg_before, &cfg_after),
			"server_config should rebuild from the rotated SVID rather than serve the stale cache"
		);
		assert_eq!(client.spiffe_id().as_deref(), Some(spiffe_id));

		handle.abort();
	}
}
