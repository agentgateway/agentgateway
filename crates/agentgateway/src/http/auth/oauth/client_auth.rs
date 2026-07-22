use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rustls::pki_types::PrivateKeyDer;
use rustls::pki_types::pem::PemObject;
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use tracing::warn;

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
		/// PEM-encoded X.509 certificate chain, leaf first.
		#[cfg_attr(
			feature = "schema",
			schemars(with = "Option<crate::serdes::FileOrInline>")
		)]
		certificate: Option<FileOrInline>,
		/// JWS certificate header emitted from `certificate`. Required when `certificate` is set.
		certificate_header: Option<CertificateHeader>,
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
	/// OAuth 2.0 client secret sent via HTTP Basic auth to the authorization server.
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
				certificate,
				certificate_header,
				alg,
				kid,
				assertion_audience,
			}) => {
				let private_key_jwt = PrivateKeyJwt::try_from(RawPrivateKeyJwt {
					signing_key,
					certificate,
					certificate_header,
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

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(try_from = "RawPrivateKeyJwt", rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PrivateKeyJwt {
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	signing_key: ParsedEncodingKey,
	#[serde(default)]
	alg: SigningAlg,
	#[serde(skip_serializing_if = "Option::is_none")]
	kid: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	x5c: Option<Vec<String>>,
	#[serde(rename = "x5t#S256", skip_serializing_if = "Option::is_none")]
	x5t_s256: Option<String>,
	assertion_audience: String,
}

impl fmt::Debug for PrivateKeyJwt {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("PrivateKeyJwt")
			.field("signing_key", &"<redacted>")
			.field("alg", &self.alg)
			.field("kid", &self.kid)
			.field("x5c", &self.x5c)
			.field("x5t#S256", &self.x5t_s256)
			.field("assertion_audience", &self.assertion_audience)
			.finish()
	}
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub(super) struct RawPrivateKeyJwt {
	/// PEM-encoded private signing key (RSA or EC, matching `alg`).
	#[cfg_attr(feature = "schema", schemars(with = "crate::serdes::FileOrInline"))]
	pub(super) signing_key: FileOrInline,
	/// PEM-encoded X.509 certificate chain, leaf first.
	#[cfg_attr(
		feature = "schema",
		schemars(with = "Option<crate::serdes::FileOrInline>")
	)]
	pub(super) certificate: Option<FileOrInline>,
	/// JWS certificate header emitted from `certificate`. Required when `certificate` is set.
	pub(super) certificate_header: Option<CertificateHeader>,
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
		let signing_key_pem = raw
			.signing_key
			.load()
			.map_err(|e| format!("failed to load oauth private_key_jwt signing_key: {e}"))?;
		let signing_key = raw
			.alg
			.encoding_key(signing_key_pem.trim().as_bytes())
			.map_err(|e| format!("failed to parse oauth private_key_jwt signing_key: {e}"))?;
		let certificate_headers = match (raw.certificate, raw.certificate_header) {
			(Some(certificate), Some(certificate_header)) => {
				load_certificate_headers(certificate, certificate_header, &signing_key_pem)?
			},
			(Some(_), None) => {
				return Err(
					"oauth private_key_jwt certificate_header is required when certificate is set".into(),
				);
			},
			(None, Some(_)) => {
				return Err(
					"oauth private_key_jwt certificate is required when certificate_header is set".into(),
				);
			},
			(None, None) => CertificateHeaders::default(),
		};
		Ok(Self {
			signing_key: ParsedEncodingKey(signing_key),
			alg: raw.alg,
			kid: raw.kid,
			x5c: certificate_headers.x5c,
			x5t_s256: certificate_headers.x5t_s256,
			assertion_audience: raw.assertion_audience,
		})
	}
}

#[derive(Default)]
struct CertificateHeaders {
	x5c: Option<Vec<String>>,
	x5t_s256: Option<String>,
}

fn load_certificate_headers(
	certificate: FileOrInline,
	certificate_header: CertificateHeader,
	signing_key_pem: &str,
) -> Result<CertificateHeaders, String> {
	let certificate_pem = certificate
		.load()
		.map_err(|e| format!("failed to load oauth private_key_jwt certificate: {e}"))?;
	let certificates = pem::parse_many(certificate_pem)
		.map_err(|e| format!("failed to parse oauth private_key_jwt certificate: {e}"))?;
	let leaf = certificates.first().ok_or_else(|| {
		"failed to parse oauth private_key_jwt certificate: no PEM blocks found".to_string()
	})?;

	for certificate in &certificates {
		if certificate.tag() != "CERTIFICATE" {
			return Err(format!(
				"failed to parse oauth private_key_jwt certificate: expected CERTIFICATE PEM block, found {}",
				certificate.tag()
			));
		}
		x509_parser::parse_x509_certificate(certificate.contents())
			.map_err(|e| format!("failed to parse oauth private_key_jwt certificate: {e}"))?;
	}

	warn_if_certificate_key_mismatch(signing_key_pem, leaf.contents());

	Ok(match certificate_header {
		CertificateHeader::X5c => CertificateHeaders {
			x5c: Some(
				certificates
					.into_iter()
					.map(|certificate| STANDARD.encode(certificate.contents()))
					.collect(),
			),
			x5t_s256: None,
		},
		CertificateHeader::X5tS256 => CertificateHeaders {
			x5c: None,
			x5t_s256: Some(URL_SAFE_NO_PAD.encode(Sha256::digest(leaf.contents()))),
		},
	})
}

fn warn_if_certificate_key_mismatch(signing_key_pem: &str, leaf_certificate_der: &[u8]) {
	match certificate_key_matches(signing_key_pem, leaf_certificate_der) {
		Ok(true) => {},
		Ok(false) => {
			warn!("oauth private_key_jwt certificate public key does not match signing_key");
		},
		Err(error) => {
			warn!(%error, "unable to compare oauth private_key_jwt certificate public key with signing_key");
		},
	}
}

fn certificate_key_matches(
	signing_key_pem: &str,
	leaf_certificate_der: &[u8],
) -> Result<bool, String> {
	let signing_key = PrivateKeyDer::from_pem_slice(signing_key_pem.as_bytes()).map_err(|e| {
		format!("failed to validate oauth private_key_jwt signing_key against certificate: {e}")
	})?;
	let signing_key = crate::transport::tls::provider()
		.key_provider
		.load_private_key(signing_key)
		.map_err(|e| {
			format!("failed to validate oauth private_key_jwt signing_key against certificate: {e}")
		})?;
	let signing_key_spki = signing_key.public_key().ok_or_else(|| {
		"failed to validate oauth private_key_jwt signing_key against certificate: public key is unavailable"
			.to_string()
	})?;
	let (_, certificate) = x509_parser::parse_x509_certificate(leaf_certificate_der)
		.map_err(|e| format!("failed to parse oauth private_key_jwt certificate: {e}"))?;
	Ok(signing_key_spki.as_ref() == certificate.public_key().raw)
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
			certificate: (!private_key_jwt.certificate.is_empty())
				.then_some(FileOrInline::Inline(private_key_jwt.certificate)),
			certificate_header: certificate_header_from_proto(private_key_jwt.certificate_header)?,
			alg: signing_alg_from_proto(private_key_jwt.alg)?,
			kid: private_key_jwt.kid,
			assertion_audience: private_key_jwt.assertion_audience,
		})
		.map_err(ProtoError::Generic)
	}
}

#[apply(schema_enum!)]
pub enum CertificateHeader {
	/// Send the X.509 certificate chain in `x5c`.
	#[serde(rename = "x5c")]
	X5c,
	/// Send the leaf certificate's SHA-256 thumbprint in `x5t#S256`.
	#[serde(rename = "x5t#S256")]
	X5tS256,
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

fn certificate_header_from_proto(header: i32) -> Result<Option<CertificateHeader>, ProtoError> {
	use proto::o_auth_client_auth::private_key_jwt::CertificateHeader as ProtoCertificateHeader;

	match ProtoCertificateHeader::try_from(header) {
		Ok(ProtoCertificateHeader::Unspecified) => Ok(None),
		Ok(ProtoCertificateHeader::X5c) => Ok(Some(CertificateHeader::X5c)),
		Ok(ProtoCertificateHeader::X5tS256) => Ok(Some(CertificateHeader::X5tS256)),
		Err(_) => Err(ProtoError::EnumParse(
			"unknown oauth private_key_jwt certificate header".into(),
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
	header.x5c = private_key.x5c.clone();
	header.x5t_s256 = private_key.x5t_s256.clone();
	jsonwebtoken::encode(&header, &claims, &private_key.signing_key.0)
		.context("failed to sign client assertion")
}
