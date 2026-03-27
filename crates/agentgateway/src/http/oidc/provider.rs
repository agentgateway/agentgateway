use std::time::Duration;

use ::http::{Method, StatusCode, header};
use anyhow::Context;
use base64::Engine;
use jsonwebtoken::jwk::JwkSet;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use crate::client::Client;
use crate::http::filters::BackendRequestTimeout;
use crate::http::{Body, Uri};
use crate::proxy::httpproxy::PolicyClient;
use crate::serdes::FileInlineOrRemote;

use super::config::ResolvedProvider;
use super::{Error, Provider, TokenEndpointAuth};

#[derive(Debug, Deserialize)]
struct OidcDiscoveryDocument {
	issuer: String,
	authorization_endpoint: String,
	token_endpoint: String,
	jwks_uri: String,
	#[serde(default)]
	token_endpoint_auth_methods_supported: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TokenResponse {
	#[serde(default)]
	pub id_token: Option<String>,
}

pub(super) struct DiscoveredProviderMetadata {
	pub authorization_endpoint: Uri,
	pub token_endpoint: Uri,
	pub token_endpoint_auth: TokenEndpointAuth,
	pub jwks: FileInlineOrRemote,
}

const DEFAULT_TOKEN_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(10);
const TOKEN_RESPONSE_BODY_LIMIT: usize = 64 * 1024;

pub(super) async fn discover_provider_metadata(
	client: Client,
	issuer: &str,
	discovery: Option<FileInlineOrRemote>,
) -> Result<DiscoveredProviderMetadata, Error> {
	let discovery = match discovery {
		Some(discovery) => discovery,
		None => FileInlineOrRemote::Remote {
			url: default_discovery_url(issuer)?,
		},
	};
	let document = discovery
		.load::<OidcDiscoveryDocument>(client.clone())
		.await
		.map_err(|e| {
			Error::Config(format!(
				"failed to decode oidc discovery response from {}: {e}",
				describe_file_inline_or_remote(&discovery)
			))
		})?;
	if document.issuer != issuer {
		return Err(Error::Config(format!(
			"oidc discovery issuer mismatch: expected {issuer}, got {}",
			document.issuer
		)));
	}

	let token_endpoint_auth =
		parse_token_endpoint_auth_methods(document.token_endpoint_auth_methods_supported)?;
	let jwks = FileInlineOrRemote::Remote {
		url: document
			.jwks_uri
			.parse()
			.map_err(|e| Error::Config(format!("invalid jwks uri: {e}")))?,
	};
	Ok(DiscoveredProviderMetadata {
		authorization_endpoint: document
			.authorization_endpoint
			.parse()
			.map_err(|e| Error::Config(format!("invalid authorization endpoint uri: {e}")))?,
		token_endpoint: document
			.token_endpoint
			.parse()
			.map_err(|e| Error::Config(format!("invalid token endpoint uri: {e}")))?,
		token_endpoint_auth,
		jwks,
	})
}

pub(super) async fn build_explicit_provider(
	client: Client,
	issuer: String,
	authorization_endpoint: Uri,
	token_endpoint: Uri,
	jwks: FileInlineOrRemote,
	token_endpoint_auth_methods_supported: Vec<TokenEndpointAuth>,
) -> Result<ResolvedProvider, Error> {
	let jwks = load_jwks(client, jwks, JwksLoadSource::Explicit).await?;
	let token_endpoint_auth =
		normalize_token_endpoint_auth_methods(token_endpoint_auth_methods_supported)?;

	Ok(ResolvedProvider {
		issuer,
		authorization_endpoint,
		token_endpoint,
		token_endpoint_auth,
		id_token_jwks: jwks,
	})
}

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
		.uri(provider.token_endpoint.to_string())
		.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
		.header(header::ACCEPT, "application/json");
	match client_config.token_endpoint_auth {
		TokenEndpointAuth::ClientSecretBasic => {
			let encoded_client_id = form_urlencode_component(&client_config.client_id);
			let encoded_client_secret =
				form_urlencode_component(client_config.client_secret.expose_secret());
			let auth = format!(
				"Basic {}",
				base64::engine::general_purpose::STANDARD
					.encode(format!("{}:{}", encoded_client_id, encoded_client_secret))
			);
			req = req.header(header::AUTHORIZATION, auth);
		},
		TokenEndpointAuth::ClientSecretPost => {
			form.push(("client_id", client_config.client_id.clone()));
			form.push((
				"client_secret",
				client_config.client_secret.expose_secret().to_string(),
			));
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
		return Err(Error::Http(anyhow::anyhow!(
			"token endpoint returned {status}: {}",
			format_token_endpoint_error_body(&body)
		)));
	}
	serde_json::from_slice::<TokenResponse>(&body)
		.context("failed to decode token response")
		.map_err(Error::Http)
}

pub(super) fn default_discovery_url(issuer: &str) -> Result<Uri, Error> {
	format!(
		"{}/.well-known/openid-configuration",
		issuer.trim_end_matches('/')
	)
	.parse()
	.map_err(|e| {
		Error::Config(format!(
			"invalid discovery uri derived from issuer '{issuer}': {e}"
		))
	})
}

pub(super) async fn load_jwks(
	client: Client,
	jwks: FileInlineOrRemote,
	source: JwksLoadSource,
) -> Result<JwkSet, Error> {
	let jwks = jwks.load::<JwkSet>(client).await.map_err(|e| {
		Error::Config(format!(
			"failed to load oidc jwks from {} {}: {e}",
			source.describe(),
			describe_file_inline_or_remote(&jwks)
		))
	})?;
	Ok(jwks)
}

fn form_urlencode_component(value: &str) -> String {
	let encoded = url::form_urlencoded::Serializer::new(String::new())
		.append_pair("", value)
		.finish();
	encoded.strip_prefix('=').unwrap_or(&encoded).to_string()
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

pub(crate) fn parse_token_endpoint_auth_methods(
	methods: Option<Vec<String>>,
) -> Result<TokenEndpointAuth, Error> {
	let methods = methods.unwrap_or_else(|| vec!["client_secret_basic".into()]);
	let mut parsed = Vec::with_capacity(methods.len());
	for method in methods {
		match method.as_str() {
			"client_secret_basic" => parsed.push(TokenEndpointAuth::ClientSecretBasic),
			"client_secret_post" => parsed.push(TokenEndpointAuth::ClientSecretPost),
			_ => {},
		}
	}
	normalize_token_endpoint_auth_methods(parsed)
}

pub(super) fn normalize_token_endpoint_auth_methods(
	mut methods: Vec<TokenEndpointAuth>,
) -> Result<TokenEndpointAuth, Error> {
	methods.sort();
	methods.dedup();
	if methods.contains(&TokenEndpointAuth::ClientSecretBasic) {
		Ok(TokenEndpointAuth::ClientSecretBasic)
	} else if methods.contains(&TokenEndpointAuth::ClientSecretPost) {
		Ok(TokenEndpointAuth::ClientSecretPost)
	} else {
		Err(Error::Config(
			"token endpoint auth methods must include clientSecretBasic or clientSecretPost".into(),
		))
	}
}

#[derive(Debug, Clone, Copy)]
pub(super) enum JwksLoadSource {
	Discovered,
	Explicit,
}

impl JwksLoadSource {
	fn describe(self) -> &'static str {
		match self {
			Self::Discovered => "discovered jwks source",
			Self::Explicit => "explicit jwks source",
		}
	}
}

fn describe_file_inline_or_remote(source: &FileInlineOrRemote) -> String {
	match source {
		FileInlineOrRemote::File { file } => format!("file '{}'", file.display()),
		FileInlineOrRemote::Inline(_) => "inline configuration".into(),
		FileInlineOrRemote::Remote { url } => format!("uri '{url}'"),
	}
}

#[cfg(test)]
mod tests {
	use super::OidcDiscoveryDocument;

	#[test]
	fn oidc_discovery_document_uses_standard_snake_case_fields() {
		let document: OidcDiscoveryDocument = serde_json::from_value(serde_json::json!({
			"issuer": "https://issuer.example.com",
			"authorization_endpoint": "https://issuer.example.com/authorize",
			"token_endpoint": "https://issuer.example.com/token",
			"jwks_uri": "https://issuer.example.com/jwks.json",
			"end_session_endpoint": "https://issuer.example.com/logout",
			"token_endpoint_auth_methods_supported": ["client_secret_basic"]
		}))
		.expect("standard oidc discovery document");

		assert_eq!(document.issuer, "https://issuer.example.com");
		assert_eq!(
			document.authorization_endpoint,
			"https://issuer.example.com/authorize"
		);
		assert_eq!(document.token_endpoint, "https://issuer.example.com/token");
		assert_eq!(document.jwks_uri, "https://issuer.example.com/jwks.json");
		assert_eq!(
			document.token_endpoint_auth_methods_supported,
			Some(vec!["client_secret_basic".to_string()])
		);
	}
}
