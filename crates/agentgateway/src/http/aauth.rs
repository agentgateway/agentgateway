//! AAuth (HTTP Message Signature) authentication policy.
//!
//! Verifies incoming requests per [draft-hardt-oauth-aauth-protocol] and
//! [draft-hardt-httpbis-signature-key]. Three signature-key schemes are supported:
//!
//! - **`hwk`** (Header Web Key): pseudonymous; the public key is inlined in the `Signature-Key`
//!   header. No identity claim — useful for abuse prevention without authentication.
//! - **`jwks_uri`**: identified; the verifier fetches the signer's metadata from
//!   `{id}/.well-known/{dwk}`, follows `jwks_uri`, and selects a key by `kid`. Establishes
//!   agent identity without an authorization token.
//! - **`jwt`** (priority): authorized; the `Signature-Key` carries a JWT whose `cnf.jwk` binds
//!   the HTTP-signing key (RFC 7800). The JWT itself is verified against the issuer's JWKS.
//!
//! [draft-hardt-oauth-aauth-protocol]: https://datatracker.ietf.org/doc/draft-hardt-oauth-aauth-protocol/
//! [draft-hardt-httpbis-signature-key]: https://datatracker.ietf.org/doc/draft-hardt-httpbis-signature-key/

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ::aauth::AAuthError;
use ::aauth::tokens::{
	decode_jwt_claims_unverified, decode_jwt_header, get_string_claim, validate_agent_token,
	validate_auth_token,
};
use ::cel::types::dynamic::DynamicType;
use http_message_sig::headers::{SignatureKey, parse_signature_key};
use http_message_sig::keys::ed25519::PublicKey;
use http_message_sig::keys::jwk::JWK;
use http_message_sig::keys::jwk_thumbprint::calculate_jwk_thumbprint;
use http_message_sig::signing::{SignatureScheme, resolve_hwk_public_key, verify_signature};
use macro_rules_attribute::apply;
use parking_lot::RwLock;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::http::{Body, PolicyResponse, Request};
use crate::proxy::httpproxy::PolicyClient;
use crate::proxy::{ProxyError, ProxyResponse, dtrace};
use crate::store::RequestPolicyTrait;
use crate::telemetry::log::RequestLog;
use crate::telemetry::metrics::{OutboundCallKind, OutboundCallSubtype};
use crate::*; // brings in JsonSchema (under `schema` feature), schema_de!, schema_enum!, etc.

#[cfg(test)]
#[path = "aauth_tests.rs"]
mod tests;

/// Captured by `dtrace::pol_result!` from the calling scope — referenced indirectly via
/// macro expansion, not directly in this file, so `dead_code` is the wrong lint.
const TRACE_POLICY_KIND: &str = "aauth";

/// Default cache TTL for JWKS documents.
const JWKS_CACHE_TTL: Duration = Duration::from_secs(300);

/// Default clock skew tolerance for `Signature-Input;created` (seconds).
const DEFAULT_TIMESTAMP_TOLERANCE: u64 = 60;

// ────────────────────────────────────────────────────────────────────────────
// Config (deserializable)
// ────────────────────────────────────────────────────────────────────────────

/// User-facing AAuth policy configuration. Parsed from YAML and turned into [`AAuth`] via
/// [`LocalAAuthConfig::try_build`].
#[apply(schema_de!)]
pub struct LocalAAuthConfig {
	/// Controls whether requests must carry a valid AAuth signature.
	#[serde(default)]
	pub mode: Mode,

	/// Minimum acceptable signature-key scheme.
	///
	/// The schemes are ordered by strength: `hwk` < `jwks` < `jwt`. A request signed with a
	/// stronger scheme always satisfies a weaker requirement.
	#[serde(default)]
	pub required_scheme: RequiredScheme,

	/// Maximum permitted clock skew in seconds between the signer's `created` timestamp and
	/// local time. Defaults to 60.
	#[serde(default = "default_timestamp_tolerance")]
	pub timestamp_tolerance: u64,

	/// Accept `http://` in JWT `iss` claims for agent and auth tokens.
	///
	/// Intended for local development against `http://localhost` only. Production deployments
	/// MUST leave this `false` — enabling it lets unauthenticated HTTP origins pose as AAuth
	/// servers.
	#[serde(default)]
	pub allow_insecure_http_issuer: bool,
}

fn default_timestamp_tolerance() -> u64 {
	DEFAULT_TIMESTAMP_TOLERANCE
}

/// Controls how missing or invalid signatures are handled.
#[apply(schema_enum!)]
#[cfg_attr(feature = "schema", schemars(rename = "AAuthMode"))]
#[derive(Default)]
pub enum Mode {
	/// Require a valid AAuth signature meeting `requiredScheme`. Reject otherwise.
	/// This is the default.
	#[default]
	Strict,
	/// Verify the signature if present, but allow unsigned requests through.
	/// Useful for incremental rollout — log first, enforce later.
	Optional,
	/// Verify the signature if present, but allow requests with invalid signatures through.
	/// Useful for extracting AAuth claims for observability without enforcing — verification
	/// outcome is still recorded in logs.
	Permissive,
}

