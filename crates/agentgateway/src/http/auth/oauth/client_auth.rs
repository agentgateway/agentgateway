use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use secrecy::{ExposeSecret, SecretString};

use crate::serdes::FileOrInline;
use crate::types::proto::{ProtoError, agent as proto};
use crate::{apply, schema_enum, ser_redact};

// Keep privateKeyJwt assertions short-lived to limit replay exposure while
// allowing reasonable clock skew and token endpoint latency.
const CLIENT_ASSERTION_LIFETIME: Duration = Duration::from_secs(300);

#[serde_with::serde_as]
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthClientAuth {
	/// `client_id` parameter identifying the gateway at the authorization server.
	pub client_id: String,
	/// RFC 6749 §2.3 client authentication method.
	#[serde(flatten)]
	pub method: OAuthClientAuthMethod,
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for OAuthClientAuth {
	fn schema_name() -> std::borrow::Cow<'static, str> {
		std::borrow::Cow::Borrowed("OAuthClientAuth")
	}

	fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
		<RawOAuthClientAuthConfig as schemars::JsonSchema>::json_schema(generator)
	}
}

impl<'de> serde::Deserialize<'de> for OAuthClientAuth {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		RawOAuthClientAuthConfig::deserialize(deserializer)?
			.try_into()
			.map_err(serde::de::Error::custom)
	}
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(untagged)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
enum RawOAuthClientAuthConfig {
	Tagged(RawOAuthClientAuth),
	DefaultClientSecretBasic(RawDefaultClientSecretBasicAuth),
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, tag = "method")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
enum RawOAuthClientAuth {
	/// `client_id`/`client_secret` sent in the HTTP Basic Authorization header (RFC 6749 §2.3.1).
	#[serde(rename_all = "camelCase")]
	ClientSecretBasic {
		/// `client_id` parameter identifying the gateway at the authorization server.
		client_id: String,
		#[cfg_attr(feature = "schema", schemars(with = "crate::serdes::FileOrInline"))]
		#[serde(
			rename = "clientSecret",
			deserialize_with = "crate::serdes::deser_key_from_file"
		)]
		client_secret: SecretString,
	},
	/// `client_id`/`client_secret` sent in the request form body.
	#[serde(rename_all = "camelCase")]
	ClientSecretPost {
		/// `client_id` parameter identifying the gateway at the authorization server.
		client_id: String,
		#[cfg_attr(
			feature = "schema",
			schemars(with = "Option<crate::serdes::FileOrInline>")
		)]
		#[serde(
			rename = "clientSecret",
			default,
			deserialize_with = "crate::serdes::deser_key_from_file_option"
		)]
		client_secret: Option<SecretString>,
	},
	/// `privateKeyJwt` client assertion (RFC 7523).
	#[serde(rename_all = "camelCase")]
	PrivateKeyJwt {
		/// `client_id` parameter identifying the gateway at the authorization server.
		client_id: String,
		/// PEM-encoded private signing key (RSA or EC, matching `alg`).
		#[cfg_attr(feature = "schema", schemars(with = "crate::serdes::FileOrInline"))]
		signing_key: FileOrInline,
		#[serde(default)]
		alg: SigningAlg,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		kid: Option<String>,
		assertion_audience: String,
	},
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct RawDefaultClientSecretBasicAuth {
	/// `client_id` parameter identifying the gateway at the authorization server.
	client_id: String,
	#[cfg_attr(feature = "schema", schemars(with = "crate::serdes::FileOrInline"))]
	#[serde(
		rename = "clientSecret",
		deserialize_with = "crate::serdes::deser_key_from_file"
	)]
	client_secret: SecretString,
}

impl TryFrom<RawOAuthClientAuthConfig> for OAuthClientAuth {
	type Error = String;

	fn try_from(raw: RawOAuthClientAuthConfig) -> Result<Self, Self::Error> {
		let (client_id, method) = match raw {
			RawOAuthClientAuthConfig::Tagged(RawOAuthClientAuth::ClientSecretBasic {
				client_id,
				client_secret,
			})
			| RawOAuthClientAuthConfig::DefaultClientSecretBasic(RawDefaultClientSecretBasicAuth {
				client_id,
				client_secret,
			}) => (
				client_id,
				OAuthClientAuthMethod::ClientSecretBasic { client_secret },
			),
			RawOAuthClientAuthConfig::Tagged(RawOAuthClientAuth::ClientSecretPost {
				client_id,
				client_secret,
			}) => (
				client_id,
				OAuthClientAuthMethod::ClientSecretPost { client_secret },
			),
			RawOAuthClientAuthConfig::Tagged(RawOAuthClientAuth::PrivateKeyJwt {
				client_id,
				signing_key,
				alg,
				kid,
				assertion_audience,
			}) => {
				let private_key_jwt = PrivateKeyJwt::try_from(RawPrivateKeyJwt {
					signing_key,
					alg,
					kid,
					assertion_audience,
				})?;
				(
					client_id,
					OAuthClientAuthMethod::PrivateKeyJwt(private_key_jwt),
				)
			},
		};
		Ok(Self { client_id, method })
	}
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "method")]
pub enum OAuthClientAuthMethod {
	/// `client_id`/`client_secret` sent in the HTTP Basic Authorization header (RFC 6749 §2.3.1).
	ClientSecretBasic {
		#[serde(rename = "clientSecret", serialize_with = "ser_redact")]
		client_secret: SecretString,
	},
	/// `client_id`/`client_secret` sent in the request form body.
	ClientSecretPost {
		#[serde(
			rename = "clientSecret",
			skip_serializing_if = "Option::is_none",
			serialize_with = "ser_redact"
		)]
		client_secret: Option<SecretString>,
	},
	/// `privateKeyJwt` client assertion (RFC 7523).
	#[serde(rename_all = "camelCase")]
	PrivateKeyJwt(PrivateKeyJwt),
}

