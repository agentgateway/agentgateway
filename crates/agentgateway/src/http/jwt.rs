// Inspired by https://github.com/cdriehuys/axum-jwks/blob/main/axum-jwks/src/jwks.rs (MIT license)
use std::collections::HashMap;
use std::str::FromStr;

use axum_core::RequestExt;
use axum_extra::TypedHeader;
use axum_extra::headers::Authorization;
use axum_extra::headers::authorization::Bearer;
use base64::Engine; // for URL-safe base64 decoding
use jsonwebtoken::jwk::{AlgorithmParameters, JwkSet, KeyAlgorithm};
use jsonwebtoken::{DecodingKey, Validation, decode, decode_header};
use secrecy::SecretString;
use serde_json::{Map, Value};
use std::sync::OnceLock;

use crate::client::Client;
use crate::http::Request;
use crate::telemetry::log::RequestLog;
use crate::*;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum TokenError {
	#[error("the token is invalid or malformed: {0:?}")]
	Invalid(jsonwebtoken::errors::Error),

	#[error("the token header is malformed: {0:?}")]
	InvalidHeader(jsonwebtoken::errors::Error),

	#[error("no bearer token found")]
	Missing,

	#[error("the token header does not specify a `kid`")]
	MissingKeyId,

	#[error("token uses the unknown key {0:?}")]
	UnknownKeyId(String),
}

#[derive(thiserror::Error, Debug)]
pub enum JwkError {
	#[error("failed to load JWKS: {0}")]
	JwkLoadError(anyhow::Error),
	#[error("failed to parse JWKS: {0}")]
	JwksParseError(#[from] serde_json::Error),
	#[error("the key is missing the `kid` attribute")]
	MissingKeyId,
	#[error("could not construct a decoding key for {key_id:?}: {error:?}")]
	DecodingError {
		key_id: String,
		error: jsonwebtoken::errors::Error,
	},
	#[error("the key {key_id:?} uses a non-RSA algorithm {algorithm:?}")]
	UnexpectedAlgorithm {
		algorithm: AlgorithmParameters,
		key_id: String,
	},
}

#[derive(Clone)]
pub struct Jwt {
	mode: Mode,
	keys: HashMap<String, Jwk>,
	issuer: String, // expected issuer for diagnostics
}

// TODO: can we give anything useful here?
impl serde::Serialize for Jwt {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		#[derive(serde::Serialize)]
		pub struct Serde<'a> {
			mode: Mode,
			issuer: &'a str,
			keys: Vec<&'a str>,
		}
		Serde {
			mode: self.mode,
			issuer: &self.issuer,
			keys: self.keys.keys().map(|x| x.as_str()).collect::<Vec<_>>(),
		}
		.serialize(serializer)
	}
}

impl Debug for Jwt {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Jwt").finish()
	}
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct LocalJwtConfig {
	#[serde(default)]
	pub mode: Mode,
	pub issuer: String,
	pub audiences: Vec<String>,
	pub jwks: serdes::FileInlineOrRemote,
}

#[derive(Default, Debug, Clone, Copy, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub enum Mode {
	/// A valid token, issued by a configured issuer, must be present.
	Strict,
	/// If a token exists, validate it.
	/// This is the default option.
	/// Warning: this allows requests without a JWT token!
	#[default]
	Optional,
	/// Requests are never rejected. This is useful for usage of claims in later steps (authorization, logging, etc).
	/// Warning: this allows requests without a JWT token!
	Permissive,
}