/// Minimum scheme strength accepted by the policy.
///
/// Strength hierarchy (low → high): `hwk` < `jwks` < `agentJwt`. A request signed at
/// level N satisfies any requirement at level ≤ N.
///
/// **Naming note:** the policy values here (`hwk`, `jwks`, `agentJwt`) are policy-level
/// identifiers for *categories* of signing schemes, NOT the on-the-wire Signature-Key
/// scheme tokens defined by draft-hardt-httpbis-signature-key. The mapping is:
///
/// | Policy value | Accepted `Signature-Key` schemes on the wire        |
/// |--------------|------------------------------------------------------|
/// | `hwk`        | any (`hwk`, `jwks_uri`, or `jwt`)                    |
/// | `jwks`       | `jwks_uri` or `jwt` — the bare `jwks` token is NOT  |
/// |              | a valid wire scheme and is rejected by the verifier  |
/// | `agentJwt`   | `jwt` (with `typ=aa-agent+jwt` or `typ=aa-auth+jwt`) |
///
/// `agentJwt` is the strongest level we support today; an `aa-auth+jwt` (authorization
/// token) is accepted at this level because it embeds the same agent identity that
/// `aa-agent+jwt` does. The gateway does not currently issue or require auth tokens
/// (draft-hardt-oauth-aauth-protocol §6.6 `requirement=auth-token`); agent identity is
/// the only token-level guarantee enforced.
#[apply(schema_enum!)]
#[cfg_attr(feature = "schema", schemars(rename = "AAuthRequiredScheme"))]
#[derive(Default)]
pub enum RequiredScheme {
	/// Accepts any signed request — pseudonymous `hwk`, identified `jwks_uri`, or
	/// JWT-bound `jwt`.
	#[default]
	Hwk,
	/// Requires identified scheme or stronger. On the wire this means
	/// `Signature-Key: ...=jwks_uri;...` or `Signature-Key: ...=jwt;...` — the bare
	/// `scheme=jwks` (used by some pre-04 drafts) is not a valid wire form and is
	/// rejected by the verifier.
	Jwks,
	/// Requires a JWT-bound signing key (`Signature-Key: ...=jwt;jwt="..."`). The JWT's
	/// `typ` may be `aa-agent+jwt` (agent identity) or `aa-auth+jwt` (authorization);
	/// both satisfy this requirement because both carry verified agent identity.
	///
	/// On rejection, the gateway advertises `AAuth-Requirement: requirement=agent-token`
	/// per draft-hardt-oauth-aauth-protocol §6.3.
	AgentJwt,
}

impl RequiredScheme {
	/// Whether a verification result satisfies this requirement.
	fn allows(self, presented: &SignatureScheme) -> bool {
		use RequiredScheme::*;
		use SignatureScheme as S;
		matches!(
			(self, presented),
			(Hwk, _) | (Jwks, S::Jwks | S::Jwt) | (AgentJwt, S::Jwt)
		)
	}

	fn challenge_token(self) -> &'static str {
		match self {
			RequiredScheme::Hwk => "pseudonym",
			RequiredScheme::Jwks => "identity",
			RequiredScheme::AgentJwt => "agent-token",
		}
	}
}

// ────────────────────────────────────────────────────────────────────────────
// Runtime policy
// ────────────────────────────────────────────────────────────────────────────

/// Realized AAuth policy. Holds the verifier configuration and the JWKS cache.
///
/// JWKS fetches are performed via the per-request [`PolicyClient`] passed into
/// [`RequestPolicyTrait::apply`], so the policy itself does not hold a client handle.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AAuth {
	mode: Mode,
	required_scheme: RequiredScheme,
	timestamp_tolerance: u64,
	allow_insecure_http_issuer: bool,
	#[serde(skip)]
	#[allow(
		dead_code,
		reason = "exposed via the policy's apply path, not via serde"
	)]
	jwks_cache: JwksCache,
}

impl LocalAAuthConfig {
	pub fn try_build(self) -> AAuth {
		AAuth {
			mode: self.mode,
			required_scheme: self.required_scheme,
			timestamp_tolerance: self.timestamp_tolerance,
			allow_insecure_http_issuer: self.allow_insecure_http_issuer,
			jwks_cache: JwksCache::default(),
		}
	}
}

impl AAuth {
	#[cfg(test)]
	pub fn new(
		mode: Mode,
		required_scheme: RequiredScheme,
		timestamp_tolerance: u64,
		allow_insecure_http_issuer: bool,
	) -> Self {
		Self {
			mode,
			required_scheme,
			timestamp_tolerance,
			allow_insecure_http_issuer,
			jwks_cache: JwksCache::default(),
		}
	}

	/// Build the value of the `AAuth-Requirement` response header advertised on
	/// insufficient-scheme rejections, per draft-hardt-oauth-aauth-protocol §6.
	///
	/// The value is an RFC 8941 Structured Field Dictionary keyed by `requirement=<token>`.
	/// We only emit identity-level challenges (`pseudonym` / `identity` / `agent-token`);
	/// `requirement=auth-token` (which would also carry `resource-token` / `auth-server`
	/// parameters) is not supported because the gateway does not currently mint or
	/// require authorization tokens.
	pub fn build_challenge_response(&self) -> String {
		format!("requirement={}", self.required_scheme.challenge_token())
	}
}

// ────────────────────────────────────────────────────────────────────────────
// JWKS cache
// ────────────────────────────────────────────────────────────────────────────

/// Cache key for a fetched JWKS. The discovery-document name (`dwk`) is part of the key because
/// a single issuer can legitimately publish multiple discovery documents (e.g.
/// `aauth-agent.json` and `aauth-issuer.json`) that point to different JWKS URIs holding
/// different keys.
type JwksCacheKey = (String, String);

/// Per-issuer JWKS cache with single-flight semantics on a miss.
///
/// Cheap to clone — the inner state is shared across all references. The cache map is guarded
/// by a parking_lot `RwLock` for synchronous get/insert. The single-flight table sits behind a
/// parking_lot `Mutex` (very short critical section: insert-or-clone an `Arc`), with the actual
/// in-flight fetch serialized on a `tokio::sync::Mutex<()>` per key so async waiters don't block
/// runtime threads.
#[derive(Clone, Default)]
pub struct JwksCache {
	entries: Arc<RwLock<HashMap<JwksCacheKey, CachedJwks>>>,
	in_flight: Arc<RwLock<HashMap<JwksCacheKey, Arc<tokio::sync::Mutex<()>>>>>,
}

