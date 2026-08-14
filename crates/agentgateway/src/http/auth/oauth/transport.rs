use std::time::Duration;

use ::http::StatusCode;
use ::http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use anyhow::anyhow;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use tracing::debug;
use url::form_urlencoded;

use super::{
	ChainedExchange, ExchangeRequest, OAuthClientAuth, OAuthClientAuthMethod, OAuthGrantType,
	OAuthTokenExchangeAuth, OAuthTokenType, sign_client_assertion,
};
use crate::http::filters::BackendRequestTimeout;
use crate::http::oauth::{
	CLIENT_ASSERTION_TYPE_JWT_BEARER, GRANT_TYPE_JWT_BEARER, GRANT_TYPE_TOKEN_EXCHANGE,
	encode_client_secret_basic, format_token_endpoint_error_body,
};
use crate::http::{self, Body};
use crate::json;
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::types::agent::{BackendTrafficPolicy, SimpleBackendReference};

/// Default token-endpoint timeout, overridable by backend request-timeout policy
const DEFAULT_TOKEN_ENDPOINT_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) struct TokenEndpointResponse {
	pub(super) access_token: SecretString,
	pub(super) expires_in: Option<u64>,
}

/// Classifies token failures: request-token 4xx as client errors, transport and auth failures as upstream faults
#[derive(Debug, thiserror::Error)]
pub(in crate::http::auth) enum FetchError {
	#[error("{source}")]
	Client {
		status: ::http::StatusCode,
		#[source]
		source: anyhow::Error,
	},
	#[error("{0}")]
	Upstream(anyhow::Error),
}

impl FetchError {
	pub(in crate::http::auth) fn into_proxy_error(self) -> ProxyError {
		match self {
			FetchError::Client { status, source } => {
				// The authorization server rejected the request/subject token; surface
				// as a client error (4xx), not a gateway fault
				debug!(%status, error = %source, "oauth token exchange rejected by authorization server");
				ProxyError::InvalidRequest
			},
			FetchError::Upstream(e) => ProxyError::BackendAuthenticationFailed(e),
		}
	}

	pub(super) fn chained_exchange(self) -> Self {
		match self {
			FetchError::Client { status, source } => {
				debug!(%status, error = %source, "chained oauth token exchange rejected by authorization server");
				FetchError::Upstream(anyhow!("chained token exchange returned status {status}"))
			},
			err => err,
		}
	}
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
	access_token: SecretString,
	// Optional for RFC 7523 jwt-bearer responses
	#[serde(default)]
	issued_token_type: Option<String>,
	#[serde(default)]
	token_type: Option<String>,
	#[serde(default)]
	expires_in: Option<u64>,
}

impl TokenResponse {
	fn into_token(
		self,
		expected_issued_token_type: Option<OAuthTokenType>,
	) -> Result<TokenEndpointResponse, FetchError> {
		if expected_issued_token_type == Some(OAuthTokenType::IdJag) {
			let issued = self.issued_token_type.as_deref().ok_or_else(|| {
				FetchError::Upstream(anyhow!("token exchange response missing issued_token_type"))
			})?;
			let issued = OAuthTokenType::from_urn(issued).ok_or_else(|| {
				FetchError::Upstream(anyhow!(
					"token exchange returned unusable issued_token_type: {issued}"
				))
			})?;
			if issued != OAuthTokenType::IdJag {
				return Err(FetchError::Upstream(anyhow!(
					"token exchange returned issued_token_type {}, expected {}",
					issued.as_str(),
					OAuthTokenType::IdJag.as_str()
				)));
			}
			if let Some(token_type) = self.token_type.as_deref()
				&& !token_type.eq_ignore_ascii_case("N_A")
			{
				return Err(FetchError::Upstream(anyhow!(
					"token exchange returned unsupported token_type for id-jag: {token_type}",
				)));
			}
		} else {
			// Only bearer-style tokens are forwarded
			let Some(token_type) = self.token_type.as_deref() else {
				return Err(FetchError::Upstream(anyhow!(
					"token exchange response missing token_type"
				)));
			};
			if !token_type.eq_ignore_ascii_case("Bearer") {
				return Err(FetchError::Upstream(anyhow!(
					"token exchange returned unsupported token_type: {token_type}",
				)));
			}

			if let (Some(expected), Some(issued)) = (expected_issued_token_type, &self.issued_token_type)
			{
				let issued = OAuthTokenType::from_urn(issued).ok_or_else(|| {
					FetchError::Upstream(anyhow!(
						"token exchange returned unusable issued_token_type: {issued}"
					))
				})?;
				// Requested token types must match the response
				if issued != expected {
					return Err(FetchError::Upstream(anyhow!(
						"token exchange returned issued_token_type {}, expected {}",
						issued.as_str(),
						expected.as_str()
					)));
				}
			}
		}

		if self.access_token.expose_secret().is_empty() {
			return Err(FetchError::Upstream(anyhow!(
				"token exchange response contained an empty access_token"
			)));
		}

		Ok(TokenEndpointResponse {
			access_token: self.access_token,
			expires_in: self.expires_in,
		})
	}
}

