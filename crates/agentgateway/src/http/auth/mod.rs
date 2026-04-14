pub mod aws;
pub mod azure;
pub mod gcp;

pub use aws::AwsAuth;
pub use azure::AzureAuth;
pub use gcp::GcpAuth;

use crate::http::Request;
use crate::http::jwt::Claims;
use crate::proxy::ProxyError;
use crate::proxy::ProxyError::ProcessingString;
use crate::serdes::deser_key_from_file;
use crate::types::agent::{BackendTarget, Target};
use crate::*;
use ::http::HeaderValue;
use secrecy::{ExposeSecret, SecretString};

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
	Gcp(gcp::GcpAuth),
	#[serde(rename = "aws")]
	Aws(aws::AwsAuth),
	#[serde(rename = "azure")]
	Azure(azure::AzureAuth),
}

#[derive(Clone)]
pub struct BackendInfo {
	pub target: BackendTarget,
	pub call_target: Target,
	pub inputs: Arc<ProxyInputs>,
}

pub fn apply_tunnel_auth(auth: &BackendAuth) -> Result<HeaderValue, ProxyError> {
	match auth {
		BackendAuth::Key(k) => {
			// TODO: currently we only support basic auth; this is not great but we are pending the ability
			// to customize this
			let mut token = http::HeaderValue::from_str(&format!("Basic {}", k.expose_secret()))
				.map_err(|e| ProxyError::Processing(e.into()))?;
			token.set_sensitive(true);

			Ok(token)
		},
		_ => Err(ProcessingString(
			"only key auth is supported in tunnel".to_string(),
		)),
	}
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
			let token = azure::get_token(
				&backend_info.inputs.upstream,
				azure_auth,
				&backend_info.call_target,
			)
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
#[path = "tests.rs"]
mod tests;
