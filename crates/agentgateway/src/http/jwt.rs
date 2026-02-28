// Inspired by https://github.com/cdriehuys/axum-jwks/blob/main/axum-jwks/src/jwks.rs (MIT license)
use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use ::cel::types::dynamic::DynamicType;
use axum_core::RequestExt;
use axum_extra::TypedHeader;
use axum_extra::headers::Authorization;
use axum_extra::headers::authorization::Bearer;
use jsonwebtoken::jwk::{AlgorithmParameters, EllipticCurve, JwkSet, KeyAlgorithm, PublicKeyUse};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use secrecy::SecretString;
use serde_json::{Map, Value};

use crate::client::Client;
use crate::http::Request;
use crate::http::oidc::{OidcCallContext, OidcProvider};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::RequestLog;
use crate::types::agent::SimpleBackendReference;
use crate::*;

#[cfg(test)]
#[path = "jwt_tests.rs"]
mod tests;

#[derive(Clone, Debug, thiserror::Error, PartialEq)]
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
	#[error("the key {key_id:?} uses unsupported elliptic curve {curve:?}")]
	UnsupportedEllipticCurve {
		key_id: String,
		curve: EllipticCurve,
	},
	#[error("no usable signing keys found in JWKS")]
	NoUsableSigningKeys,
}

#[derive(Clone)]
pub struct Jwt {
	mode: Mode,
	forward: bool,
	providers: Vec<Provider>,
	oidc_infos: Vec<Arc<OidcInfo>>,
}

#[derive(Clone)]
pub struct OidcInfo {
	pub issuer: String,
	pub audiences: Option<Vec<String>>,
	pub provider_backend: Option<SimpleBackendReference>,
}

#[derive(Clone)]
pub struct Provider {
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
			forward: bool,
			providers: &'a Vec<Provider>,
		}
		Serde {
			mode: self.mode,
			forward: self.forward,
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
#[serde(untagged, deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub enum LocalJwtConfig {
	#[serde(rename_all = "camelCase")]
	Multi {
		#[serde(default)]
		mode: Mode,
		#[serde(default)]
		forward: bool,
		providers: Vec<ProviderConfig>,
	},
	#[serde(rename_all = "camelCase")]
	Single {
		#[serde(default)]
		mode: Mode,
		#[serde(default)]
		forward: bool,
		issuer: String,
		audiences: Option<Vec<String>>,
		jwks: serdes::FileInlineOrRemote,
		#[serde(default)]
		jwt_validation_options: JWTValidationOptions,
	},
	Oidc {
		#[serde(default)]
		mode: Mode,
		#[serde(default)]
		forward: bool,
		issuer: String,
		audiences: Option<Vec<String>>,
		#[serde(default, rename = "providerBackend", alias = "provider_backend")]
		provider_backend: Option<SimpleBackendReference>,
	},
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ProviderConfig {
	pub issuer: String,
	pub audiences: Option<Vec<String>>,
	pub jwks: serdes::FileInlineOrRemote,
	#[serde(default)]
	pub jwt_validation_options: JWTValidationOptions,
}

#[apply(schema_enum!)]
#[derive(Default)]
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

/// JWT validation options controlling which claims must be present in a token.
///
/// The `required_claims` set specifies which RFC 7519 registered claims must
/// exist in the token payload before validation proceeds. Only the following
/// values are recognized: `exp`, `nbf`, `aud`, `iss`, `sub`. Other registered
/// claims such as `iat` and `jti` are **not** enforced by the underlying
/// `jsonwebtoken` library and will be silently ignored.
///
/// This only enforces **presence**. Standard claims like `exp` and `nbf`
/// have their values validated independently (e.g., expiry is always checked
/// when the `exp` claim is present, regardless of this setting).
///
/// Defaults to `["exp"]`.
#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct JWTValidationOptions {
	/// Claims that must be present in the token before validation.
	/// Only "exp", "nbf", "aud", "iss", "sub" are enforced; others
	/// (including "iat" and "jti") are ignored.
	/// Defaults to ["exp"]. Use an empty list to require no claims.
	#[serde(default = "default_required_claims")]
	pub required_claims: HashSet<String>,
}

fn default_required_claims() -> HashSet<String> {
	HashSet::from(["exp".to_owned()])
}

