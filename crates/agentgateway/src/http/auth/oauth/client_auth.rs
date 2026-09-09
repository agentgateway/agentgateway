use std::fmt;
use std::time::Duration;

use anyhow::Context;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use rustls::pki_types::PrivateKeyDer;
use rustls::pki_types::pem::PemObject;
use secrecy::{ExposeSecret, SecretString};

use super::super::jws::{JwtSigningAlg, SigningKey, signing_alg_from_proto};
use super::super::{jwt_claim_times, unix_timestamp_now};
use super::{OAuthConfigWarning, log_config_warnings};
use crate::serdes::FileOrInline;
use crate::types::proto::{ProtoError, agent as proto};
use crate::{apply, schema_enum, ser_redact};

// Keep privateKeyJwt assertions short-lived to limit replay exposure while
// allowing reasonable clock skew and token endpoint latency.
const CLIENT_ASSERTION_LIFETIME: Duration = Duration::from_secs(300);
// Match Google auth's issuance margin to avoid future iat/nbf values under clock skew:
const CLIENT_ASSERTION_CLOCK_SKEW: Duration = Duration::from_secs(10);

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
		let (auth, warnings) = Self::from_raw(RawOAuthClientAuthConfig::deserialize(deserializer)?)
			.map_err(serde::de::Error::custom)?;
		log_config_warnings(warnings);
		Ok(auth)
	}
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(untagged)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
enum RawOAuthClientAuthConfig {
	Tagged(RawOAuthClientAuth),
	DefaultClientSecretBasic(RawDefaultClientSecretBasicAuth),
}

#[derive(Clone, serde::Deserialize)]
#[serde(transparent)]
pub(super) struct RedactedCertificate(FileOrInline);

impl fmt::Debug for RedactedCertificate {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("[REDACTED]")
	}
}

impl From<FileOrInline> for RedactedCertificate {
	fn from(value: FileOrInline) -> Self {
		Self(value)
	}
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
		#[serde(deserialize_with = "crate::serdes::deser_key_from_file")]
		signing_key: SecretString,
		/// PEM-encoded X.509 certificate chain, leaf first. The leaf public key must
		/// correspond to `signing_key` for token endpoints to validate assertions.
		/// A mismatch or comparison failure is reported as a load warning and does
		/// not prevent loading.
		#[cfg_attr(
			feature = "schema",
			schemars(with = "Option<crate::serdes::FileOrInline>")
		)]
		certificate: Option<RedactedCertificate>,
		/// JWS certificate header emitted from `certificate`. Required when `certificate` is set.
		certificate_header: Option<CertificateHeader>,
		#[serde(default)]
		alg: JwtSigningAlg,
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

impl OAuthClientAuth {
	fn from_raw(raw: RawOAuthClientAuthConfig) -> Result<(Self, Vec<OAuthConfigWarning>), String> {
		let mut warnings = Vec::new();
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
				let (private_key_jwt, warning) = PrivateKeyJwt::load(RawPrivateKeyJwt {
					signing_key,
					certificate,
					certificate_header,
					alg,
					kid,
					assertion_audience,
				})?;
				warnings.extend(warning);
				(
					client_id,
					OAuthClientAuthMethod::PrivateKeyJwt(private_key_jwt),
				)
			},
		};
		Ok((Self { client_id, method }, warnings))
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

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PrivateKeyJwt {
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	signing_key: SigningKey,
	#[serde(default)]
	alg: JwtSigningAlg,
	#[serde(skip_serializing_if = "Option::is_none")]
	kid: Option<String>,
	#[serde(flatten, skip_serializing_if = "Option::is_none")]
	certificate_header: Option<JwtCertificateHeader>,
	assertion_audience: String,
}

#[derive(Clone, serde::Serialize)]
#[serde(untagged)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
enum JwtCertificateHeader {
	X5c {
		x5c: Vec<String>,
	},
	X5tS256 {
		#[serde(rename = "x5t#S256")]
		thumbprint: String,
	},
}