#[derive(Clone, serde::Deserialize)]
#[serde(try_from = "RawPrivateKeyJwt", rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PrivateKeyJwt {
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	signing_key: ParsedEncodingKey,
	#[serde(default)]
	alg: SigningAlg,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	kid: Option<String>,
	assertion_audience: String,
}

impl fmt::Debug for PrivateKeyJwt {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("PrivateKeyJwt")
			.field("signing_key", &"<redacted>")
			.field("alg", &self.alg)
			.field("kid", &self.kid)
			.field("assertion_audience", &self.assertion_audience)
			.finish()
	}
}

impl serde::Serialize for PrivateKeyJwt {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		use serde::ser::SerializeStruct;

		let mut state = serializer.serialize_struct("PrivateKeyJwt", 3)?;
		state.serialize_field("alg", &self.alg)?;
		state.serialize_field("kid", &self.kid)?;
		state.serialize_field("assertionAudience", &self.assertion_audience)?;
		state.end()
	}
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub(super) struct RawPrivateKeyJwt {
	/// PEM-encoded private signing key (RSA or EC, matching `alg`).
	#[cfg_attr(feature = "schema", schemars(with = "crate::serdes::FileOrInline"))]
	pub(super) signing_key: FileOrInline,
	#[serde(default)]
	pub(super) alg: SigningAlg,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub(super) kid: Option<String>,
	pub(super) assertion_audience: String,
}

impl TryFrom<RawPrivateKeyJwt> for PrivateKeyJwt {
	type Error = String;

	fn try_from(raw: RawPrivateKeyJwt) -> Result<Self, Self::Error> {
		if raw.assertion_audience.is_empty() {
			return Err("oauth private_key_jwt assertion_audience must not be empty".into());
		}
		// TODO: file-based keys are read once at config load; consider reload/rotation (K8s secret remounts need a restart)
		let pem = raw
			.signing_key
			.load()
			.map_err(|e| format!("failed to load oauth private_key_jwt signing_key: {e}"))?;
		let signing_key = raw
			.alg
			.encoding_key(pem.trim().as_bytes())
			.map_err(|e| format!("failed to parse oauth private_key_jwt signing_key: {e}"))?;
		Ok(Self {
			signing_key: ParsedEncodingKey(signing_key),
			alg: raw.alg,
			kid: raw.kid,
			assertion_audience: raw.assertion_audience,
		})
	}
}

struct ParsedEncodingKey(EncodingKey);

impl Clone for ParsedEncodingKey {
	fn clone(&self) -> Self {
		Self(self.0.clone())
	}
}

impl fmt::Debug for ParsedEncodingKey {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("<redacted>")
	}
}

impl OAuthClientAuth {
	pub fn new(client_id: String, method: OAuthClientAuthMethod) -> Self {
		Self { client_id, method }
	}

	pub(super) fn validate_load(&self) -> Result<(), String> {
		if self.client_id.is_empty() {
			return Err("oauth token exchange client_id must not be empty".into());
		}
		match &self.method {
			OAuthClientAuthMethod::ClientSecretBasic { client_secret } => {
				if client_secret.expose_secret().is_empty() {
					return Err(
						"oauth token exchange client_secret is required with the client_secret_basic method"
							.into(),
					);
				}
			},
			OAuthClientAuthMethod::ClientSecretPost { client_secret } => {
				if client_secret
					.as_ref()
					.is_some_and(|secret| secret.expose_secret().is_empty())
				{
					return Err("oauth token exchange client_secret must not be empty".into());
				}
			},
			OAuthClientAuthMethod::PrivateKeyJwt(key) => {
				if key.assertion_audience.is_empty() {
					return Err("oauth private_key_jwt assertion_audience must not be empty".into());
				}
			},
		}
		Ok(())
	}
}

impl TryFrom<proto::OAuthClientAuth> for OAuthClientAuth {
	type Error = ProtoError;

