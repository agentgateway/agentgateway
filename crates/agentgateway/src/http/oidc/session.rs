use std::fmt::Write as _;
use std::time::Duration;

use base64::Engine;
use cookie::{Cookie, SameSite};
use rand::RngExt;
use secrecy::{ExposeSecret, SecretString};
use serde::{Serialize, Serializer};

use super::{Error, PolicyId, now_unix};
use crate::http::sessionpersistence;

pub const RESERVED_COOKIE_PREFIX: &str = "agw_oidc_";
// Use a conservative budget below the common ~4 KiB browser per-cookie limit so
// attributes and user-agent differences do not turn borderline values into
// silently dropped session cookies.
const MAX_BROWSER_COOKIE_VALUE_SIZE: usize = 3800;
const ORIGINAL_URI_LIMIT: usize = 2048;

pub(super) fn default_session_ttl() -> Duration {
	Duration::from_secs(60 * 60)
}

pub(super) fn default_transaction_ttl() -> Duration {
	Duration::from_secs(5 * 60)
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionState {
	pub policy_id: PolicyId,
	pub transaction_id: String,
	pub csrf_state: String,
	pub nonce: String,
	#[serde(serialize_with = "crate::serdes::ser_redact")]
	pub pkce_verifier: SecretString,
	pub original_uri: String,
	pub expires_at_unix: u64,
}

impl Serialize for TransactionState {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		// Cookie payloads must serialize the raw secret values so the encrypted blob can be
		// round-tripped. The field-level redact serializers are for external config/debug output.
		#[derive(Serialize)]
		#[serde(rename_all = "camelCase")]
		struct SerializableTransactionState<'a> {
			policy_id: &'a PolicyId,
			transaction_id: &'a str,
			csrf_state: &'a str,
			nonce: &'a str,
			pkce_verifier: &'a str,
			original_uri: &'a str,
			expires_at_unix: u64,
		}

		SerializableTransactionState {
			policy_id: &self.policy_id,
			transaction_id: &self.transaction_id,
			csrf_state: &self.csrf_state,
			nonce: &self.nonce,
			pkce_verifier: self.pkce_verifier.expose_secret(),
			original_uri: &self.original_uri,
			expires_at_unix: self.expires_at_unix,
		}
		.serialize(serializer)
	}
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSession {
	pub policy_id: PolicyId,
	#[serde(serialize_with = "crate::serdes::ser_redact")]
	pub raw_id_token: SecretString,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub expires_at_unix: Option<u64>,
}

impl Serialize for BrowserSession {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		// Cookie payloads must serialize the raw secret values so the encrypted blob can be
		// round-tripped. The field-level redact serializers are for external config/debug output.
		#[derive(Serialize)]
		#[serde(rename_all = "camelCase")]
		struct SerializableBrowserSession<'a> {
			policy_id: &'a PolicyId,
			raw_id_token: &'a str,
			expires_at_unix: Option<u64>,
		}

		SerializableBrowserSession {
			policy_id: &self.policy_id,
			raw_id_token: self.raw_id_token.expose_secret(),
			expires_at_unix: self.expires_at_unix,
		}
		.serialize(serializer)
	}
}

impl BrowserSession {
	pub fn is_expired(&self) -> bool {
		self
			.expires_at_unix
			.is_some_and(|expires_at| expires_at <= now_unix())
	}
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(transparent)]
pub(crate) struct OidcCookieEncoder(sessionpersistence::Encoder);

impl OidcCookieEncoder {
	pub(crate) fn from_session_encoder(encoder: &sessionpersistence::Encoder) -> Result<Self, Error> {
		match encoder {
			sessionpersistence::Encoder::Aes(_) => Ok(Self(encoder.clone())),
			sessionpersistence::Encoder::Base64(_) => Err(Error::Config(
				"SESSION_KEY is required when oidc is configured".into(),
			)),
		}
	}

	fn encrypt(&self, plaintext: &str) -> Result<String, sessionpersistence::Error> {
		self.0.encrypt(plaintext)
	}

	fn decrypt(&self, encoded: &str) -> Result<Vec<u8>, sessionpersistence::Error> {
		self.0.decrypt(encoded)
	}
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfig {
	pub cookie_name: String,
	pub transaction_cookie_prefix: String,
	pub same_site: SameSiteMode,
	pub secure: CookieSecureMode,
	#[serde(with = "crate::serdes::serde_dur")]
	pub ttl: Duration,
	#[serde(with = "crate::serdes::serde_dur")]
	pub transaction_ttl: Duration,
	pub(crate) encoder: OidcCookieEncoder,
}

impl SessionConfig {
	pub fn decode_transaction(&self, cookie: &str) -> Result<TransactionState, Error> {
		let decoded = self
			.encoder
			.decrypt(cookie)
			.map_err(|_| Error::InvalidTransaction)?;
		let state: TransactionState =
			serde_json::from_slice(&decoded).map_err(|_| Error::InvalidTransaction)?;
		if state.expires_at_unix <= now_unix() {
			return Err(Error::InvalidTransaction);
		}
		Ok(state)
	}

