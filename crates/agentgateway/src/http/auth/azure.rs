use crate::ser_redact;
use azure_core::credentials::{AccessToken, TokenCredential, TokenRequestOptions};
use azure_identity::UserAssignedId;
use secrecy::{ExposeSecret, SecretString};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::trace;

use crate::serdes::schema;
use crate::{apply, client};

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

/// Per-instance credential cache for [`AzureAuth`].
///
/// Each [`AzureAuth`] value owns its own cache so that different backends
/// (e.g. two `ExplicitConfig` entries with different client secrets) get
/// independent credentials instead of sharing a single global cache.
/// Clones share the same underlying `Arc`, so the credential is built at
/// most once per config instance.
#[derive(Default, Clone)]
pub struct AzureCredentialCache(
	Arc<tokio::sync::OnceCell<Arc<dyn azure_core::credentials::TokenCredential>>>,
);

impl std::fmt::Debug for AzureCredentialCache {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("AzureCredentialCache")
	}
}

#[apply(schema!)]
pub enum AzureAuth {
	/// Use explicit Azure credentials
	#[serde(rename_all = "camelCase")]
	ExplicitConfig {
		#[serde(flatten)]
		credential_source: AzureAuthCredentialSource,
		/// Cached credential, populated on first use.
		#[serde(skip)]
		#[cfg_attr(feature = "schema", schemars(skip))]
		cached_cred: AzureCredentialCache,
	},
	/// Use implicit Azure auth. Note that this is for developer use-cases only!
	DeveloperImplicit {
		/// Cached credential, populated on first use.
		#[serde(skip)]
		#[cfg_attr(feature = "schema", schemars(skip))]
		cached_cred: AzureCredentialCache,
	},
	/// Automatically detect authentication method based on environment.
	/// Uses Workload Identity on K8s, Managed Identity on Azure VMs, or Developer Tools locally.
	Implicit {
		/// Cached credential, populated on first use.
		#[serde(skip)]
		#[cfg_attr(feature = "schema", schemars(skip))]
		cached_cred: AzureCredentialCache,
	},
}

impl Default for AzureAuth {
	fn default() -> Self {
		Self::Implicit {
			cached_cred: Default::default(),
		}
	}
}

const SCOPES: &[&str] = &["https://cognitiveservices.azure.com/.default"];
const FOUNDRY_SCOPES: &[&str] = &["https://ai.azure.com/.default"];

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
		if cached_index != usize::MAX
			&& let Some((name, source)) = self.sources.get(cached_index)
		{
			trace!("DefaultAzureCredential: using cached credential: {name}");
			return source.get_token(scopes, options).await;
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
	std::env::var_os("IDENTITY_ENDPOINT").is_some() || std::env::var_os("MSI_ENDPOINT").is_some()
}

async fn build_credential(
	client: &client::Client,
	auth: &AzureAuth,
) -> anyhow::Result<Arc<dyn TokenCredential>> {
	let client_options = azure_core::http::ClientOptions {
		transport: Some(azure_core::http::Transport::new(Arc::new(client.clone()))),
		..Default::default()
	};
	match auth {
		AzureAuth::ExplicitConfig {
			credential_source, ..
		} => match credential_source {
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
				// Always construct ManagedIdentityCredentialOptions so that the
				// custom reqwest-backed transport is injected regardless of whether
				// a user-assigned identity is specified.
				//
				// Before this fix, .map() short-circuited to None for system-assigned
				// identity (managedIdentity: {}), causing ManagedIdentityCredential::new(None)
				// to fall back to NoopClient which panics on every IMDS request.
				// See: https://github.com/agentgateway/agentgateway/issues/900
				let options = azure_identity::ManagedIdentityCredentialOptions {
					user_assigned_id: user_assigned_identity.as_ref().map(|uami| match uami {
						AzureUserAssignedIdentity::ClientId(cid) => UserAssignedId::ClientId(cid.to_string()),
						AzureUserAssignedIdentity::ObjectId(oid) => UserAssignedId::ObjectId(oid.to_string()),
						AzureUserAssignedIdentity::ResourceId(rid) => {
							UserAssignedId::ResourceId(rid.to_string())
						},
					}),
					client_options,
				};
				Ok(azure_identity::ManagedIdentityCredential::new(Some(
					options,
				))?)
			},
			AzureAuthCredentialSource::WorkloadIdentity {} => {
				Ok(azure_identity::WorkloadIdentityCredential::new(Some(
					azure_identity::WorkloadIdentityCredentialOptions {
						credential_options: azure_identity::ClientAssertionCredentialOptions { client_options },
						..Default::default()
					},
				))?)
			},
		},
		AzureAuth::DeveloperImplicit { .. } => Ok(azure_identity::DeveloperToolsCredential::new(None)?),
		AzureAuth::Implicit { .. } => {
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
						trace!("DefaultAzureCredential: EnvironmentCredential construction failed: {e}");
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
					trace!("DefaultAzureCredential: added WorkloadIdentityCredential to chain");
					sources.push(("WorkloadIdentityCredential", cred));
				},
				Err(e) => {
					trace!("DefaultAzureCredential: WorkloadIdentityCredential not available: {e}");
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
						trace!(
							"DefaultAzureCredential: IMDS not reachable within {IMDS_PROBE_TIMEOUT:?}, skipping ManagedIdentityCredential"
						);
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
							trace!("DefaultAzureCredential: added ManagedIdentityCredential to chain");
							sources.push(("ManagedIdentityCredential", cred));
						},
						Err(e) => {
							trace!("DefaultAzureCredential: ManagedIdentityCredential not available: {e}");
							errors.push(format!("ManagedIdentityCredential: {e}"));
						},
					}
				} else {
					errors
						.push("ManagedIdentityCredential: IMDS not reachable (probe timed out)".to_string());
				}
			}

			// 4. DeveloperToolsCredential — Azure CLI and Azure Developer CLI.
			// This is the fallback for local development. The credential runs
			// `az account get-access-token` or `azd auth token` under the hood.
			match azure_identity::DeveloperToolsCredential::new(None) {
				Ok(cred) => {
					trace!("DefaultAzureCredential: added DeveloperToolsCredential to chain");
					sources.push(("DeveloperToolsCredential", cred));
				},
				Err(e) => {
					trace!("DefaultAzureCredential: DeveloperToolsCredential construction failed: {e}");
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
pub(super) async fn get_token(
	client: &client::Client,
	auth: &AzureAuth,
	target: &crate::types::agent::Target,
) -> anyhow::Result<http::HeaderValue> {
	let cache = match auth {
		AzureAuth::Implicit { cached_cred, .. } => &cached_cred.0,
		AzureAuth::DeveloperImplicit { cached_cred, .. } => &cached_cred.0,
		AzureAuth::ExplicitConfig { cached_cred, .. } => &cached_cred.0,
	};
	let cred = cache
		.get_or_try_init(|| build_credential(client, auth))
		.await?
		.clone();
	// Foundry endpoints (.services.ai.azure.com) require the ai.azure.com scope
	let is_foundry = matches!(target, crate::types::agent::Target::Hostname(h, _) if h.ends_with(".services.ai.azure.com"));
	let scopes = if is_foundry { FOUNDRY_SCOPES } else { SCOPES };
	let token = cred.get_token(scopes, None).await?;
	let mut hv = http::HeaderValue::from_str(&format!("Bearer {}", token.token.secret()))?;
	hv.set_sensitive(true);
	trace!("attached Azure token (scope: {})", scopes[0]);
	Ok(hv)
}