impl std::fmt::Debug for JwksCache {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "JwksCache(<runtime state>)")
	}
}

struct CachedJwks {
	keys: HashMap<String, JWK>,
	fetched_at: Instant,
}

impl JwksCache {
	pub fn get(&self, id: &str, dwk: &str, kid: &str) -> Option<JWK> {
		let key = (id.to_string(), dwk.to_string());
		// Fast path: warm cache hit.
		{
			let entries = self.entries.read();
			if let Some(cached) = entries.get(&key) {
				if cached.fetched_at.elapsed() <= JWKS_CACHE_TTL {
					return cached.keys.get(kid).cloned();
				}
			} else {
				return None;
			}
		}
		// Slow path: the entry is stale. Lazily evict it so the cache doesn't grow unbounded
		// across unique (id, dwk) pairs that have rolled past TTL. Re-check under the write
		// lock — a concurrent writer might have refreshed it in between, in which case we
		// fall through and pick up the fresh keys.
		let mut entries = self.entries.write();
		let still_stale = entries
			.get(&key)
			.map(|c| c.fetched_at.elapsed() > JWKS_CACHE_TTL)
			.unwrap_or(false);
		if still_stale {
			entries.remove(&key);
			return None;
		}
		entries.get(&key).and_then(|c| c.keys.get(kid).cloned())
	}

	pub fn insert(&self, id: &str, dwk: &str, keys: &[JWK]) {
		let mut entries = self.entries.write();
		let mut key_map = HashMap::with_capacity(keys.len());
		for jwk in keys {
			if let Some(kid) = &jwk.kid {
				key_map.insert(kid.clone(), jwk.clone());
			}
		}
		entries.insert(
			(id.to_string(), dwk.to_string()),
			CachedJwks {
				keys: key_map,
				fetched_at: Instant::now(),
			},
		);
	}

	/// Return (or create) the per-key fetch lock used to serialize concurrent cache misses.
	fn fetch_lock(&self, id: &str, dwk: &str) -> Arc<tokio::sync::Mutex<()>> {
		let key = (id.to_string(), dwk.to_string());
		// Fast path: lock already present.
		{
			let in_flight = self.in_flight.read();
			if let Some(lock) = in_flight.get(&key) {
				return lock.clone();
			}
		}
		// Slow path: insert a fresh lock (or take an existing one inserted concurrently).
		let mut in_flight = self.in_flight.write();
		in_flight
			.entry(key)
			.or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
			.clone()
	}

	fn remove_fetch_lock(&self, id: &str, dwk: &str) {
		self
			.in_flight
			.write()
			.remove(&(id.to_string(), dwk.to_string()));
	}

	/// Test-only: backdate an entry's `fetched_at` so the next `get` sees it as stale and
	/// evicts it. Used to exercise the lazy-eviction path without waiting out the real TTL.
	#[cfg(test)]
	pub(crate) fn backdate(&self, id: &str, dwk: &str, by: Duration) {
		let mut entries = self.entries.write();
		if let Some(cached) = entries.get_mut(&(id.to_string(), dwk.to_string())) {
			cached.fetched_at = cached
				.fetched_at
				.checked_sub(by)
				.unwrap_or(cached.fetched_at);
		}
	}

	#[cfg(test)]
	pub(crate) fn entry_count(&self) -> usize {
		self.entries.read().len()
	}
}

// ────────────────────────────────────────────────────────────────────────────
// Discovery wire types
// ────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct AgentMetadata {
	jwks_uri: String,
}

#[derive(Deserialize)]
struct JwksResponse {
	keys: Vec<JWK>,
}

// ────────────────────────────────────────────────────────────────────────────
// CEL-visible claims
// ────────────────────────────────────────────────────────────────────────────

/// AAuth claims exposed to CEL expressions as `aauth.*`.
///
/// Common fields populated by the policy:
/// - `scheme`: one of `hwk`, `jwks_uri`, `jwt`.
/// - `agent`: the agent identifier (jwks `id`, agent-token `iss`, auth-token `agent`).
/// - `agent_delegate`: the agent token's stable `sub` (agent+jwt only).
/// - `user`: the user the agent acts for (auth+jwt only).
/// - `scope`: array of granted scopes (auth+jwt only).
/// - `token_type`: `agent+jwt` or `auth+jwt`.
/// - `thumbprint`: RFC 7638 thumbprint of the signing key.
/// - `jwt_claims`: full JWT claims object for downstream policy decisions.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[cfg_attr(feature = "schema", schemars(with = "Map<String, Value>"))]
pub struct AAuthClaims {
	pub inner: Map<String, Value>,
}

impl DynamicType for AAuthClaims {
	fn auto_materialize(&self) -> bool {
		true
	}

	fn materialize(&self) -> cel::Value<'_> {
		self.inner.materialize()
	}

	fn field(&self, field: &str) -> Option<cel::Value<'_>> {
		self.inner.field(field)
	}
}

// ────────────────────────────────────────────────────────────────────────────
// Policy error
// ────────────────────────────────────────────────────────────────────────────

/// Outcome of a failed AAuth verification, used to shape the 401 response.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("invalid_signature: {description}")]
	InvalidSignature {
		description: String,
		required_components: Option<Vec<&'static str>>,
	},

	#[error("invalid_agent_token: {0}")]
	InvalidAgentToken(String),

	#[error("invalid_auth_token: {0}")]
	InvalidAuthToken(String),

	#[error("insufficient authentication level")]
	InsufficientLevel { challenge: String },
}

