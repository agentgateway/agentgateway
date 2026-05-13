use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context, anyhow};
use google_cloud_auth::credentials::{self, AccessTokenCredentials};
use headers::HeaderMapExt;
use http::HeaderMap;
use once_cell::sync::Lazy;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::Value;
use tracing::trace;

use crate::serdes::{FileOrInline, schema};
use crate::types::agent::Target;
use crate::{ConstString, apply, const_string, ser_redact};

const_string!(IdToken = "idToken");
const_string!(AccessToken = "accessToken");

#[apply(schema!)]
#[serde(untagged)]
pub enum GcpAuth {
	/// Fetch an id token
	#[serde(rename_all = "camelCase")]
	IdToken {
		r#type: IdToken,
		/// Audience for the token. If not set, the destination host will be used.
		audience: Option<String>,
		/// ADC-compatible Google credential JSON. If not set, ambient credentials are used.
		#[serde(
			default,
			serialize_with = "ser_redact",
			deserialize_with = "deser_optional_credential",
			skip_serializing_if = "Option::is_none"
		)]
		#[cfg_attr(feature = "schema", schemars(with = "Option<FileOrInline>"))]
		credential: Option<GcpCredential>,
	},
	/// Fetch an access token
	AccessToken {
		#[serde(default)]
		r#type: Option<AccessToken>,
		/// ADC-compatible Google credential JSON. If not set, ambient credentials are used.
		#[serde(
			default,
			serialize_with = "ser_redact",
			deserialize_with = "deser_optional_credential",
			skip_serializing_if = "Option::is_none"
		)]
		#[cfg_attr(feature = "schema", schemars(with = "Option<FileOrInline>"))]
		credential: Option<GcpCredential>,
	},
}

impl Default for GcpAuth {
	fn default() -> Self {
		Self::AccessToken {
			r#type: Default::default(),
			credential: Default::default(),
		}
	}
}

fn deser_optional_credential<'de, D>(deserializer: D) -> Result<Option<GcpCredential>, D::Error>
where
	D: serde::Deserializer<'de>,
{
	Option::<FileOrInline>::deserialize(deserializer)?
		.map(|input| {
			input
				.load()
				.map(|s| SecretString::from(s.trim().to_string()))
				.map_err(|e| serde::de::Error::custom(e.to_string()))
				.and_then(|credential| {
					GcpCredential::new(credential).map_err(|e| serde::de::Error::custom(e.to_string()))
				})
		})
		.transpose()
}

#[derive(Clone)]
pub struct GcpCredential {
	access_token: Option<AccessTokenCredentials>,
	raw: SecretString,
	credential_type: GcpCredentialType,
	id_tokens: Arc<Mutex<HashMap<String, Arc<credentials::idtoken::IDTokenCredentials>>>>,
	gdch_tokens: Arc<Mutex<HashMap<String, Arc<credentials::AccessTokenCredentials>>>>,
}

impl GcpCredential {
	pub(crate) fn new(raw: SecretString) -> anyhow::Result<Self> {
		let json = parse_credential_json(&raw)?;
		let credential_type = GcpCredentialType::from_json(&json)?;
		let access_token = match credential_type {
			GcpCredentialType::GdchServiceAccount => None,
			GcpCredentialType::Other => Some(build_access_token_credentials(json)?),
		};
		Ok(Self {
			access_token,
			raw,
			credential_type,
			id_tokens: Default::default(),
			gdch_tokens: Default::default(),
		})
	}
}

impl std::fmt::Debug for GcpCredential {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("GcpCredential")
	}
}

static CREDS: Lazy<anyhow::Result<credentials::AccessTokenCredentials>> = Lazy::new(|| {
	credentials::Builder::default()
		.build_access_token_credentials()
		.map_err(Into::into)
});

fn creds() -> anyhow::Result<&'static credentials::AccessTokenCredentials> {
	match CREDS.as_ref() {
		Ok(creds) => Ok(creds),
		Err(e) => {
			let msg = format!("Failed to initialize credentials: {}", e);
			Err(anyhow::anyhow!(msg))
		},
	}
}

fn parse_credential_json(credential: &SecretString) -> anyhow::Result<Value> {
	serde_json::from_str(credential.expose_secret()).context("failed to parse GCP credential JSON")
}