impl fmt::Debug for PrivateKeyJwt {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("PrivateKeyJwt")
			.field("signing_key", &"[REDACTED]")
			.field("alg", &self.alg)
			.field("kid", &self.kid)
			.field("certificate_header", &self.certificate_header)
			.field("assertion_audience", &self.assertion_audience)
			.finish()
	}
}

impl fmt::Debug for JwtCertificateHeader {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::X5c { .. } => f.write_str("X5c([REDACTED])"),
			Self::X5tS256 { thumbprint } => f
				.debug_struct("X5tS256")
				.field("thumbprint", thumbprint)
				.finish(),
		}
	}
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub(super) struct RawPrivateKeyJwt {
	/// PEM-encoded private signing key (RSA or EC, matching `alg`).
	#[cfg_attr(feature = "schema", schemars(with = "crate::serdes::FileOrInline"))]
	#[serde(deserialize_with = "crate::serdes::deser_key_from_file")]
	pub(super) signing_key: SecretString,
	/// PEM-encoded X.509 certificate chain, leaf first. The leaf public key must
	/// correspond to `signing_key` for token endpoints to validate assertions.
	/// A mismatch or comparison failure is reported as a load warning and does
	/// not prevent loading.
	#[cfg_attr(
		feature = "schema",
		schemars(with = "Option<crate::serdes::FileOrInline>")
	)]
	pub(super) certificate: Option<RedactedCertificate>,
	/// JWS certificate header emitted from `certificate`. Required when `certificate` is set.
	pub(super) certificate_header: Option<CertificateHeader>,
	#[serde(default)]
	pub(super) alg: JwtSigningAlg,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub(super) kid: Option<String>,
	pub(super) assertion_audience: String,
}