/// The only claim names the jsonwebtoken library actually enforces.
const SUPPORTED_REQUIRED_CLAIMS: &[&str] = &["exp", "nbf", "aud", "iss", "sub"];

/// Log a warning for each claim in `required_claims` that the library silently ignores.
fn warn_unsupported_claims(required_claims: &HashSet<String>) {
	for claim in required_claims {
		if !SUPPORTED_REQUIRED_CLAIMS.contains(&claim.as_str()) {
			tracing::warn!(
				claim = %claim,
				supported = ?SUPPORTED_REQUIRED_CLAIMS,
				"ignoring unrecognized required claim"
			);
		}
	}
}

impl Default for JWTValidationOptions {
	fn default() -> Self {
		Self {
			required_claims: default_required_claims(),
		}
	}
}

impl LocalJwtConfig {
	pub async fn try_into(
		self,
		client: Client,
		oidc_provider: Arc<OidcProvider>,
	) -> Result<Jwt, JwkError> {
		match self {
			LocalJwtConfig::Multi {
				mode,
				forward,
				providers: providers_cfg,
			} => {
				let mut providers = Vec::with_capacity(providers_cfg.len());
				for pc in providers_cfg {
					let jwks: JwkSet = pc
						.jwks
						.load::<JwkSet>(client.clone())
						.await
						.map_err(JwkError::JwkLoadError)?;
					let provider =
						Provider::from_jwks(jwks, pc.issuer, pc.audiences, pc.jwt_validation_options)?;
					providers.push(provider);
				}
				Ok(Jwt {
					mode,
					forward,
					providers,
					oidc_infos: vec![],
				})
			},
			LocalJwtConfig::Single {
				mode,
				forward,
				issuer,
				audiences,
				jwks,
				jwt_validation_options,
			} => {
				let jwks: JwkSet = jwks
					.load::<JwkSet>(client.clone())
					.await
					.map_err(JwkError::JwkLoadError)?;
				let provider = Provider::from_jwks(jwks, issuer, audiences, jwt_validation_options)?;
				Ok(Jwt {
					mode,
					forward,
					providers: vec![provider],
					oidc_infos: vec![],
				})
			},
			LocalJwtConfig::Oidc {
				mode,
				forward,
				issuer,
				audiences,
				provider_backend,
			} => {
				if provider_backend.is_some() {
					return Ok(Jwt {
						mode,
						forward,
						providers: vec![],
						oidc_infos: vec![Arc::new(OidcInfo {
							issuer,
							audiences,
							provider_backend,
						})],
					});
				}

				let (_metadata, jwt) = oidc_provider
					.get_info(
						OidcCallContext::new(&client, None, provider_backend.as_ref()),
						&issuer,
						audiences.clone(),
					)
					.await
					.map_err(|e| JwkError::JwkLoadError(e.into()))?;
				let mut jwt = Arc::unwrap_or_clone(jwt);

				jwt.mode = mode; // Override mode from config
				jwt.forward = forward; // Override forward behavior from config

				// If audiences were provided, update validation for all providers in this jwt instance
				if let Some(audiences) = &audiences {
					for provider in &mut jwt.providers {
						for jwk in provider.keys.values_mut() {
							jwk.validation.set_audience(audiences);
						}
					}
				}

				jwt.oidc_infos = vec![Arc::new(OidcInfo {
					issuer,
					audiences,
					provider_backend,
				})];
				Ok(jwt)
			},
		}
	}
}

