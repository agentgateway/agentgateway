use std::hash::Hash;

use ::cel::Value;
use macro_rules_attribute::apply;
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};

use crate::http::Request;
use crate::http::auth::AuthorizationLocation;
use crate::proxy::dtrace::{self, pol_result};
use crate::proxy::{ProxyError, ProxyResponse};
use crate::*;

#[cfg(test)]
#[path = "apikey_tests.rs"]
mod tests;

const TRACE_POLICY_KIND: &str = "api_key";

/// SHA-256 digest format must stay byte-identical to the `sha256.encode` CEL function
/// (lowercase hex) so digests are interchangeable between the native field and the
/// `location.expression` workaround.
const SHA256_PREFIX: &str = "sha256:";

/// bcrypt entries are verified by a linear scan running the intentionally-slow bcrypt
/// KDF (each digest embeds its own salt, so no O(1) lookup is possible). Past this count,
/// per-request latency degrades; sha256/plaintext entries are unaffected.
const MAX_BCRYPT_ENTRIES: usize = 100;

#[derive(thiserror::Error, Debug)]
pub enum Error {
	#[error("no API Key found")]
	Missing,

	#[error("invalid credentials")]
	InvalidCredentials,
}

/// Validation mode for API key authentication.
#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "APIKeyMode"))]
#[derive(Copy, PartialEq, Eq, Default)]
pub enum Mode {
	/// Require a valid API key.
	Strict,
	/// Validate the API key when present.
	/// This is the default option.
	/// Warning: this allows requests without an API key.
	#[default]
	Optional,
	/// Decode valid API keys for later policy use.
	/// Warning: this allows requests with missing or invalid API keys.
	Permissive,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")] // Intentionally NOT deny_unknown_fields since we use flatten
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(::cel::DynamicType)]
pub struct Claims {
	/// The API key value. Redacted by default; use `apiKey.key.unredacted()` to access the actual value.
	#[dynamic(with_value = "api_key_to_value")]
	pub key: APIKey,
	#[serde(default, flatten)]
	#[dynamic(flatten)]
	pub metadata: UserMetadata,
}

#[apply(schema!)]
pub struct APIKey(
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	#[serde(serialize_with = "ser_redact", deserialize_with = "deser_key")]
	SecretString,
);

impl APIKey {
	pub fn new(s: impl Into<Box<str>>) -> Self {
		APIKey(SecretString::new(s.into()))
	}
}

pub fn api_key_to_value<'a>(key: &'a APIKey) -> Value<'a> {
	crate::cel::secret_string_to_value(&key.0)
}

type UserMetadata = serde_json::Value;

impl Hash for APIKey {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		self.0.expose_secret().hash(state);
	}
}

impl PartialEq for APIKey {
	fn eq(&self, other: &Self) -> bool {
		self.0.expose_secret() == other.0.expose_secret()
	}
}

impl Eq for APIKey {}

fn sha256_hex(raw: &str) -> String {
	hex::encode(Sha256::digest(raw.as_bytes()))
}

fn is_bcrypt(s: &str) -> bool {
	s.starts_with("$2a$") || s.starts_with("$2b$") || s.starts_with("$2y$")
}

enum StoredKey {
	Plain(String),
	Sha256Hex(String),
	Bcrypt(String),
}

fn parse_stored_key(raw: &str) -> StoredKey {
	if let Some(hexpart) = raw.strip_prefix(SHA256_PREFIX) {
		let normalized = hexpart.to_ascii_lowercase();
		if normalized.len() == 64 && normalized.bytes().all(|b| b.is_ascii_hexdigit()) {
			return StoredKey::Sha256Hex(normalized);
		}
		warn!("ignoring malformed sha256 API key digest; falling back to plaintext comparison");
		return StoredKey::Plain(raw.to_string());
	}
	if is_bcrypt(raw) {
		return StoredKey::Bcrypt(raw.to_string());
	}
	StoredKey::Plain(raw.to_string())
}

#[derive(Debug, Clone)]
struct SaltedEntry {
	hash: String,
	metadata: UserMetadata,
}

#[apply(schema_ser!)]
pub struct APIKeyAuthentication {
	/// Exact-match lookup keyed by a deterministic token: the raw key for plaintext
	/// entries, or the lowercase-hex SHA-256 digest for sha256 entries.
	#[serde(serialize_with = "ser_redact")]
	pub users: Arc<HashMap<APIKey, UserMetadata>>,

	/// bcrypt entries, scanned linearly because each digest embeds its own salt.
	#[serde(skip)]
	salted: Arc<Vec<SaltedEntry>>,

	/// Validation mode for API Key authentication
	pub mode: Mode,

	#[serde(default)]
	pub location: AuthorizationLocation,
}