impl PrivateKeyJwt {
	pub(super) fn load(raw: RawPrivateKeyJwt) -> Result<(Self, Option<OAuthConfigWarning>), String> {
		if raw.assertion_audience.is_empty() {
			return Err("oauth private_key_jwt assertion_audience must not be empty".into());
		}
		// TODO: file-based keys are loaded once at config load; consider reload/rotation (K8s secret remounts need a restart)
		let signing_key_pem = raw.signing_key.expose_secret();
		let signing_key = SigningKey::from_pem(raw.alg, signing_key_pem.trim().as_bytes())
			.map_err(|e| format!("failed to parse oauth private_key_jwt signing_key: {e}"))?;
		let (certificate_header, warning) = match (raw.certificate, raw.certificate_header) {
			(Some(certificate), Some(certificate_header)) => {
				let (certificate_header, warning) =
					load_certificate_headers(certificate, certificate_header, signing_key_pem)?;
				(Some(certificate_header), warning)
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
			(None, None) => (None, None),
		};
		Ok((
			Self {
				signing_key,
				alg: raw.alg,
				kid: raw.kid,
				certificate_header,
				assertion_audience: raw.assertion_audience,
			},
			warning,
		))
	}
}

fn load_certificate_headers(
	certificate: RedactedCertificate,
	certificate_header: CertificateHeader,
	signing_key_pem: &str,
) -> Result<(JwtCertificateHeader, Option<OAuthConfigWarning>), String> {
	let certificate_pem = certificate
		.0
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

	let warning = certificate_key_mismatch_warning(signing_key_pem, leaf.contents());

	let header = match certificate_header {
		CertificateHeader::X5c => JwtCertificateHeader::X5c {
			x5c: certificates
				.into_iter()
				.map(|certificate| STANDARD.encode(certificate.contents()))
				.collect(),
		},
		CertificateHeader::X5tS256 => JwtCertificateHeader::X5tS256 {
			thumbprint: URL_SAFE_NO_PAD.encode(crate::crypto::digest::sha256(leaf.contents())),
		},
	};
	Ok((header, warning))
}

fn certificate_key_mismatch_warning(
	signing_key_pem: &str,
	leaf_certificate_der: &[u8],
) -> Option<OAuthConfigWarning> {
	match certificate_key_matches(signing_key_pem, leaf_certificate_der) {
		Ok(true) => None,
		Ok(false) => Some(OAuthConfigWarning::CertificateKeyMismatch),
		Err(error) => Some(OAuthConfigWarning::CertificateKeyComparisonFailed(error)),
	}
}

fn certificate_key_matches(
	signing_key_pem: &str,
	leaf_certificate_der: &[u8],
) -> Result<bool, String> {
	let provider = crate::transport::tls::provider();
	let signing_key = crate::crypto::tls::key_provider(&provider)
		.load_private_key(
			PrivateKeyDer::from_pem_slice(signing_key_pem.as_bytes())
				.map_err(|e| format!("cannot parse signingKey for comparison: {e}"))?,
		)
		.map_err(|e| format!("cannot load signingKey for comparison: {e}"))?;

	let signing_key_spki = signing_key
		.public_key()
		.ok_or_else(|| "signingKey public key is unavailable".to_string())?;

	let (_, certificate) = x509_parser::parse_x509_certificate(leaf_certificate_der)
		.map_err(|e| format!("cannot parse certificate: {e}"))?;

	Ok(signing_key_spki.as_ref() == certificate.public_key().raw)
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

impl OAuthClientAuth {
	pub(super) fn from_proto(
		c: proto::OAuthClientAuth,
	) -> Result<(Self, Vec<OAuthConfigWarning>), ProtoError> {
		use proto::o_auth_client_auth::Method;

		let mut warnings = Vec::new();
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
				let private_key_jwt = c.private_key_jwt.ok_or_else(|| {
					ProtoError::Generic(
						"oauth private_key_jwt settings are required with the PRIVATE_KEY_JWT method".into(),
					)
				})?;
				let (private_key_jwt, warning) = load_private_key_jwt_from_proto(private_key_jwt)?;
				warnings.extend(warning);
				OAuthClientAuthMethod::PrivateKeyJwt(private_key_jwt)
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
		Ok((auth, warnings))
	}
}

fn load_private_key_jwt_from_proto(
	private_key_jwt: proto::o_auth_client_auth::PrivateKeyJwt,
) -> Result<(PrivateKeyJwt, Option<OAuthConfigWarning>), ProtoError> {
	PrivateKeyJwt::load(RawPrivateKeyJwt {
		signing_key: SecretString::from(private_key_jwt.signing_key),
		certificate: (!private_key_jwt.certificate.is_empty())
			.then_some(FileOrInline::Inline(private_key_jwt.certificate).into()),
		certificate_header: certificate_header_from_proto(private_key_jwt.certificate_header)?,
		alg: signing_alg_from_proto(private_key_jwt.alg)
			.ok_or_else(|| ProtoError::EnumParse("unknown oauth private_key_jwt signing alg".into()))?,
		kid: private_key_jwt.kid,
		assertion_audience: private_key_jwt.assertion_audience,
	})
	.map_err(ProtoError::Generic)
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
		nbf: u64,
		iat: u64,
		exp: u64,
	}

	let now = unix_timestamp_now()?;
	let times = jwt_claim_times(now, CLIENT_ASSERTION_LIFETIME, CLIENT_ASSERTION_CLOCK_SKEW)?;
	let claims = ClientAssertionClaims {
		iss: client_id,
		sub: client_id,
		aud: &private_key.assertion_audience,
		jti: uuid::Uuid::new_v4().to_string(),
		nbf: times.issued_at,
		iat: times.issued_at,
		exp: times.expires_at,
	};

	let mut header = private_key.alg.header(private_key.kid.clone());
	match &private_key.certificate_header {
		Some(JwtCertificateHeader::X5c { x5c }) => header.x5c = Some(x5c.clone()),
		Some(JwtCertificateHeader::X5tS256 { thumbprint }) => {
			header.x5t_s256 = Some(thumbprint.clone());
		},
		None => {},
	}
	private_key
		.signing_key
		.encode(&header, &claims)
		.context("failed to sign client assertion")
}