impl Error {
	fn invalid_signature(description: impl Into<String>) -> Self {
		Error::InvalidSignature {
			description: description.into(),
			required_components: None,
		}
	}

	fn invalid_signature_with_required(description: impl Into<String>) -> Self {
		Error::InvalidSignature {
			description: description.into(),
			required_components: Some(vec!["@method", "@authority", "@path", "signature-key"]),
		}
	}
}

// ────────────────────────────────────────────────────────────────────────────
// Apply (the actual policy logic)
// ────────────────────────────────────────────────────────────────────────────

impl AAuth {
	async fn apply_inner(&self, req: &mut Request, client: &PolicyClient) -> Result<(), Error> {
		// Pull all three signature headers up-front so we can short-circuit when none are present.
		let sig_key_header = req
			.headers()
			.get("signature-key")
			.and_then(|h| h.to_str().ok())
			.map(str::to_owned);
		let sig_input_header = req.headers().get("signature-input").is_some();
		let sig_header = req.headers().get("signature").is_some();
		let has_signature = sig_key_header.is_some() && sig_input_header && sig_header;

		if !has_signature {
			return match self.mode {
				Mode::Strict => Err(Error::invalid_signature("missing signature headers")),
				Mode::Optional | Mode::Permissive => Ok(()),
			};
		}

		let sig_key_str = sig_key_header.expect("checked above");
		let parsed_sig_key = parse_signature_key(&sig_key_str).map_err(|e| {
			tracing::debug!(error = %e, "AAuth: failed to parse signature-key header");
			Error::invalid_signature(e.to_string())
		})?;
		tracing::debug!(
			scheme = %parsed_sig_key.scheme,
			label = %parsed_sig_key.label,
			"AAuth: signature-key parsed"
		);

		// Build the request authority and the resource audience the JWT must target.
		//
		// `signed_authority` is what the client used as `@authority` when building the signature
		// base. Per RFC 9421 §2.2.3 and the AAuth profile, this is the request-line authority. We
		// use the request's URI authority if present, falling back to the `Host` header — this
		// matches what HTTP/1.1 (Host) and HTTP/2 (`:authority`) require.
		//
		// `audience` is what the request appeared to be addressed to (scheme + authority); auth+jwt
		// tokens must be issued for this exact resource.
		let scheme = req.uri().scheme().map(|s| s.as_str()).unwrap_or("https");
		let signed_authority = req
			.uri()
			.authority()
			.map(|a| a.as_str().to_owned())
			.or_else(|| {
				req
					.headers()
					.get(http::header::HOST)
					.and_then(|h| h.to_str().ok())
					.map(str::to_owned)
			})
			.ok_or_else(|| Error::invalid_signature("missing request authority"))?;
		let audience = format!("{}://{}", scheme, signed_authority);

		let url = format!(
			"{}://{}{}",
			scheme,
			signed_authority,
			req
				.uri()
				.path_and_query()
				.map(http::uri::PathAndQuery::as_str)
				.unwrap_or("")
		);

		// Resolve the signature-verifying public key based on the scheme.
		//
		// `prefetched_jwks_key` is populated for the `jwks_uri` scheme (after metadata + JWKS
		// fetch); `prefetched_jwt_key` is populated for the `jwt` scheme (extracted from the
		// JWT's `cnf.jwk` after JWT signature verification). The `hwk` scheme has nothing to
		// prefetch — its key is inlined in the Signature-Key header and resolved by the
		// verifier's closure below.
		let scheme_str = parsed_sig_key.scheme.as_str();
		let (prefetched_jwks_key, prefetched_jwt_key, jwt_context) = match scheme_str {
			"hwk" => (None, None, None),
			"jwks_uri" => {
				let id = parsed_sig_key
					.params
					.get("id")
					.ok_or_else(|| Error::invalid_signature("jwks_uri: missing 'id' parameter"))?;
				let kid = parsed_sig_key
					.params
					.get("kid")
					.ok_or_else(|| Error::invalid_signature("jwks_uri: missing 'kid' parameter"))?;
				let dwk = parsed_sig_key
					.params
					.get("dwk")
					.ok_or_else(|| Error::invalid_signature("jwks_uri: missing 'dwk' parameter"))?;
				validate_discovery_id(id, self.allow_insecure_http_issuer)?;
				validate_dwk(dwk)?;
				let jwk = self.get_jwks_key(client, id, kid, dwk).await?;
				let pubkey = jwk
					.to_ed25519_public_key()
					.map_err(|e| Error::invalid_signature(format!("jwks key is not OKP/Ed25519: {}", e)))?;
				(Some(pubkey), None, None)
			},
			"jwt" => {
				let (pubkey, context) = self
					.resolve_jwt_scheme(client, &parsed_sig_key, &audience)
					.await?;
				(None, Some(pubkey), Some(context))
			},
			other => {
				return Err(Error::invalid_signature(format!(
					"unsupported scheme: {}",
					other
				)));
			},
		};

		// Snapshot the request headers into a plain map for the verifier — http::HeaderMap
		// doesn't fit cleanly into the `&HashMap<String, String>` API.
		let header_map = snapshot_headers_for_signature(req.headers());

		// VerifyingKey is `Copy`, so move-by-value into the closure works without cloning.
		let jwks_key_for_resolver = prefetched_jwks_key;
		let jwt_key_for_resolver = prefetched_jwt_key;
		let resolver = move |sig_key: &SignatureKey| -> Result<PublicKey, http_message_sig::Error> {
			match sig_key.scheme.as_str() {
				"hwk" => resolve_hwk_public_key(sig_key),
				"jwks_uri" => jwks_key_for_resolver.ok_or_else(|| {
					http_message_sig::Error::InvalidKey("jwks_uri key not pre-resolved".to_string())
				}),
				"jwt" => jwt_key_for_resolver.ok_or_else(|| {
					http_message_sig::Error::InvalidKey("jwt key not pre-resolved".to_string())
				}),
				other => Err(http_message_sig::Error::UnsupportedScheme(
					other.to_string(),
				)),
			}
		};

		// The verifier rebuilds @authority from `url` by default. We override with the
		// request authority so the lower-level URL parsing doesn't strip a port we want signed.
		let verify_result = verify_signature(
			req.method().as_str(),
			&url,
			&header_map,
			None,
			self.timestamp_tolerance,
			&resolver,
			Some(signed_authority.as_str()),
		)
		.map_err(map_signing_error)?;

		// Strength check: the presented scheme must meet or exceed the required minimum.
		if !self.required_scheme.allows(&verify_result.scheme) {
			let challenge = self.build_challenge_response();
			return Err(Error::InsufficientLevel { challenge });
		}

		// Build claims for CEL / logging.
		let thumbprint = match jwt_context.as_ref() {
			Some(ctx) => calculate_jwk_thumbprint(&ctx.cnf_jwk).ok(),
			None => {
				signature_key_to_jwk(&parsed_sig_key).and_then(|jwk| calculate_jwk_thumbprint(&jwk).ok())
			},
		}
		.unwrap_or_default();

		let mut claims_map = Map::new();
		claims_map.insert(
			"scheme".to_string(),
			Value::String(scheme_name(&verify_result.scheme).to_owned()),
		);
		claims_map.insert("thumbprint".to_string(), Value::String(thumbprint));

		match jwt_context {
			Some(ctx) => {
				claims_map.insert("agent".to_string(), Value::String(ctx.agent_id));
				if let Some(delegate) = ctx.agent_delegate {
					claims_map.insert("agent_delegate".to_string(), Value::String(delegate));
				}
				if let Some(user) = ctx.user {
					claims_map.insert("user".to_string(), Value::String(user));
				}
				if let Some(scopes) = ctx.scopes {
					claims_map.insert(
						"scope".to_string(),
						Value::Array(scopes.into_iter().map(Value::String).collect()),
					);
				}
				claims_map.insert(
					"token_type".to_string(),
					Value::String(
						match ctx.kind {
							JwtKind::Agent => "agent+jwt",
							JwtKind::Auth => "auth+jwt",
						}
						.to_string(),
					),
				);
				claims_map.insert("jwt_claims".to_string(), Value::Object(ctx.claims));
			},
			None => {
				if let Some(agent) = verify_result.agent_id {
					claims_map.insert("agent".to_string(), Value::String(agent));
				}
			},
		}

		req
			.extensions_mut()
			.insert(AAuthClaims { inner: claims_map });
		Ok(())
	}

