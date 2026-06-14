use std::hash::Hash;

use ::cel::Value;
use macro_rules_attribute::apply;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, Serializer};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::http::Request;
use crate::http::auth::AuthorizationLocation;
use crate::proxy::dtrace::{self, pol_result};
use crate::proxy::{ProxyError, ProxyResponse};
use crate::*;

#[cfg(test)]
#[path = "apikey_tests.rs"]
mod tests;

const TRACE_POLICY_KIND: &str = "api_key";

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
		// Use a constant-time comparison; a short-circuiting comparison would leak how many
		// leading bytes of a candidate key match a configured key through response timing.
		self
			.0
			.expose_secret()
			.as_bytes()
			.ct_eq(other.0.expose_secret().as_bytes())
			.into()
	}
}

impl Eq for APIKey {}

#[apply(schema!)]
#[derive(Hash, PartialEq, Eq)]
pub struct APIKeyHash(
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	#[serde(serialize_with = "ser_key_hash", deserialize_with = "deser_key_hash")]
	String,
);

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

fn deser_key_hash<'de, D>(deserializer: D) -> Result<String, D::Error>
where
	D: Deserializer<'de>,
{
	let input = String::deserialize(deserializer)?;
	APIKeyHash::parse(&input)
		.map(|hash| hash.0)
		.map_err(serde::de::Error::custom)
}

fn ser_key_hash<S>(digest: &str, serializer: S) -> Result<S::Ok, S::Error>
where
	S: Serializer,
{
	serializer.serialize_str(&format!("sha256:{digest}"))
}

#[apply(schema_ser!)]
pub struct APIKeyAuthentication {
	// A map of API keys to the metadata for that key
	#[serde(serialize_with = "ser_redact")]
	pub users: Arc<HashMap<APIKeyHash, UserMetadata>>,

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
		Self {
			users: Arc::new(
				keys
					.into_iter()
					.map(|(key, meta)| (key.sha256(), meta))
					.collect(),
			),
			mode,
			location,
		}
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

		let key = APIKey::new(key);
		if let Some(meta) = self.users.get(&key.sha256()) {
			pol_result!(
				dtrace::Info,
				Apply,
				"authenticated request with API key with metadata {}",
				serde_json::to_string(meta).unwrap_or_default()
			);
			let claims = Claims {
				key,
				metadata: meta.clone(),
			};
			Ok(Some(claims))
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
	Sha256 {
		/// SHA-256 hash of an API key value to accept, in `sha256:<hex>` format.
		#[serde(rename = "keyHash")]
		key_hash: APIKeyHash,
		/// Optional metadata attached to requests authenticated with this key.
		metadata: Option<UserMetadata>,
	},
}

impl LocalAPIKey {
	fn into_parts(self) -> (APIKeyHash, UserMetadata) {
		match self {
			LocalAPIKey::Key { key, metadata } => (key.sha256(), metadata.unwrap_or_default()),
			LocalAPIKey::Sha256 { key_hash, metadata } => (key_hash, metadata.unwrap_or_default()),
		}
	}
}

impl LocalAPIKeys {
	pub fn into(self) -> APIKeyAuthentication {
		APIKeyAuthentication {
			users: Arc::new(self.keys.into_iter().map(LocalAPIKey::into_parts).collect()),
			mode: self.mode,
			location: self.location,
		}
	}
}
