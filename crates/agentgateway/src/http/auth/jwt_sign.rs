use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;

use super::jws::{JwtSigningAlg, SigningKey};
use super::{AuthorizationLocation, jwt_claim_times, unix_timestamp_now};
use crate::resource_manager::{ResourceFetcher, ResourceRef};
use crate::serdes::FileOrInline;
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

#[derive(Clone)]
enum JwtSignState {
	Parsed(SigningKey),
	File(PathBuf),
	Invalid(String),
}

/// Signs a short-lived JWT with a private key on each request and sends it to
/// the backend. For upstreams that require per-request keypair JWTs (e.g. the
/// Snowflake SQL API) rather than a static credential.
#[derive(Clone, serde::Deserialize)]
#[serde(try_from = "RawJwtSignAuth", rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct JwtSignAuth {
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	state: JwtSignState,
	#[serde(default)]
	alg: JwtSigningAlg,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	kid: Option<String>,
	claims: BTreeMap<String, serde_json::Value>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	ttl: Option<Duration>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub(super) location: Option<AuthorizationLocation>,
}

impl fmt::Debug for JwtSignAuth {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let mut debug = f.debug_struct("JwtSignAuth");
		match &self.state {
			JwtSignState::Invalid(error) => {
				debug.field("translation_error", error);
			},
			JwtSignState::Parsed(_) | JwtSignState::File(_) => {
				debug
					.field("signing_key", &"<redacted>")
					.field("alg", &self.alg)
					.field("kid", &self.kid)
					.field("claims", &self.claims)
					.field("ttl", &self.ttl)
					.field("location", &self.location);
			},
		}
		debug.finish()
	}
}

impl serde::Serialize for JwtSignAuth {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		use serde::ser::SerializeStruct;

		if let JwtSignState::Invalid(error) = &self.state {
			let mut state = serializer.serialize_struct("JwtSignAuth", 1)?;
			state.serialize_field("translationError", error)?;
			return state.end();
		}

		let mut state = serializer.serialize_struct("JwtSignAuth", 5)?;
		state.serialize_field("alg", &self.alg)?;
		state.serialize_field("kid", &self.kid)?;
		state.serialize_field("claims", &self.claims)?;
		state.serialize_field("ttl", &self.ttl.map(SerializableDuration))?;
		state.serialize_field("location", &self.location)?;
		state.end()
	}
}

#[derive(serde::Serialize)]
struct SerializableDuration(#[serde(with = "crate::serdes::serde_dur")] Duration);

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct RawJwtSignAuth {
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
	#[cfg_attr(feature = "schema", schemars(extend("minProperties" = 1)))]
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

impl TryFrom<RawJwtSignAuth> for JwtSignAuth {
	type Error = String;

	fn try_from(raw: RawJwtSignAuth) -> Result<Self, Self::Error> {
		validate_config(&raw.claims, raw.ttl)?;
		// Inline keys are parsed eagerly so misconfigurations fail at parse
		// time. File keys are deferred to `resolve`, which fetches through the
		// resource manager so the file is watched and changes reload the config.
		let state = match raw.signing_key {
			FileOrInline::Inline(pem) => JwtSignState::Parsed(parse_signing_key(raw.alg, pem.trim())?),
			FileOrInline::File { file } => JwtSignState::File(file),
		};
		Ok(Self {
			state,
			alg: raw.alg,
			kid: raw.kid,
			claims: raw.claims,
			ttl: raw.ttl,
			location: raw.location,
		})
	}
}

fn validate_config(
	claims: &BTreeMap<String, serde_json::Value>,
	ttl: Option<Duration>,
) -> Result<(), String> {
	if claims.is_empty() {
		return Err("jwtSign requires at least one claim".into());
	}
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
		Self {
			state: JwtSignState::Invalid(error),
			alg: JwtSigningAlg::default(),
			kid: None,
			claims: BTreeMap::new(),
			ttl: None,
			location: None,
		}
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
		let state = JwtSignState::Parsed(parse_signing_key(alg, signing_key_pem)?);
		Ok(Self {
			state,
			alg,
			kid,
			claims,
			ttl,
			location,
		})
	}

	/// Resolves a file-based signing key through the resource manager, which
	/// registers the file so changes trigger a config reload. Inline keys are
	/// already parsed and are left untouched.
	pub async fn resolve(&mut self, resources: &ResourceFetcher) -> anyhow::Result<()> {
		let JwtSignState::File(path) = &self.state else {
			return Ok(());
		};
		let pem = resources
			.fetch(ResourceRef::File(path.clone()))
			.await
			.context("failed to load jwtSign signingKey")?;
		let pem = std::str::from_utf8(&pem).context("jwtSign signingKey is not valid UTF-8")?;
		self.state =
			JwtSignState::Parsed(parse_signing_key(self.alg, pem.trim()).map_err(anyhow::Error::msg)?);
		Ok(())
	}

	pub(super) fn sign(&self) -> anyhow::Result<String> {
		let signing_key = match &self.state {
			JwtSignState::Parsed(signing_key) => signing_key,
			JwtSignState::File(_) => {
				anyhow::bail!("jwtSign file-based signingKey was not resolved at config load");
			},
			JwtSignState::Invalid(error) => {
				tracing::debug!(
					error = %error,
					"rejecting request: jwtSign configuration is invalid"
				);
				anyhow::bail!("jwtSign configuration is invalid");
			},
		};
		let now = unix_timestamp_now()?;
		let ttl = self.ttl.unwrap_or(DEFAULT_TTL);
		let times = jwt_claim_times(now, ttl, ISSUED_AT_BACKDATE)?;

		let mut claims = serde_json::Map::with_capacity(self.claims.len() + RESERVED_CLAIMS.len());
		for (key, value) in &self.claims {
			claims.insert(key.clone(), value.clone());
		}
		claims.insert("iat".to_string(), times.issued_at.into());
		claims.insert("exp".to_string(), times.expires_at.into());

		let header = self.alg.header(self.kid.clone());
		signing_key
			.encode(&header, &serde_json::Value::Object(claims))
			.context("failed to sign backend JWT")
	}
}