impl LocalJwtConfig {
	pub async fn try_into(self, client: Client) -> Result<Jwt, JwkError> {
		let jwks: JwkSet = self
			.jwks
			.load::<JwkSet>(client)
			.await
			.map_err(JwkError::JwkLoadError)?;

		let mut keys = HashMap::new();
		let to_supported_alg = |key_algorithm: Option<KeyAlgorithm>| match key_algorithm {
			Some(key_alg) => jsonwebtoken::Algorithm::from_str(key_alg.to_string().as_str()).ok(),
			_ => None,
		};

		for jwk in jwks.keys {
			if let Some(key_alg) = to_supported_alg(jwk.common.key_algorithm) {
				let kid = jwk.common.key_id.ok_or(JwkError::MissingKeyId)?;

				let decoding_key = match &jwk.algorithm {
					AlgorithmParameters::RSA(rsa) => DecodingKey::from_rsa_components(&rsa.n, &rsa.e)
						.map_err(|err| JwkError::DecodingError {
							key_id: kid.clone(),
							error: err,
						})?,
					AlgorithmParameters::EllipticCurve(ec) => DecodingKey::from_ec_components(&ec.x, &ec.y)
						.map_err(|err| JwkError::DecodingError {
						key_id: kid.clone(),
						error: err,
					})?,
					other => {
						return Err(JwkError::UnexpectedAlgorithm {
							key_id: kid,
							algorithm: other.to_owned(),
						});
					},
				};

				let mut validation = Validation::new(key_alg);
				validation.set_audience(self.audiences.as_slice());
				validation.set_issuer(std::slice::from_ref(&self.issuer));

				keys.insert(
					kid,
					Jwk {
						decoding: decoding_key,
						validation,
					},
				);
			} else {
				warn!(
					"JWK key algorithm {:?} is not supported. Tokens signed by that key will not be accepted.",
					jwk.common.key_algorithm
				)
			}
		}

		Ok(Jwt {
			mode: self.mode,
			keys,
			issuer: self.issuer,
		})
	}
}

#[derive(Clone)]
struct Jwk {
	decoding: DecodingKey,
	validation: Validation,
}

#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[cfg_attr(feature = "schema", schemars(with = "Map<String, Value>"))]
pub struct Claims {
	pub inner: Map<String, Value>,
	#[cfg_attr(feature = "schema", schemars(skip))]
	pub jwt: SecretString,
}

impl Serialize for Claims {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		self.inner.serialize(serializer)
	}
}

impl Jwt {
	pub async fn apply(&self, log: &mut RequestLog, req: &mut Request) -> Result<(), TokenError> {
		// First attempt standard Authorization: Bearer header
		let bearer_token: Option<String> = if let Ok(TypedHeader(Authorization(bearer))) = req
			.extract_parts::<TypedHeader<Authorization<Bearer>>>()
			.await
		{
			Some(bearer.token().to_string())
		} else {
			// Fallback: additional headers defined in env JWT_ASSERTION_HEADERS (comma-separated)
			static ADDITIONAL_JWT_HEADERS: OnceLock<Vec<http::header::HeaderName>> = OnceLock::new();
			let headers = ADDITIONAL_JWT_HEADERS.get_or_init(|| {
				std::env::var("JWT_ASSERTION_HEADERS")
					.ok()
					.map(|v| {
						v.split(',')
							.map(|s| s.trim())
							.filter(|s| !s.is_empty())
							.filter_map(|name| {
								http::header::HeaderName::from_bytes(name.to_ascii_lowercase().as_bytes()).ok()
							})
							.collect()
					})
					.unwrap_or_default()
			});
			let mut found: Option<String> = None;
			for h in headers.iter() {
				if let Some(val) = req.headers().get(h) {
					if let Ok(s) = val.to_str() {
						if !s.is_empty() {
							found = Some(s.to_string());
							break;
						}
					}
				}
			}
			found
		};
		let Some(token) = bearer_token else {
			if self.mode == Mode::Strict {
				return Err(TokenError::Missing);
			}
			return Ok(());
		};
		let claims = match self.validate_claims(&token) {
			Ok(claims) => claims,
			Err(e) if self.mode == Mode::Permissive => {
				debug!("token verification failed ({e}), continue due to permissive mode");
				return Ok(());
			},
			Err(e) => return Err(e),
		};
		if let Some(serde_json::Value::String(sub)) = claims.inner.get("sub") {
			log.jwt_sub = Some(sub.to_string());
		};
		log.cel.ctx().with_jwt(&claims);
		// Remove the token. TODO: allow keep it
		req.headers_mut().remove(http::header::AUTHORIZATION);
		// Insert the claims into extensions so we can reference it later
		req.extensions_mut().insert(claims);
		Ok(())
	}

