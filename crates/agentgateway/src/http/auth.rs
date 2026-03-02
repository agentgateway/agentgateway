use secrecy::{ExposeSecret, SecretString};

use crate::http::Request;
use crate::http::jwt::Claims;
use crate::proxy::ProxyError;
use crate::serdes::deser_key_from_file;
use crate::types::agent::{BackendTarget, Target};
use crate::*;

#[apply(schema!)]
#[serde(untagged)]
pub enum AwsAuth {
	/// Use explicit AWS credentials
	#[serde(rename_all = "camelCase")]
	ExplicitConfig {
		#[serde(serialize_with = "ser_redact")]
		#[cfg_attr(feature = "schema", schemars(with = "String"))]
		access_key_id: SecretString,
		#[serde(serialize_with = "ser_redact")]
		#[cfg_attr(feature = "schema", schemars(with = "String"))]
		secret_access_key: SecretString,
		region: Option<String>,
		#[serde(serialize_with = "ser_redact", skip_serializing_if = "Option::is_none")]
		#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
		session_token: Option<SecretString>,
		// TODO: make service configurable (only bedrock for now)
	},
	/// Use implicit AWS authentication (environment variables, IAM roles, etc.)
	Implicit {},
}

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
	},
	/// Fetch an access token
	AccessToken {
		#[serde(default)]
		r#type: Option<AccessToken>,
	},
}

impl Default for GcpAuth {
	fn default() -> Self {
		Self::AccessToken {
			r#type: Default::default(),
		}
	}
}

// The Rust sdk for Azure is the only one that requires users to manually specify their auth method
// for all non-developer use-cases. Therefore, we have to carry these different options in our API....
// More context here: https://github.com/Azure/azure-sdk-for-rust/issues/2283
#[apply(schema!)]
pub enum AzureAuthCredentialSource {
	ClientSecret {
		#[cfg_attr(feature = "schema", schemars(with = "String"))]
		tenant_id: String,
		#[cfg_attr(feature = "schema", schemars(with = "String"))]
		client_id: String,
		#[serde(serialize_with = "ser_redact")]
		#[cfg_attr(feature = "schema", schemars(with = "String"))]
		client_secret: SecretString,
	},
	#[serde(rename_all = "camelCase")]
	ManagedIdentity {
		user_assigned_identity: Option<AzureUserAssignedIdentity>,
	},
	WorkloadIdentity {},
}

#[apply(schema!)]
pub enum AzureUserAssignedIdentity {
	ClientId(String),
	ObjectId(String),
	ResourceId(String),
}

#[apply(schema!)]
pub enum AzureAuth {
	/// Use explicit Azure credentials
	#[serde(rename_all = "camelCase")]
	ExplicitConfig {
		#[serde(flatten)]
		credential_source: AzureAuthCredentialSource,
	},
	/// Use implicit Azure auth. Note that this is for developer use-cases only!
	DeveloperImplicit {},
}

#[apply(schema!)]
pub enum SimpleBackendAuth {
	Passthrough {},
	Key(
		#[cfg_attr(feature = "schema", schemars(with = "FileOrInline"))]
		#[serde(
			serialize_with = "ser_redact",
			deserialize_with = "deser_key_from_file"
		)]
		SecretString,
	),
}

impl From<SimpleBackendAuth> for BackendAuth {
	fn from(value: SimpleBackendAuth) -> Self {
		match value {
			SimpleBackendAuth::Passthrough {} => BackendAuth::Passthrough {},
			SimpleBackendAuth::Key(key) => BackendAuth::Key(key),
		}
	}
}

#[apply(schema!)]
pub enum BackendAuth {
	Passthrough {},
	Key(
		#[cfg_attr(feature = "schema", schemars(with = "FileOrInline"))]
		#[serde(
			serialize_with = "ser_redact",
			deserialize_with = "deser_key_from_file"
		)]
		SecretString,
	),
	#[serde(rename = "gcp")]
	Gcp(GcpAuth),
	#[serde(rename = "aws")]
	Aws(AwsAuth),
	#[serde(rename = "azure")]
	Azure(AzureAuth),
}

