use std::hash::Hash;

use crate::http::Request;
use crate::proxy::ProxyError;
use crate::telemetry::log::RequestLog;
use crate::*;
use axum_core::RequestExt;
use axum_extra::TypedHeader;
use axum_extra::headers::Authorization;
use headers::authorization::Bearer;
use macro_rules_attribute::apply;
use secrecy::{ExposeSecret, SecretString};

#[derive(thiserror::Error, Debug)]
pub enum Error {
	#[error("no API Key found")]
	Missing,

	#[error("invalid credentials")]
	InvalidCredentials,
}

/// Validation mode for API Key authentication
#[apply(schema!)]
#[derive(Copy, PartialEq, Eq, Default)]
pub enum Mode {
	/// A valid API Key must be present.
	Strict,
	/// If credentials exist, validate them.
	/// This is the default option.
	/// Warning: this allows requests without credentials!
	#[default]
	Optional,
}

#[apply(schema_ser!)]
pub struct Claims {
	pub key: APIKey,
	#[serde(flatten)]
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

#[apply(schema_ser!)]
#[cfg_attr(feature = "schema", schemars(with = "LocalAPIKeys"))]
pub struct APIKeyAuthentication {
	// A map of API keys to the metadata for that key
	#[serde(serialize_with = "ser_redact")]
	pub users: Arc<HashMap<APIKey, UserMetadata>>,

	/// Validation mode for API Key authentication
	pub mode: Mode,
}

impl APIKeyAuthentication {
	pub fn new(keys: impl IntoIterator<Item = (APIKey, UserMetadata)>, mode: Mode) -> Self {
		Self {
			users: Arc::new(keys.into_iter().collect()),
			mode,
		}
	}

	async fn verify(&self, req: &mut Request) -> Result<Option<Claims>, ProxyError> {
		// Extract Bearer authorization header
		// TODO: allow extracting from other places
		let Ok(TypedHeader(Authorization(bearer))) = req
			.extract_parts::<TypedHeader<Authorization<Bearer>>>()
			.await
		else {
			// In strict mode, we require credentials
			if self.mode == Mode::Strict {
				return Err(ProxyError::APIKeyAuthenticationFailure(Error::Missing));
			}
			// Otherwise without credentials, don't attempt to authenticate
			return Ok(None);
		};

		let key = APIKey::new(bearer.token());
		if let Some(meta) = self.users.get(&key) {
			let claims = Claims {
				key,
				metadata: meta.clone(),
			};
			Ok(Some(claims))
		} else {
			Err(ProxyError::APIKeyAuthenticationFailure(
				Error::InvalidCredentials,
			))
		}
	}

	pub async fn apply(&self, log: &mut RequestLog, req: &mut Request) -> Result<(), ProxyError> {
		let res = self.verify(req).await?;
		if let Some(claims) = res {
			log.cel.ctx().with_api_key(&claims);
			req.headers_mut().remove(http::header::AUTHORIZATION);
			// Insert the claims into extensions so we can reference it later
			req.extensions_mut().insert(claims);
		}
		Ok(())
	}
}

#[apply(schema_de!)]
pub struct LocalAPIKeys {
	/// List of API keys
	pub keys: Vec<LocalAPIKey>,

	/// Validation mode for API keys
	#[serde(default)]
	pub mode: Mode,
}

#[apply(schema_de!)]
pub struct LocalAPIKey {
	pub key: APIKey,
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
		)
	}
}