pub(super) struct TokenRequestSpec<'a> {
	target: &'a SimpleBackendReference,
	policies: &'a [BackendTrafficPolicy],
	path: &'a str,
	grant_type: OAuthGrantType,
	client_auth: Option<&'a OAuthClientAuth>,
	audiences: &'a [String],
	scopes: &'a [String],
	resources: &'a [String],
	requested_token_type: Option<OAuthTokenType>,
	expected_issued_token_type: Option<OAuthTokenType>,
}

impl<'a> From<&'a OAuthTokenExchangeAuth> for TokenRequestSpec<'a> {
	fn from(auth: &'a OAuthTokenExchangeAuth) -> Self {
		Self {
			target: auth.target.target.as_ref(),
			policies: &auth.target.policies,
			path: &auth.path,
			grant_type: auth.grant_type,
			client_auth: auth.client_auth.as_ref(),
			audiences: &auth.audiences,
			scopes: &auth.scopes,
			resources: &auth.resources,
			requested_token_type: auth.requested_token_type_param(),
			expected_issued_token_type: auth.expected_issued_token_type(),
		}
	}
}

impl<'a> From<&'a ChainedExchange> for TokenRequestSpec<'a> {
	fn from(auth: &'a ChainedExchange) -> Self {
		Self {
			target: auth.target.target.as_ref(),
			policies: &auth.target.policies,
			path: &auth.path,
			grant_type: OAuthGrantType::JwtBearer,
			client_auth: auth.client_auth.as_ref(),
			audiences: &auth.audiences,
			scopes: &auth.scopes,
			resources: &auth.resources,
			requested_token_type: None,
			expected_issued_token_type: None,
		}
	}
}

pub(super) async fn request_token(
	client: &PolicyClient,
	spec: &TokenRequestSpec<'_>,
	req: &ExchangeRequest,
) -> Result<TokenEndpointResponse, FetchError> {
	let mut req = build_token_request(spec, req)?;
	// Default timeout, overridable by backend request-timeout policy
	req
		.extensions_mut()
		.insert(BackendRequestTimeout(DEFAULT_TOKEN_ENDPOINT_TIMEOUT));

	let resp = client
		.call_reference_with_policies(req, spec.target, spec.policies)
		.await
		.map_err(|e| FetchError::Upstream(anyhow!("token exchange request failed: {e}")))?;

	let status = resp.status();
	let limit = http::response_buffer_limit(&resp);
	if !status.is_success() {
		let body = http::read_body_with_limit(resp.into_body(), limit)
			.await
			.unwrap_or_default();
		let body = format_token_endpoint_error_body(&body, 256);
		return Err(classify_token_endpoint_error(status, body));
	}

	json::from_body_with_limit::<TokenResponse>(resp.into_body(), limit)
		.await
		.map_err(|e| FetchError::Upstream(anyhow!("token exchange response decode failed: {e}")))?
		.into_token(spec.expected_issued_token_type.clone())
}

