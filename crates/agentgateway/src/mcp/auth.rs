use std::str::FromStr;

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
const CLIENT_REGISTRATION_SUFFIX: &str = "/client-registration";

#[derive(Debug, Clone)]
pub(crate) struct PassthroughProtectedResource {
	pub upstream_uri: Uri,
	pub resource: String,
	pub authorization_server: String,
	pub resource_metadata: String,
}

#[derive(Debug, Clone)]
pub(crate) enum PassthroughWellKnown {
	UnsupportedAuthorizationServer,
	ProtectedResource(PassthroughProtectedResource),
}

pub(super) fn is_well_known_endpoint(path: &str) -> bool {
	path.starts_with(OAUTH_PROTECTED_RESOURCE_PREFIX)
		|| path.starts_with(OAUTH_AUTHORIZATION_SERVER_PREFIX)
}

fn request_uri(req: &Request) -> Uri {
	req
		.extensions()
		.get::<filters::OriginalUrl>()
		.map(|u| u.0.clone())
		.unwrap_or_else(|| req.uri().clone())
}

fn request_path(req: &Request) -> &str {
	req
		.extensions()
		.get::<filters::OriginalUrl>()
		.map(|u| u.0.path())
		.unwrap_or_else(|| req.uri().path())
}

fn normalize_identifier_suffix(suffix: &str) -> Option<String> {
	if suffix.is_empty() || suffix == "/" {
		return None;
	}
	Some(if suffix.starts_with('/') {
		suffix.to_string()
	} else {
		format!("/{suffix}")
	})
}

fn request_origin(req: &Request) -> Option<String> {
	let mut parts = request_uri(req).into_parts();
	parts.path_and_query = Some(PathAndQuery::from_static(""));
	Some(
		Uri::from_parts(parts)
			.ok()?
			.to_string()
			.trim_end_matches('/')
			.to_string(),
	)
}

fn well_known_backend_path(path: &str) -> Option<String> {
	let suffix = if let Some(s) = path.strip_prefix(OAUTH_PROTECTED_RESOURCE_PREFIX) {
		s
	} else if let Some(s) = path.strip_prefix(OAUTH_AUTHORIZATION_SERVER_PREFIX) {
		s.strip_suffix(CLIENT_REGISTRATION_SUFFIX).unwrap_or(s)
	} else {
		return None;
	};
	normalize_identifier_suffix(suffix)
}

pub(crate) fn pre_route_rewrite_uri(req: &Request) -> Option<Uri> {
	let new_path = well_known_backend_path(req.uri().path())?;
	let new_path_and_query = if let Some(query) = req.uri().query() {
		format!("{new_path}?{query}")
	} else {
		new_path
	};
	let mut parts = req.uri().clone().into_parts();
	parts.path_and_query = Some(PathAndQuery::from_str(&new_path_and_query).ok()?);
	Uri::from_parts(parts).ok()
}

pub(crate) fn passthrough_well_known(req: &Request) -> Option<PassthroughWellKnown> {
	let path = request_path(req);
	if path.trim_end_matches('/') == OAUTH_AUTHORIZATION_SERVER_PREFIX {
		return Some(PassthroughWellKnown::UnsupportedAuthorizationServer);
	}

	let suffix = path.strip_prefix(OAUTH_PROTECTED_RESOURCE_PREFIX)?;
	let identifier = normalize_identifier_suffix(suffix)?;
	let new_path_and_query = if let Some(query) = req.uri().query() {
		format!("{OAUTH_PROTECTED_RESOURCE_PREFIX}?{query}")
	} else {
		OAUTH_PROTECTED_RESOURCE_PREFIX.to_string()
	};
	let mut parts = req.uri().clone().into_parts();
	parts.path_and_query = Some(PathAndQuery::from_str(&new_path_and_query).ok()?);
	let upstream_uri = Uri::from_parts(parts).ok()?;
	let origin = request_origin(req)?;
	Some(PassthroughWellKnown::ProtectedResource(
		PassthroughProtectedResource {
			upstream_uri,
			resource: format!("{origin}{identifier}"),
			authorization_server: format!("{origin}{OAUTH_AUTHORIZATION_SERVER_PREFIX}{identifier}"),
			resource_metadata: format!("{origin}{OAUTH_PROTECTED_RESOURCE_PREFIX}{identifier}"),
		},
	))
}