impl Provider {
	pub fn from_jwks(
		jwks: JwkSet,
		issuer: String,
		audiences: Option<Vec<String>>,
		jwt_validation_options: JWTValidationOptions,
	) -> Result<Provider, JwkError> {
		warn_unsupported_claims(&jwt_validation_options.required_claims);

		let mut keys = HashMap::new();
		let to_supported_alg = |key_algorithm: Option<KeyAlgorithm>| match key_algorithm {
			Some(key_alg) => jsonwebtoken::Algorithm::from_str(key_alg.to_string().as_str()).ok(),
			_ => None,
		};

		for jwk in jwks.keys {
			let kid = jwk.common.key_id.ok_or(JwkError::MissingKeyId)?;
			if let Some(public_key_use) = &jwk.common.public_key_use
				&& *public_key_use != PublicKeyUse::Signature
			{
				debug!(%kid, ?public_key_use, "Skipping non-signature JWK");
				continue;
			}

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
						AlgorithmParameters::EllipticCurve(ec) => match &ec.curve {
							EllipticCurve::P256 => vec![Algorithm::ES256],
							EllipticCurve::P384 => vec![Algorithm::ES384],
							curve => {
								return Err(JwkError::UnsupportedEllipticCurve {
									key_id: kid.clone(),
									curve: curve.clone(),
								});
							},
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
			let first_algorithm = *supported_algorithms
				.first()
				.expect("supported_algorithms is non-empty by construction");
			let mut validation = Validation::new(first_algorithm);
			validation.algorithms = supported_algorithms;
			// only set audience if audiences were provided
			// otherwise, disable audience validation
			if let Some(audiences) = &audiences {
				validation.set_audience(audiences);
			} else {
				validation.validate_aud = false;
			}
			validation.set_issuer(std::slice::from_ref(&issuer));

			// Override required_spec_claims with the user-configured set.
			// validate_exp remains true, so exp is still validated if present.
			validation.required_spec_claims = jwt_validation_options.required_claims.clone();

			keys.insert(
				kid,
				Jwk {
					decoding: decoding_key,
					validation,
				},
			);
		}

		if keys.is_empty() {
			return Err(JwkError::NoUsableSigningKeys);
		}

		Ok(Provider { issuer, keys })
	}
}

impl Jwt {
	pub fn from_providers(providers: Vec<Provider>, mode: Mode) -> Jwt {
		Self::from_providers_with_forward(providers, mode, false)
	}

	pub fn from_providers_with_forward(providers: Vec<Provider>, mode: Mode, forward: bool) -> Jwt {
		Jwt {
			mode,
			forward,
			providers,
			oidc_infos: vec![],
		}
	}

	pub fn from_providers_with_oidc_infos(
		providers: Vec<Provider>,
		mode: Mode,
		forward: bool,
		oidc_infos: Vec<OidcInfo>,
	) -> Jwt {
		Jwt {
			mode,
			forward,
			providers,
			oidc_infos: oidc_infos.into_iter().map(Arc::new).collect(),
		}
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

impl DynamicType for Claims {
	fn materialize(&self) -> cel::Value<'_> {
		self.inner.materialize()
	}

	fn field(&self, field: &str) -> Option<cel::Value<'_>> {
		self.inner.field(field)
	}
}

impl Serialize for Claims {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		self.inner.serialize(serializer)
	}
}

impl<'de> Deserialize<'de> for Claims {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		let inner = Map::deserialize(deserializer)?;
		Ok(Claims {
			inner,
			jwt: SecretString::new("".into()),
		})
	}
}

impl Jwt {
	pub async fn apply(
		&self,
		client: &Client,
		oidc_provider: &OidcProvider,
		policy_client: Option<&PolicyClient>,
		log: Option<&mut RequestLog>,
		req: &mut Request,
	) -> Result<(), TokenError> {
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

		let mut claims = self.validate_claims(bearer.token());

		if let Err(TokenError::UnknownKeyId(_)) = &claims
			&& !self.oidc_infos.is_empty()
		{
			for oidc in &self.oidc_infos {
				debug!(
					"Unknown key ID, attempting dynamic OIDC refresh for {}",
					oidc.issuer
				);
				match oidc_provider
					.validate_token(
						OidcCallContext::new(client, policy_client, oidc.provider_backend.as_ref()),
						&oidc.issuer,
						oidc.audiences.clone(),
						bearer.token(),
					)
					.await
				{
					Ok(c) => {
						claims = Ok(c);
						break;
					},
					Err(e) => debug!("Dynamic OIDC refresh failed for {}: {}", oidc.issuer, e),
				}
			}
		}

		let claims = match claims {
			Ok(claims) => claims,
			Err(e) if self.mode == Mode::Permissive => {
				debug!("token verification failed ({e}), continue due to permissive mode");
				return Ok(());
			},
			Err(e) => return Err(e),
		};
		if let Some(serde_json::Value::String(sub)) = claims.inner.get("sub")
			&& let Some(log) = log
		{
			log.jwt_sub = Some(sub.to_string());
		};
		// Keep the bearer token when explicitly configured to forward it.
		if !self.forward {
			req.headers_mut().remove(http::header::AUTHORIZATION);
		}
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
