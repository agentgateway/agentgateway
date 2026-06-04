use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use aws_config::sts::AssumeRoleProvider;
use aws_config::{BehaviorVersion, SdkConfig};
use aws_credential_types::Credentials;
use aws_credential_types::provider::ProvideCredentials;
use aws_sigv4::http_request::{SignableBody, sign};
use aws_sigv4::sign::v4::SigningParams;
use aws_types::region::Region;
use secrecy::{ExposeSecret, SecretString};
use tokio::sync::{Mutex, OnceCell};

use crate::llm::bedrock::AwsRegion;
use crate::*;

#[derive(Clone, Debug)]
pub struct DefaultAwsServiceName(pub String);

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
		/// AWS SigV4 signing service name (for example, "bedrock", "bedrock-agentcore", or "execute-api").
		#[serde(skip_serializing_if = "Option::is_none")]
		service_name: Option<String>,
	},
	/// Use implicit AWS authentication (environment variables, IAM roles, etc.)
	#[serde(rename_all = "camelCase")]
	Implicit {
		/// AWS SigV4 signing service name (for example, "bedrock", "bedrock-agentcore", or "execute-api").
		#[serde(skip_serializing_if = "Option::is_none")]
		service_name: Option<String>,
		/// Optional AWS STS role to assume before signing requests.
		#[serde(skip_serializing_if = "Option::is_none")]
		assume_role: Option<AwsAssumeRole>,
		/// Cached source credentials, populated on first use.
		#[serde(skip)]
		#[cfg_attr(feature = "schema", schemars(skip))]
		source_credentials_cache: AwsCredentialsCache,
		/// Cached AssumeRole credentials, populated on first use.
		#[serde(skip)]
		#[cfg_attr(feature = "schema", schemars(skip))]
		assume_role_cache: AwsAssumeRoleCache,
	},
}

#[derive(Default, Clone)]
pub struct AwsCredentialsCache(Arc<Mutex<Option<Credentials>>>);

impl std::fmt::Debug for AwsCredentialsCache {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("AwsCredentialsCache")
	}
}

#[derive(Default, Clone)]
pub struct AwsAssumeRoleCache(Arc<Mutex<HashMap<AssumeRoleCacheKey, Credentials>>>);

impl std::fmt::Debug for AwsAssumeRoleCache {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("AwsAssumeRoleCache")
	}
}

#[derive(PartialEq, Eq, Hash)]
#[apply(schema!)]
pub struct AwsAssumeRole {
	/// AWS IAM role ARN to assume.
	pub role_arn: String,
}

impl AwsAuth {
	fn service_name(&self) -> Option<&str> {
		match self {
			AwsAuth::ExplicitConfig { service_name, .. } | AwsAuth::Implicit { service_name, .. } => {
				service_name.as_deref()
			},
		}
	}

	fn assume_role(&self) -> Option<&AwsAssumeRole> {
		match self {
			AwsAuth::ExplicitConfig { .. } => None,
			AwsAuth::Implicit { assume_role, .. } => assume_role.as_ref(),
		}
	}

	fn assume_role_cache(&self) -> Option<&AwsAssumeRoleCache> {
		match self {
			AwsAuth::ExplicitConfig { .. } => None,
			AwsAuth::Implicit {
				assume_role_cache, ..
			} => Some(assume_role_cache),
		}
	}

	fn source_credentials_cache(&self) -> Option<&AwsCredentialsCache> {
		match self {
			AwsAuth::ExplicitConfig { .. } => None,
			AwsAuth::Implicit {
				source_credentials_cache,
				..
			} => Some(source_credentials_cache),
		}
	}
}

fn signing_service_name<'a>(req: &'a http::Request, aws_auth: &'a AwsAuth) -> &'a str {
	aws_auth
		.service_name()
		.or_else(|| {
			req
				.extensions()
				.get::<DefaultAwsServiceName>()
				.map(|default| default.0.as_str())
		})
		.unwrap_or("bedrock")
}