	/// Fetch a JWKS key, hitting the cache first.
	///
	/// On a miss, fetches `{id}/.well-known/{dwk}`, follows `jwks_uri`, populates the cache, and
	/// returns the matching key by `kid`. Returns an error if the metadata or JWKS can't be
	/// fetched, parsed, or doesn't contain the requested `kid`.
	///
	/// **Single-flight**: concurrent cache misses on the same `(id, dwk)` serialize behind a
	/// per-key async mutex so only one request fans out to the issuer; the rest re-check the
	/// (now warm) cache once they acquire the lock.
	///
	/// **Scheme guard**: `metadata.jwks_uri` is required to be `https://` unless
	/// `allow_insecure_http_issuer` is set. The metadata document itself is fetched over the
	/// issuer's own scheme, but its declared `jwks_uri` is the actual key-bearing URL, so an
	/// attacker (or compromised CDN) that injects an `http://attacker/jwks.json` would otherwise
	/// have the gateway accept arbitrary signing keys over plaintext.
	async fn get_jwks_key(
		&self,
		client: &PolicyClient,
		id: &str,
		kid: &str,
		dwk: &str,
	) -> Result<JWK, Error> {
		if let Some(jwk) = self.jwks_cache.get(id, dwk, kid) {
			return Ok(jwk);
		}

		// Single-flight: acquire (or create) a per-key lock and re-check the cache once we hold
		// it. If another concurrent request already populated the entry while we were waiting,
		// we return that without doing a duplicate network call.
		let fetch_lock = self.jwks_cache.fetch_lock(id, dwk);
		let _guard = fetch_lock.lock().await;
		if let Some(jwk) = self.jwks_cache.get(id, dwk, kid) {
			return Ok(jwk);
		}

		let result = async {
			let metadata_url = format!("{}/.well-known/{}", id.trim_end_matches('/'), dwk);
			let metadata: AgentMetadata = fetch_json(client, &metadata_url)
				.await
				.map_err(|e| Error::invalid_signature(format!("fetch metadata: {}", e)))?;

			validate_jwks_uri(&metadata.jwks_uri, id, self.allow_insecure_http_issuer)?;

			let jwks: JwksResponse = fetch_json(client, &metadata.jwks_uri)
				.await
				.map_err(|e| Error::invalid_signature(format!("fetch jwks: {}", e)))?;

			self.jwks_cache.insert(id, dwk, &jwks.keys);

			self.jwks_cache.get(id, dwk, kid).ok_or_else(|| {
				let available: Vec<&str> = jwks.keys.iter().filter_map(|k| k.kid.as_deref()).collect();
				tracing::info!(
					agent_id = id,
					requested_kid = kid,
					available_kids = ?available,
					"AAuth: kid not present in fetched JWKS"
				);
				Error::invalid_signature(format!("key {} not found in JWKS", kid))
			})
		}
		.await;
		self.jwks_cache.remove_fetch_lock(id, dwk);
		result
	}