fn extract_credential_type(json: &Value) -> anyhow::Result<&str> {
	json
		.get("type")
		.ok_or_else(|| anyhow!("GCP credential JSON missing `type` field"))?
		.as_str()
		.ok_or_else(|| anyhow!("GCP credential JSON `type` field is not a string"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GcpCredentialType {
	GdchServiceAccount,
	Other,
}

impl GcpCredentialType {
	fn from_json(json: &Value) -> anyhow::Result<Self> {
		match extract_credential_type(json)? {
			"gdch_service_account" => Ok(Self::GdchServiceAccount),
			_ => Ok(Self::Other),
		}
	}
}

fn build_access_token_credentials(json: Value) -> anyhow::Result<AccessTokenCredentials> {
	match extract_credential_type(&json)? {
		"authorized_user" => {
			Ok(credentials::user_account::Builder::new(json).build_access_token_credentials()?)
		},
		"service_account" => {
			Ok(credentials::service_account::Builder::new(json).build_access_token_credentials()?)
		},
		"impersonated_service_account" => {
			Ok(credentials::impersonated::Builder::new(json).build_access_token_credentials()?)
		},
		"external_account" => {
			Ok(credentials::external_account::Builder::new(json).build_access_token_credentials()?)
		},
		"gdch_service_account" => Err(anyhow!(
			"GCP gdch_service_account credentials require idToken auth with an audience"
		)),
		cred_type => Err(anyhow!("unsupported GCP credential type: {cred_type}")),
	}
}

async fn explicit_access_token(credential: &GcpCredential) -> anyhow::Result<String> {
	let access_token = credential.access_token.as_ref().ok_or_else(|| {
		anyhow!("GCP gdch_service_account credentials require idToken auth with an audience")
	})?;
	let token = access_token.access_token().await?;
	Ok(token.token)
}

enum IdTokenBuilder {
	UserAccount(credentials::idtoken::IDTokenCredentials),
	GdchServiceAccount(Value),
	Other,
}

static ID_TOKEN_BUILDER: Lazy<anyhow::Result<IdTokenBuilder>> =
	Lazy::new(|| match adc::adc_credential_type()? {
		adc::AdcCredentialType::AuthorizedUser(adc) => Ok(IdTokenBuilder::UserAccount(
			credentials::idtoken::user_account::Builder::new(adc).build()?,
		)),
		adc::AdcCredentialType::GdchServiceAccount(adc) => Ok(IdTokenBuilder::GdchServiceAccount(adc)),
		adc::AdcCredentialType::Other => Ok(IdTokenBuilder::Other),
	});

#[allow(clippy::type_complexity)]
static ID_TOKEN_CACHE: Lazy<
	Arc<Mutex<HashMap<String, Arc<credentials::idtoken::IDTokenCredentials>>>>,
> = Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

#[allow(clippy::type_complexity)]
static GDCH_TOKEN_CACHE: Lazy<
	Arc<Mutex<HashMap<String, Arc<credentials::AccessTokenCredentials>>>>,
> = Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

fn build_id_token_credentials(
	aud: &str,
	credential: &SecretString,
) -> anyhow::Result<credentials::idtoken::IDTokenCredentials> {
	let json = parse_credential_json(credential)?;
	match extract_credential_type(&json)? {
		"authorized_user" => Ok(credentials::idtoken::user_account::Builder::new(json).build()?),
		"service_account" => {
			Ok(credentials::idtoken::service_account::Builder::new(aud, json).build()?)
		},
		"impersonated_service_account" => Ok(
			credentials::idtoken::impersonated::Builder::new(aud, json)
				.with_include_email()
				.build()?,
		),
		"external_account" => Err(anyhow!(
			"GCP external_account credentials do not support idToken auth"
		)),
		cred_type => Err(anyhow!("unsupported GCP credential type: {cred_type}")),
	}
}

async fn explicit_id_token(aud: &str, credential: &GcpCredential) -> anyhow::Result<String> {
	if credential.credential_type == GcpCredentialType::GdchServiceAccount {
		return explicit_gdch_token(aud, credential).await;
	}

	let id_token_creds = {
		let mut cache_guard = credential.id_tokens.lock().unwrap();
		if let Some(creds) = cache_guard.get(aud) {
			creds.clone()
		} else {
			let creds = Arc::new(build_id_token_credentials(aud, &credential.raw)?);
			cache_guard.insert(aud.to_string(), creds.clone());
			creds
		}
	};
	Ok(id_token_creds.id_token().await?)
}

async fn explicit_gdch_token(aud: &str, credential: &GcpCredential) -> anyhow::Result<String> {
	let access_token_creds = {
		let mut cache_guard = credential.gdch_tokens.lock().unwrap();
		if let Some(creds) = cache_guard.get(aud) {
			creds.clone()
		} else {
			let creds = Arc::new(build_gdch_access_token_credentials(aud, &credential.raw)?);
			cache_guard.insert(aud.to_string(), creds.clone());
			creds
		}
	};

	let token = access_token_creds.access_token().await?;
	Ok(token.token)
}

fn build_gdch_access_token_credentials(
	aud: &str,
	credential: &SecretString,
) -> anyhow::Result<credentials::AccessTokenCredentials> {
	let json = parse_credential_json(credential)?;
	credentials::gdch::Builder::new(aud, json)
		.build_access_token_credentials()
		.map_err(anyhow::Error::from)
}

async fn fetch_id_token(aud: &str) -> anyhow::Result<String> {
	match ID_TOKEN_BUILDER.as_ref() {
		Ok(creds) => match creds {
			IdTokenBuilder::UserAccount(c) => Ok(c.id_token().await?),
			IdTokenBuilder::GdchServiceAccount(adc) => {
				let cache = GDCH_TOKEN_CACHE.clone();
				let access_token_creds = {
					let mut cache_guard = cache.lock().unwrap();
					if !cache_guard.contains_key(aud) {
						let access_token_creds =
							credentials::gdch::Builder::new(aud, adc.clone()).build_access_token_credentials()?;
						let v = Arc::new(access_token_creds);
						cache_guard.insert(aud.to_string(), v.clone());
						v
					} else {
						cache_guard.get(aud).unwrap().clone()
					}
				};

				let token = access_token_creds.access_token().await?;
				Ok(token.token)
			},
			IdTokenBuilder::Other => {
				// Check cache first, get or create the IDTokenCredentials for this audience
				let cache = ID_TOKEN_CACHE.clone();
				let id_token_creds = {
					let mut cache_guard = cache.lock().unwrap();
					// Get or create the IDTokenCredentials for this audience
					if !cache_guard.contains_key(aud) {
						let id_token_creds = credentials::idtoken::Builder::new(aud)
							.with_include_email()
							.build()?;
						let v = Arc::new(id_token_creds);
						cache_guard.insert(aud.to_string(), v.clone());
						v
					} else {
						// Clone the Arc so we can drop the lock before awaiting
						cache_guard.get(aud).unwrap().clone()
					}
				};

				// IDTokenCredentials handles caching internally, so just call id_token()
				// Lock is dropped, so we can safely await
				Ok(id_token_creds.id_token().await?)
			},
		},
		Err(e) => {
			let msg = format!("Failed to initialize credentials: {}", e);
			Err(anyhow::anyhow!(msg))
		},
	}
}

pub(super) async fn insert_token(
	g: &GcpAuth,
	call_target: &Target,
	hm: &mut HeaderMap,
) -> anyhow::Result<()> {
	let token = match g {
		GcpAuth::IdToken {
			audience,
			credential,
			..
		} => {
			let aud = match (audience, call_target) {
				(Some(aud), _) => Cow::Borrowed(aud.as_str()),
				(None, Target::Hostname(host, _)) => Cow::Owned(format!("https://{host}")),
				_ => anyhow::bail!("idToken auth requires a hostname target or explicit audience"),
			};
			match credential {
				Some(credential) => explicit_id_token(aud.as_ref(), credential).await?,
				None => fetch_id_token(aud.as_ref()).await?,
			}
		},
		GcpAuth::AccessToken { credential, .. } => match credential {
			Some(credential) => explicit_access_token(credential).await?,
			None => {
				let token = creds()?.access_token().await?;
				token.token
			},
		},
	};
	let header = headers::Authorization::bearer(&token)?;
	hm.typed_insert(header);
	trace!("attached GCP token");
	Ok(())
}

// The SDK doesn't make it easy to use idtokens with user ADC. See https://github.com/googleapis/google-cloud-rust/issues/4215
// To allow this (for development use cases primarily), we copy-paste some of their code.
mod adc {
	use std::io;
	use std::path::PathBuf;

	use anyhow::anyhow;
	use serde_json::Value;

	fn adc_path() -> Option<PathBuf> {
		if let Ok(path) = std::env::var("GOOGLE_APPLICATION_CREDENTIALS") {
			return Some(path.into());
		}
		Some(adc_well_known_path()?.into())
	}

	fn extract_credential_type(json: &Value) -> anyhow::Result<&str> {
		json
			.get("type")
			.ok_or_else(|| anyhow!("no `type` field found."))?
			.as_str()
			.ok_or_else(|| anyhow!("`type` field is not a string."))
	}

	pub enum AdcCredentialType {
		AuthorizedUser(Value),
		GdchServiceAccount(Value),
		Other,
	}

	pub fn adc_credential_type() -> anyhow::Result<AdcCredentialType> {
		let adc = load_adc()?;
		match adc {
			None => Ok(AdcCredentialType::Other),
			Some(d) => {
				let cred = extract_credential_type(&d)?;
				match cred {
					"authorized_user" => Ok(AdcCredentialType::AuthorizedUser(d)),
					"gdch_service_account" => Ok(AdcCredentialType::GdchServiceAccount(d)),
					_ => Ok(AdcCredentialType::Other),
				}
			},
		}
	}

	fn load_adc() -> anyhow::Result<Option<serde_json::Value>> {
		let Some(adc) = match adc_path() {
			None => Ok(None),
			Some(path) => match fs_err::read_to_string(&path) {
				Ok(contents) => Ok(Some(contents)),
				Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
				Err(e) => Err(anyhow::Error::new(e)),
			},
		}?
		else {
			return Ok(None);
		};
		Ok(serde_json::from_str(&adc)?)
	}

	/// The well-known path to ADC on Windows, as specified in [AIP-4113].
	#[cfg(target_os = "windows")]
	fn adc_well_known_path() -> Option<String> {
		std::env::var("APPDATA")
			.ok()
			.map(|root| root + "/gcloud/application_default_credentials.json")
	}

	/// The well-known path to ADC on Linux and Mac, as specified in [AIP-4113].
	#[cfg(not(target_os = "windows"))]
	fn adc_well_known_path() -> Option<String> {
		std::env::var("HOME")
			.ok()
			.map(|root| root + "/.config/gcloud/application_default_credentials.json")
	}
}
