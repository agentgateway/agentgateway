use std::time::Duration;

use ::http::{Method, StatusCode, header};
use anyhow::Context;
use base64::Engine;
use secrecy::{ExposeSecret, SecretString};

use super::{ClientCredentials, Error, Provider};
use crate::http::Body;
use crate::http::filters::BackendRequestTimeout;
use crate::proxy::httpproxy::PolicyClient;

#[derive(Debug, serde::Deserialize)]
pub(crate) struct TokenResponse {
	#[serde(default)]
	pub id_token: Option<String>,
}

const DEFAULT_TOKEN_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(10);
const TOKEN_RESPONSE_BODY_LIMIT: usize = 64 * 1024;

pub(crate) async fn exchange_code(
	client: PolicyClient,
	provider: &Provider,
	client_config: &super::ClientConfig,
	redirect_uri: &str,
	code: &str,
	pkce_verifier: &SecretString,
) -> Result<TokenResponse, Error> {
	exchange_code_with_timeout(
		client,
		provider,
		client_config,
		redirect_uri,
		code,
		pkce_verifier,
		DEFAULT_TOKEN_EXCHANGE_TIMEOUT,
	)
	.await
}

pub(crate) async fn exchange_code_with_timeout(
	client: PolicyClient,
	provider: &Provider,
	client_config: &super::ClientConfig,
	redirect_uri: &str,
	code: &str,
	pkce_verifier: &SecretString,
	timeout: Duration,
) -> Result<TokenResponse, Error> {
	let mut form = vec![
		("grant_type", "authorization_code".to_string()),
		("code", code.to_string()),
		("redirect_uri", redirect_uri.to_string()),
		("code_verifier", pkce_verifier.expose_secret().to_string()),
	];
	let mut req = ::http::Request::builder()
		.method(Method::POST)
		.uri(provider.token_endpoint.as_str())
		.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
		.header(header::ACCEPT, "application/json");
	// The token endpoint always needs to know which client is exchanging the
	// code; confidential clients additionally prove their identity with a
	// secret. Public clients (`None`) send only the client_id and rely on
	// PKCE to bind the request to the authorization.
	match &client_config.credentials {
		ClientCredentials::ClientSecretBasic { client_secret } => {
			let encoded_client_id = form_urlencode_component(&client_config.client_id);
			let encoded_client_secret = form_urlencode_component(client_secret.expose_secret());
			let auth = format!(
				"Basic {}",
				base64::engine::general_purpose::STANDARD
					.encode(format!("{}:{}", encoded_client_id, encoded_client_secret))
			);
			req = req.header(header::AUTHORIZATION, auth);
		},
		ClientCredentials::ClientSecretPost { client_secret } => {
			form.push(("client_id", client_config.client_id.clone()));
			form.push(("client_secret", client_secret.expose_secret().to_string()));
		},
		ClientCredentials::None => {
			form.push(("client_id", client_config.client_id.clone()));
		},
	}
	let body = serde_urlencoded::to_string(form).map_err(anyhow::Error::from)?;
	let mut req = req
		.body(Body::from(body))
		.map_err(|e| Error::Config(format!("failed to build token exchange request: {e}")))?;
	req.extensions_mut().insert(BackendRequestTimeout(timeout));
	let resp = client
		.simple_call(req)
		.await
		.map_err(anyhow::Error::from)
		.map_err(Error::TokenExchangeFailed)?;
	let status = resp.status();
	let (_, body) = {
		let (parts, body) = resp.into_parts();
		let body = crate::http::read_body_with_limit(body, TOKEN_RESPONSE_BODY_LIMIT)
			.await
			.map_err(anyhow::Error::from)
			.map_err(Error::TokenExchangeFailed)?;
		(parts, body)
	};
	if status != StatusCode::OK {
		return Err(Error::TokenExchangeFailed(anyhow::anyhow!(
			"token endpoint returned {status}: {}",
			format_token_endpoint_error_body(&body)
		)));
	}
	serde_json::from_slice::<TokenResponse>(&body)
		.context("failed to decode token response")
		.map_err(Error::TokenExchangeFailed)
}

fn form_urlencode_component(value: &str) -> String {
	url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn format_token_endpoint_error_body(body: &[u8]) -> String {
	const LIMIT: usize = 1024;

	let mut out = String::with_capacity(body.len().min(LIMIT));
	let mut truncated = false;
	for ch in String::from_utf8_lossy(body).chars() {
		let ch = if ch.is_control() { ' ' } else { ch };
		if out.len() + ch.len_utf8() > LIMIT {
			truncated = true;
			break;
		}
		out.push(ch);
	}
	if truncated {
		out.push_str("...");
	}
	out
}