	async fn resolve_jwt_scheme(
		&self,
		client: &PolicyClient,
		parsed_sig_key: &SignatureKey,
		audience: &str,
	) -> Result<(PublicKey, VerifiedJwtContext), Error> {
		let jwt = parsed_sig_key
			.params
			.get("jwt")
			.ok_or_else(|| Error::InvalidAuthToken("jwt scheme missing 'jwt' parameter".to_string()))?;

		let header = decode_jwt_header(jwt)
			.map_err(|e| Error::InvalidAuthToken(format!("invalid JWT header: {}", e)))?;
		let typ = header.typ.as_deref().unwrap_or("");
		let kid = header
			.kid
			.as_deref()
			.ok_or_else(|| Error::InvalidAuthToken("JWT missing 'kid' in header".to_string()))?;

		let unverified_claims = decode_jwt_claims_unverified(jwt)
			.map_err(|e| Error::InvalidAuthToken(format!("invalid JWT claims: {}", e)))?;
		let issuer = get_string_claim(&unverified_claims, "iss")
			.ok_or_else(|| Error::InvalidAuthToken("JWT missing 'iss' claim".to_string()))?;
		validate_discovery_id(&issuer, self.allow_insecure_http_issuer)
			.map_err(|e| Error::InvalidAuthToken(e.to_string()))?;

		// Each token type discovers its signing JWKS via a different well-known document name.
		let dwk = match typ {
			"aa-agent+jwt" => "aauth-agent.json",
			"aa-auth+jwt" => "aauth-issuer.json",
			other => {
				return Err(Error::InvalidAuthToken(format!(
					"unsupported JWT typ: {}",
					other
				)));
			},
		};

		let issuer_jwk = self
			.get_jwks_key(client, &issuer, kid, dwk)
			.await
			.map_err(|e| match typ {
				"aa-agent+jwt" => Error::InvalidAgentToken(e.to_string()),
				_ => Error::InvalidAuthToken(e.to_string()),
			})?;

		let context = match typ {
			"aa-agent+jwt" => {
				// Agent tokens are identity assertions ("I am agent X"); they are not bound to a
				// specific resource. Per the AAuth draft the audience belongs on auth tokens, and
				// the reference implementations omit `aud` from agent tokens. Pass None so a
				// missing/mismatched `aud` doesn't reject otherwise-valid agent identity tokens.
				let result = validate_agent_token(jwt, &issuer_jwk, None, self.allow_insecure_http_issuer)
					.map_err(|e| map_aauth_error(e, JwtKind::Agent))?;
				VerifiedJwtContext {
					kind: JwtKind::Agent,
					agent_id: result.agent_id,
					agent_delegate: Some(result.subject),
					user: None,
					scopes: None,
					claims: result.claims,
					cnf_jwk: result.cnf_jwk,
				}
			},
			"aa-auth+jwt" => {
				let result = validate_auth_token(
					jwt,
					&issuer_jwk,
					audience,
					None,
					self.allow_insecure_http_issuer,
				)
				.map_err(|e| map_aauth_error(e, JwtKind::Auth))?;
				VerifiedJwtContext {
					kind: JwtKind::Auth,
					agent_id: result.agent_id,
					agent_delegate: None,
					user: result.user_id,
					scopes: result.scopes,
					claims: result.claims,
					cnf_jwk: result.cnf_jwk,
				}
			},
			_ => unreachable!("typ validated above"),
		};

		let pubkey = context
			.cnf_jwk
			.to_ed25519_public_key()
			.map_err(|e| Error::InvalidAuthToken(format!("invalid cnf.jwk: {}", e)))?;
		Ok((pubkey, context))
	}
}

// ────────────────────────────────────────────────────────────────────────────
// Trait integration
// ────────────────────────────────────────────────────────────────────────────

impl RequestPolicyTrait for AAuth {
	async fn apply(
		&self,
		client: &PolicyClient,
		_log: &mut RequestLog,
		req: &mut Request,
	) -> Result<PolicyResponse, ProxyResponse> {
		match self.apply_inner(req, client).await {
			Ok(()) => {
				dtrace::pol_result!(dtrace::Info, Apply, "verified AAuth signature");
				Ok(PolicyResponse::default())
			},
			// Permissive mode reports the failure but lets the request through unauthenticated.
			// No claims are inserted because the signature wasn't validated.
			Err(e) if self.mode == Mode::Permissive => {
				dtrace::pol_result!(
					dtrace::Warn,
					Skip,
					"AAuth verification failed in permissive mode: {e}"
				);
				Ok(PolicyResponse::default())
			},
			Err(e) => {
				dtrace::pol_result!(dtrace::Error, Apply, "AAuth rejected request: {e}");
				let resp = render_error_response(e);
				Ok(PolicyResponse::default().with_response(resp))
			},
		}
	}
}

// ────────────────────────────────────────────────────────────────────────────
// Helpers
// ────────────────────────────────────────────────────────────────────────────

/// Which AAuth JWT type was carried in the `Signature-Key: sig=jwt;jwt=...` parameter.
/// Determined from the JWT's `typ` header during validation. Used only to populate the
/// `aauth.token_type` CEL claim; both kinds satisfy `RequiredScheme::AgentJwt` because
/// both carry verified agent identity.
enum JwtKind {
	Agent,
	Auth,
}

