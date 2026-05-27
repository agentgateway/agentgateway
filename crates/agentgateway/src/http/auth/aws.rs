use std::collections::HashMap;
use std::sync::LazyLock;
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
		/// Optional AWS STS role to assume before signing requests.
		#[serde(skip_serializing_if = "Option::is_none")]
		assume_role: Option<AwsAssumeRole>,
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
	},
}

#[derive(PartialEq, Eq, Hash)]
#[apply(schema!)]
pub struct AwsAssumeRole {
	/// AWS IAM role ARN to assume.
	pub role_arn: String,
	/// Optional STS role session name.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub session_name: Option<String>,
	/// Optional STS external ID for cross-account access.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub external_id: Option<String>,
	/// Optional STS role session duration.
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		with = "serde_dur_option"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub duration: Option<Duration>,
	/// Optional AWS region to use for the STS AssumeRole call.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub sts_region: Option<String>,
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
			AwsAuth::ExplicitConfig { assume_role, .. } | AwsAuth::Implicit { assume_role, .. } => {
				assume_role.as_ref()
			},
		}
	}
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

	let service = aws_auth.service_name().unwrap_or("bedrock");
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
	let source_credentials = load_source_credentials(aws_auth).await?;
	if let Some(assume_role) = aws_auth.assume_role() {
		load_assumed_credentials(source_credentials, assume_role, signing_region).await
	} else {
		Ok(source_credentials)
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
			assume_role: _,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AssumeRoleCacheKey {
	source_access_key_id: String,
	assume_role: AwsAssumeRole,
	resolved_sts_region: String,
}

static ASSUME_ROLE_CACHE: LazyLock<Mutex<HashMap<AssumeRoleCacheKey, Credentials>>> =
	LazyLock::new(|| Mutex::new(HashMap::new()));

const ASSUMED_CREDENTIAL_REFRESH_BUFFER: Duration = Duration::from_secs(60);

async fn load_assumed_credentials(
	source_credentials: Credentials,
	assume_role: &AwsAssumeRole,
	signing_region: &str,
) -> anyhow::Result<Credentials> {
	let sts_region = resolve_sts_region(assume_role, signing_region).await?;
	let key = AssumeRoleCacheKey {
		source_access_key_id: source_credentials.access_key_id().to_string(),
		assume_role: assume_role.clone(),
		resolved_sts_region: sts_region.clone(),
	};

	if let Some(creds) = ASSUME_ROLE_CACHE.lock().await.get(&key)
		&& credentials_valid(creds)
	{
		return Ok(creds.clone());
	}

	let config = Box::pin(sdk_config()).await;
	let mut builder = AssumeRoleProvider::builder(&assume_role.role_arn)
		.configure(config)
		.region(Region::new(sts_region));

	if let Some(session_name) = &assume_role.session_name {
		builder = builder.session_name(session_name);
	}
	if let Some(external_id) = &assume_role.external_id {
		builder = builder.external_id(external_id);
	}
	if let Some(duration) = assume_role.duration {
		builder = builder.session_length(duration);
	}

	let provider = builder.build_from_provider(source_credentials).await;
	let creds = provider.provide_credentials().await?;
	ASSUME_ROLE_CACHE.lock().await.insert(key, creds.clone());
	Ok(creds)
}

async fn resolve_sts_region(
	assume_role: &AwsAssumeRole,
	signing_region: &str,
) -> anyhow::Result<String> {
	if let Some(sts_region) = &assume_role.sts_region {
		return Ok(sts_region.clone());
	}
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