fn classify_token_endpoint_error(status: StatusCode, body: String) -> FetchError {
	let detailed = anyhow!("token exchange returned status {status}: {body}");
	// 401/403 usually mean gateway client auth or token-endpoint policy failed.
	if status.is_client_error() && !matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
	{
		FetchError::Client {
			status,
			source: detailed,
		}
	} else {
		debug!(%status, error = %detailed, "oauth token exchange returned non-success status");
		FetchError::Upstream(anyhow!("token exchange returned status {status}"))
	}
}

fn build_token_request(
	spec: &TokenRequestSpec<'_>,
	req: &ExchangeRequest,
) -> Result<::http::Request<Body>, FetchError> {
	let form = build_token_request_form(spec, req)?;

	let builder = ::http::Request::builder()
		.method(::http::Method::POST)
		.uri(if spec.path.is_empty() { "/" } else { spec.path })
		.header(CONTENT_TYPE, "application/x-www-form-urlencoded")
		.header(ACCEPT, "application/json");

	form
		.basic_auth
		.iter()
		.fold(builder, |builder, basic| {
			builder.header(AUTHORIZATION, format!("Basic {basic}"))
		})
		.body(Body::from(form.body.into_bytes()))
		.map_err(|e| FetchError::Upstream(e.into()))
}

struct TokenRequestForm {
	body: String,
	basic_auth: Option<String>,
}

fn build_token_request_form(
	spec: &TokenRequestSpec<'_>,
	req: &ExchangeRequest,
) -> Result<TokenRequestForm, FetchError> {
	let mut basic_auth = None;
	let mut ser = form_urlencoded::Serializer::new(String::new());
	let subject_token = req.subject_token.expose_secret();
	match spec.grant_type {
		OAuthGrantType::TokenExchange => {
			// RFC 8693 sends the incoming credential as subject_token
			ser
				.append_pair("grant_type", GRANT_TYPE_TOKEN_EXCHANGE)
				.append_pair("subject_token", subject_token)
				.append_pair("subject_token_type", req.subject_token_type.as_str());
			if let Some((actor_token, actor_token_type)) = &req.actor {
				ser
					.append_pair("actor_token", actor_token.expose_secret())
					.append_pair("actor_token_type", actor_token_type.as_str());
			}
			if let Some(rtt) = &spec.requested_token_type {
				ser.append_pair("requested_token_type", rtt.as_str());
			}
		},
		OAuthGrantType::JwtBearer => {
			// RFC 7523 sends the incoming credential as assertion
			ser
				.append_pair("grant_type", GRANT_TYPE_JWT_BEARER)
				.append_pair("assertion", subject_token);
		},
	}
	for audience in spec.audiences {
		ser.append_pair("audience", audience);
	}
	if !spec.scopes.is_empty() {
		ser.append_pair("scope", &spec.scopes.join(" "));
	}
	for resource in spec.resources {
		ser.append_pair("resource", resource);
	}
	for (key, value) in &req.extra_params {
		ser.append_pair(key, value);
	}
	if let Some(client_auth) = spec.client_auth {
		match &client_auth.method {
			OAuthClientAuthMethod::ClientSecretBasic { client_secret } => {
				// Basic auth stays in the header, not the form body
				basic_auth = Some(encode_client_secret_basic(
					&client_auth.client_id,
					client_secret,
				));
			},
			OAuthClientAuthMethod::ClientSecretPost { client_secret } => {
				ser.append_pair("client_id", &client_auth.client_id);
				if let Some(secret) = client_secret {
					ser.append_pair("client_secret", secret.expose_secret());
				}
			},
			OAuthClientAuthMethod::PrivateKeyJwt(private_key) => {
				let assertion = sign_client_assertion(&client_auth.client_id, private_key)
					.map_err(FetchError::Upstream)?;
				// client_id is OPTIONAL per RFC 7521, but many providers require it
				// alongside the assertion; include it for interop.
				ser.append_pair("client_id", &client_auth.client_id);
				ser.append_pair("client_assertion_type", CLIENT_ASSERTION_TYPE_JWT_BEARER);
				ser.append_pair("client_assertion", &assertion);
			},
		}
	}

	Ok(TokenRequestForm {
		body: ser.finish(),
		basic_auth,
	})
}
