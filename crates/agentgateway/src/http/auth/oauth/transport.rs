use ::http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use url::form_urlencoded;

use super::{ExchangeRequest, OAuthClientAuthMethod, OAuthGrantType, OAuthTokenExchangeAuth};
use crate::http::oauth::{
	GRANT_TYPE_JWT_BEARER, GRANT_TYPE_TOKEN_EXCHANGE, encode_client_secret_basic,
	format_token_endpoint_error_body, supported_oauth_token_type,
};
use crate::http::{self, Body};
use crate::json;
use crate::proxy::httpproxy::PolicyClient;

const TOKEN_ENDPOINT_ERROR_BODY_LIMIT: usize = 256;

pub(super) struct TokenEndpointResponse {
	pub(super) access_token: SecretString,
	pub(super) expires_in: Option<u64>,
}

/// Classifies a token exchange failure. A 4xx from the authorization server means
/// the request or subject token is bad (a client error, surfaced downstream as a
/// 4xx); transport failures and 5xx are upstream faults (surfaced as 5xx).
#[derive(Debug, thiserror::Error)]
pub(in crate::http::auth) enum FetchError {
	#[error("{0}")]
	Client(anyhow::Error),
	#[error("{0}")]
	Upstream(anyhow::Error),
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
	access_token: SecretString,
	// Absent under RFC 7523 (jwt-bearer), which returns a plain RFC 6749 token response.
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
		expected_issued_token_type: Option<&str>,
	) -> Result<TokenEndpointResponse, FetchError> {
		// This path only forwards bearer-style tokens to backends.
		let Some(token_type) = self.token_type.as_deref() else {
			return Err(FetchError::Upstream(anyhow::anyhow!(
				"token exchange response missing token_type"
			)));
		};
		if !token_type.eq_ignore_ascii_case("Bearer") {
			return Err(FetchError::Upstream(anyhow::anyhow!(
				"token exchange returned unsupported token_type: {token_type}",
			)));
		}

		if let Some(issued) = &self.issued_token_type {
			if let Some(expected) = expected_issued_token_type {
				// RFC 8693 lets the client ask for a specific token type; if we asked,
				// the response has to agree.
				if issued != expected {
					return Err(FetchError::Upstream(anyhow::anyhow!(
						"token exchange returned issued_token_type {issued}, expected {expected}"
					)));
				}
			} else if !supported_oauth_token_type(issued) {
				return Err(FetchError::Upstream(anyhow::anyhow!(
					"token exchange returned unusable issued_token_type: {issued}"
				)));
			}
		}

		if self.access_token.expose_secret().is_empty() {
			return Err(FetchError::Upstream(anyhow::anyhow!(
				"token exchange response contained an empty access_token"
			)));
		}

		Ok(TokenEndpointResponse {
			access_token: self.access_token,
			expires_in: self.expires_in,
		})
	}
}

pub(super) async fn request_token(
	client: &PolicyClient,
	auth: &OAuthTokenExchangeAuth,
	request: &ExchangeRequest,
) -> Result<TokenEndpointResponse, FetchError> {
	let req = build_token_request(auth, request)?;
	let exchange = async {
		let resp = client
			.call_reference(req, &auth.token_endpoint)
			.await
			.map_err(|e| FetchError::Upstream(anyhow::anyhow!("token exchange request failed: {e}")))?;

		let status = resp.status();
		let limit = http::response_buffer_limit(&resp);
		if !status.is_success() {
			let body = http::read_body_with_limit(resp.into_body(), limit)
				.await
				.unwrap_or_default();
			let body = format_token_endpoint_error_body(&body, TOKEN_ENDPOINT_ERROR_BODY_LIMIT);
			let err = anyhow::anyhow!("token exchange returned status {status}: {body}");
			// A 4xx means the authorization server rejected the request/subject token;
			// anything else (5xx, 3xx) is treated as an upstream fault.
			return Err(if status.is_client_error() {
				FetchError::Client(err)
			} else {
				FetchError::Upstream(err)
			});
		}

		json::from_body_with_limit::<TokenResponse>(resp.into_body(), limit)
			.await
			.map_err(|e| {
				FetchError::Upstream(anyhow::anyhow!(
					"token exchange response decode failed: {e}"
				))
			})?
			.into_token(auth.expected_issued_token_type())
	};

	// Bound the whole exchange, including the response body read and decode.
	tokio::time::timeout(auth.token_endpoint_timeout, exchange)
		.await
		.map_err(|_| {
			FetchError::Upstream(anyhow::anyhow!(
				"token exchange timed out after {:?}",
				auth.token_endpoint_timeout
			))
		})?
}