fn rewrite_www_authenticate_resource_metadata(
	value: &str,
	resource_metadata: &str,
) -> Result<HeaderValue, ::http::header::InvalidHeaderValue> {
	let token = "resource_metadata=\"";
	let updated = if let Some(start) = value.find(token) {
		let value_start = start + token.len();
		if let Some(end_rel) = value[value_start..].find('"') {
			let end = value_start + end_rel;
			format!(
				"{}{}{}",
				&value[..value_start],
				resource_metadata,
				&value[end..]
			)
		} else {
			format!("{value}, resource_metadata=\"{resource_metadata}\"")
		}
	} else if value.trim().is_empty() {
		format!("Bearer resource_metadata=\"{resource_metadata}\"")
	} else {
		format!("{value}, resource_metadata=\"{resource_metadata}\"")
	};
	HeaderValue::from_str(&updated)
}

pub(crate) fn rewrite_passthrough_www_authenticate(
	response: &mut Response,
	rewrite: &PassthroughProtectedResource,
) -> Result<(), ProxyError> {
	if response.status() != StatusCode::UNAUTHORIZED {
		return Ok(());
	}
	let Some(current) = response.headers().get(http::header::WWW_AUTHENTICATE) else {
		return Ok(());
	};
	let current = current
		.to_str()
		.map_err(|e| ProxyError::ProcessingString(e.to_string()))?;
	let updated = rewrite_www_authenticate_resource_metadata(current, &rewrite.resource_metadata)
		.map_err(|e| ProxyError::ProcessingString(e.to_string()))?;
	response
		.headers_mut()
		.insert(http::header::WWW_AUTHENTICATE, updated);
	Ok(())
}

pub(crate) async fn rewrite_passthrough_protected_resource_metadata(
	response: &mut Response,
	rewrite: &PassthroughProtectedResource,
) -> Result<(), ProxyError> {
	if !response.status().is_success() {
		return Ok(());
	}
	let limit = crate::http::response_buffer_limit(response);
	let body = std::mem::take(response.body_mut());
	let bytes = crate::http::read_body_with_limit(body, limit)
		.await
		.map_err(ProxyError::Body)?;
	let mut json: serde_json::Value = match serde_json::from_slice(bytes.as_ref()) {
		Ok(v) => v,
		Err(_) => {
			*response.body_mut() = Body::from(bytes);
			return Ok(());
		},
	};
	let Some(obj) = json.as_object_mut() else {
		*response.body_mut() = Body::from(bytes);
		return Ok(());
	};
	obj.insert(
		"resource".to_string(),
		serde_json::Value::String(rewrite.resource.clone()),
	);
	obj.insert(
		"authorization_servers".to_string(),
		serde_json::Value::Array(vec![serde_json::Value::String(
			rewrite.authorization_server.clone(),
		)]),
	);

	let rewritten_body =
		serde_json::to_vec(&json).map_err(|e| ProxyError::Body(crate::http::Error::new(e)))?;
	response.headers_mut().remove(http::header::CONTENT_LENGTH);
	response.headers_mut().insert(
		http::header::CONTENT_TYPE,
		HeaderValue::from_static("application/json"),
	);
	*response.body_mut() = Body::from(rewritten_body);
	Ok(())
}