	pub fn validate_claims(&self, token: &str) -> Result<Claims, TokenError> {
		let header = decode_header(token).map_err(|error| {
			debug!(?error, "Received token with invalid header.");

			TokenError::InvalidHeader(error)
		})?;
		let kid = header.kid.as_ref().ok_or_else(|| {
			debug!(?header, "Header is missing the `kid` attribute.");

			TokenError::MissingKeyId
		})?;

		let key = self.keys.get(kid).ok_or_else(|| {
			debug!(%kid, "Token refers to an unknown key.");

			TokenError::UnknownKeyId(kid.to_owned())
		})?;

		let decoded_token = decode::<Map<String, Value>>(token, &key.decoding, &key.validation)
			.map_err(|error| {
				// Extra issuer diagnostics
				if matches!(error.kind(), jsonwebtoken::errors::ErrorKind::InvalidIssuer) {
					if let Some(actual_iss) = decode_iss(token) {
						debug!(expected_iss=%self.issuer, actual_iss, "JWT issuer mismatch");
					} else {
						debug!(expected_iss=%self.issuer, "JWT issuer mismatch; failed to extract actual iss");
					}
				}
				debug!(?error, "Token is malformed or does not pass validation.");
				TokenError::Invalid(error)
			})?;

		let claims = Claims {
			inner: decoded_token.claims,
			jwt: SecretString::new(token.into()),
		};
		Ok(claims)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::http::Request;
	use ::http::header::HeaderName; // alias is Request = ::http::Request<Body>
	// Lightweight constructor for tests
	fn empty_jwt(mode: Mode) -> Jwt {
		Jwt {
			mode,
			keys: HashMap::new(),
			issuer: String::new(),
		}
	}

	#[tokio::test]
	async fn fallback_header_used_in_optional_mode() {
		unsafe {
			std::env::set_var("JWT_ASSERTION_HEADERS", "x-jwt-assertion");
		}
		let jwt = empty_jwt(Mode::Permissive);
		// craft minimal 3-part token that decodes header with kid "k" so validate fails gracefully
		let token = "eyJraWQiOiJrIiwiYWxnIjoiUlMyNTYifQ.eyJpc3MiOiJtZSJ9.sig"; // base64url parts
		let mut req: Request = Request::new(crate::http::Body::empty());
		*req.uri_mut() = "http://x".parse().unwrap();
		req.headers_mut().insert(
			HeaderName::from_static("x-jwt-assertion"),
			::http::HeaderValue::from_str(token).unwrap(),
		);
		let cfg = crate::telemetry::log::Config {
			filter: None,
			fields: Default::default(),
			metric_fields: Default::default(),
		};
		let tracing_cfg = crate::telemetry::trc::Config {
			endpoint: None,
			headers: Default::default(),
			protocol: crate::telemetry::trc::Protocol::Grpc,
			fields: Default::default(),
			random_sampling: None,
			client_sampling: None,
		};
		let cel = crate::telemetry::log::CelLogging::new(cfg, tracing_cfg);
		let mut registry = prometheus_client::registry::Registry::default();
		let metrics = Arc::new(crate::telemetry::metrics::Metrics::new(&mut registry));
		let mut log = crate::telemetry::log::RequestLog::new(
			cel,
			metrics,
			std::time::Instant::now(),
			crate::transport::stream::TCPConnectionInfo {
				local_addr: "127.0.0.1:1".parse().unwrap(),
				peer_addr: "127.0.0.1:2".parse().unwrap(),
				start: std::time::Instant::now(),
			},
		);
		// Should not error despite unknown key, due to permissive mode
		let res = jwt.apply(&mut log, &mut req).await;
		assert!(res.is_ok());
	}
}

// Lightweight helper to decode issuer claim without full validation (diagnostic only)
fn decode_iss(token: &str) -> Option<String> {
	let parts: Vec<&str> = token.split('.').collect();
	if parts.len() != 3 {
		return None;
	}
	let payload_b64 = parts[1];
	let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
		.decode(payload_b64.as_bytes())
		.ok()?;
	let v: serde_json::Value = serde_json::from_slice(&payload_bytes).ok()?;
	v.get("iss")
		.and_then(|iss| iss.as_str().map(|s| s.to_string()))
}
