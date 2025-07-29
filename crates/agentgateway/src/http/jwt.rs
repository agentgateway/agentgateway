// Inspired by https://github.com/cdriehuys/axum-jwks/blob/main/axum-jwks/src/jwks.rs (MIT license)
use std::collections::HashMap;
use std::str::FromStr;

use axum_core::RequestExt;
use axum_extra::TypedHeader;
use axum_extra::headers::Authorization;
use axum_extra::headers::authorization::Bearer;
use jsonwebtoken::jwk::{self, AlgorithmParameters, JwkSet, KeyAlgorithm};
use jsonwebtoken::{DecodingKey, TokenData, Validation, decode, decode_header};
use secrecy::SecretString;
use serde::de::Error;
use serde::ser::SerializeMap;
use serde_json::{Map, Value};

use crate::client::Client;
use crate::http::Request;
use crate::telemetry::log::RequestLog;
use crate::types::agent::{HostRedirect, PathRedirect};
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
	keys: HashMap<String, Jwk>,
}

// TODO: can we give anything useful here?
impl serde::Serialize for Jwt {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		self.keys.keys().collect::<Vec<_>>().serialize(serializer)
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
	pub issuer: String,
	pub audiences: Vec<String>,
	pub jwks: serdes::FileInlineOrRemote,
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

		Ok(Jwt { keys })
	}
}

#[derive(Clone)]
struct Jwk {
	decoding: DecodingKey,
	validation: Validation,
}

#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
		let Ok(TypedHeader(Authorization(bearer))) = req
			.extract_parts::<TypedHeader<Authorization<Bearer>>>()
			.await
		else {
			// No token, so don't attempt to authenticate.
			// TODO: we need authorization policies to allow requiring it
			return Ok(());
		};
		let claims = self.validate_claims(bearer.token())?;
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

	pub fn validate_claims(&self, token: &str) -> Result<Claims, TokenError>
where {
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
