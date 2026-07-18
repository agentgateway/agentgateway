use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use jsonwebtoken::Header;

use super::AuthorizationLocation;
use super::oauth::SigningAlg;
use super::oauth::client_auth::ParsedEncodingKey;
use crate::resource_manager::{ResourceFetcher, ResourceRef};
use crate::serdes::FileOrInline;
use crate::*;

/// Default token lifetime. Keep signed tokens short-lived to limit replay
/// exposure; upstreams like Snowflake cap `exp` at one hour anyway.
const DEFAULT_TTL: Duration = Duration::from_secs(300);

/// Backdate `iat` and extend `exp` by this amount so validators with slightly
/// skewed clocks do not reject freshly minted tokens, matching Google's auth
/// library behavior.
const CLOCK_SKEW_FUDGE: Duration = Duration::from_secs(10);

/// Time-based claims the signer owns; user-configured claims must not collide
/// with these. `iat` and `exp` are always set by the signer. `nbf` is not
/// emitted (validators treat `iat` as the issue time and a static `nbf` makes
/// no sense for per-request tokens) but stays reserved.
const RESERVED_CLAIMS: &[&str] = &["iat", "exp", "nbf"];

/// Rounds a ttl up to the next whole second so a sub-second component (e.g.
/// 1500ms) never yields a shorter lifetime than configured, in both the
/// signed token and any serialized/debug representation of the config.
fn ttl_secs_ceil(ttl: Duration) -> u64 {
	ttl.as_secs() + u64::from(ttl.subsec_nanos() > 0)
}

/// The signing key, either parsed eagerly (inline PEM) or deferred to
/// [`JwtSignAuth::resolve`] (file paths), so file-based keys register with the
/// resource manager and reload when the file changes.
#[derive(Clone)]
enum SigningKey {
	Parsed(ParsedEncodingKey),
	File(PathBuf),
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
	signing_key: SigningKey,
	#[serde(default)]
	alg: SigningAlg,
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
		state.serialize_field(
			"ttl",
			&self.ttl.map(|ttl| format!("{}s", ttl_secs_ceil(ttl))),
		)?;
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
		let signing_key = match raw.signing_key {
			FileOrInline::Inline(pem) => SigningKey::Parsed(parse_signing_key(raw.alg, pem.trim())?),
			FileOrInline::File { file } => SigningKey::File(file),
		};
		Ok(Self {
			signing_key,
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

fn parse_signing_key(alg: SigningAlg, pem: &str) -> Result<ParsedEncodingKey, String> {
	alg
		.encoding_key(pem.as_bytes())
		.map(ParsedEncodingKey)
		.map_err(|e| format!("failed to parse jwtSign signingKey: {e}"))
}

impl JwtSignAuth {
	pub fn try_new(
		signing_key_pem: &str,
		alg: SigningAlg,
		kid: Option<String>,
		claims: BTreeMap<String, serde_json::Value>,
		ttl: Option<Duration>,
		location: Option<AuthorizationLocation>,
	) -> Result<Self, String> {
		validate_config(&claims, ttl)?;
		let signing_key = SigningKey::Parsed(parse_signing_key(alg, signing_key_pem)?);
		Ok(Self {
			signing_key,
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
		if let SigningKey::File(path) = &self.signing_key {
			let pem = resources
				.fetch(ResourceRef::File(path.clone()))
				.await
				.context("failed to load jwtSign signingKey")?;
			let pem = std::str::from_utf8(&pem).context("jwtSign signingKey is not valid UTF-8")?;
			self.signing_key =
				SigningKey::Parsed(parse_signing_key(self.alg, pem.trim()).map_err(anyhow::Error::msg)?);
		}
		Ok(())
	}

	pub(super) fn sign(&self) -> anyhow::Result<String> {
		let SigningKey::Parsed(signing_key) = &self.signing_key else {
			anyhow::bail!("jwtSign file-based signingKey was not resolved at config load");
		};
		let now = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.context("system clock is before the unix epoch")?
			.as_secs();
		let ttl = self.ttl.unwrap_or(DEFAULT_TTL);
		let skew = CLOCK_SKEW_FUDGE.as_secs();

		let mut claims = serde_json::Map::with_capacity(self.claims.len() + RESERVED_CLAIMS.len());
		for (key, value) in &self.claims {
			claims.insert(key.clone(), value.clone());
		}
		let iat = now.saturating_sub(skew);
		let exp = now
			.checked_add(skew)
			.and_then(|t| t.checked_add(ttl_secs_ceil(ttl)))
			.context("jwtSign ttl overflows the exp timestamp")?;
		claims.insert("iat".to_string(), iat.into());
		claims.insert("exp".to_string(), exp.into());

		let mut header = Header::new(self.alg.algorithm());
		header.kid = self.kid.clone();
		jsonwebtoken::encode(&header, &serde_json::Value::Object(claims), &signing_key.0)
			.context("failed to sign backend JWT")
	}
}
