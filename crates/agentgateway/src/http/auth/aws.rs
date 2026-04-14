use aws_config::{BehaviorVersion, SdkConfig};
use aws_credential_types::Credentials;
use aws_credential_types::provider::ProvideCredentials;
use aws_sigv4::http_request::{SignableBody, sign};
use aws_sigv4::sign::v4::SigningParams;
use secrecy::{ExposeSecret, SecretString};
use tokio::sync::OnceCell;

use crate::llm::bedrock::{AwsRegion, AwsServiceName};
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

pub(super) async fn sign_request(
	req: &mut http::Request,
	aws_auth: &AwsAuth,
) -> anyhow::Result<()> {
	let creds = load_credentials(aws_auth).await?.into();
	let lim = crate::http::buffer_limit(req);
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

	let service = req
		.extensions()
		.get::<AwsServiceName>()
		.map(|s| s.name)
		.unwrap_or("bedrock");
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
