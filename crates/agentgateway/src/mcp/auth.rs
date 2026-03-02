use axum::http::StatusCode;
use axum::response::Response;
use axum_core::response::IntoResponse;
use bytes::Bytes;
use http::Method;
use http::uri::PathAndQuery;
use tracing::{debug, warn};

use crate::http::jwt::Claims;
use crate::http::*;
use crate::json;
use crate::json::from_body_with_limit;
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::types::agent::{McpAuthentication, McpIDP};

const OAUTH_PROTECTED_RESOURCE_PREFIX: &str = "/.well-known/oauth-protected-resource";
const OAUTH_AUTHORIZATION_SERVER_PREFIX: &str = "/.well-known/oauth-authorization-server";
const CLIENT_REGISTRATION_SEGMENT: &str = "client-registration";

pub(super) fn is_well_known_endpoint(path: &str) -> bool {
	path.starts_with(OAUTH_PROTECTED_RESOURCE_PREFIX)
		|| path.starts_with(OAUTH_AUTHORIZATION_SERVER_PREFIX)
}

fn has_authorization_server_prefix(path: &str) -> bool {
	path
		.strip_prefix(OAUTH_AUTHORIZATION_SERVER_PREFIX)
		.is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

fn is_client_registration_endpoint(path: &str) -> bool {
	has_authorization_server_prefix(path)
		&& path
			.rsplit('/')
			.next()
			.is_some_and(|segment| segment == CLIENT_REGISTRATION_SEGMENT)
}

pub(super) async fn apply_token_validation(
	req: &mut Request,
	auth: &McpAuthentication,
	client: &PolicyClient,
) -> Result<(), ProxyError> {
	// skip well-known OAuth endpoints for authn
	if is_well_known_endpoint(req.uri().path()) {
		return Ok(());
	}
	let has_claims = req.extensions().get::<Claims>().is_some();

	if has_claims {
		// if mcp authn is configured but JWT already validated (claims exist from previous layer),
		// reject because we cannot validate MCP-specific auth requirements
		let err = ProxyError::ProcessingString(
			"MCP backend authentication configured but JWT token already validated and stripped by Gateway or Route level policy".to_string(),
		);
		return Err(create_auth_required_response(err, req, auth));
	}

	debug!(
		"MCP auth configured; validating Authorization header (mode={:?})",
		auth.mode
	);
	auth
		.jwt_validator
		.apply(
			&client.inputs.upstream,
			client.inputs.oidc.as_ref(),
			None,
			None,
			req,
		)
		.await
		.map_err(|e| {
			create_auth_required_response(ProxyError::JwtAuthenticationFailure(e), req, auth)
		})?;
	Ok(())
}

pub(super) async fn enforce_authentication(
	req: &mut Request,
	auth: &McpAuthentication,
	client: &PolicyClient,
) -> Result<Option<Response>, ProxyError> {
	// skip well-known OAuth endpoints for authn
	if !is_well_known_endpoint(req.uri().path()) {
		apply_token_validation(req, auth, client).await?;
	}

	match req.uri().path() {
		// TODO: indicate this is a DirectResponse
		path if is_client_registration_endpoint(path) => Ok(Some(
			client_registration(req, auth, client.clone())
				.await
				.map_err(|e| {
					warn!("client_registration error: {}", e);
					StatusCode::INTERNAL_SERVER_ERROR
				})
				.into_response(),
		)),
		path if path.starts_with(OAUTH_PROTECTED_RESOURCE_PREFIX) => Ok(Some(
			protected_resource_metadata(req, auth).await.into_response(),
		)),
		path if path.starts_with(OAUTH_AUTHORIZATION_SERVER_PREFIX) => Ok(Some(
			authorization_server_metadata(req, auth, client.clone())
				.await
				.map_err(|e| {
					warn!("authorization_server_metadata error: {}", e);
					StatusCode::INTERNAL_SERVER_ERROR
				})
				.into_response(),
		)),
		_ => {
			// Not handled
			Ok(None)
		},
	}
}

pub(super) fn create_auth_required_response(
	inner: ProxyError,
	req: &Request,
	auth: &McpAuthentication,
) -> ProxyError {
	let request_path = req.uri().path();
	// If the `resource` is explicitly configured, use that as the base. otherwise, derive it from the
	// the request URL
	let proxy_url = auth
		.resource_metadata
		.extra
		.get("resource")
		.and_then(|v| v.as_str())
		.and_then(|u| http::uri::Uri::try_from(u).ok())
		.and_then(|uri| {
			let mut parts = uri.into_parts();
			parts.path_and_query = Some(PathAndQuery::from_static(""));
			Uri::from_parts(parts).ok()
		})
		.and_then(|uri| uri.to_string().strip_suffix("/").map(ToString::to_string))
		.unwrap_or_else(|| get_redirect_url(req, request_path));
	let www_authenticate_value = format!(
		"Bearer resource_metadata=\"{proxy_url}/.well-known/oauth-protected-resource{request_path}\""
	);

	ProxyError::McpJwtAuthenticationFailure(Box::new(inner), www_authenticate_value)
}

pub(super) async fn protected_resource_metadata(
	req: &mut Request,
	auth: &McpAuthentication,
) -> Response {
	let new_uri = strip_oauth_protected_resource_prefix(req);

	// Determine the issuer to use - either use the same request URL and path that it was initially with,
	// or else keep the auth.issuer
	let issuer = if auth.provider.is_some() {
		// When a provider is configured, use the same request URL with the well-known prefix stripped
		strip_oauth_protected_resource_prefix(req)
	} else {
		// No provider configured, use the original issuer
		auth.issuer.clone()
	};

	let json_body = auth.resource_metadata.to_rfc_json(new_uri, issuer);

	::http::Response::builder()
		.status(StatusCode::OK)
		.header("content-type", "application/json")
		.header("access-control-allow-origin", "*")
		.header("access-control-allow-methods", "GET, OPTIONS")
		.header("access-control-allow-headers", "content-type")
		.body(axum::body::Body::from(Bytes::from(
			serde_json::to_string(&json_body).unwrap_or_default(),
		)))
		.unwrap_or_else(|_| {
			::http::Response::builder()
				.status(StatusCode::INTERNAL_SERVER_ERROR)
				.body(axum::body::Body::empty())
				.unwrap()
		})
}

fn get_redirect_url(req: &Request, strip_base: &str) -> String {
	let uri = req
		.extensions()
		.get::<filters::OriginalUrl>()
		.map(|u| u.0.clone())
		.unwrap_or_else(|| req.uri().clone());

	uri
		.path()
		.strip_suffix(strip_base)
		.map(|p| uri.to_string().replace(uri.path(), p))
		.unwrap_or(uri.to_string())
}

fn strip_oauth_protected_resource_prefix(req: &Request) -> String {
	let uri = req
		.extensions()
		.get::<filters::OriginalUrl>()
		.map(|u| u.0.clone())
		.unwrap_or_else(|| req.uri().clone());

	let path = uri.path();
	const OAUTH_PREFIX: &str = "/.well-known/oauth-protected-resource";

	// Remove the oauth-protected-resource prefix and keep the remaining path
	if let Some(remaining_path) = path.strip_prefix(OAUTH_PREFIX) {
		uri.to_string().replace(path, remaining_path)
	} else {
		// If the prefix is not found, return the original URI
		uri.to_string()
	}
}

pub(super) async fn authorization_server_metadata(
	req: &mut Request,
	auth: &McpAuthentication,
	client: PolicyClient,
) -> Result<Response, ProxyError> {
	// Normalize issuer URL by removing trailing slashes to avoid double-slash in path
	let issuer = auth.issuer.trim_end_matches('/');
	let ureq = ::http::Request::builder()
		.uri(format!("{issuer}/.well-known/oauth-authorization-server"))
		.body(Body::empty())?;
	let upstream = client.simple_call(ureq).await?;
	let limit = crate::http::response_buffer_limit(&upstream);
	let mut resp: serde_json::Value = from_body_with_limit(upstream.into_body(), limit)
		.await
		.map_err(ProxyError::Body)?;
	match &auth.provider {
		Some(McpIDP::Auth0 {}) => {
			// Auth0 does not support RFC 8707. We can workaround this by prepending an audience
			let Some(serde_json::Value::String(ae)) =
				json::traverse_mut(&mut resp, &["authorization_endpoint"])
			else {
				return Err(ProxyError::ProcessingString(
					"authorization_endpoint missing".to_string(),
				));
			};
			// If the user provided multiple audiences with auth0, just prepend the first one
			if let Some(aud) = auth.audiences.first() {
				ae.push_str(&format!("?audience={}", aud));
			}
		},
		Some(McpIDP::Keycloak { .. }) => {
			// Keycloak does not support RFC 8707.
			// We do not currently have a workload :-(
			// users will have to hardcode the audience.
			// https://github.com/keycloak/keycloak/issues/10169 and https://github.com/keycloak/keycloak/issues/14355

			// Keycloak doesn't do CORS for client registrations
			// https://github.com/keycloak/keycloak/issues/39629
			// We can workaround this by proxying it

			let current_uri = req
				.extensions()
				.get::<filters::OriginalUrl>()
				.map(|u| u.0.clone())
				.unwrap_or_else(|| req.uri().clone());
			let Some(serde_json::Value::String(re)) =
				json::traverse_mut(&mut resp, &["registration_endpoint"])
			else {
				return Err(ProxyError::ProcessingString(
					"registration_endpoint missing".to_string(),
				));
			};
			*re = format!("{current_uri}/client-registration");
		},
		_ => {},
	}

	let response = ::http::Response::builder()
		.status(StatusCode::OK)
		.header("content-type", "application/json")
		.header("access-control-allow-origin", "*")
		.header("access-control-allow-methods", "GET, OPTIONS")
		.header("access-control-allow-headers", "content-type")
		.body(axum::body::Body::from(Bytes::from(
			serde_json::to_string(&resp).map_err(|e| ProxyError::Body(crate::http::Error::new(e)))?,
		)))?;

	Ok(response)
}

pub(super) async fn client_registration(
	req: &mut Request,
	auth: &McpAuthentication,
	client: PolicyClient,
) -> Result<Response, ProxyError> {
	// Normalize issuer URL by removing trailing slashes to avoid double-slash in path
	let issuer = auth.issuer.trim_end_matches('/');
	let body = std::mem::take(req.body_mut());
	let ureq = ::http::Request::builder()
		.uri(format!("{issuer}/clients-registrations/openid-connect"))
		.method(Method::POST)
		.body(body)?;

	let mut upstream = client.simple_call(ureq).await?;

	// Add CORS headers to the response
	let headers = upstream.headers_mut();
	headers.insert("access-control-allow-origin", "*".parse().unwrap());
	headers.insert(
		"access-control-allow-methods",
		"POST, OPTIONS".parse().unwrap(),
	);
	headers.insert(
		"access-control-allow-headers",
		"content-type".parse().unwrap(),
	);

	Ok(upstream)
}

#[cfg(test)]
mod tests {
	use super::is_client_registration_endpoint;

	#[test]
	fn client_registration_endpoint_requires_expected_prefix_and_segment_suffix() {
		assert!(is_client_registration_endpoint(
			"/.well-known/oauth-authorization-server/mcp/client-registration"
		));
		assert!(is_client_registration_endpoint(
			"/.well-known/oauth-authorization-server/client-registration"
		));

		assert!(!is_client_registration_endpoint(
			"/.well-known/oauth-authorization-server/mcp/client-registration-extra"
		));
		assert!(!is_client_registration_endpoint(
			"/.well-known/oauth-authorization-serverevil/mcp/client-registration"
		));
		assert!(!is_client_registration_endpoint("/mcp/client-registration"));
		assert!(!is_client_registration_endpoint(
			"/.well-known/oauth-authorization-server/mcp/client-registration/"
		));
	}
}
