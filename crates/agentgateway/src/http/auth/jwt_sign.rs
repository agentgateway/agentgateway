use std::collections::BTreeMap;
use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use jsonwebtoken::Header;

use super::AuthorizationLocation;
use super::oauth::SigningAlg;
use super::oauth::client_auth::ParsedEncodingKey;
use crate::serdes::FileOrInline;
use crate::*;

/// Default token lifetime. Keep signed tokens short-lived to limit replay
/// exposure; upstreams like Snowflake cap `exp` at one hour anyway.
const DEFAULT_TTL: Duration = Duration::from_secs(300);

/// Claims the signer always sets itself; user-configured claims must not
/// collide with these.
const RESERVED_CLAIMS: &[&str] = &["iat", "exp"];

/// Signs a short-lived JWT with a private key on each request and sends it to
/// the backend. For upstreams that require per-request keypair JWTs (e.g. the
/// Snowflake SQL API) rather than a static credential.
#[derive(Clone, serde::Deserialize)]
#[serde(try_from = "RawJwtSignAuth", rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct JwtSignAuth {
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	signing_key: ParsedEncodingKey,
	#[serde(default)]
	alg: SigningAlg,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	kid: Option<String>,
	claims: BTreeMap<String, String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	ttl: Option<Duration>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub(super) location: Option<AuthorizationLocation>,
}

impl fmt::Debug for JwtSignAuth {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("JwtSignAuth")
			.field("signing_key", &"<redacted>")
			.field("alg", &self.alg)
			.field("kid", &self.kid)
			.field("claims", &self.claims)
			.field("ttl", &self.ttl)
			.field("location", &self.location)
			.finish()
	}
}

impl serde::Serialize for JwtSignAuth {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		use serde::ser::SerializeStruct;

		let mut state = serializer.serialize_struct("JwtSignAuth", 5)?;
		state.serialize_field("alg", &self.alg)?;
		state.serialize_field("kid", &self.kid)?;
		state.serialize_field("claims", &self.claims)?;
		state.serialize_field("ttl", &self.ttl.map(|ttl| format!("{}s", ttl.as_secs())))?;
		state.serialize_field("location", &self.location)?;
		state.end()
	}
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct RawJwtSignAuth {
	/// PEM-encoded private signing key (RSA or EC, matching `alg`).
	#[cfg_attr(feature = "schema", schemars(with = "crate::serdes::FileOrInline"))]
	signing_key: FileOrInline,
	/// JWS signing algorithm. Defaults to RS256.
	#[serde(default)]
	alg: SigningAlg,
	/// Optional JWS key ID header.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	kid: Option<String>,
	/// Static claims added to every token (e.g. iss, sub, aud). `iat` and
	/// `exp` are always set by the signer and cannot be configured here.
	#[cfg_attr(feature = "schema", schemars(extend("minProperties" = 1)))]
	claims: BTreeMap<String, String>,
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
		// TODO: file-based keys are read once at config load; consider reload/rotation (K8s secret remounts need a restart)
		let pem = raw
			.signing_key
			.load()
			.map_err(|e| format!("failed to load jwtSign signing_key: {e}"))?;
		Self::try_new(
			pem.trim(),
			raw.alg,
			raw.kid,
			raw.claims,
			raw.ttl,
			raw.location,
		)
	}
}

impl JwtSignAuth {
	pub fn try_new(
		signing_key_pem: &str,
		alg: SigningAlg,
		kid: Option<String>,
		claims: BTreeMap<String, String>,
		ttl: Option<Duration>,
		location: Option<AuthorizationLocation>,
	) -> Result<Self, String> {
		if claims.is_empty() {
			return Err("jwtSign requires at least one claim".into());
		}
		for reserved in RESERVED_CLAIMS {
			if claims.contains_key(*reserved) {
				return Err(format!(
					"jwtSign claim {reserved:?} is set by the signer and cannot be configured"
				));
			}
		}
		if let Some(ttl) = ttl
			&& ttl.as_secs() == 0
		{
			return Err("jwtSign ttl must be at least one second".into());
		}
		let signing_key = alg
			.encoding_key(signing_key_pem.as_bytes())
			.map_err(|e| format!("failed to parse jwtSign signing_key: {e}"))?;
		Ok(Self {
			signing_key: ParsedEncodingKey(signing_key),
			alg,
			kid,
			claims,
			ttl,
			location,
		})
	}

	pub(super) fn sign(&self) -> anyhow::Result<String> {
		let now = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.context("system clock is before the unix epoch")?
			.as_secs();
		let ttl = self.ttl.unwrap_or(DEFAULT_TTL);

		let mut claims = serde_json::Map::with_capacity(self.claims.len() + RESERVED_CLAIMS.len());
		for (key, value) in &self.claims {
			claims.insert(key.clone(), serde_json::Value::String(value.clone()));
		}
		let exp = now
			.checked_add(ttl.as_secs())
			.context("jwtSign ttl overflows the exp timestamp")?;
		claims.insert("iat".to_string(), now.into());
		claims.insert("exp".to_string(), exp.into());

		let mut header = Header::new(self.alg.algorithm());
		header.kid = self.kid.clone();
		jsonwebtoken::encode(
			&header,
			&serde_json::Value::Object(claims),
			&self.signing_key.0,
		)
		.context("failed to sign backend JWT")
	}
}
