// Inspired by https://github.com/cdriehuys/axum-jwks/blob/main/axum-jwks/src/jwks.rs (MIT license)
use std::collections::HashMap;
use std::str::FromStr;

use axum_core::RequestExt;
use axum_extra::TypedHeader;
use axum_extra::headers::Authorization;
use axum_extra::headers::authorization::Bearer;
use jsonwebtoken::jwk::{AlgorithmParameters, JwkSet, KeyAlgorithm};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use secrecy::SecretString;
use serde_json::{Map, Value};

use crate::client::Client;
use crate::http::Request;
use crate::telemetry::log::RequestLog;
use crate::*;

#[cfg(test)]
#[path = "jwt_tests.rs"]
mod tests;

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
	providers: Vec<Provider>,
}

#[derive(Clone)]
struct Provider {
	issuer: String,
	keys: HashMap<String, Jwk>,
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
			providers: &'a Vec<Provider>,
		}
		Serde {
			mode: self.mode,
			providers: &self.providers,
		}
		.serialize(serializer)
	}
}

impl serde::Serialize for Provider {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		#[derive(serde::Serialize)]
		pub struct Serde<'a> {
			issuer: &'a str,
			keys: Vec<&'a str>,
		}
		Serde {
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
#[serde(untagged, rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub enum LocalJwtConfig {
	Multi {
		#[serde(default)]
		mode: Mode,
		providers: Vec<ProviderConfig>,
	},
	Single {
		#[serde(default)]
		mode: Mode,
		issuer: String,
		audiences: Vec<String>,
		jwks: serdes::FileInlineOrRemote,
	},
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ProviderConfig {
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
		let (mode, providers_cfg) = match self {
			LocalJwtConfig::Multi { mode, providers } => (mode, providers),
			LocalJwtConfig::Single {
				mode,
				issuer,
				audiences,
				jwks,
			} => (
				mode,
				vec![ProviderConfig {
					issuer,
					audiences,
					jwks,
				}],
			),
		};

		let mut providers = Vec::with_capacity(providers_cfg.len());
		for pc in providers_cfg {
			let jwks: JwkSet = pc
				.jwks
				.load::<JwkSet>(client.clone())
				.await
				.map_err(JwkError::JwkLoadError)?;
			let provider = Provider::from_jwks(jwks, pc.issuer, pc.audiences)?;
			providers.push(provider);
		}
		Ok(Jwt { mode, providers })
	}
}

impl Provider {
	pub fn from_jwks(
		jwks: JwkSet,
		issuer: String,
		audiences: Vec<String>,
	) -> Result<Provider, JwkError> {
		let mut keys = HashMap::new();
		let to_supported_alg = |key_algorithm: Option<KeyAlgorithm>| match key_algorithm {
			Some(key_alg) => jsonwebtoken::Algorithm::from_str(key_alg.to_string().as_str()).ok(),
			_ => None,
		};

		for jwk in jwks.keys {
			let kid = jwk.common.key_id.ok_or(JwkError::MissingKeyId)?;

			let decoding_key =
				match &jwk.algorithm {
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

			let supported_algorithms = match to_supported_alg(jwk.common.key_algorithm) {
				None => {
					// If they did not explicitly set the key algorithm, which is optional, then we can infer it
					// based on the algorithm properties.
					// Add each key algorithm in the correct family.
					match &jwk.algorithm {
						AlgorithmParameters::EllipticCurve(_) => {
							vec![Algorithm::ES256, Algorithm::ES384]
						},
						AlgorithmParameters::RSA(_) => {
							vec![Algorithm::RS256, Algorithm::RS384, Algorithm::RS512]
						},
						_ => unreachable!(),
					}
				},
				Some(explicit_alg) => {
					vec![explicit_alg]
				},
			};
			// The new() requires 1 algorithm, so just pass the first before we override it
			let mut validation = Validation::new(*supported_algorithms.first().unwrap());
			validation.algorithms = supported_algorithms;
			validation.set_audience(&audiences);
			validation.set_issuer(std::slice::from_ref(&issuer));

			keys.insert(
				kid,
				Jwk {
					decoding: decoding_key,
					validation,
				},
			);
		}

		Ok(Provider { issuer, keys })
	}
}

impl Jwt {
	pub fn from_jwks(
		jwks: JwkSet,
		mode: Mode,
		issuer: String,
		audiences: Vec<String>,
	) -> Result<Jwt, JwkError> {
		let provider = Provider::from_jwks(jwks, issuer, audiences)?;
		Ok(Jwt {
			mode,
			providers: vec![provider],
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
		let Ok(TypedHeader(Authorization(bearer))) = req
			.extract_parts::<TypedHeader<Authorization<Bearer>>>()
			.await
		else {
			// In strict mode, we require a token
			if self.mode == Mode::Strict {
				return Err(TokenError::Missing);
			}
			// Otherwise with no, don't attempt to authenticate.
			return Ok(());
		};
		let claims = match self.validate_claims(bearer.token()) {
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

		// Search for the key across all providers
		let key = self
			.providers
			.iter()
			.find_map(|provider| provider.keys.get(kid))
			.ok_or_else(|| {
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