struct VerifiedJwtContext {
	kind: JwtKind,
	agent_id: String,
	agent_delegate: Option<String>,
	user: Option<String>,
	scopes: Option<Vec<String>>,
	claims: Map<String, Value>,
	cnf_jwk: JWK,
}

fn scheme_name(scheme: &SignatureScheme) -> &'static str {
	match scheme {
		SignatureScheme::Hwk => "hwk",
		SignatureScheme::Jwks => "jwks_uri",
		SignatureScheme::Jwt => "jwt",
	}
}

/// Convert a parsed Signature-Key into a JWK suitable for thumbprint calculation. Only the `hwk`
/// scheme carries enough material inline; other schemes return `None` and the caller falls back
/// to the cnf.jwk thumbprint.
fn signature_key_to_jwk(sig_key: &SignatureKey) -> Option<JWK> {
	if sig_key.scheme != "hwk" {
		return None;
	}
	Some(JWK {
		kty: sig_key.params.get("kty")?.clone(),
		crv: sig_key.params.get("crv").cloned(),
		x: sig_key.params.get("x").cloned(),
		y: sig_key.params.get("y").cloned(),
		d: None,
		n: sig_key.params.get("n").cloned(),
		e: sig_key.params.get("e").cloned(),
		kid: sig_key.params.get("kid").cloned(),
		alg: None,
		extra: serde_json::Map::new(),
	})
}

fn snapshot_headers_for_signature(headers: &::http::HeaderMap) -> HashMap<String, String> {
	let mut header_map = HashMap::with_capacity(headers.len());
	for (name, value) in headers {
		if let Ok(s) = value.to_str() {
			header_map
				.entry(name.as_str().to_owned())
				.and_modify(|existing: &mut String| {
					existing.push_str(", ");
					existing.push_str(s.trim());
				})
				.or_insert_with(|| s.trim().to_owned());
		}
	}
	header_map
}

/// Translate an `http_message_sig::Error` into the policy-level [`Error`] enum, preserving any
/// required-component hints so the response body can name the missing components.
fn map_signing_error(error: http_message_sig::Error) -> Error {
	use http_message_sig::Error as E;
	match &error {
		E::InvalidSignature(s) if s.contains("missing required component") => {
			Error::invalid_signature_with_required(error.to_string())
		},
		E::SignatureKeyNotCovered => Error::invalid_signature_with_required(error.to_string()),
		E::ContentDigestMismatch => Error::invalid_signature(error.to_string()),
		_ => Error::invalid_signature(error.to_string()),
	}
}

/// Map an `AAuthError` from the underlying token validation crate into a policy-level [`Error`]
/// that knows whether it should surface as an agent-token or auth-token problem.
fn map_aauth_error(error: AAuthError, kind: JwtKind) -> Error {
	match kind {
		JwtKind::Agent => Error::InvalidAgentToken(error.to_string()),
		JwtKind::Auth => Error::InvalidAuthToken(error.to_string()),
	}
}