impl APIKeyAuthentication {
	pub fn new(
		keys: impl IntoIterator<Item = (APIKey, UserMetadata)>,
		mode: Mode,
		location: AuthorizationLocation,
	) -> Self {
		let mut exact = HashMap::new();
		let mut salted = Vec::new();
		for (key, metadata) in keys {
			match parse_stored_key(key.0.expose_secret()) {
				StoredKey::Plain(token) => {
					exact.insert(APIKey::new(token), metadata);
				},
				StoredKey::Sha256Hex(digest) => {
					exact.insert(APIKey::new(digest), metadata);
				},
				StoredKey::Bcrypt(hash) => salted.push(SaltedEntry { hash, metadata }),
			}
		}
		if salted.len() > MAX_BCRYPT_ENTRIES {
			warn!(
				count = salted.len(),
				limit = MAX_BCRYPT_ENTRIES,
				"large number of bcrypt API keys; each request scans them linearly, degrading latency"
			);
		}
		Self {
			users: Arc::new(exact),
			salted: Arc::new(salted),
			mode,
			location,
		}
	}

	fn lookup(&self, presented: &str) -> Option<UserMetadata> {
		if let Some(meta) = self.users.get(&APIKey::new(presented)) {
			return Some(meta.clone());
		}
		if let Some(meta) = self.users.get(&APIKey::new(sha256_hex(presented))) {
			return Some(meta.clone());
		}
		self
			.salted
			.iter()
			.find(|entry| bcrypt::verify(presented, &entry.hash).unwrap_or(false))
			.map(|entry| entry.metadata.clone())
	}

	async fn verify(&self, req: &mut Request) -> Result<Option<Claims>, ProxyError> {
		let Some(key) = self.location.extract(req) else {
			// In strict mode, we require credentials
			if self.mode == Mode::Strict {
				pol_result!(
					dtrace::Error,
					Apply,
					"rejected request because API key is required but missing"
				);
				return Err(ProxyError::APIKeyAuthenticationFailure(Error::Missing));
			}
			// Otherwise without credentials, don't attempt to authenticate
			pol_result!(
				dtrace::Info,
				Skip,
				"request has no API key and auth mode is not strict"
			);
			return Ok(None);
		};

		if let Some(metadata) = self.lookup(key.as_ref()) {
			pol_result!(
				dtrace::Info,
				Apply,
				"authenticated request with API key with metadata {}",
				serde_json::to_string(&metadata).unwrap_or_default()
			);
			Ok(Some(Claims {
				key: APIKey::new(key),
				metadata,
			}))
		} else if self.mode == Mode::Permissive {
			pol_result!(
				dtrace::Warn,
				Skip,
				"API key verification failed, continue due to permissive mode"
			);
			Ok(None)
		} else {
			pol_result!(
				dtrace::Error,
				Apply,
				"rejected request because API key credentials are invalid"
			);
			Err(ProxyError::APIKeyAuthenticationFailure(
				Error::InvalidCredentials,
			))
		}
	}
}

impl crate::store::RequestPolicyTrait for APIKeyAuthentication {
	async fn apply(
		&self,
		_client: &crate::proxy::httpproxy::PolicyClient,
		_log: &mut crate::telemetry::log::RequestLog,
		req: &mut Request,
	) -> Result<crate::http::PolicyResponse, ProxyResponse> {
		let res = self.verify(req).await.map_err(ProxyResponse::from)?;
		if let Some(claims) = res {
			self.location.remove(req).map_err(ProxyResponse::from)?;
			// Insert the claims into extensions so we can reference it later
			req.extensions_mut().insert(claims);
		}
		Ok(crate::http::PolicyResponse::default())
	}

	fn expressions(&self) -> impl Iterator<Item = &crate::cel::Expression> {
		self.location.expression().into_iter()
	}
}

#[apply(schema_de!)]
pub struct LocalAPIKeys {
	/// API keys that are accepted by this policy.
	pub keys: Vec<LocalAPIKey>,

	/// Controls whether requests must include a valid API key.
	#[serde(default)]
	pub mode: Mode,

	/// Where to read the API key from in incoming requests.
	#[serde(default)]
	pub location: AuthorizationLocation,
}

#[apply(schema_de!)]
pub struct LocalAPIKey {
	/// API key value to accept.
	pub key: APIKey,
	/// Optional metadata attached to requests authenticated with this key.
	pub metadata: Option<UserMetadata>,
}

impl LocalAPIKeys {
	pub fn into(self) -> APIKeyAuthentication {
		APIKeyAuthentication::new(
			self
				.keys
				.into_iter()
				.map(|k| (k.key, k.metadata.unwrap_or_default())),
			self.mode,
			self.location,
		)
	}
}