#[derive(Clone)]
pub struct BackendInfo {
	pub target: BackendTarget,
	pub call_target: Target,
	pub inputs: Arc<ProxyInputs>,
}

#[derive(Clone)]
pub struct PassthroughBearerToken(pub SecretString);

pub async fn apply_backend_auth(
	backend_info: &BackendInfo,
	auth: &BackendAuth,
	req: &mut Request,
) -> Result<(), ProxyError> {
	match auth {
		BackendAuth::Passthrough {} => {
			if let Some(token_value) = req
				.extensions()
				.get::<PassthroughBearerToken>()
				.map(|token| token.0.expose_secret())
				.or_else(|| {
					req
						.extensions()
						.get::<Claims>()
						.map(|claim| claim.jwt.expose_secret())
				}) && let Ok(mut token) = http::HeaderValue::from_str(&format!("Bearer {token_value}"))
			{
				token.set_sensitive(true);
				req.headers_mut().insert(http::header::AUTHORIZATION, token);
			}
		},
		BackendAuth::Key(k) => {
			// TODO: is it always a Bearer?
			if let Ok(mut token) = http::HeaderValue::from_str(&format!("Bearer {}", k.expose_secret())) {
				token.set_sensitive(true);
				req.headers_mut().insert(http::header::AUTHORIZATION, token);
			}
		},
		BackendAuth::Gcp(g) => {
			gcp::insert_token(g, &backend_info.call_target, req.headers_mut())
				.await
				.map_err(ProxyError::BackendAuthenticationFailed)?;
		},
		BackendAuth::Aws(_) => {
			// We handle this in 'apply_late_backend_auth' since it must come at the end (due to request signing)!
		},
		BackendAuth::Azure(azure_auth) => {
			let token = azure::get_token(&backend_info.inputs.upstream, azure_auth)
				.await
				.map_err(ProxyError::BackendAuthenticationFailed)?;
			req.headers_mut().insert(http::header::AUTHORIZATION, token);
		},
	}
	Ok(())
}