	pub fn encode_transaction(&self, state: &TransactionState) -> Result<String, Error> {
		let json = serde_json::to_string(state).map_err(anyhow::Error::from)?;
		self
			.encoder
			.encrypt(&json)
			.map_err(|_| Error::InvalidTransaction)
	}

	pub fn decode_browser_session(&self, cookie: &str) -> Result<BrowserSession, Error> {
		let decoded = self
			.encoder
			.decrypt(cookie)
			.map_err(|_| Error::InvalidSession)?;
		let session: BrowserSession =
			serde_json::from_slice(&decoded).map_err(|_| Error::InvalidSession)?;
		if session.is_expired() {
			return Err(Error::InvalidSession);
		}
		Ok(session)
	}

	pub fn encode_browser_session(&self, session: &BrowserSession) -> Result<String, Error> {
		let json = serde_json::to_string(session).map_err(anyhow::Error::from)?;
		let encoded = self
			.encoder
			.encrypt(&json)
			.map_err(|_| Error::InvalidSession)?;
		if encoded.len() > MAX_BROWSER_COOKIE_VALUE_SIZE {
			return Err(Error::SessionCookieTooLarge);
		}
		Ok(encoded)
	}

	pub fn set_cookie(&self, name: &str, value: &str, is_https: bool, ttl: Duration) -> String {
		cookie_header(
			name,
			value,
			self.same_site,
			self.secure,
			is_https,
			Some(ttl),
			false,
		)
	}

	pub fn clear_cookie(&self, name: &str, is_https: bool) -> String {
		cookie_header(
			name,
			"",
			self.same_site,
			self.secure,
			is_https,
			Some(Duration::ZERO),
			true,
		)
	}

	pub fn transaction_cookie_name(&self, transaction_id: &str) -> String {
		format!("{}.{}", self.transaction_cookie_prefix, transaction_id)
	}
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SameSiteMode {
	#[default]
	Lax,
	Strict,
	None,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CookieSecureMode {
	#[default]
	Auto,
	Always,
	Never,
}

pub(super) fn derive_cookie_names(policy_id: &PolicyId) -> (String, String) {
	let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, policy_id.as_str().as_bytes());
	let mut hex = String::with_capacity(32);
	for byte in digest.as_ref().iter().take(8) {
		let _ = write!(&mut hex, "{byte:02x}");
	}
	(
		format!("{RESERVED_COOKIE_PREFIX}s_{hex}"),
		format!("{RESERVED_COOKIE_PREFIX}t_{hex}"),
	)
}

pub(super) fn generate_nonce() -> String {
	random_token(16)
}

pub(super) fn generate_state() -> String {
	random_token(16)
}

pub(super) fn generate_transaction_id() -> String {
	random_token(16)
}

pub(super) fn generate_pkce_verifier() -> String {
	random_token(32)
}

/// Capture only local post-login redirect targets so callback can safely reflect the stored value.
pub(super) fn normalize_original_uri(path_and_query: Option<&http::uri::PathAndQuery>) -> String {
	let original = path_and_query
		.map(http::uri::PathAndQuery::as_str)
		.unwrap_or("/");
	if original.len() > ORIGINAL_URI_LIMIT || !is_safe_local_redirect_target(original) {
		return "/".into();
	}

	original.to_string()
}

fn is_safe_local_redirect_target(target: &str) -> bool {
	if !target.starts_with('/') || target.starts_with("//") || target.contains('\\') {
		return false;
	}

	let decoded = percent_encoding::percent_decode_str(target).decode_utf8_lossy();
	decoded.starts_with('/') && !decoded.starts_with("//") && !decoded.contains('\\')
}

fn random_token(bytes: usize) -> String {
	let mut random = vec![0; bytes];
	rand::rng().fill(random.as_mut_slice());
	base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random)
}

fn cookie_header(
	name: &str,
	value: &str,
	same_site: SameSiteMode,
	secure_mode: CookieSecureMode,
	is_https: bool,
	max_age: Option<Duration>,
	expire_now: bool,
) -> String {
	let secure = match secure_mode {
		CookieSecureMode::Always => true,
		CookieSecureMode::Never => false,
		CookieSecureMode::Auto => is_https,
	};
	let mut cookie = Cookie::build((name, value))
		.path("/")
		.http_only(true)
		.same_site(match same_site {
			SameSiteMode::Lax => SameSite::Lax,
			SameSiteMode::Strict => SameSite::Strict,
			SameSiteMode::None => SameSite::None,
		})
		.secure(secure);
	if let Some(max_age) = max_age {
		let secs = if expire_now { 0 } else { max_age.as_secs() };
		let secs = i64::try_from(secs).unwrap_or(i64::MAX);
		cookie = cookie.max_age(cookie::time::Duration::seconds(secs));
	}
	cookie.build().to_string()
}
