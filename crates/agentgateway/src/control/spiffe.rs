//! SPIFFE Workload API integration.
//!
//! [`SpiffeClient`] wraps a [`X509Source`], which connects to the local SPIFFE Workload API
//! endpoint and keeps the gateway's X.509-SVID and trust bundles up to date in the background,
//! rotating them automatically.
//!
//! From the current SVID (the cert chain + private key) it builds, on demand, both a
//! [`ServerConfig`] for terminating TLS on listeners and a [`ClientConfig`] for outbound mTLS to
//! upstream backends.
use ::spiffe::{X509Context, X509Source, X509SourceUpdates, X509Svid};
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
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ServerConfigKey {
	alpns: Vec<Vec<u8>>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ClientConfigCacheKey {
	alpns: Vec<Vec<u8>>,
	verify_sans: Vec<String>,
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
}

impl std::fmt::Debug for SpiffeClient {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("SpiffeClient").finish_non_exhaustive()
	}
}

impl SpiffeClient {
	/// Connects to the SPIFFE Workload API and performs the initial SVID/bundle sync.
	pub async fn new(endpoint: String) -> Result<Self, Error> {
		info!(endpoint = %endpoint, "connecting to SPIFFE workload API");
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

	/// Builds (or returns a cached) `rustls::ServerConfig` from the current SVID and trust bundle.
	///
	/// Incoming connections must present a client SVID that chains to the gateway's own trust domain
	/// bundle (mutual TLS is always required). Only the local trust domain is accepted; SPIFFE
	/// federation across trust domains is not supported.
	/// Use the `source.spiffeId` CEL field for applying further restrictions.
	pub fn server_config(&self, alpns: Vec<Vec<u8>>) -> Result<Arc<ServerConfig>, Error> {
		let seq = self.updates.last();
		let key = ServerConfigKey {
			alpns: alpns.clone(),
		};

		if let Some(cfg) = self.server_cache.lock().unwrap().get(seq, &key) {
			return Ok(cfg);
		}

		let cfg = Arc::new(self.build_server_config(alpns)?);
		Ok(self.server_cache.lock().unwrap().insert(seq, key, cfg))
	}

	fn build_server_config(&self, alpns: Vec<Vec<u8>>) -> Result<ServerConfig, Error> {
		let ctx = self.source.x509_context()?;
		let provider = transport::tls::provider();
		// Verify inbound client SVIDs against the gateway's local trust domain bundle.
		let verifier = build_client_verifier(&ctx, provider.clone())?;
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
	/// certificate is verified against the gateway's local trust domain bundle; when `verify_sans` is
	/// empty any SVID chaining to the bundle is accepted (DNS hostname checks do not apply to SPIFFE
	/// SVIDs), otherwise the upstream's SPIFFE ID must match one of the provided `spiffe://` URIs.
	pub fn client_config(
		&self,
		alpns: Vec<Vec<u8>>,
		verify_sans: Vec<String>,
	) -> Result<Arc<ClientConfig>, Error> {
		// Sequence sampled before the SVID read; see server_config.
		let seq = self.updates.last();
		let key = ClientConfigCacheKey {
			alpns: alpns.clone(),
			verify_sans: verify_sans.clone(),
		};

		if let Some(cfg) = self.client_cache.lock().unwrap().get(seq, &key) {
			return Ok(cfg);
		}

		let cfg = Arc::new(self.build_client_config(alpns, verify_sans)?);
		Ok(self.client_cache.lock().unwrap().insert(seq, key, cfg))
	}

	fn build_client_config(
		&self,
		alpns: Vec<Vec<u8>>,
		verify_sans: Vec<String>,
	) -> Result<ClientConfig, Error> {
		let ctx = self.source.x509_context()?;
		let provider = transport::tls::provider();
		let sans_count = verify_sans.len();
		let verifier = build_server_verifier(&ctx, verify_sans)?;
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

/// Builds a `RootCertStore` from the gateway's own trust domain bundle, federated bundles are ignored
fn local_roots(ctx: &X509Context, purpose: &str) -> Result<Arc<rustls::RootCertStore>, Error> {
	let svid = snapshot_svid(ctx)?;
	let td = svid.spiffe_id().trust_domain();
	let bundle = ctx.bundle_set().get(td).ok_or(Error::EmptyBundle)?;
	let mut roots = rustls::RootCertStore::empty();
	for authority in bundle.authorities() {
		roots
			.add(CertificateDer::from(authority.as_bytes().to_vec()))
			.map_err(Error::Rustls)?;
	}
	if roots.is_empty() {
		return Err(Error::EmptyBundle);
	}
	debug!(trust_domain = %td, purpose, "loaded SPIFFE trust bundle");
	Ok(Arc::new(roots))
}

fn build_client_verifier(
	ctx: &X509Context,
	provider: Arc<rustls::crypto::CryptoProvider>,
) -> Result<Arc<dyn ClientCertVerifier>, Error> {
	let roots = local_roots(ctx, "client certificate verification")?;
	let verifier =
		rustls::server::WebPkiClientVerifier::builder_with_provider(roots, provider).build()?;
	Ok(verifier)
}

fn build_server_verifier(
	ctx: &X509Context,
	verify_sans: Vec<String>,
) -> Result<Arc<dyn ServerCertVerifier>, Error> {
	// Verify the upstream SVID against the gateway's local trust domain bundle, then optionally
	// pin its SPIFFE ID. Note that SPIFFE SVIDs carry a `spiffe://` URI SAN and no DNS SAN, so
	// standard WebPKI hostname verification does not apply.

	let roots = local_roots(ctx, "upstream server verification")?;
	let verifier: Arc<dyn ServerCertVerifier> = if verify_sans.is_empty() {
		// No SPIFFE ID pinned: accept any SVID that chains to the bundle.
		let inner = rustls::client::WebPkiServerVerifier::builder_with_provider(
			roots,
			transport::tls::provider(),
		)
		.build()?;
		Arc::new(transport::tls::insecure::NoServerNameVerification::new(
			inner,
		))
	} else {
		let alt_names = verify_sans
			.iter()
			.cloned()
			.map(transport::tls::ExtendedServerName::try_from)
			.collect::<Result<Box<[_]>, _>>()
			.map_err(|e| Error::InvalidSan(e.to_string()))?;
		Arc::new(transport::tls::insecure::AltHostnameVerifier::new(
			roots, alt_names,
		))
	};
	Ok(verifier)
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

		let client = SpiffeClient::new(endpoint)
			.await
			.expect("SpiffeClient should connect to the fake Workload API");

		assert_eq!(client.spiffe_id().as_deref(), Some(spiffe_id));
		let alpns = vec![b"h2".to_vec()];
		client
			.server_config(alpns.clone())
			.expect("server config should build from the streamed SVID");
		client
			.client_config(alpns, vec![spiffe_id.to_string()])
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
		let client = SpiffeClient::new(endpoint)
			.await
			.expect("SpiffeClient should connect to the fake Workload API");

		let provider = transport::tls::provider();
		let ctx = client.source.x509_context().unwrap();
		let client_verifier = build_client_verifier(&ctx, provider).unwrap();
		let server_verifier = build_server_verifier(&ctx, vec![]).unwrap();
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
				protocol: ListenerProtocol::HTTPS(crate::types::agent::ServerTLSConfig::spiffe(vec![
					b"h2".to_vec(),
					b"http/1.1".to_vec(),
				])),
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
			SpiffeClient::new(endpoint)
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
			SpiffeClient::new(endpoint)
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

		let client = SpiffeClient::new(endpoint)
			.await
			.expect("SpiffeClient should connect to the fake Workload API");

		let alpns = vec![b"h2".to_vec()];
		let seq_before = client.source.updated().last();
		let cfg_before = client.server_config(alpns.clone()).unwrap();
		// While the SVID is unchanged, the same key returns the cached config (same allocation).
		assert!(
			Arc::ptr_eq(&cfg_before, &client.server_config(alpns.clone()).unwrap()),
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

		let cfg_after = client.server_config(alpns).unwrap();
		assert!(
			!Arc::ptr_eq(&cfg_before, &cfg_after),
			"server_config should rebuild from the rotated SVID rather than serve the stale cache"
		);
		assert_eq!(client.spiffe_id().as_deref(), Some(spiffe_id));

		handle.abort();
	}
}