/// Build the 401 response for a rejected AAuth request.
fn render_error_response(error: Error) -> ::http::Response<Body> {
	use ::http::{Response as HttpResponse, StatusCode};
	use serde_json::json;

	let (status, body, extra_header): (StatusCode, Value, Option<(&'static str, String)>) =
		match error {
			Error::InsufficientLevel { challenge } => (
				StatusCode::UNAUTHORIZED,
				json!({}),
				// `AAuth-Requirement` is the spec-defined challenge header
				// (draft-hardt-oauth-aauth-protocol §6); the value is an RFC 8941
				// Structured Field Dictionary built by `build_challenge_response`.
				Some(("AAuth-Requirement", challenge)),
			),
			Error::InvalidSignature {
				description,
				required_components,
			} => {
				let mut body = json!({
					"error": "invalid_signature",
					"error_description": description,
				});
				if let Some(rc) = required_components {
					body["required_components"] = json!(rc);
				}
				(StatusCode::UNAUTHORIZED, body, None)
			},
			Error::InvalidAgentToken(description) => (
				StatusCode::UNAUTHORIZED,
				json!({
					"error": "invalid_agent_token",
					"error_description": description,
				}),
				None,
			),
			Error::InvalidAuthToken(description) => (
				StatusCode::UNAUTHORIZED,
				json!({
					"error": "invalid_auth_token",
					"error_description": description,
				}),
				None,
			),
		};

	let body_bytes = body.to_string();
	let mut builder = HttpResponse::builder()
		.status(status)
		.header(::http::header::CONTENT_TYPE, "application/json");
	if let Some((name, value)) = extra_header {
		builder = builder.header(name, value);
	}
	builder
		.body(Body::from(body_bytes))
		.expect("response builder fields are static")
}

/// Reject a `jwks_uri` value that would have keys served over plaintext HTTP.
///
/// Even if the issuer URL is HTTPS, the metadata document it returns is what tells us where the
/// JWKS lives — a compromised issuer (or CDN serving the well-known doc) can inject
/// `http://attacker/jwks.json` and downgrade the transport for the actual signing keys.
///
/// When `allow_insecure_http` is set, plaintext is only accepted for loopback hosts (`localhost`,
/// `127.0.0.0/8`, `::1`). The dev-mode flag is documented as enabling local testing; this
/// enforces that documented boundary so the same flag can't be exploited to point at an external
/// HTTP host.
fn validate_jwks_uri(
	jwks_uri: &str,
	metadata_issuer: &str,
	allow_insecure_http: bool,
) -> Result<(), Error> {
	let parsed = url::Url::parse(jwks_uri)
		.map_err(|e| Error::invalid_signature(format!("invalid jwks_uri {:?}: {}", jwks_uri, e)))?;
	validate_fetch_url_admission(&parsed, allow_insecure_http, "jwks_uri")?;
	validate_same_origin(jwks_uri, metadata_issuer)?;
	Ok(())
}

fn validate_discovery_id(id: &str, allow_insecure_http: bool) -> Result<(), Error> {
	let parsed = url::Url::parse(id)
		.map_err(|e| Error::invalid_signature(format!("invalid discovery id {:?}: {}", id, e)))?;
	validate_fetch_url_admission(&parsed, allow_insecure_http, "discovery id")?;
	if !parsed.username().is_empty() || parsed.password().is_some() {
		return Err(Error::invalid_signature(
			"discovery id must not contain userinfo",
		));
	}
	if parsed.query().is_some() || parsed.fragment().is_some() {
		return Err(Error::invalid_signature(
			"discovery id must not contain query or fragment",
		));
	}
	let path = parsed.path();
	if !path.is_empty() && path != "/" {
		return Err(Error::invalid_signature(
			"discovery id must not contain a path",
		));
	}
	Ok(())
}

fn validate_fetch_url_admission(
	parsed: &url::Url,
	allow_insecure_http: bool,
	field: &str,
) -> Result<(), Error> {
	match parsed.scheme() {
		"https" => {
			reject_special_ip_hosts(parsed, field)?;
			Ok(())
		},
		"http" if allow_insecure_http => {
			if is_loopback_host(parsed) {
				Ok(())
			} else {
				Err(Error::invalid_signature(format!(
					"{field} http:// is only allowed for loopback hosts under allowInsecureHttpIssuer; got {}",
					parsed.host_str().unwrap_or("(none)"),
				)))
			}
		},
		other => Err(Error::invalid_signature(format!(
			"{field} must use https (or http loopback when allowInsecureHttpIssuer=true); got {}",
			other
		))),
	}
}

fn validate_same_origin(jwks_uri: &str, metadata_issuer: &str) -> Result<(), Error> {
	let jwks = url::Url::parse(jwks_uri)
		.map_err(|e| Error::invalid_signature(format!("invalid jwks_uri {:?}: {}", jwks_uri, e)))?;
	let issuer = url::Url::parse(metadata_issuer).map_err(|e| {
		Error::invalid_signature(format!(
			"invalid metadata issuer {:?}: {}",
			metadata_issuer, e
		))
	})?;
	if jwks.scheme() == issuer.scheme()
		&& jwks.host_str() == issuer.host_str()
		&& jwks.port_or_known_default() == issuer.port_or_known_default()
	{
		Ok(())
	} else {
		Err(Error::invalid_signature(
			"cross-origin jwks_uri is not allowed without explicit deployment admission",
		))
	}
}

fn validate_dwk(dwk: &str) -> Result<(), Error> {
	if dwk.is_empty()
		|| dwk == "."
		|| dwk == ".."
		|| dwk.contains('/')
		|| dwk.contains('\\')
		|| dwk.contains('?')
		|| dwk.contains('#')
		|| dwk.contains('%')
	{
		return Err(Error::invalid_signature(
			"dwk must be a single .well-known document name",
		));
	}
	Ok(())
}

/// Whether a parsed URL's host is a loopback address: `localhost`, an IPv4 address in
/// 127.0.0.0/8, or the IPv6 address `::1`.
fn is_loopback_host(parsed: &url::Url) -> bool {
	match parsed.host() {
		Some(url::Host::Domain(d)) => d == "localhost",
		Some(url::Host::Ipv4(addr)) => addr.is_loopback(),
		Some(url::Host::Ipv6(addr)) => addr.is_loopback(),
		None => false,
	}
}

fn reject_special_ip_hosts(parsed: &url::Url, field: &str) -> Result<(), Error> {
	let Some(host) = parsed.host() else {
		return Err(Error::invalid_signature(format!(
			"{field} must include a host"
		)));
	};
	let ip = match host {
		url::Host::Ipv4(addr) => IpAddr::V4(addr),
		url::Host::Ipv6(addr) => IpAddr::V6(addr),
		url::Host::Domain(_) => return Ok(()),
	};
	if ip.is_loopback()
		|| ip.is_unspecified()
		|| ip.is_multicast()
		|| match ip {
			IpAddr::V4(addr) => {
				addr.is_private()
					|| addr.is_link_local()
					|| addr.octets()[0] == 169 && addr.octets()[1] == 254
			},
			IpAddr::V6(addr) => {
				addr.is_unique_local()
					|| addr.is_unicast_link_local()
					|| addr.segments()[0] & 0xffc0 == 0xfe80
			},
		} {
		return Err(Error::invalid_signature(format!(
			"{field} host must not be private, loopback, link-local, multicast, or unspecified"
		)));
	}
	Ok(())
}

/// Fetch a JSON document over the policy client.
async fn fetch_json<T: serde::de::DeserializeOwned>(
	client: &PolicyClient,
	url: &str,
) -> Result<T, ProxyError> {
	let req = ::http::Request::builder()
		.uri(url)
		.body(Body::empty())
		.map_err(|e| ProxyError::ProcessingString(format!("failed to build JWKS request: {}", e)))?;

	let resp = client
		.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::AAuth)
		.simple_call(req)
		.await?;
	let status = resp.status();
	if !status.is_success() {
		return Err(ProxyError::ProcessingString(format!(
			"JWKS endpoint {} returned status {}",
			url, status
		)));
	}
	crate::json::from_response_body::<T>(resp)
		.await
		.map_err(|e| ProxyError::ProcessingString(format!("failed to decode JSON from {}: {}", url, e)))
}