pub(super) async fn sign_request(
	req: &mut http::Request,
	aws_auth: &AwsAuth,
) -> anyhow::Result<()> {
	let lim = crate::http::buffer_limit(req);
	let orig_body = std::mem::take(req.body_mut());
	// Get the region based on auth mode
	let region = match aws_auth {
		AwsAuth::ExplicitConfig {
			region: Some(region),
			..
		} => region.as_str(),
		AwsAuth::ExplicitConfig { region: None, .. } | AwsAuth::Implicit { .. } => {
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
	let creds = load_credentials(aws_auth, region).await?.into();

	let service = signing_service_name(req, aws_auth);
	trace!("AWS signing with region: {}, service: {}", region, service);

	// Sign the request
	let signing_params = SigningParams::builder()
		.identity(&creds)
		.region(region)
		.name(service)
		.time(std::time::SystemTime::now())
		.settings(aws_sigv4::http_request::SigningSettings::default())
		.build()?
		.into();

	let body = http::read_body_with_limit(orig_body, lim).await?;
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
			.filter(|(k, _)| should_sign_header(k)),
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

fn should_sign_header(name: &str) -> bool {
	name == http::header::HOST.as_str()
		|| name == http::header::CONTENT_TYPE.as_str()
		|| name == http::header::DATE.as_str()
		|| name.starts_with("x-amz-")
		|| name.starts_with("x-amzn-")
}

static SDK_CONFIG: OnceCell<SdkConfig> = OnceCell::const_new();
async fn sdk_config<'a>() -> &'a SdkConfig {
	SDK_CONFIG
		.get_or_init(|| async { aws_config::load_defaults(BehaviorVersion::v2026_01_12()).await })
		.await
}

async fn load_credentials(aws_auth: &AwsAuth, signing_region: &str) -> anyhow::Result<Credentials> {
	if let (Some(assume_role), Some(cache)) = (aws_auth.assume_role(), aws_auth.assume_role_cache()) {
		load_assumed_credentials(assume_role, cache, signing_region).await
	} else {
		load_source_credentials(aws_auth).await
	}
}

async fn load_source_credentials(aws_auth: &AwsAuth) -> anyhow::Result<Credentials> {
	match aws_auth {
		AwsAuth::ExplicitConfig {
			access_key_id,
			secret_access_key,
			session_token,
			region: _,
			service_name: _,
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
		AwsAuth::Implicit { .. } => {
			let cache = aws_auth
				.source_credentials_cache()
				.expect("implicit AWS auth always has a source credential cache");
			{
				let mut cached = cache.0.lock().await;
				if let Some(creds) = cached.as_ref() {
					if credentials_valid(creds) {
						return Ok(creds.clone());
					}
					*cached = None;
				}
			}

			// Load AWS configuration and credentials from environment/IAM
			let config = Box::pin(sdk_config()).await;

			// Get credentials from the config
			let creds = config
				.credentials_provider()
				.ok_or(anyhow::anyhow!(
					"No credentials provider found in AWS config"
				))?
				.provide_credentials()
				.await?;
			*cache.0.lock().await = Some(creds.clone());
			Ok(creds)
		},
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AssumeRoleCacheKey {
	role_arn: String,
	resolved_sts_region: String,
}

const ASSUMED_CREDENTIAL_REFRESH_BUFFER: Duration = Duration::from_secs(60);

async fn load_assumed_credentials(
	assume_role: &AwsAssumeRole,
	cache: &AwsAssumeRoleCache,
	signing_region: &str,
) -> anyhow::Result<Credentials> {
	let sts_region = resolve_sts_region(assume_role, signing_region).await?;
	let key = AssumeRoleCacheKey {
		role_arn: assume_role.role_arn.clone(),
		resolved_sts_region: sts_region.clone(),
	};

	{
		let mut cached = cache.0.lock().await;
		cached.retain(|_, creds| credentials_valid(creds));
		if let Some(creds) = cached.get(&key) {
			return Ok(creds.clone());
		}
	}

	let config = Box::pin(sdk_config()).await;
	let builder = AssumeRoleProvider::builder(&assume_role.role_arn)
		.configure(config)
		.region(Region::new(sts_region));

	let source_credentials_provider = config.credentials_provider().ok_or(anyhow::anyhow!(
		"No credentials provider found in AWS config"
	))?;
	let provider = builder
		.build_from_provider(source_credentials_provider.clone())
		.await;
	let creds = provider.provide_credentials().await?;
	cache.0.lock().await.insert(key, creds.clone());
	Ok(creds)
}

async fn resolve_sts_region(
	_assume_role: &AwsAssumeRole,
	signing_region: &str,
) -> anyhow::Result<String> {
	if !signing_region.is_empty() {
		return Ok(signing_region.to_string());
	}
	let config = Box::pin(sdk_config()).await;
	config
		.region()
		.map(|r| r.as_ref().to_string())
		.ok_or(anyhow::anyhow!(
			"No region found in AWS config or request extensions"
		))
}

fn credentials_valid(creds: &Credentials) -> bool {
	match creds.expiry() {
		Some(expiry) => expiry
			.duration_since(SystemTime::now())
			.is_ok_and(|ttl| ttl > ASSUMED_CREDENTIAL_REFRESH_BUFFER),
		None => true,
	}
}