	fn try_from(c: proto::OAuthClientAuth) -> Result<Self, Self::Error> {
		use proto::o_auth_client_auth::Method;

		let method = match Method::try_from(c.method) {
			Ok(Method::Unspecified | Method::ClientSecretBasic) => {
				if c.private_key_jwt.is_some() {
					return Err(ProtoError::Generic(
						"oauth private_key_jwt requires the PRIVATE_KEY_JWT method".into(),
					));
				}
				OAuthClientAuthMethod::ClientSecretBasic {
					client_secret: c.client_secret.map(Into::into).unwrap_or_else(|| "".into()),
				}
			},
			Ok(Method::ClientSecretPost) => {
				if c.private_key_jwt.is_some() {
					return Err(ProtoError::Generic(
						"oauth private_key_jwt requires the PRIVATE_KEY_JWT method".into(),
					));
				}
				OAuthClientAuthMethod::ClientSecretPost {
					client_secret: c.client_secret.map(Into::into),
				}
			},
			Ok(Method::PrivateKeyJwt) => {
				if c.client_secret.is_some() {
					return Err(ProtoError::Generic(
						"oauth private_key_jwt must not set client_secret".into(),
					));
				}
				OAuthClientAuthMethod::PrivateKeyJwt(
					c.private_key_jwt
						.ok_or_else(|| {
							ProtoError::Generic(
								"oauth private_key_jwt settings are required with the PRIVATE_KEY_JWT method"
									.into(),
							)
						})?
						.try_into()?,
				)
			},
			Err(_) => {
				return Err(ProtoError::EnumParse(
					"unknown oauth client auth method".into(),
				));
			},
		};
		let auth = Self {
			client_id: c.client_id,
			method,
		};
		auth.validate_load().map_err(ProtoError::Generic)?;
		Ok(auth)
	}
}

impl TryFrom<proto::o_auth_client_auth::PrivateKeyJwt> for PrivateKeyJwt {
	type Error = ProtoError;

	fn try_from(
		private_key_jwt: proto::o_auth_client_auth::PrivateKeyJwt,
	) -> Result<Self, Self::Error> {
		Self::try_from(RawPrivateKeyJwt {
			signing_key: FileOrInline::Inline(private_key_jwt.signing_key),
			alg: signing_alg_from_proto(private_key_jwt.alg)?,
			kid: private_key_jwt.kid,
			assertion_audience: private_key_jwt.assertion_audience,
		})
		.map_err(ProtoError::Generic)
	}
}

#[apply(schema_enum!)]
#[derive(Default)]
pub enum SigningAlg {
	#[default]
	#[serde(rename = "RS256")]
	Rs256,
	#[serde(rename = "RS384")]
	Rs384,
	#[serde(rename = "RS512")]
	Rs512,
	#[serde(rename = "ES256")]
	Es256,
	#[serde(rename = "ES384")]
	Es384,
}

impl SigningAlg {
	fn algorithm(self) -> Algorithm {
		match self {
			Self::Rs256 => Algorithm::RS256,
			Self::Rs384 => Algorithm::RS384,
			Self::Rs512 => Algorithm::RS512,
			Self::Es256 => Algorithm::ES256,
			Self::Es384 => Algorithm::ES384,
		}
	}

	fn encoding_key(self, pem: &[u8]) -> anyhow::Result<EncodingKey> {
		match self {
			Self::Rs256 | Self::Rs384 | Self::Rs512 => {
				EncodingKey::from_rsa_pem(pem).context("failed to load RSA signing key")
			},
			Self::Es256 | Self::Es384 => {
				EncodingKey::from_ec_pem(pem).context("failed to load EC signing key")
			},
		}
	}
}

fn signing_alg_from_proto(alg: i32) -> Result<SigningAlg, ProtoError> {
	use proto::o_auth_client_auth::private_key_jwt::SigningAlg as ProtoSigningAlg;

	match ProtoSigningAlg::try_from(alg) {
		Ok(ProtoSigningAlg::Unspecified) => Ok(SigningAlg::Rs256),
		Ok(ProtoSigningAlg::Rs256) => Ok(SigningAlg::Rs256),
		Ok(ProtoSigningAlg::Rs384) => Ok(SigningAlg::Rs384),
		Ok(ProtoSigningAlg::Rs512) => Ok(SigningAlg::Rs512),
		Ok(ProtoSigningAlg::Es256) => Ok(SigningAlg::Es256),
		Ok(ProtoSigningAlg::Es384) => Ok(SigningAlg::Es384),
		Err(_) => Err(ProtoError::EnumParse(
			"unknown oauth private_key_jwt signing alg".into(),
		)),
	}
}

pub(super) fn sign_client_assertion(
	client_id: &str,
	private_key: &PrivateKeyJwt,
) -> anyhow::Result<String> {
	#[derive(serde::Serialize)]
	struct ClientAssertionClaims<'a> {
		iss: &'a str,
		sub: &'a str,
		aud: &'a str,
		jti: String,
		iat: u64,
		exp: u64,
	}

	let now = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.context("system clock is before the unix epoch")?
		.as_secs();
	let claims = ClientAssertionClaims {
		iss: client_id,
		sub: client_id,
		aud: &private_key.assertion_audience,
		jti: uuid::Uuid::new_v4().to_string(),
		iat: now,
		exp: now + CLIENT_ASSERTION_LIFETIME.as_secs(),
	};

	let mut header = Header::new(private_key.alg.algorithm());
	header.kid = private_key.kid.clone();
	jsonwebtoken::encode(&header, &claims, &private_key.signing_key.0)
		.context("failed to sign client assertion")
}