pub async fn apply_late_backend_auth(
	auth: Option<&BackendAuth>,
	req: &mut Request,
) -> Result<(), ProxyError> {
	let Some(auth) = auth else {
		return Ok(());
	};
	match auth {
		BackendAuth::Passthrough {} => {},
		BackendAuth::Key(_) => {},
		BackendAuth::Gcp(_) => {},
		BackendAuth::Aws(aws_auth) => {
			aws::sign_request(req, aws_auth)
				.await
				.map_err(ProxyError::BackendAuthenticationFailed)?;
		},
		BackendAuth::Azure(_) => {},
	};
	Ok(())
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;

mod gcp {
	use std::collections::HashMap;
	use std::sync::{Arc, Mutex};

	use google_cloud_auth::credentials;
	use headers::HeaderMapExt;
	use http::HeaderMap;
	use once_cell::sync::Lazy;
	use tracing::trace;

	use crate::http::auth::GcpAuth;
	use crate::types::agent::Target;

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

	struct IdTokenBuilder {
		user_account: Option<credentials::idtoken::IDTokenCredentials>,
	}

	static ID_TOKEN_BUILDER: Lazy<anyhow::Result<IdTokenBuilder>> = Lazy::new(|| {
		if let Some(adc) = adc::adc_is_authorized_user()? {
			Ok(IdTokenBuilder {
				user_account: Some(credentials::idtoken::user_account::Builder::new(adc).build()?),
			})
		} else {
			Ok(IdTokenBuilder { user_account: None })
		}
	});

	#[allow(clippy::type_complexity)]
	static ID_TOKEN_CACHE: Lazy<
		Arc<Mutex<HashMap<String, Arc<credentials::idtoken::IDTokenCredentials>>>>,
	> = Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

	async fn fetch_id_token(aud: &str) -> anyhow::Result<String> {
		match ID_TOKEN_BUILDER.as_ref() {
			Ok(creds) => match &creds.user_account {
				Some(c) => Ok(c.id_token().await?),
				None => {
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

	pub async fn insert_token(
		g: &GcpAuth,
		call_target: &Target,
		hm: &mut HeaderMap,
	) -> anyhow::Result<()> {
		let token = match g {
			GcpAuth::IdToken { audience, .. } => {
				let aud = match (audience, call_target) {
					(Some(aud), _) => aud.as_str(),
					(None, Target::Hostname(host, _)) => host.as_str(),
					_ => anyhow::bail!("idToken auth requires a hostname target or explicit audience"),
				};
				fetch_id_token(aud).await?
			},
			GcpAuth::AccessToken { .. } => {
				let token = creds()?.access_token().await?;
				token.token
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

		pub fn adc_is_authorized_user() -> anyhow::Result<Option<Value>> {
			let adc = load_adc()?;
			match adc {
				None => Ok(None),
				Some(d) => {
					let cred = extract_credential_type(&d)?;
					if cred == "authorized_user" {
						Ok(Some(d))
					} else {
						Ok(None)
					}
				},
			}
		}

		fn load_adc() -> anyhow::Result<Option<serde_json::Value>> {
			let Some(adc) = match adc_path() {
				None => Ok(None),
				Some(path) => match fs_err::read_to_string(&path) {
					Ok(contents) => Ok(Some(contents)),
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
}

mod aws {
	use aws_config::{BehaviorVersion, SdkConfig};
	use aws_credential_types::Credentials;
	use aws_credential_types::provider::ProvideCredentials;
	use aws_sigv4::http_request::{SignableBody, sign};
	use aws_sigv4::sign::v4::SigningParams;
	use http_body_util::BodyExt;
	use secrecy::ExposeSecret;
	use tokio::sync::OnceCell;

	use crate::http::auth::AwsAuth;
	use crate::llm::bedrock::AwsRegion;
	use crate::*;

	pub async fn sign_request(req: &mut http::Request, aws_auth: &AwsAuth) -> anyhow::Result<()> {
		let creds = load_credentials(aws_auth).await?.into();
		let orig_body = std::mem::take(req.body_mut());
		// Get the region based on auth mode
		let region = match aws_auth {
			AwsAuth::ExplicitConfig {
				region: Some(region),
				..
			} => region.as_str(),
			AwsAuth::ExplicitConfig { region: None, .. } | AwsAuth::Implicit {} => {
				// Try to get region from request extensions first, then fall back to AWS config
				if let Some(aws_region) = req.extensions().get::<AwsRegion>() {
					aws_region.region.as_str()
				} else {
					// Fall back to region from AWS config
					let config = Box::pin(sdk_config()).await;
					config.region().map(|r| r.as_ref()).ok_or(anyhow::anyhow!(
						"No region found in AWS config or request extensions"
					))?
				}
			},
		};

		trace!("AWS signing with region: {}, service: bedrock", region);

		// Sign the request
		let signing_params = SigningParams::builder()
			.identity(&creds)
			.region(region)
			.name("bedrock")
			.time(std::time::SystemTime::now())
			.settings(aws_sigv4::http_request::SigningSettings::default())
			.build()?
			.into();

		let body = orig_body.collect().await?.to_bytes();
		let signable_request = aws_sigv4::http_request::SignableRequest::new(
			req.method().as_str(),
			req.uri().to_string().replace("http://", "https://"),
			req
				.headers()
				.iter()
				.filter_map(|(k, v)| {
					std::str::from_utf8(v.as_bytes())
						.ok()
						.map(|v_str| (k.as_str(), v_str))
				})
				.filter(|(k, _)| k != &http::header::CONTENT_LENGTH),
			// SignableBody::UnsignedPayload,
			SignableBody::Bytes(body.as_ref()),
		)?;

		let (signature, _sig) = sign(signable_request, &signing_params)?.into_parts();
		signature.apply_to_request_http1x(req);

		req.headers_mut().insert(
			http::header::CONTENT_LENGTH,
			http::HeaderValue::from_str(&format!("{}", body.as_ref().len()))?,
		);
		*req.body_mut() = http::Body::from(body);

		trace!("signed AWS request");
		Ok(())
	}

	static SDK_CONFIG: OnceCell<SdkConfig> = OnceCell::const_new();
	async fn sdk_config<'a>() -> &'a SdkConfig {
		SDK_CONFIG
			.get_or_init(|| async { aws_config::load_defaults(BehaviorVersion::v2026_01_12()).await })
			.await
	}

	async fn load_credentials(aws_auth: &AwsAuth) -> anyhow::Result<Credentials> {
		match aws_auth {
			AwsAuth::ExplicitConfig {
				access_key_id,
				secret_access_key,
				session_token,
				region: _,
			} => {
				// Use explicit credentials
				let mut builder = Credentials::builder()
					.access_key_id(access_key_id.expose_secret())
					.secret_access_key(secret_access_key.expose_secret())
					.provider_name("bedrock");

				if let Some(token) = session_token {
					builder = builder.session_token(token.expose_secret());
				}

				Ok(builder.build())
			},
			AwsAuth::Implicit {} => {
				// Load AWS configuration and credentials from environment/IAM
				let config = Box::pin(sdk_config()).await;

				// Get credentials from the config
				// TODO this is not caching!!
				Ok(
					config
						.credentials_provider()
						.ok_or(anyhow::anyhow!(
							"No credentials provider found in AWS config"
						))?
						.provide_credentials()
						.await?,
				)
			},
		}
	}
}

mod azure {
	use std::sync::Arc;

	use azure_core::credentials::TokenCredential;
	use azure_identity::UserAssignedId;
	use secrecy::ExposeSecret;
	use tracing::trace;

	use crate::client;
	use crate::http::auth::{AzureAuth, AzureAuthCredentialSource, AzureUserAssignedIdentity};

	const SCOPES: &[&str] = &["https://cognitiveservices.azure.com/.default"];
	fn token_credential_from_auth(
		client: &client::Client,
		auth: &AzureAuth,
	) -> anyhow::Result<Arc<dyn TokenCredential>> {
		let client_options = azure_core::http::ClientOptions {
			transport: Some(azure_core::http::Transport::new(Arc::new(client.clone()))),
			..Default::default()
		};
		match auth {
			AzureAuth::ExplicitConfig { credential_source } => match credential_source {
				AzureAuthCredentialSource::ClientSecret {
					tenant_id,
					client_id,
					client_secret,
				} => Ok(azure_identity::ClientSecretCredential::new(
					tenant_id,
					client_id.to_string(),
					azure_core::credentials::Secret::new(client_secret.expose_secret().to_string()),
					Some(azure_identity::ClientSecretCredentialOptions { client_options }),
				)?),
				AzureAuthCredentialSource::ManagedIdentity {
					user_assigned_identity,
				} => {
					let options: Option<azure_identity::ManagedIdentityCredentialOptions> =
						user_assigned_identity.as_ref().map(|uami| {
							azure_identity::ManagedIdentityCredentialOptions {
								user_assigned_id: match uami {
									AzureUserAssignedIdentity::ClientId(cid) => {
										Some(UserAssignedId::ClientId(cid.to_string()))
									},
									AzureUserAssignedIdentity::ObjectId(oid) => {
										Some(UserAssignedId::ObjectId(oid.to_string()))
									},
									AzureUserAssignedIdentity::ResourceId(rid) => {
										Some(UserAssignedId::ResourceId(rid.to_string()))
									},
								},
								client_options,
							}
						});
					Ok(azure_identity::ManagedIdentityCredential::new(options)?)
				},
				AzureAuthCredentialSource::WorkloadIdentity {} => {
					Ok(azure_identity::WorkloadIdentityCredential::new(Some(
						azure_identity::WorkloadIdentityCredentialOptions {
							credential_options: azure_identity::ClientAssertionCredentialOptions {
								client_options,
							},
							..Default::default()
						},
					))?)
				},
			},
			AzureAuth::DeveloperImplicit {} => Ok(azure_identity::DeveloperToolsCredential::new(None)?),
		}
	}
	pub async fn get_token(
		client: &client::Client,
		auth: &AzureAuth,
	) -> anyhow::Result<http::HeaderValue> {
		let cred = token_credential_from_auth(client, auth)?;
		let token = cred.get_token(SCOPES, None).await?;
		let mut hv = http::HeaderValue::from_str(&format!("Bearer {}", token.token.secret()))?;
		hv.set_sensitive(true);
		trace!("attached Azure token");
		Ok(hv)
	}
}
