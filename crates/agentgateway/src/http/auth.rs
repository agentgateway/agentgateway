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
	/// Automatically detect authentication method based on environment.
	/// Uses Workload Identity on K8s, Managed Identity on Azure VMs, or Developer Tools locally.
	Implicit {},
}

impl Default for AzureAuth {
	fn default() -> Self {
		Self::Implicit {}
	}
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
	Azure(Option<AzureAuth>),
}

#[derive(Clone)]
pub struct BackendInfo {
	pub target: BackendTarget,
	pub call_target: Target,
	pub inputs: Arc<ProxyInputs>,
}

pub async fn apply_backend_auth(
	backend_info: &BackendInfo,
	auth: &BackendAuth,
	req: &mut Request,
) -> Result<(), ProxyError> {
	match auth {
		BackendAuth::Passthrough {} => {
			// They should have a JWT policy defined. That will strip the token. Here we add it back
			if let Some(claim) = req.extensions().get::<Claims>()
				&& let Ok(mut token) =
					http::HeaderValue::from_str(&format!("Bearer {}", claim.jwt.expose_secret()))
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
			let auth = azure_auth.as_ref().unwrap_or(&AzureAuth::Implicit {});
			let token = azure::get_token(&backend_info.inputs.upstream, auth)
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
	use std::sync::atomic::{AtomicUsize, Ordering};
	use std::sync::Arc;

	use azure_core::credentials::{AccessToken, TokenCredential, TokenRequestOptions};
	use azure_identity::UserAssignedId;
	use secrecy::ExposeSecret;
	use tracing::trace;

	use crate::client;
	use crate::http::auth::{AzureAuth, AzureAuthCredentialSource, AzureUserAssignedIdentity};

	const SCOPES: &[&str] = &["https://cognitiveservices.azure.com/.default"];

	/// A credential chain that mirrors the Azure Go SDK's DefaultAzureCredential.
	///
	/// DefaultAzureCredential is an opinionated, preconfigured chain of credentials
	/// designed to support many environments along with the most common authentication
	/// flows and developer tools.
	///
	/// The chain tries each credential in order, stopping when one provides a token:
	///
	/// 1. **EnvironmentCredential** - Reads `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, and
	///    `AZURE_CLIENT_SECRET` to authenticate as a service principal. Most often used
	///    in server environments but can also be used locally.
	/// 2. **WorkloadIdentityCredential** - If deployed to a Kubernetes host with Workload
	///    Identity enabled (detected via `AZURE_FEDERATED_TOKEN_FILE`, `AZURE_TENANT_ID`,
	///    `AZURE_CLIENT_ID`), authenticates using the federated token.
	/// 3. **ManagedIdentityCredential** - If deployed to an Azure host with Managed Identity
	///    enabled (App Service, Azure VMs via IMDS, etc.), authenticates using that identity.
	///    Supports user-assigned identity via `AZURE_CLIENT_ID`.
	/// 4. **DeveloperToolsCredential** - Falls back to developer tools: Azure CLI
	///    (`az login`) and Azure Developer CLI (`azd auth login`).
	///
	/// Once a credential successfully provides a token, it is cached and used for all
	/// subsequent token requests.
	///
	/// Reference: <https://learn.microsoft.com/azure/developer/go/azure-sdk-authentication>
	struct DefaultAzureCredential {
		sources: Vec<(&'static str, Arc<dyn TokenCredential>)>,
		/// Index of the source that first provided a token.
		/// `usize::MAX` indicates no source has provided a token yet.
		cached_source_index: AtomicUsize,
	}

	impl std::fmt::Debug for DefaultAzureCredential {
		fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			f.write_str("DefaultAzureCredential")
		}
	}

	#[async_trait::async_trait]
	impl TokenCredential for DefaultAzureCredential {
		async fn get_token(
			&self,
			scopes: &[&str],
			options: Option<TokenRequestOptions<'_>>,
		) -> azure_core::Result<AccessToken> {
			// If a credential has previously succeeded, use it directly.
			let cached_index = self.cached_source_index.load(Ordering::Relaxed);
			if cached_index != usize::MAX {
				if let Some((name, source)) = self.sources.get(cached_index) {
					trace!("DefaultAzureCredential: using cached credential: {name}");
					return source.get_token(scopes, options).await;
				}
			}

			// Try each credential in order, caching the first one that succeeds.
			let mut errors = Vec::new();
			for (index, (name, source)) in self.sources.iter().enumerate() {
				match source.get_token(scopes, options.clone()).await {
					Ok(token) => {
						trace!("DefaultAzureCredential: authenticated with {name}");
						self.cached_source_index.store(index, Ordering::Relaxed);
						return Ok(token);
					},
					Err(error) => {
						trace!("DefaultAzureCredential: {name} failed: {error}");
						errors.push(error);
					},
				}
			}

			Err(azure_core::Error::with_message_fn(
				azure_core::error::ErrorKind::Credential,
				|| {
					format!(
						"DefaultAzureCredential: all credentials failed:\n{}",
						format_credential_errors(&errors)
					)
				},
			))
		}
	}

	fn format_credential_errors(errors: &[azure_core::Error]) -> String {
		use std::error::Error;
		errors
			.iter()
			.map(|e| {
				let mut current: Option<&dyn Error> = Some(e);
				let mut stack = vec![];
				while let Some(err) = current.take() {
					stack.push(err.to_string());
					current = err.source();
				}
				stack.join(" - ")
			})
			.collect::<Vec<String>>()
			.join("\n")
	}

	/// The IMDS endpoint used by ManagedIdentityCredential when no other
	/// managed-identity source is detected via environment variables.
	const IMDS_ADDR: &str = "169.254.169.254:80";

	/// Quick TCP probe timeout. If we can't connect to IMDS within this
	/// duration, skip ManagedIdentityCredential to avoid the SDK's long
	/// retry loop (~99 s with 5 retries + exponential backoff).
	const IMDS_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

	/// Returns true if IMDS appears reachable (TCP connect within timeout).
	async fn imds_is_reachable() -> bool {
		tokio::time::timeout(
			IMDS_PROBE_TIMEOUT,
			tokio::net::TcpStream::connect(IMDS_ADDR),
		)
		.await
		.map(|r| r.is_ok())
		.unwrap_or(false)
	}

	/// Returns true when a managed-identity env-var source is configured
	/// (App Service, Service Fabric, Cloud Shell, Arc). In those cases we
	/// skip the IMDS probe because the SDK will use the env-var endpoint
	/// instead.
	fn has_managed_identity_env_vars() -> bool {
		std::env::var_os("IDENTITY_ENDPOINT").is_some()
			|| std::env::var_os("MSI_ENDPOINT").is_some()
	}

	async fn token_credential_from_auth(
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
			AzureAuth::Implicit {} => {
				// Build a DefaultAzureCredential chain following the Azure Go SDK pattern.
				// Each credential is tried in order; the first to succeed is cached and
				// used for all subsequent requests.
				//
				// Order:
				// 1. EnvironmentCredential (service principal via env vars)
				// 2. WorkloadIdentityCredential (Kubernetes workload identity)
				// 3. ManagedIdentityCredential (Azure VMs, App Service, etc.)
				// 4. DeveloperToolsCredential (Azure CLI, Azure Developer CLI)
				let mut sources: Vec<(&'static str, Arc<dyn TokenCredential>)> = Vec::new();
				let mut errors: Vec<String> = Vec::new();

				// 1. EnvironmentCredential — authenticate as a service principal.
				// Checks AZURE_TENANT_ID + AZURE_CLIENT_ID + AZURE_CLIENT_SECRET.
				// This mirrors the Go SDK's EnvironmentCredential which also supports
				// certificate and username/password flows, but client secret is the
				// most common server-side pattern.
				if let (Ok(tenant_id), Ok(client_id), Ok(client_secret)) = (
					std::env::var("AZURE_TENANT_ID"),
					std::env::var("AZURE_CLIENT_ID"),
					std::env::var("AZURE_CLIENT_SECRET"),
				) {
					match azure_identity::ClientSecretCredential::new(
						&tenant_id,
						client_id,
						azure_core::credentials::Secret::new(client_secret),
						Some(azure_identity::ClientSecretCredentialOptions {
							client_options: client_options.clone(),
						}),
					) {
						Ok(cred) => {
							trace!("DefaultAzureCredential: added EnvironmentCredential to chain");
							sources.push(("EnvironmentCredential", cred));
						},
						Err(e) => {
							trace!(
								"DefaultAzureCredential: EnvironmentCredential construction failed: {e}"
							);
							errors.push(format!("EnvironmentCredential: {e}"));
						},
					}
				}

				// 2. WorkloadIdentityCredential — Kubernetes workload identity.
				// The constructor reads AZURE_FEDERATED_TOKEN_FILE, AZURE_TENANT_ID,
				// and AZURE_CLIENT_ID internally and returns an error if they're not set.
				match azure_identity::WorkloadIdentityCredential::new(Some(
					azure_identity::WorkloadIdentityCredentialOptions {
						credential_options: azure_identity::ClientAssertionCredentialOptions {
							client_options: client_options.clone(),
						},
						..Default::default()
					},
				)) {
					Ok(cred) => {
						trace!(
							"DefaultAzureCredential: added WorkloadIdentityCredential to chain"
						);
						sources.push(("WorkloadIdentityCredential", cred));
					},
					Err(e) => {
						trace!(
							"DefaultAzureCredential: WorkloadIdentityCredential not available: {e}"
						);
						errors.push(format!("WorkloadIdentityCredential: {e}"));
					},
				}

				// 3. ManagedIdentityCredential — Azure VMs, App Service, etc.
				// The constructor detects the managed identity source from env vars
				// (IDENTITY_ENDPOINT, MSI_ENDPOINT, etc.) and defaults to IMDS.
				// Supports user-assigned identity via AZURE_CLIENT_ID.
				//
				// When no env-var source is detected, the SDK falls back to IMDS at
				// 169.254.169.254. If we're not on an Azure VM, the SDK's internal
				// retry policy will hammer the unreachable endpoint for ~99 s before
				// giving up. To avoid this, we do a quick 1 s TCP probe first.
				{
					let should_try_mi = if has_managed_identity_env_vars() {
						// A known endpoint is set (App Service, Service Fabric, etc.).
						// Skip the IMDS probe — the SDK will use the env-var endpoint.
						trace!("DefaultAzureCredential: managed-identity env vars detected, skipping IMDS probe");
						true
					} else {
						let reachable = imds_is_reachable().await;
						if reachable {
							trace!("DefaultAzureCredential: IMDS is reachable");
						} else {
							trace!("DefaultAzureCredential: IMDS not reachable within {IMDS_PROBE_TIMEOUT:?}, skipping ManagedIdentityCredential");
						}
						reachable
					};

					if should_try_mi {
						let mi_options = azure_identity::ManagedIdentityCredentialOptions {
							user_assigned_id: std::env::var("AZURE_CLIENT_ID")
								.ok()
								.map(UserAssignedId::ClientId),
							client_options: client_options.clone(),
						};
						match azure_identity::ManagedIdentityCredential::new(Some(mi_options)) {
							Ok(cred) => {
								trace!(
									"DefaultAzureCredential: added ManagedIdentityCredential to chain"
								);
								sources.push(("ManagedIdentityCredential", cred));
							},
							Err(e) => {
								trace!(
									"DefaultAzureCredential: ManagedIdentityCredential not available: {e}"
								);
								errors.push(format!("ManagedIdentityCredential: {e}"));
							},
						}
					} else {
						errors.push("ManagedIdentityCredential: IMDS not reachable (probe timed out)".to_string());
					}
				}

				// 4. DeveloperToolsCredential — Azure CLI and Azure Developer CLI.
				// This is the fallback for local development. The credential runs
				// `az account get-access-token` or `azd auth token` under the hood.
				match azure_identity::DeveloperToolsCredential::new(None) {
					Ok(cred) => {
						trace!(
							"DefaultAzureCredential: added DeveloperToolsCredential to chain"
						);
						sources.push(("DeveloperToolsCredential", cred));
					},
					Err(e) => {
						trace!(
							"DefaultAzureCredential: DeveloperToolsCredential construction failed: {e}"
						);
						errors.push(format!("DeveloperToolsCredential: {e}"));
					},
				}

				if sources.is_empty() {
					anyhow::bail!(
						"DefaultAzureCredential: no credentials could be constructed. Errors:\n{}",
						errors.join("\n")
					);
				}

				if !errors.is_empty() {
					trace!(
						"DefaultAzureCredential: some credentials could not be constructed:\n{}",
						errors.join("\n")
					);
				}

				Ok(Arc::new(DefaultAzureCredential {
					sources,
					cached_source_index: AtomicUsize::new(usize::MAX),
				}))
			},
		}
	}
	pub async fn get_token(
		client: &client::Client,
		auth: &AzureAuth,
	) -> anyhow::Result<http::HeaderValue> {
		let cred = token_credential_from_auth(client, auth).await?;
		let token = cred.get_token(SCOPES, None).await?;
		let mut hv = http::HeaderValue::from_str(&format!("Bearer {}", token.token.secret()))?;
		hv.set_sensitive(true);
		trace!("attached Azure token");
		Ok(hv)
	}
}
