use std::hash::Hash;

use ::cel::Value;
use macro_rules_attribute::apply;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer};
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

	pub(crate) fn sha256(&self) -> APIKeyHash {
		APIKeyHash::from_raw_key(self.0.expose_secret())
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

/// SHA-256 hex digest of an API key. The hex string (without the `sha256:` prefix)
/// is byte-identical to the `sha256.encode` CEL function output, so a `keyHash` is
/// interchangeable with the `location.expression` hashing workaround.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct APIKeyHash(String);

impl APIKeyHash {
	pub fn from_raw_key(key: &str) -> Self {
		let digest = Sha256::digest(key.as_bytes());
		APIKeyHash(hex::encode(digest))
	}

	pub fn parse(key_hash: &str) -> Result<Self, String> {
		let Some(digest) = key_hash.strip_prefix("sha256:") else {
			return Err("keyHash must use the sha256:<hex> format".to_string());
		};
		let decoded = hex::decode(digest).map_err(|e| e.to_string())?;
		if decoded.len() != 32 {
			return Err("sha256 keyHash must decode to 32 bytes".to_string());
		}
		Ok(APIKeyHash(digest.to_ascii_lowercase()))
	}
}

fn is_bcrypt(s: &str) -> bool {
	s.starts_with("$2a$") || s.starts_with("$2b$") || s.starts_with("$2y$")
}

pub(crate) enum StoredKey {
	Sha256(APIKeyHash),
	Bcrypt(SecretString),
}

impl StoredKey {
	pub(crate) fn parse_hash(key_hash: &str) -> Result<Self, String> {
		if is_bcrypt(key_hash) {
			return Ok(StoredKey::Bcrypt(SecretString::new(key_hash.into())));
		}
		APIKeyHash::parse(key_hash).map(StoredKey::Sha256)
	}
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct HashedKey(#[cfg_attr(feature = "schema", schemars(with = "String"))] String);

impl HashedKey {
	fn into_stored(self) -> StoredKey {
		StoredKey::parse_hash(&self.0).expect("validated during deserialization")
	}
}

fn deser_stored_hash<'de, D>(deserializer: D) -> Result<HashedKey, D::Error>
where
	D: Deserializer<'de>,
{
	let input = String::deserialize(deserializer)?;
	StoredKey::parse_hash(&input).map_err(serde::de::Error::custom)?;
	Ok(HashedKey(input))
}

/// A bcrypt-hashed key. bcrypt embeds a per-entry salt, so these cannot be looked up
/// by digest and are verified by a linear scan. The hash is itself a credential, so it
/// is wrapped in `SecretString` for consistent redaction.
#[derive(Debug, Clone)]
struct SaltedEntry {
	hash: SecretString,
	metadata: UserMetadata,
}

#[apply(schema_ser!)]
pub struct APIKeyAuthentication {
	/// Exact-match lookup of every plaintext/sha256 key by its SHA-256 digest, so raw
	/// keys are never retained at rest.
	#[serde(serialize_with = "ser_redact")]
	pub users: Arc<HashMap<APIKeyHash, UserMetadata>>,

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
		Self::from_entries(
			keys
				.into_iter()
				.map(|(key, meta)| (StoredKey::Sha256(key.sha256()), meta)),
			mode,
			location,
		)
	}

	pub(crate) fn from_entries(
		entries: impl IntoIterator<Item = (StoredKey, UserMetadata)>,
		mode: Mode,
		location: AuthorizationLocation,
	) -> Self {
		let mut users = HashMap::new();
		let mut salted = Vec::new();
		for (key, metadata) in entries {
			match key {
				StoredKey::Sha256(digest) => {
					users.insert(digest, metadata);
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
			users: Arc::new(users),
			salted: Arc::new(salted),
			mode,
			location,
		}
	}

	fn lookup(&self, presented: &str) -> Option<UserMetadata> {
		if let Some(meta) = self.users.get(&APIKeyHash::from_raw_key(presented)) {
			return Some(meta.clone());
		}
		self
			.salted
			.iter()
			.find(|entry| bcrypt::verify(presented, entry.hash.expose_secret()).unwrap_or(false))
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
#[serde(untagged)]
pub enum LocalAPIKey {
	Key {
		/// API key value to accept.
		key: APIKey,
		/// Optional metadata attached to requests authenticated with this key.
		metadata: Option<UserMetadata>,
	},
	Hashed {
		/// Hashed API key to accept, either a `sha256:<hex>` digest or a bcrypt digest
		/// (modular crypt format, e.g. `$2b$...`).
		#[serde(rename = "keyHash", deserialize_with = "deser_stored_hash")]
		key_hash: HashedKey,
		/// Optional metadata attached to requests authenticated with this key.
		metadata: Option<UserMetadata>,
	},
}

impl LocalAPIKey {
	fn into_parts(self) -> (StoredKey, UserMetadata) {
		match self {
			LocalAPIKey::Key { key, metadata } => (
				StoredKey::Sha256(key.sha256()),
				metadata.unwrap_or_default(),
			),
			LocalAPIKey::Hashed { key_hash, metadata } => {
				(key_hash.into_stored(), metadata.unwrap_or_default())
			},
		}
	}
}

impl LocalAPIKeys {
	pub fn into(self) -> APIKeyAuthentication {
		APIKeyAuthentication::from_entries(
			self.keys.into_iter().map(LocalAPIKey::into_parts),
			self.mode,
			self.location,
		)
	}
}
