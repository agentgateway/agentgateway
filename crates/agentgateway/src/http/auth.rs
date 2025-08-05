use once_cell::sync::Lazy;
use secrecy::{ExposeSecret, SecretString};
use tracing::trace;

use crate::http::Request;
use crate::http::jwt::Claims;
use crate::llm::bedrock::AwsRegion;
use crate::proxy::ProxyError;
use crate::serdes::deser_key_from_file;
use crate::*;

// TODO: xds support
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
	Gcp {},
	#[serde(rename = "aws")]
	Aws {},
}

pub async fn apply_backend_auth(
	auth: Option<&BackendAuth>,
	req: &mut Request,
) -> Result<(), ProxyError> {
	let Some(auth) = auth else {
		return Ok(());
	};
	match auth {
		BackendAuth::Passthrough {} => {
			// They should have a JWT policy defined. That will strip the token. Here we add it back
			if let Some(claim) = req.extensions().get::<Claims>() {
				if let Ok(mut token) =
					http::HeaderValue::from_str(&format!("Bearer {}", claim.jwt.expose_secret()))
				{
					token.set_sensitive(true);
					req.headers_mut().insert(http::header::AUTHORIZATION, token);
				}
			}
		},
		BackendAuth::Key(k) => {
			// TODO: is it always a Bearer?
			if let Ok(mut token) = http::HeaderValue::from_str(&format!("Bearer {}", k.expose_secret())) {
				token.set_sensitive(true);
				req.headers_mut().insert(http::header::AUTHORIZATION, token);
			}
		},
		BackendAuth::Gcp {} => {
			let token = gcp::get_token()
				.await
				.map_err(ProxyError::BackendAuthenticationFailed)?;
			req.headers_mut().insert(http::header::AUTHORIZATION, token);
		},
		BackendAuth::Aws {} => {
			// We handle this in 'apply_late_backend_auth' since it must come at the end!
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
		BackendAuth::Key(k) => {},
		BackendAuth::Gcp {} => {},
		BackendAuth::Aws {} => {
			aws::sign_request(req)
				.await
				.map_err(ProxyError::BackendAuthenticationFailed)?;
		},
	};
	Ok(())
}

mod gcp {
	use anyhow::anyhow;
	use aws_config::{BehaviorVersion, SdkConfig};
	use google_cloud_auth::credentials::CacheableResource;
	use google_cloud_auth::errors::CredentialsError;
	use google_cloud_auth::{credentials, errors};
	use http::{HeaderMap, HeaderName, HeaderValue};
	use tokio::sync::OnceCell;
	use tracing::trace;

	static CREDS: OnceCell<credentials::Credentials> = OnceCell::const_new();
	async fn creds<'a>() -> anyhow::Result<&'a credentials::Credentials> {
		Ok(
			CREDS
				.get_or_try_init(|| async { credentials::Builder::default().build() })
				.await?,
		)
	}

	pub async fn get_token() -> anyhow::Result<HeaderValue> {
		let mut token = get_headers_from_cache(creds().await?.headers(http::Extensions::new()).await?)?;
		let mut hv = token
			.remove(http::header::AUTHORIZATION)
			.ok_or(anyhow!("no authorization header"))?;
		hv.set_sensitive(true);
		trace!("attached GCP token");
		Ok(hv)
	}
	// What a terrible API... the older versions of this crate were usable but pulled in legacy dependency
	pub fn get_headers_from_cache(
		headers: CacheableResource<HeaderMap>,
	) -> Result<HeaderMap, CredentialsError> {
		match headers {
			CacheableResource::New { data, .. } => Ok(data),
			CacheableResource::NotModified => Err(CredentialsError::from_msg(
				false,
				"Expecting headers to be present",
			)),
		}
	}
}

mod aws {
	use std::time::SystemTime;

	use aws_config::{BehaviorVersion, SdkConfig};
	use aws_credential_types::Credentials;
	use aws_credential_types::provider::ProvideCredentials;
	use aws_sigv4::http_request::{SignableBody, sign};
	use aws_sigv4::sign::v4::SigningParams;
	use http_body_util::BodyExt;
	use tokio::sync::OnceCell;

	use crate::llm::bedrock::AwsRegion;
	use crate::*;

	pub async fn sign_request(req: &mut http::Request) -> anyhow::Result<()> {
		let creds = load_credentials().await?.into();

		// Get the region from request extensions (set by setup_request) or fall back to AWS config
		let region = req
			.extensions()
			.get::<AwsRegion>()
			.map(|r| r.region.clone())
			.ok_or_else(|| {
				anyhow::anyhow!("Region not found in request extensions - bedrock provider should set this")
			})?;

		trace!("AWS signing with region: {}, service: bedrock", region);

		// Sign the request
		let signing_params = SigningParams::builder()
			.identity(&creds)
			.region(&region)
			.name("bedrock")
			.time(SystemTime::now())
			.settings(aws_sigv4::http_request::SigningSettings::default())
			.build()?
			.into();

		let orig_body = std::mem::take(req.body_mut());
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
				.filter(|(k, v)| k != &http::header::CONTENT_LENGTH),
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
			.get_or_init(|| async { aws_config::load_defaults(BehaviorVersion::latest()).await })
			.await
	}

	async fn load_credentials() -> anyhow::Result<Credentials> {
		// Load AWS configuration and credentials
		let config = sdk_config().await;

		// Get credentials from the config
		// TODO this is not caching!!
		Ok(
			config
				.credentials_provider()
				.unwrap()
				.provide_credentials()
				.await?,
		)
	}
}