pub(super) async fn apply_token_validation(
	req: &mut Request,
	auth: &McpAuthentication,
) -> Result<(), ProxyError> {
	// skip well-known OAuth endpoints for authn
	if is_well_known_endpoint(request_path(req)) {
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
	auth.jwt_validator.apply(None, req).await.map_err(|e| {
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
	let path = request_path(req).to_string();
	if !is_well_known_endpoint(path.as_str()) {
		apply_token_validation(req, auth).await?;
	}

	match path.as_str() {
		// TODO: indicate this is a DirectResponse
		path if path.ends_with("client-registration") => Ok(Some(
			client_registration(req, auth, client.clone())
				.await
				.map_err(|e| {
					warn!("client_registration error: {}", e);
					StatusCode::INTERNAL_SERVER_ERROR
				})
				.into_response(),
		)),
		path if path.starts_with("/.well-known/oauth-protected-resource") => Ok(Some(
			protected_resource_metadata(req, auth).await.into_response(),
		)),
		path if path.starts_with("/.well-known/oauth-authorization-server") => Ok(Some(
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
	let request_path = request_path(req);
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

	// Remove the oauth-protected-resource prefix and keep the remaining path
	if let Some(remaining_path) = path.strip_prefix(OAUTH_PROTECTED_RESOURCE_PREFIX) {
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
	use super::{
		OAUTH_PROTECTED_RESOURCE_PREFIX, PassthroughWellKnown, passthrough_well_known,
		pre_route_rewrite_uri, rewrite_passthrough_protected_resource_metadata,
		rewrite_passthrough_www_authenticate,
	};
	use crate::http::tests_common::request_for_uri;
	use crate::http::{Body, StatusCode};
	use serde_json::json;

	#[test]
	fn pre_route_rewrites_protected_resource_path() {
		let req =
			request_for_uri("http://example.com/.well-known/oauth-protected-resource/mcp/gitlab?foo=bar");
		let rewritten = pre_route_rewrite_uri(&req).expect("path should be rewritten");
		assert_eq!(rewritten.path(), "/mcp/gitlab");
		assert_eq!(rewritten.query(), Some("foo=bar"));
	}

	#[test]
	fn pre_route_rewrites_authorization_server_path() {
		let req =
			request_for_uri("http://example.com/.well-known/oauth-authorization-server/mcp/gitlab");
		let rewritten = pre_route_rewrite_uri(&req).expect("path should be rewritten");
		assert_eq!(rewritten.path(), "/mcp/gitlab");
	}

	#[test]
	fn pre_route_rewrites_client_registration_path() {
		let req = request_for_uri(
			"http://example.com/.well-known/oauth-authorization-server/mcp/gitlab/client-registration",
		);
		let rewritten = pre_route_rewrite_uri(&req).expect("path should be rewritten");
		assert_eq!(rewritten.path(), "/mcp/gitlab");
	}

	#[test]
	fn pre_route_does_not_rewrite_unscoped_authorization_server_path() {
		let req = request_for_uri("http://example.com/.well-known/oauth-authorization-server");
		assert!(pre_route_rewrite_uri(&req).is_none());
	}

	#[test]
	fn passthrough_marks_unscoped_authorization_server_unsupported() {
		let req = request_for_uri("http://example.com/.well-known/oauth-authorization-server");
		assert!(matches!(
			passthrough_well_known(&req),
			Some(PassthroughWellKnown::UnsupportedAuthorizationServer)
		));
	}

	#[test]
	fn passthrough_rewrites_protected_resource_upstream_and_metadata_urls() {
		let req =
			request_for_uri("http://example.com/.well-known/oauth-protected-resource/mcp/gitlab?foo=bar");
		let Some(PassthroughWellKnown::ProtectedResource(rewrite)) = passthrough_well_known(&req)
		else {
			panic!("expected protected resource passthrough rewrite");
		};
		assert_eq!(rewrite.upstream_uri.path(), OAUTH_PROTECTED_RESOURCE_PREFIX);
		assert_eq!(rewrite.upstream_uri.query(), Some("foo=bar"));
		assert_eq!(rewrite.resource, "http://example.com/mcp/gitlab");
		assert_eq!(
			rewrite.authorization_server,
			"http://example.com/.well-known/oauth-authorization-server/mcp/gitlab"
		);
		assert_eq!(
			rewrite.resource_metadata,
			"http://example.com/.well-known/oauth-protected-resource/mcp/gitlab"
		);
	}

	#[tokio::test]
	async fn rewrite_passthrough_metadata_response_sets_resource_and_auth_servers() {
		let req = request_for_uri("http://example.com/.well-known/oauth-protected-resource/mcp/gitlab");
		let Some(PassthroughWellKnown::ProtectedResource(rewrite)) = passthrough_well_known(&req)
		else {
			panic!("expected protected resource passthrough rewrite");
		};

		let mut response = ::http::Response::builder()
			.status(StatusCode::OK)
			.header("content-type", "application/json")
			.body(Body::from(
				r#"{"resource":"https://upstream.example/mcp","authorization_servers":["https://idp.example"],"foo":"bar"}"#,
			))
			.unwrap();
		rewrite_passthrough_protected_resource_metadata(&mut response, &rewrite)
			.await
			.unwrap();
		let parsed: serde_json::Value = crate::json::from_response_body(response).await.unwrap();
		assert_eq!(parsed["resource"], "http://example.com/mcp/gitlab");
		assert_eq!(
			parsed["authorization_servers"],
			json!(["http://example.com/.well-known/oauth-authorization-server/mcp/gitlab"])
		);
		assert_eq!(parsed["foo"], "bar");
	}

	#[test]
	fn rewrite_passthrough_www_authenticate_adds_resource_metadata() {
		let req = request_for_uri("http://example.com/.well-known/oauth-protected-resource/mcp/notion");
		let Some(PassthroughWellKnown::ProtectedResource(rewrite)) = passthrough_well_known(&req)
		else {
			panic!("expected protected resource passthrough rewrite");
		};
		let mut response = ::http::Response::builder()
			.status(StatusCode::UNAUTHORIZED)
			.header(
				http::header::WWW_AUTHENTICATE,
				"Bearer error=\"invalid_token\", error_description=\"The access token is required\"",
			)
			.body(Body::empty())
			.unwrap();
		rewrite_passthrough_www_authenticate(&mut response, &rewrite).unwrap();
		let header = response
			.headers()
			.get(http::header::WWW_AUTHENTICATE)
			.and_then(|v| v.to_str().ok())
			.unwrap_or_default();
		assert!(header.contains("error=\"invalid_token\""));
		assert!(header.contains("error_description=\"The access token is required\""));
		assert!(header.contains(
			"resource_metadata=\"http://example.com/.well-known/oauth-protected-resource/mcp/notion\""
		));
	}

	#[test]
	fn rewrite_passthrough_www_authenticate_replaces_existing_resource_metadata() {
		let req = request_for_uri("http://example.com/.well-known/oauth-protected-resource/mcp/notion");
		let Some(PassthroughWellKnown::ProtectedResource(rewrite)) = passthrough_well_known(&req)
		else {
			panic!("expected protected resource passthrough rewrite");
		};
		let mut response = ::http::Response::builder()
			.status(StatusCode::UNAUTHORIZED)
			.header(
				http::header::WWW_AUTHENTICATE,
				"Bearer error=\"invalid_token\", resource_metadata=\"http://upstream/.well-known/oauth-protected-resource\"",
			)
			.body(Body::empty())
			.unwrap();
		rewrite_passthrough_www_authenticate(&mut response, &rewrite).unwrap();
		let header = response
			.headers()
			.get(http::header::WWW_AUTHENTICATE)
			.and_then(|v| v.to_str().ok())
			.unwrap_or_default();
		assert!(header.contains(
			"resource_metadata=\"http://example.com/.well-known/oauth-protected-resource/mcp/notion\""
		));
		assert!(!header.contains("http://upstream/.well-known/oauth-protected-resource"));
	}
}
