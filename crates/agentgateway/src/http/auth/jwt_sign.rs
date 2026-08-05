use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::Context;

use super::jws::{JwtSigningAlg, SigningKey};
use super::{AuthorizationLocation, jwt_claim_times, unix_timestamp_now};
use crate::resource_manager::ResourceFetcher;
use crate::serdes::{FileOrInline, load_file_or_inline};
use crate::*;

/// Default token lifetime. Keep signed tokens short-lived to limit replay
/// exposure; upstreams like Snowflake cap `exp` at one hour anyway.
const DEFAULT_TTL: Duration = Duration::from_secs(300);

/// Backdate `iat` to tolerate validators whose clocks trail the gateway.
/// Expiration still uses the configured lifetime measured from signing time.
const ISSUED_AT_BACKDATE: Duration = Duration::from_secs(10);

/// Time-based claims the signer owns; user-configured claims must not collide
/// with these. `iat` and `exp` are always set by the signer. `nbf` is not
/// emitted (validators treat `iat` as the issue time and a static `nbf` makes
/// no sense for per-request tokens) but stays reserved.
const RESERVED_CLAIMS: &[&str] = &["iat", "exp", "nbf"];

#[derive(Clone, Debug, serde::Serialize)]
#[serde(untagged)]
enum JwtSignState {
	Valid(JwtSignConfig),
	Invalid {
		#[serde(rename = "translationError")]
		reason: String,
	},
}

/// Signs a short-lived JWT with a private key on each request and sends it to
/// the backend. For upstreams that require per-request keypair JWTs (e.g. the
/// Snowflake SQL API) rather than a static credential.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(transparent)]
pub struct JwtSignAuth(JwtSignState);

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct JwtSignConfig {
	#[serde(skip)]
	signing_key: SigningKey,
	#[serde(default)]
	alg: JwtSigningAlg,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	kid: Option<String>,
	claims: BTreeMap<String, serde_json::Value>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	#[serde(with = "crate::serdes::serde_dur_option")]
	ttl: Option<Duration>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub(super) location: Option<AuthorizationLocation>,
}

/// Signs a short-lived JWT with a private key on each request and sends it to
/// the backend. For upstreams that require per-request keypair JWTs (e.g. the
/// Snowflake SQL API) rather than a static credential.
#[apply(schema_de!)]
#[cfg_attr(feature = "schema", schemars(rename = "JwtSignAuth"))]
pub(crate) struct LocalJwtSignAuth {
	/// PEM-encoded private signing key (RSA or EC, matching `alg`).
	#[cfg_attr(feature = "schema", schemars(with = "crate::serdes::FileOrInline"))]
	signing_key: FileOrInline,
	/// JWS signing algorithm. Defaults to RS256.
	#[serde(default)]
	alg: JwtSigningAlg,
	/// Optional JWS key ID header.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	kid: Option<String>,
	/// Static claims added to every token (e.g. iss, sub, aud). Values may be
	/// any JSON value (e.g. a string, number, bool, or array). `iat`, `exp`,
	/// and `nbf` are reserved for the signer and cannot be configured here.
	#[serde(default)]
	claims: BTreeMap<String, serde_json::Value>,
	/// Token lifetime used for `exp`. Defaults to 300s.
	#[serde(
		default,
		with = "crate::serdes::serde_dur_option",
		skip_serializing_if = "Option::is_none"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	ttl: Option<Duration>,
	/// Where the signed token is written. Defaults to the Authorization
	/// header with a `Bearer ` prefix.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	location: Option<AuthorizationLocation>,
}

impl LocalJwtSignAuth {
	pub(crate) async fn try_into(self, resources: &ResourceFetcher) -> anyhow::Result<JwtSignAuth> {
		let pem = load_file_or_inline(&self.signing_key, resources)
			.await
			.context("failed to load jwtSign signingKey")?;
		JwtSignAuth::try_new(
			pem.trim(),
			self.alg,
			self.kid,
			self.claims,
			self.ttl,
			self.location,
		)
		.map_err(anyhow::Error::msg)
	}
}

fn validate_config(
	claims: &BTreeMap<String, serde_json::Value>,
	ttl: Option<Duration>,
) -> Result<(), String> {
	for reserved in RESERVED_CLAIMS {
		if claims.contains_key(*reserved) {
			return Err(format!(
				"jwtSign claim {reserved:?} is reserved for the signer and cannot be configured"
			));
		}
	}
	if let Some(ttl) = ttl
		&& ttl.as_secs() == 0
	{
		return Err("jwtSign ttl must be at least one second".into());
	}
	Ok(())
}

fn parse_signing_key(alg: JwtSigningAlg, pem: &str) -> Result<SigningKey, String> {
	SigningKey::from_pem(alg, pem.as_bytes())
		.map_err(|e| format!("failed to parse jwtSign signingKey: {e}"))
}

impl JwtSignAuth {
	/// Returns a jwtSign configuration that always rejects requests
	pub(crate) fn new_invalid(error: String) -> Self {
		Self(JwtSignState::Invalid { reason: error })
	}

	pub fn try_new(
		signing_key_pem: &str,
		alg: JwtSigningAlg,
		kid: Option<String>,
		claims: BTreeMap<String, serde_json::Value>,
		ttl: Option<Duration>,
		location: Option<AuthorizationLocation>,
	) -> Result<Self, String> {
		validate_config(&claims, ttl)?;
		let signing_key = parse_signing_key(alg, signing_key_pem)?;
		Ok(Self(JwtSignState::Valid(JwtSignConfig {
			signing_key,
			alg,
			kid,
			claims,
			ttl,
			location,
		})))
	}

	pub(super) fn sign(&self) -> anyhow::Result<String> {
		let config = match &self.0 {
			JwtSignState::Valid(config) => config,
			JwtSignState::Invalid { reason } => {
				tracing::debug!(
						error = %reason,
						"rejecting request: jwtSign configuration is invalid"
				);
				anyhow::bail!("jwtSign configuration is invalid");
			},
		};
		let now = unix_timestamp_now()?;
		let ttl = config.ttl.unwrap_or(DEFAULT_TTL);
		let times = jwt_claim_times(now, ttl, ISSUED_AT_BACKDATE)?;

		let mut claims = serde_json::Map::with_capacity(config.claims.len() + RESERVED_CLAIMS.len());
		for (key, value) in &config.claims {
			claims.insert(key.clone(), value.clone());
		}
		claims.insert("iat".to_string(), times.issued_at.into());
		claims.insert("exp".to_string(), times.expires_at.into());

		let header = config.alg.header(config.kid.clone());
		config
			.signing_key
			.encode(&header, &serde_json::Value::Object(claims))
			.context("failed to sign backend JWT")
	}

	pub(super) fn location(&self) -> Option<&AuthorizationLocation> {
		match &self.0 {
			JwtSignState::Valid(config) => config.location.as_ref(),
			JwtSignState::Invalid { .. } => None,
		}
	}
}