fn build_token_request(
	auth: &OAuthTokenExchangeAuth,
	request: &ExchangeRequest,
) -> Result<::http::Request<Body>, FetchError> {
	let form = build_token_request_form(auth, request);
	let path = if auth.token_endpoint_path.is_empty() {
		"/"
	} else {
		auth.token_endpoint_path.as_str()
	};
	let mut builder = ::http::Request::builder()
		.method(::http::Method::POST)
		.uri(path)
		.header(CONTENT_TYPE, "application/x-www-form-urlencoded")
		.header(ACCEPT, "application/json");
	if let Some(basic) = &form.basic_auth {
		builder = builder.header(AUTHORIZATION, format!("Basic {basic}"));
	}
	builder
		.body(Body::from(form.body.into_bytes()))
		.map_err(|e| FetchError::Upstream(e.into()))
}

struct TokenRequestForm {
	body: String,
	basic_auth: Option<String>,
}

fn build_token_request_form(
	auth: &OAuthTokenExchangeAuth,
	request: &ExchangeRequest,
) -> TokenRequestForm {
	let mut basic_auth = None;
	let mut ser = form_urlencoded::Serializer::new(String::new());
	let subject_token = request.subject_token.expose_secret();
	match auth.grant_type {
		OAuthGrantType::TokenExchange => {
			// RFC 8693 sends the incoming credential as subject_token.
			ser
				.append_pair("grant_type", GRANT_TYPE_TOKEN_EXCHANGE)
				.append_pair("subject_token", subject_token)
				.append_pair("subject_token_type", &request.subject_token_type);
			if let Some((actor_token, actor_token_type)) = &request.actor {
				ser
					.append_pair("actor_token", actor_token.expose_secret())
					.append_pair("actor_token_type", actor_token_type);
			}
			if let Some(rtt) = &auth.requested_token_type {
				ser.append_pair("requested_token_type", rtt);
			}
		},
		OAuthGrantType::JwtBearer => {
			// RFC 7523 sends the same incoming credential as assertion instead.
			ser
				.append_pair("grant_type", GRANT_TYPE_JWT_BEARER)
				.append_pair("assertion", subject_token);
		},
	}
	for audience in &auth.audiences {
		ser.append_pair("audience", audience);
	}
	if !auth.scopes.is_empty() {
		ser.append_pair("scope", &auth.scopes.join(" "));
	}
	for resource in &auth.resources {
		ser.append_pair("resource", resource);
	}
	for (key, value) in &request.extra_params {
		ser.append_pair(key, value);
	}
	if let Some(client_auth) = &auth.client_auth {
		match client_auth.method {
			OAuthClientAuthMethod::ClientSecretBasic => {
				// Basic auth stays in the header, not the form body.
				if let Some(secret) = &client_auth.client_secret {
					basic_auth = Some(encode_client_secret_basic(&client_auth.client_id, secret));
				}
			},
			OAuthClientAuthMethod::ClientSecretPost => {
				ser.append_pair("client_id", &client_auth.client_id);
				if let Some(secret) = &client_auth.client_secret {
					ser.append_pair("client_secret", secret.expose_secret());
				}
			},
		}
	}

	TokenRequestForm {
		body: ser.finish(),
		basic_auth,
	}
}
