use axum::http::StatusCode;
use axum_core::response::IntoResponse;
use bytes::Bytes;
use http::Method;
use http::uri::PathAndQuery;
use secrecy::ExposeSecret;
use tracing::{debug, warn};

use crate::http::jwt::Claims;
use crate::http::oauth::{
	authorization_server_metadata_url, entra_endpoints, openid_configuration_metadata_url,
};
use crate::http::*;
use crate::json;
use crate::json::from_body_with_limit;
use crate::mcp::relay_state;
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::metrics::{OutboundCallKind, OutboundCallSubtype};
use crate::types::agent::{McpAuthentication, McpIDP};

pub(crate) fn is_well_known_endpoint(path: &str) -> bool {
	path == "/.well-known/oauth-protected-resource"
		|| path.starts_with("/.well-known/oauth-protected-resource/")
		|| path == "/.well-known/oauth-authorization-server"
		|| path.starts_with("/.well-known/oauth-authorization-server/")
}

pub(super) async fn apply_token_validation(
	req: &mut Request,
	auth: &McpAuthentication,
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
	auth.jwt_validator.apply(None, req).await.map_err(|e| {
		create_auth_required_response(ProxyError::JwtAuthenticationFailure(e), req, auth)
	})?;
	Ok(())
}

pub(crate) async fn enforce_authentication(
	req: &mut Request,
	auth: &McpAuthentication,
	client: &PolicyClient,
) -> Result<Option<Response>, ProxyError> {
	// skip well-known OAuth endpoints for authn
	if !is_well_known_endpoint(req.uri().path()) {
		apply_token_validation(req, auth).await?;
	}

	handle_mcp_request(req, auth, client).await
}

pub(crate) async fn handle_mcp_request(
	req: &mut Request,
	auth: &McpAuthentication,
	client: &PolicyClient,
) -> Result<Option<Response>, ProxyError> {
	match req.uri().path() {
		// TODO: indicate this is a DirectResponse
		path if path.ends_with("client-registration") => Ok(Some(
			client_registration(req, auth, client.clone())
				.await
				.map_err(|e| {
					warn!("client_registration error: {}", e);
					StatusCode::INTERNAL_SERVER_ERROR
				})
				.into_response()
				.map(Body::new),
		)),
		path
			if path == "/.well-known/oauth-protected-resource"
				|| path.starts_with("/.well-known/oauth-protected-resource/") =>
		{
			Ok(Some(
				protected_resource_metadata(req, auth)
					.await
					.into_response()
					.map(Body::new),
			))
		},
		// Entra rejects the RFC 8707 `resource` parameter (AADSTS9010010), so the gateway
		// advertises proxied authorization/token endpoints (under the served AS metadata path)
		// that strip it before forwarding to Entra.
		path
			if matches!(auth.provider, Some(McpIDP::Entra {}))
				&& path.starts_with("/.well-known/oauth-authorization-server/")
				&& path.ends_with("/authorize") =>
		{
			Ok(Some(
				entra_authorize(req, auth)
					.map_err(|e| {
						warn!("entra authorize error: {}", e);
						StatusCode::INTERNAL_SERVER_ERROR
					})
					.into_response()
					.map(Body::new),
			))
		},
		path
			if matches!(auth.provider, Some(McpIDP::Entra {}))
				&& path.starts_with("/.well-known/oauth-authorization-server/")
				&& path.ends_with("/token") =>
		{
			Ok(Some(
				entra_token(req, auth, client.clone())
					.await
					.map_err(|e| {
						warn!("entra token error: {}", e);
						StatusCode::INTERNAL_SERVER_ERROR
					})
					.into_response()
					.map(Body::new),
			))
		},
		path
			if matches!(auth.provider, Some(McpIDP::Keycloak { .. }))
				&& path.starts_with("/.well-known/oauth-authorization-server/")
				&& path.ends_with("/authorize") =>
		{
			Ok(Some(
				keycloak_authorize(req, auth)
					.map_err(|e| {
						warn!("keycloak authorize error: {}", e);
						StatusCode::INTERNAL_SERVER_ERROR
					})
					.into_response()
					.map(Body::new),
			))
		},
		path
			if matches!(auth.provider, Some(McpIDP::Keycloak { .. }))
				&& path.starts_with("/.well-known/oauth-authorization-server/")
				&& path.ends_with("/callback") =>
		{
			Ok(Some(
				keycloak_callback(req, auth)
					.map_err(|e| {
						warn!("keycloak callback error: {}", e);
						StatusCode::INTERNAL_SERVER_ERROR
					})
					.into_response()
					.map(Body::new),
			))
		},
		path
			if matches!(auth.provider, Some(McpIDP::Keycloak { .. }))
				&& path.starts_with("/.well-known/oauth-authorization-server/")
				&& path.ends_with("/token") =>
		{
			Ok(Some(
				keycloak_token(req, auth, client.clone())
					.await
					.map_err(|e| {
						warn!("keycloak token error: {}", e);
						StatusCode::INTERNAL_SERVER_ERROR
					})
					.into_response()
					.map(Body::new),
			))
		},
		path
			if path == "/.well-known/oauth-authorization-server"
				|| path.starts_with("/.well-known/oauth-authorization-server/") =>
		{
			Ok(Some(
				authorization_server_metadata(req, auth, client.clone())
					.await
					.map_err(|e| {
						warn!("authorization_server_metadata error: {}", e);
						StatusCode::INTERNAL_SERVER_ERROR
					})
					.into_response()
					.map(Body::new),
			))
		},
		_ => {
			// Not handled
			Ok(None)
		},
	}
}

pub(crate) fn create_auth_required_response(
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
			parts.path_and_query = Some(PathAndQuery::from_static("/"));
			Uri::from_parts(parts).ok()
		})
		.and_then(|uri| uri.to_string().strip_suffix("/").map(ToString::to_string))
		.unwrap_or_else(|| get_redirect_url(req, request_path));
	let www_authenticate_value = format!(
		"Bearer resource_metadata=\"{proxy_url}/.well-known/oauth-protected-resource{request_path}\""
	);

	ProxyError::McpJwtAuthenticationFailure(Box::new(inner), www_authenticate_value)
}

fn keycloak_unterminated(auth: &McpAuthentication) -> bool {
	matches!(auth.provider, Some(McpIDP::Keycloak { .. })) && auth.client_id.is_none()
}

pub(super) async fn protected_resource_metadata(
	req: &mut Request,
	auth: &McpAuthentication,
) -> Response {
	let new_uri = strip_oauth_protected_resource_prefix(req);

	// Determine the issuer to use - either use the same request URL and path that it was initially with,
	// or else keep the auth.issuer
	let issuer = if auth.provider.is_some() && !keycloak_unterminated(auth) {
		// When a provider is configured (and, for Keycloak, actually terminates the flow), use
		// the same request URL with the well-known prefix stripped
		strip_oauth_protected_resource_prefix(req)
	} else {
		// No provider configured, or an unterminated Keycloak config: use the original issuer
		auth.issuer.clone()
	};

	let json_body = auth.resource_metadata.to_rfc_json(new_uri, issuer);

	::http::Response::builder()
		.status(StatusCode::OK)
		.header("content-type", "application/json")
		.header("access-control-allow-origin", "*")
		.header("access-control-allow-methods", "GET, OPTIONS")
		.header("access-control-allow-headers", "content-type")
		.body(Body::from(Bytes::from(
			serde_json::to_string(&json_body).unwrap_or_default(),
		)))
		.unwrap_or_else(|_| {
			::http::Response::builder()
				.status(StatusCode::INTERNAL_SERVER_ERROR)
				.body(Body::empty())
				.unwrap()
		})
}

fn get_redirect_url(req: &Request, strip_base: &str) -> String {
	let uri = request_uri_for_oauth_metadata(req);

	uri
		.path()
		.strip_suffix(strip_base)
		.map(|p| uri_with_path(uri.clone(), p))
		.unwrap_or(uri.to_string())
}

fn strip_oauth_protected_resource_prefix(req: &Request) -> String {
	let uri = request_uri_for_oauth_metadata(req);

	let path = uri.path().to_string();
	const OAUTH_PREFIX: &str = "/.well-known/oauth-protected-resource";

	// Remove the oauth-protected-resource prefix and keep the remaining path
	if let Some(remaining_path) = path.strip_prefix(OAUTH_PREFIX) {
		uri_with_path(uri, remaining_path)
	} else {
		// If the prefix is not found, return the original URI
		uri.to_string()
	}
}

fn issuer_from_authorization_server_metadata_request(req: &Request) -> Option<String> {
	const OAUTH_PREFIX: &str = "/.well-known/oauth-authorization-server";
	let external_uri = request_uri_for_oauth_metadata(req);
	let issuer_path = issuer_path_from_metadata_path(external_uri.path(), OAUTH_PREFIX)
		.or_else(|| issuer_path_from_metadata_path(req.uri().path(), OAUTH_PREFIX))?
		.to_string();
	Some(uri_with_path(external_uri, &issuer_path))
}

fn rewrite_authorization_server_issuer(
	req: &Request,
	auth: &McpAuthentication,
	metadata: &mut serde_json::Value,
) -> Result<(), ProxyError> {
	if auth.provider.is_none() {
		// Without a provider adapter, authorization server metadata should keep advertising the
		// upstream IdP issuer (auth.issuer) rather than presenting the gateway as the
		// authorization server issuer.
		return Ok(());
	}
	if keycloak_unterminated(auth) {
		return Ok(());
	}
	let Some(issuer) = issuer_from_authorization_server_metadata_request(req) else {
		return Ok(());
	};
	let Some(metadata) = metadata.as_object_mut() else {
		return Err(ProxyError::ProcessingString(
			"authorization server metadata must be a JSON object".to_string(),
		));
	};
	metadata.insert("issuer".to_string(), serde_json::Value::String(issuer));
	Ok(())
}

fn issuer_path_from_metadata_path<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
	if let Some(remaining_path) = path.strip_prefix(prefix)
		&& (remaining_path.is_empty() || remaining_path.starts_with('/'))
	{
		return Some(remaining_path);
	}

	// Older MCP clients append the well-known suffix to the resource path instead of using
	// RFC 8414's insertion-before-path form.
	path
		.strip_suffix(prefix)
		.or_else(|| path.strip_suffix(&format!("{prefix}/")))
}

fn keycloak_resource_uri(req: &Request) -> Result<Uri, ProxyError> {
	const OAUTH_PREFIX: &str = "/.well-known/oauth-authorization-server";
	let external_uri = request_uri_for_oauth_metadata(req);

	// Determine the issuer path (owned, to break lifetime dependency on external_uri)
	let issuer_path: String = {
		let external_path = external_uri.path();
		let req_path = req.uri().path();

		issuer_path_from_metadata_path(external_path, OAUTH_PREFIX)
			.or_else(|| issuer_path_from_metadata_path(req_path, OAUTH_PREFIX))
			.ok_or_else(|| {
				ProxyError::ProcessingString(format!(
					"request path {:?} is not under {OAUTH_PREFIX}",
					req_path
				))
			})?
			.to_string()
	};

	let resource_path = issuer_path
		.strip_suffix("/callback")
		.or_else(|| issuer_path.strip_suffix("/authorize"))
		.or_else(|| issuer_path.strip_suffix("/token"))
		.unwrap_or(issuer_path.as_str());

	uri_with_path(external_uri, resource_path)
		.parse()
		.map_err(|e| ProxyError::ProcessingString(format!("invalid resource uri: {e}")))
}

fn keycloak_callback_uri(req: &Request) -> Result<String, ProxyError> {
	Ok(format!(
		"{}/callback",
		authorization_server_metadata_url(&keycloak_resource_uri(req)?.to_string())
	))
}

fn uri_with_path(uri: Uri, path: &str) -> String {
	let mut parts = uri.into_parts();
	let path_and_query = if path.is_empty() {
		PathAndQuery::from_static("/")
	} else {
		PathAndQuery::try_from(path.to_string()).unwrap_or_else(|_| PathAndQuery::from_static("/"))
	};
	parts.path_and_query = Some(path_and_query);

	let uri = Uri::from_parts(parts)
		.map(|uri| uri.to_string())
		.unwrap_or_default();
	if path.is_empty() {
		uri.strip_suffix('/').unwrap_or(&uri).to_string()
	} else {
		uri
	}
}

fn request_uri_for_oauth_metadata(req: &Request) -> Uri {
	let uri = req
		.extensions()
		.get::<filters::OriginalUrl>()
		.map(|u| u.0.clone())
		.unwrap_or_else(|| req.uri().clone());

	crate::http::x_headers::apply_forwarded_scheme(uri, req.headers())
}

fn apply_keycloak_registration_rewrite(
	current_uri: &Uri,
	resp: &mut serde_json::Value,
) -> Result<(), ProxyError> {
	let Some(serde_json::Value::String(re)) = json::traverse_mut(resp, &["registration_endpoint"])
	else {
		return Err(ProxyError::ProcessingString(
			"registration_endpoint missing".to_string(),
		));
	};
	*re = format!("{current_uri}/client-registration");
	Ok(())
}

fn apply_keycloak_endpoint_rewrites(
	current_uri: &Uri,
	resp: &mut serde_json::Value,
) -> Result<(), ProxyError> {
	let Some(serde_json::Value::String(ae)) = json::traverse_mut(resp, &["authorization_endpoint"])
	else {
		return Err(ProxyError::ProcessingString(
			"authorization_endpoint missing".to_string(),
		));
	};
	*ae = format!("{current_uri}/authorize");

	let Some(serde_json::Value::String(te)) = json::traverse_mut(resp, &["token_endpoint"]) else {
		return Err(ProxyError::ProcessingString(
			"token_endpoint missing".to_string(),
		));
	};
	*te = format!("{current_uri}/token");

	apply_keycloak_registration_rewrite(current_uri, resp)
}

pub(super) async fn authorization_server_metadata(
	req: &mut Request,
	auth: &McpAuthentication,
	client: PolicyClient,
) -> Result<Response, ProxyError> {
	// RFC 8414 URL for standard AS metadata. Keycloak does not implement RFC 8414; it only
	// exposes OpenID Provider Metadata at {issuer}/.well-known/openid-configuration (OIDC Discovery).
	let metadata_uri = match &auth.provider {
		// Keycloak, Okta, Descope, and authentik do not support the RFC 8414 path-based issuer
		// format; they serve metadata at {issuer}/.well-known/openid-configuration (OIDC Discovery).
		Some(McpIDP::Keycloak { .. })
		| Some(McpIDP::Okta {})
		| Some(McpIDP::Descope {})
		| Some(McpIDP::Authentik {}) => openid_configuration_metadata_url(&auth.issuer),
		// Entra does not implement RFC 8414 either; it only serves OIDC Discovery documents.
		// Always fetch the v2.0 document (derived from the tenant in the issuer) so the
		// advertised endpoints support the scope/PKCE flows MCP clients use, even when the
		// configured issuer is the v1 form (sts.windows.net) used for token validation.
		Some(McpIDP::Entra {}) => {
			entra_endpoints(&auth.issuer)
				.map_err(ProxyError::ProcessingString)?
				.openid_configuration
		},
		_ => authorization_server_metadata_url(&auth.issuer),
	};
	let ureq = ::http::Request::builder()
		.uri(metadata_uri)
		.body(crate::http::Body::empty())?;
	let upstream = client
		.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Oidc)
		.simple_call(ureq)
		.await?;
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
		Some(McpIDP::Okta {}) => {
			// Okta does not support RFC 8707. Workaround by appending audience as a query param.
			let Some(serde_json::Value::String(ae)) =
				json::traverse_mut(&mut resp, &["authorization_endpoint"])
			else {
				return Err(ProxyError::ProcessingString(
					"authorization_endpoint missing".to_string(),
				));
			};
			if let Some(aud) = auth.audiences.first() {
				ae.push_str(&format!("?audience={}", aud));
			}

			// Okta doesn't do CORS for client registrations — proxy it (same pattern as Keycloak)
			let current_uri = request_uri_for_oauth_metadata(req);
			if let Some(serde_json::Value::String(re)) =
				json::traverse_mut(&mut resp, &["registration_endpoint"])
			{
				*re = format!("{current_uri}/client-registration");
			}
		},
		Some(McpIDP::Descope {}) => {
			// Descope supports RFC 8707, so no audience workaround needed.
			// Management DCR endpoint likely lacks CORS — proxy it.
			// Note: DCR requires a management key; recommend using clientId short-circuit instead.
			let current_uri = request_uri_for_oauth_metadata(req);
			if let Some(serde_json::Value::String(re)) =
				json::traverse_mut(&mut resp, &["registration_endpoint"])
			{
				*re = format!("{current_uri}/client-registration");
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

			let current_uri = request_uri_for_oauth_metadata(req);
			if auth.client_id.is_some() {
				apply_keycloak_endpoint_rewrites(&current_uri, &mut resp)?;
			} else {
			// No configured client_id: per-client DCR is handled directly by Keycloak.
			// Keep Keycloak's authorize/token endpoints unchanged; only proxy registration.
				apply_keycloak_registration_rewrite(&current_uri, &mut resp)?;
			}
		},
		Some(McpIDP::Authentik {}) => {
			// authentik does not support RFC 8707, and has no audience query parameter workaround.
			// Tokens carry the OAuth client ID in `aud`, so users must configure `audiences`
			// with the pre-registered client ID.

			// authentik does not implement Dynamic Client Registration (RFC 7591), so its
			// discovery metadata has no registration_endpoint at all:
			// https://github.com/goauthentik/authentik/issues/8751
			// Inject one pointing at the gateway so MCP clients can complete DCR against
			// the pre-registered client configured via `clientId`.
			let current_uri = request_uri_for_oauth_metadata(req);
			if let Some(obj) = resp.as_object_mut() {
				obj.insert(
					"registration_endpoint".to_string(),
					serde_json::Value::String(format!("{current_uri}/client-registration")),
				);
			}
		},
		Some(McpIDP::Entra {}) => {
			let current_uri = request_uri_for_oauth_metadata(req);

			// Entra rejects the RFC 8707 `resource` parameter (AADSTS9010010). Advertise
			// gateway-proxied authorization/token endpoints that strip it before forwarding.
			let Some(serde_json::Value::String(ae)) =
				json::traverse_mut(&mut resp, &["authorization_endpoint"])
			else {
				return Err(ProxyError::ProcessingString(
					"authorization_endpoint missing".to_string(),
				));
			};
			*ae = format!("{current_uri}/authorize");
			let Some(serde_json::Value::String(te)) = json::traverse_mut(&mut resp, &["token_endpoint"])
			else {
				return Err(ProxyError::ProcessingString(
					"token_endpoint missing".to_string(),
				));
			};
			*te = format!("{current_uri}/token");

			if let Some(obj) = resp.as_object_mut() {
				// Entra does not implement RFC 7591 (no registration_endpoint in its metadata);
				// advertise the gateway's registration endpoint, which short-circuits with the
				// configured clientId.
				obj.insert(
					"registration_endpoint".to_string(),
					serde_json::Value::String(format!("{current_uri}/client-registration")),
				);
				// Entra supports PKCE (S256) but omits it from its discovery document; MCP
				// clients require it to be advertised.
				obj
					.entry("code_challenge_methods_supported")
					.or_insert_with(|| serde_json::json!(["S256"]));
			}
		},
		_ => {},
	}

	rewrite_authorization_server_issuer(req, auth, &mut resp)?;

	let response = ::http::Response::builder()
		.status(StatusCode::OK)
		.header("content-type", "application/json")
		.header("access-control-allow-origin", "*")
		.header("access-control-allow-methods", "GET, OPTIONS")
		.header("access-control-allow-headers", "content-type")
		.body(Body::from(Bytes::from(
			serde_json::to_string(&resp).map_err(|e| ProxyError::Body(crate::http::Error::new(e)))?,
		)))?;

	Ok(response)
}

pub(super) async fn client_registration(
	req: &mut Request,
	auth: &McpAuthentication,
	client: PolicyClient,
) -> Result<Response, ProxyError> {
	if let Some(client_id) = &auth.client_id {
		return build_mock_dcr_response(req, client_id).await;
	}

	// Normalize issuer URL by removing trailing slashes to avoid double-slash in path
	let issuer = auth.issuer.trim_end_matches('/');
	let registration_uri = match &auth.provider {
		Some(McpIDP::Entra {}) => {
			// Entra has no Dynamic Client Registration endpoint to proxy to; registration only
			// works via the clientId short-circuit above.
			return Err(ProxyError::ProcessingString(
				"Entra ID does not support Dynamic Client Registration (RFC 7591); set `clientId` on mcpAuthentication to a pre-registered app registration".to_string(),
			));
		},
		Some(McpIDP::Okta {}) => {
			// Okta's DCR endpoint is relative to the org URL, not the issuer.
			// Issuer: https://trial-xxx.okta.com/oauth2/default
			// DCR:    https://trial-xxx.okta.com/oauth2/v1/clients
			let parsed: url::Url = issuer
				.parse()
				.map_err(|e| ProxyError::ProcessingString(format!("invalid issuer URL: {e}")))?;
			let origin = parsed.origin().ascii_serialization();
			format!("{origin}/oauth2/v1/clients")
		},
		Some(McpIDP::Descope {}) => {
			// DCR endpoint: https://api.descope.com/v1/mgmt/mcp/client/{project-id}/{server-id}/register
			// Derived from agentic issuer: https://api.descope.com/v1/apps/agentic/{project-id}/{server-id}
			let parsed: url::Url = issuer
				.parse()
				.map_err(|e| ProxyError::ProcessingString(format!("invalid issuer URL: {e}")))?;
			let segments: Vec<&str> = parsed.path().trim_start_matches('/').split('/').collect();
			if segments.len() >= 5
				&& segments[0] == "v1"
				&& segments[1] == "apps"
				&& segments[2] == "agentic"
			{
				let (project_id, server_id) = (segments[3], segments[4]);
				let origin = parsed.origin().ascii_serialization();
				format!("{origin}/v1/mgmt/mcp/client/{project_id}/{server_id}/register")
			} else {
				return Err(ProxyError::ProcessingString(
					"Descope DCR requires an agentic issuer URL".to_string(),
				));
			}
		},
		Some(McpIDP::Authentik {}) => {
			// authentik has no DCR endpoint to proxy to (RFC 7591 is unimplemented:
			// https://github.com/goauthentik/authentik/issues/8751). The only supported flow
			// is a pre-registered client via `clientId`, which is handled above.
			return Err(ProxyError::ProcessingString(
				"authentik does not support Dynamic Client Registration; set clientId to a pre-registered public client".to_string(),
			));
		},
		// Keycloak and default
		_ => format!("{issuer}/clients-registrations/openid-connect"),
	};
	let ureq = ::http::Request::builder()
		.uri(registration_uri)
		.method(Method::POST)
		.body(std::mem::take(req.body_mut()))?;

	let mut upstream = client
		.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Oidc)
		.simple_call(ureq)
		.await?;

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

/// Proxy an OAuth authorization request to Entra, stripping the RFC 8707 `resource` parameter.
///
/// Entra's v2.0 endpoint rejects requests carrying `resource` alongside v2-style `scope`
/// values with `AADSTS9010010: invalid_target`, but MCP clients are required by the MCP
/// authorization spec to send it. The gateway advertises this endpoint in the served AS
/// metadata and redirects the user agent to the real Entra authorize endpoint without it.
pub(super) fn entra_authorize(
	req: &Request,
	auth: &McpAuthentication,
) -> Result<Response, ProxyError> {
	let endpoints = entra_endpoints(&auth.issuer).map_err(ProxyError::ProcessingString)?;
	let mut location: Uri = match req.uri().query() {
		Some(query) => format!("{}?{}", endpoints.authorization_endpoint, query),
		None => endpoints.authorization_endpoint,
	}
	.parse()
	.map_err(|e| ProxyError::ProcessingString(format!("invalid authorize URL: {e}")))?;
	crate::http::modify_query_parameters(
		&mut location,
		std::iter::empty::<(&str, &str)>(),
		["resource"],
	)
	.map_err(|e| ProxyError::ProcessingString(e.to_string()))?;
	Ok(
		::http::Response::builder()
			.status(StatusCode::FOUND)
			.header(::http::header::LOCATION, location.to_string())
			.body(Body::empty())?,
	)
}

/// Allows loopback HTTP callbacks for native/CLI clients (RFC 8252), or HTTPS
/// callbacks on the gateway's own origin.
///
/// Keycloak validates only the gateway's substituted callback in this flow, so
/// the client's original redirect URI must be restricted here to prevent an
/// authorization code from being redirected to an arbitrary host.
fn is_allowed_client_redirect_uri(uri: &str, gateway_origin: &str) -> bool {
	let Ok(parsed) = url::Url::parse(uri) else {
		return false;
	};
	match parsed.scheme() {
		// `url::Url::host_str()` serializes IPv6 hosts bracketed (`"[::1]"`), so a naive
		// `Some("::1")` match would never fire for the IPv6 loopback form RFC 8252 native
		// clients are told to try alongside `127.0.0.1`. Reuse the shared host check
		// (`super::is_localhost_host`), which already strips brackets, rather than
		// duplicating that logic here.
		"http" => parsed.host_str().is_some_and(super::is_localhost_host),
		"https" => parsed.origin().ascii_serialization() == gateway_origin,
		_ => false,
	}
}

pub(super) fn keycloak_authorize(
	req: &Request,
	auth: &McpAuthentication,
) -> Result<Response, ProxyError> {
	let relay_signing_key = auth.relay_signing_key.as_ref().ok_or_else(|| {
		ProxyError::ProcessingString(
			"keycloak provider requires relaySigningKey to encrypt the relay-state token".to_string(),
		)
	})?;

	let query: std::collections::HashMap<String, String> =
		url::form_urlencoded::parse(req.uri().query().unwrap_or("").as_bytes())
			.into_owned()
			.collect();
	let client_redirect_uri = query.get("redirect_uri").cloned().ok_or_else(|| {
		ProxyError::ProcessingString("authorize request missing redirect_uri".to_string())
	})?;
	let client_state = query.get("state").cloned();

	let gateway_origin = {
		let resource_uri = keycloak_resource_uri(req)?.to_string();
		url::Url::parse(&resource_uri)
			.map_err(|e| ProxyError::ProcessingString(format!("invalid gateway origin: {e}")))?
			.origin()
			.ascii_serialization()
	};
	if !is_allowed_client_redirect_uri(&client_redirect_uri, &gateway_origin) {
		return Err(ProxyError::ProcessingString(format!(
			"redirect_uri {client_redirect_uri:?} is not an allowed client callback (must be a loopback http callback or same-origin as the gateway)"
		)));
	}

	let relay_token = relay_state::encode(
		relay_signing_key,
		&relay_state::RelayState {
			client_redirect_uri,
			client_state,
			expires_at_unix: now_unix().saturating_add(300),
		},
	)?;

	let callback_uri = keycloak_callback_uri(req)?;

	let base = format!(
		"{}/protocol/openid-connect/auth",
		auth.issuer.trim_end_matches('/')
	);
	let mut location: Uri = match req.uri().query() {
		Some(query) => format!("{base}?{query}")
			.parse()
			.map_err(|e| ProxyError::ProcessingString(format!("invalid authorize URL: {e}")))?,
		None => base
			.parse()
			.map_err(|e| ProxyError::ProcessingString(format!("invalid authorize URL: {e}")))?,
	};
	crate::http::modify_query_parameters(
		&mut location,
		[
			("redirect_uri", callback_uri.as_str()),
			("state", relay_token.as_str()),
			("response_mode", "query"),
			// The callback relay reads code/state from the query string, so other response
			// modes (e.g. fragment or form_post) would bypass the relay.
		],
		std::iter::empty::<&str>(),
	)
	.map_err(|e| ProxyError::ProcessingString(e.to_string()))?;

	Ok(
		::http::Response::builder()
			.status(StatusCode::FOUND)
			.header(::http::header::LOCATION, location.to_string())
			.body(Body::empty())?,
	)
}

pub(super) fn keycloak_callback(
	req: &Request,
	auth: &McpAuthentication,
) -> Result<Response, ProxyError> {
	let relay_signing_key = auth.relay_signing_key.as_ref().ok_or_else(|| {
		ProxyError::ProcessingString(
			"keycloak provider requires relaySigningKey to decrypt the relay-state token".to_string(),
		)
	})?;

	let query: std::collections::HashMap<String, String> =
		url::form_urlencoded::parse(req.uri().query().unwrap_or("").as_bytes())
			.into_owned()
			.collect();
	let relay_token = query
		.get("state")
		.cloned()
		.ok_or_else(|| ProxyError::ProcessingString("callback missing state".to_string()))?;
	let relay = relay_state::decode(relay_signing_key, &relay_token)?;

	let mut location: Uri = relay
		.client_redirect_uri
		.parse()
		.map_err(|e| ProxyError::ProcessingString(format!("invalid client redirect_uri: {e}")))?;

	if let Some(error) = query.get("error").cloned() {
		let gateway_issuer = keycloak_resource_uri(req)?.to_string();
		let error_description = query.get("error_description").cloned();
		let error_uri = query.get("error_uri").cloned();

		let mut extra = vec![("error", error.as_str())];
		if let Some(ed) = error_description.as_deref() {
			extra.push(("error_description", ed));
		}
		if let Some(eu) = error_uri.as_deref() {
			extra.push(("error_uri", eu));
		}
		extra.push(("iss", gateway_issuer.as_str()));
		if let Some(state) = relay.client_state.as_deref() {
			extra.push(("state", state));
		}
		crate::http::modify_query_parameters(&mut location, extra, std::iter::empty::<&str>())
			.map_err(|e| ProxyError::ProcessingString(e.to_string()))?;

		return Ok(
			::http::Response::builder()
				.status(StatusCode::FOUND)
				.header(::http::header::LOCATION, location.to_string())
				.body(Body::empty())?,
		);
	}

	let code = query
		.get("code")
		.cloned()
		.ok_or_else(|| ProxyError::ProcessingString("callback missing code".to_string()))?;
	let gateway_issuer = keycloak_resource_uri(req)?.to_string();

	let mut extra = vec![("code", code.as_str()), ("iss", gateway_issuer.as_str())];
	if let Some(state) = relay.client_state.as_deref() {
		extra.push(("state", state));
	}
	crate::http::modify_query_parameters(&mut location, extra, std::iter::empty::<&str>())
		.map_err(|e| ProxyError::ProcessingString(e.to_string()))?;

	Ok(
		::http::Response::builder()
			.status(StatusCode::FOUND)
			.header(::http::header::LOCATION, location.to_string())
			.body(Body::empty())?,
	)
}

/// Rewrites `redirect_uri` in a token-exchange form to the gateway's `/callback` URL.
/// Keycloak requires it to match the `redirect_uri` used by `/authorize`, which
/// [`keycloak_authorize`] replaces with the gateway callback. Forms without a
/// `redirect_uri` (for example, a `refresh_token` grant) are left unchanged.
fn rewrite_keycloak_token_form(body: &[u8], callback_uri: &str) -> String {
	url::form_urlencoded::Serializer::new(String::new())
		.extend_pairs(url::form_urlencoded::parse(body).map(|(k, v)| {
			if k == "redirect_uri" {
				(k.into_owned(), callback_uri.to_string())
			} else {
				(k.into_owned(), v.into_owned())
			}
		}))
		.finish()
}

struct KeycloakTokenFormFields {
	grant_type: Option<String>,
	client_id: Option<String>,
	has_client_secret: bool,
	/// Whether `grant_type`, `client_id`, or `client_secret` appears more than once.
	/// Duplicate fields could cause this validation to inspect a different value from
	/// the one ultimately used by Keycloak.
	has_duplicate_injection_fields: bool,
}

fn keycloak_token_form_fields(body: &[u8]) -> KeycloakTokenFormFields {
	let mut grant_type = None;
	let mut client_id = None;
	let mut has_client_secret = false;
	let mut grant_type_count = 0u32;
	let mut client_id_count = 0u32;
	let mut client_secret_count = 0u32;
	for (k, v) in url::form_urlencoded::parse(body) {
		match k.as_ref() {
			"grant_type" => {
				grant_type = Some(v.into_owned());
				grant_type_count += 1;
			},
			"client_id" => {
				client_id = Some(v.into_owned());
				client_id_count += 1;
			},
			"client_secret" => {
				has_client_secret = true;
				client_secret_count += 1;
			},
			_ => {},
		}
	}
	KeycloakTokenFormFields {
		grant_type,
		client_id,
		has_client_secret,
		has_duplicate_injection_fields: grant_type_count > 1
			|| client_id_count > 1
			|| client_secret_count > 1,
	}
}

/// Proxies a token-exchange request to Keycloak, rewriting `redirect_uri` to the gateway's
/// `/callback` URL. If the configured client is confidential and the request does not already
/// authenticate the client, the configured `clientSecret` is injected when allowed.
pub(super) async fn keycloak_token(
	req: &mut Request,
	auth: &McpAuthentication,
	client: PolicyClient,
) -> Result<Response, ProxyError> {
	if req.method() != Method::POST {
		return Ok(
			::http::Response::builder()
				.status(StatusCode::METHOD_NOT_ALLOWED)
				.header(::http::header::ALLOW, "POST")
				.body(Body::empty())?,
		);
	}

	let callback_uri = keycloak_callback_uri(req)?;
	let limit = crate::http::buffer_limit(req);
	let body = std::mem::take(req.body_mut());
	let bytes = crate::http::read_body_with_limit(body, limit)
		.await
		.map_err(ProxyError::Body)?;
	let mut form = rewrite_keycloak_token_form(&bytes, &callback_uri);

	let authorization = req.headers().get(::http::header::AUTHORIZATION).cloned();
	let fields = keycloak_token_form_fields(&bytes);
	let client_id_matches = auth.client_id.is_some() && fields.client_id == auth.client_id;
	if authorization.is_none()
		&& !fields.has_client_secret
		&& !fields.has_duplicate_injection_fields
		&& client_id_matches
		&& entra_grant_may_use_client_secret(fields.grant_type.as_deref())
		&& let Some(secret) = &auth.client_secret
	{
		form = url::form_urlencoded::Serializer::new(form)
			.append_pair("client_secret", secret.expose_secret())
			.finish();
	}

	let token_endpoint = format!(
		"{}/protocol/openid-connect/token",
		auth.issuer.trim_end_matches('/')
	);
	let mut builder = ::http::Request::builder()
		.uri(token_endpoint)
		.method(Method::POST)
		.header(
			::http::header::CONTENT_TYPE,
			"application/x-www-form-urlencoded",
		);
	if let Some(authorization) = authorization {
		builder = builder.header(::http::header::AUTHORIZATION, authorization);
	}
	let ureq = builder.body(Body::from(form))?;
	let upstream = client
		.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Oidc)
		.simple_call(ureq)
		.await?;

	Ok(upstream)
}

fn now_unix() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default()
		.as_secs()
}

/// Proxy an OAuth token request to Entra, stripping the RFC 8707 `resource` parameter
/// (see [`entra_authorize`]) and injecting the configured client secret when the client did
/// not supply one. Entra app registrations under the Web platform are confidential clients
/// and require the secret at the token endpoint, while public clients (PKCE-only) do not.
///
/// The secret is only attached when the request is for the configured `clientId` (the app
/// registration the secret belongs to) and uses a user-delegated grant (`authorization_code`,
/// `refresh_token`). This endpoint is reachable pre-authentication, so injecting the secret
/// into other grant types, notably `client_credentials`, would let any caller mint
/// app-level tokens with the gateway's credential.
pub(super) async fn entra_token(
	req: &mut Request,
	auth: &McpAuthentication,
	client: PolicyClient,
) -> Result<Response, ProxyError> {
	// CORS (including preflight) is the responsibility of the route's cors policy.
	if req.method() != Method::POST {
		return Ok(
			::http::Response::builder()
				.status(StatusCode::METHOD_NOT_ALLOWED)
				.header(::http::header::ALLOW, "POST")
				.body(Body::empty())?,
		);
	}

	let endpoints = entra_endpoints(&auth.issuer).map_err(ProxyError::ProcessingString)?;
	// Clients using client_secret_basic carry their credentials in the Authorization header;
	// forward it and don't inject a second credential.
	let authorization = req.headers().get(::http::header::AUTHORIZATION).cloned();
	let limit = crate::http::buffer_limit(req);
	let body = std::mem::take(req.body_mut());
	let bytes = crate::http::read_body_with_limit(body, limit)
		.await
		.map_err(ProxyError::Body)?;

	let parsed = parse_entra_token_form(&bytes);
	// The configured secret belongs to the app registration identified by the configured
	// clientId (the one the DCR short-circuit hands out); never attach it to a request for
	// any other client_id.
	let client_id_matches = auth.client_id.is_some() && parsed.client_id == auth.client_id;
	let mut form = parsed.form;
	if authorization.is_none()
		&& !parsed.has_client_secret
		&& client_id_matches
		&& entra_grant_may_use_client_secret(parsed.grant_type.as_deref())
		&& let Some(secret) = &auth.client_secret
	{
		form = url::form_urlencoded::Serializer::new(form)
			.append_pair("client_secret", secret.expose_secret())
			.finish();
	}

	let mut builder = ::http::Request::builder()
		.uri(endpoints.token_endpoint)
		.method(Method::POST)
		.header(
			::http::header::CONTENT_TYPE,
			"application/x-www-form-urlencoded",
		);
	if let Some(authorization) = authorization {
		builder = builder.header(::http::header::AUTHORIZATION, authorization);
	}
	let ureq = builder.body(Body::from(form))?;
	let upstream = client
		.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Oidc)
		.simple_call(ureq)
		.await?;

	Ok(upstream)
}

/// An OAuth token request form re-encoded without any `resource` parameters, plus the fields
/// needed to decide whether the configured client secret may be attached.
struct EntraTokenForm {
	form: String,
	has_client_secret: bool,
	grant_type: Option<String>,
	client_id: Option<String>,
}

/// Only user-delegated grants may have the gateway's client secret attached; see
/// [`entra_token`].
fn entra_grant_may_use_client_secret(grant_type: Option<&str>) -> bool {
	matches!(grant_type, Some("authorization_code" | "refresh_token"))
}

fn parse_entra_token_form(input: &[u8]) -> EntraTokenForm {
	let mut has_client_secret = false;
	let mut grant_type = None;
	let mut client_id = None;
	let mut serializer = url::form_urlencoded::Serializer::new(String::new());
	for (k, v) in url::form_urlencoded::parse(input) {
		match k.as_ref() {
			"client_secret" => has_client_secret = true,
			"grant_type" => grant_type = Some(v.to_string()),
			"client_id" => client_id = Some(v.to_string()),
			_ => {},
		}
		if k != "resource" {
			serializer.append_pair(&k, &v);
		}
	}
	EntraTokenForm {
		form: serializer.finish(),
		has_client_secret,
		grant_type,
		client_id,
	}
}

const MOCK_DCR_CLIENT_ID_ISSUED_AT: u64 = 0;

/// Build the mock Dynamic Client Registration response used when
/// `MCPAuthentication.clientId` is configured.
///
/// This path is for pre-registered IdP clients. The gateway is not creating
/// a client upstream, so return deterministic registration metadata and carry
/// forward only the requested redirect URIs that strict MCP clients validate.
async fn build_mock_dcr_response(
	req: &mut Request,
	client_id: &str,
) -> Result<Response, ProxyError> {
	let limit = crate::http::buffer_limit(req);
	let body = std::mem::take(req.body_mut());
	let bytes = crate::http::read_body_with_limit(body, limit)
		.await
		.map_err(ProxyError::Body)?;

	let redirect_uris = serde_json::from_slice::<serde_json::Value>(&bytes)
		.ok()
		.and_then(|json| json.get("redirect_uris").filter(|v| v.is_array()).cloned())
		.unwrap_or_else(|| serde_json::json!([]));

	let response_json = serde_json::json!({
		"client_id": client_id,
		"client_id_issued_at": MOCK_DCR_CLIENT_ID_ISSUED_AT,
		"token_endpoint_auth_method": "none",
		"grant_types": ["authorization_code"],
		"response_types": ["code"],
		"redirect_uris": redirect_uris,
	});

	let body_bytes = bytes::Bytes::from(
		serde_json::to_vec(&response_json).map_err(|e| ProxyError::ProcessingString(e.to_string()))?,
	);
	Ok(
		::http::Response::builder()
			.status(::http::StatusCode::CREATED)
			.header(::http::header::CONTENT_TYPE, "application/json")
			.body(Body::from(body_bytes))?,
	)
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use wiremock::matchers::{method, path};
	use wiremock::{Mock, MockServer, ResponseTemplate};

	use super::*;

	#[test]
	fn request_uri_for_oauth_metadata_uses_x_forwarded_proto() {
		let req = ::http::Request::builder()
			.uri("http://example.com/.well-known/oauth-protected-resource/mcp")
			.header("x-forwarded-proto", "https")
			.body(Body::empty())
			.expect("request should build");

		assert_eq!(
			request_uri_for_oauth_metadata(&req).to_string(),
			"https://example.com/.well-known/oauth-protected-resource/mcp"
		);
	}

	#[rstest::rstest]
	#[case::root(
		"https://gateway.example.com/.well-known/oauth-authorization-server",
		"https://gateway.example.com"
	)]
	#[case::path(
		"https://gateway.example.com/.well-known/oauth-authorization-server/example/mcp",
		"https://gateway.example.com/example/mcp"
	)]
	#[case::explicit_port_and_encoded_path(
		"https://gateway.example.com:8443/.well-known/oauth-authorization-server/tenant%2Fname",
		"https://gateway.example.com:8443/tenant%2Fname"
	)]
	#[case::trailing_slash(
		"https://gateway.example.com/.well-known/oauth-authorization-server/",
		"https://gateway.example.com/"
	)]
	fn authorization_server_issuer_matches_metadata_request(
		#[case] metadata_uri: &'static str,
		#[case] expected: &str,
	) {
		let req = ::http::Request::builder()
			.uri(metadata_uri)
			.body(Body::empty())
			.expect("request should build");

		assert_eq!(
			issuer_from_authorization_server_metadata_request(&req)
				.expect("metadata request should have an issuer"),
			expected
		);
	}

	#[test]
	fn authorization_server_issuer_uses_forwarded_scheme() {
		let req = ::http::Request::builder()
			.uri("http://gateway.example.com/.well-known/oauth-authorization-server/example/mcp")
			.header("x-forwarded-proto", "https")
			.body(Body::empty())
			.expect("request should build");

		assert_eq!(
			issuer_from_authorization_server_metadata_request(&req)
				.expect("metadata request should have an issuer"),
			"https://gateway.example.com/example/mcp"
		);
	}

	#[test]
	fn authorization_server_issuer_ignores_unexpected_path() {
		let req = ::http::Request::builder()
			.uri("https://gateway.example.com/example/mcp")
			.body(Body::empty())
			.expect("request should build");

		assert!(issuer_from_authorization_server_metadata_request(&req).is_none());
	}

	#[rstest::rstest]
	#[case::without_trailing_slash(
		"https://gateway.example.com/mcp/.well-known/oauth-authorization-server"
	)]
	#[case::with_trailing_slash(
		"https://gateway.example.com/mcp/.well-known/oauth-authorization-server/"
	)]
	fn authorization_server_issuer_supports_legacy_suffix_form(#[case] original_url: &str) {
		let mut req = ::http::Request::builder()
			.uri("http://backend.internal/.well-known/oauth-authorization-server")
			.body(Body::empty())
			.expect("request should build");
		req.extensions_mut().insert(filters::OriginalUrl(
			original_url.parse().expect("original URL should parse"),
		));

		assert_eq!(
			issuer_from_authorization_server_metadata_request(&req)
				.expect("metadata request should have an issuer"),
			"https://gateway.example.com/mcp"
		);
	}

	#[test]
	fn authorization_server_metadata_replaces_or_inserts_issuer() {
		let mut auth = default_auth();
		auth.provider = Some(McpIDP::Entra {});
		let req = ::http::Request::builder()
			.uri("https://gateway.example.com/.well-known/oauth-authorization-server/example/mcp")
			.body(Body::empty())
			.expect("request should build");

		for mut metadata in [
			serde_json::json!({"issuer": "https://idp.example.com"}),
			serde_json::json!({"issuer": 42}),
			serde_json::json!({}),
		] {
			rewrite_authorization_server_issuer(&req, &auth, &mut metadata)
				.expect("issuer should be authoritative");
			assert_eq!(
				metadata["issuer"],
				"https://gateway.example.com/example/mcp"
			);
		}
	}

	#[test]
	fn authorization_server_metadata_preserves_issuer_without_provider() {
		let req = ::http::Request::builder()
			.uri("https://gateway.example.com/.well-known/oauth-authorization-server/example/mcp")
			.body(Body::empty())
			.expect("request should build");
		let mut metadata = serde_json::json!({"issuer": "https://idp.example.com"});

		rewrite_authorization_server_issuer(&req, &default_auth(), &mut metadata)
			.expect("metadata should remain valid");
		assert_eq!(metadata["issuer"], "https://idp.example.com");
	}

	#[test]
	fn authorization_server_metadata_rejects_non_object() {
		let mut auth = default_auth();
		auth.provider = Some(McpIDP::Entra {});
		let req = ::http::Request::builder()
			.uri("https://gateway.example.com/.well-known/oauth-authorization-server/example/mcp")
			.body(Body::empty())
			.expect("request should build");
		let mut metadata = serde_json::json!([]);

		assert!(rewrite_authorization_server_issuer(&req, &auth, &mut metadata).is_err());
	}

	#[test]
	fn keycloak_metadata_rewrites_authorization_and_token_endpoints_to_gateway() {
		let current_uri: Uri = "https://gateway.example.com/.well-known/oauth-authorization-server/mcp"
			.parse()
			.expect("uri should parse");
		let mut metadata = serde_json::json!({
			"authorization_endpoint": "https://login.example.com/auth/realms/example/protocol/openid-connect/auth",
			"token_endpoint": "https://login.example.com/auth/realms/example/protocol/openid-connect/token",
			"registration_endpoint": "https://login.example.com/auth/realms/example/clients-registrations/openid-connect",
		});

		apply_keycloak_endpoint_rewrites(&current_uri, &mut metadata)
			.expect("keycloak metadata should be rewritable");

		assert_eq!(
			metadata["authorization_endpoint"],
			"https://gateway.example.com/.well-known/oauth-authorization-server/mcp/authorize"
		);
		assert_eq!(
			metadata["token_endpoint"],
			"https://gateway.example.com/.well-known/oauth-authorization-server/mcp/token"
		);
		assert_eq!(
			metadata["registration_endpoint"],
			"https://gateway.example.com/.well-known/oauth-authorization-server/mcp/client-registration"
		);
	}

	#[test]
	fn keycloak_metadata_rewrite_rejects_missing_authorization_endpoint() {
		let current_uri: Uri = "https://gateway.example.com/mcp"
			.parse()
			.expect("uri should parse");
		let mut metadata = serde_json::json!({});

		assert!(apply_keycloak_endpoint_rewrites(&current_uri, &mut metadata).is_err());
	}

	#[tokio::test]
	async fn keycloak_metadata_terminated_rewrites_authorize_and_token_endpoints() {
		// Real example deployment shape: client_id set, no client_secret.
		let mock = MockServer::start().await;
		let issuer = format!("{}/auth/realms/example", mock.uri());
		Mock::given(method("GET"))
			.and(path("/auth/realms/example/.well-known/openid-configuration"))
			.respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
				"issuer": issuer,
				"authorization_endpoint": format!("{issuer}/protocol/openid-connect/auth"),
				"token_endpoint": format!("{issuer}/protocol/openid-connect/token"),
				"registration_endpoint": format!("{issuer}/clients-registrations/openid-connect"),
				"jwks_uri": format!("{issuer}/protocol/openid-connect/certs"),
			})))
			.mount(&mock)
			.await;

		let auth = keycloak_auth_with_issuer(issuer);
		assert!(
			auth.client_secret.is_none(),
			"fixture should match the real public-client deployment shape (no client_secret)"
		);
		let client = crate::test_helpers::policy_client();
		let mut req = ::http::Request::builder()
			.uri("https://gateway.example.com/.well-known/oauth-authorization-server/mcp")
			.body(Body::empty())
			.expect("request should build");

		let resp = authorization_server_metadata(&mut req, &auth, client)
			.await
			.expect("metadata should build");
		let json = response_body_to_json(resp).await;

		assert_eq!(
			json["authorization_endpoint"],
			"https://gateway.example.com/.well-known/oauth-authorization-server/mcp/authorize"
		);
		assert_eq!(
			json["token_endpoint"],
			"https://gateway.example.com/.well-known/oauth-authorization-server/mcp/token"
		);
		assert_eq!(
			json["registration_endpoint"],
			"https://gateway.example.com/.well-known/oauth-authorization-server/mcp/client-registration"
		);
		// The gateway does terminate the flow here, so it may (and should) claim to be the
		// issuer, the mirror image of the pure-DCR case below, which must not.
		assert_eq!(json["issuer"], "https://gateway.example.com/mcp");
	}

	#[tokio::test]
	async fn keycloak_metadata_pure_dcr_keeps_real_issuer_and_endpoints() {
		// No client_id: OAuth authorization remains with Keycloak, so keep its issuer
		// and authorize/token endpoints unchanged. Registration is still proxied.
		let mock = MockServer::start().await;
		let issuer = format!("{}/auth/realms/example", mock.uri());
		Mock::given(method("GET"))
			.and(path("/auth/realms/example/.well-known/openid-configuration"))
			.respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
				"issuer": issuer,
				"authorization_endpoint": format!("{issuer}/protocol/openid-connect/auth"),
				"token_endpoint": format!("{issuer}/protocol/openid-connect/token"),
				"registration_endpoint": format!("{issuer}/clients-registrations/openid-connect"),
				"jwks_uri": format!("{issuer}/protocol/openid-connect/certs"),
			})))
			.mount(&mock)
			.await;

		let mut auth = keycloak_auth_with_issuer(issuer.clone());
		auth.client_id = None;
		auth.relay_signing_key = None;
		let client = crate::test_helpers::policy_client();
		let mut req = ::http::Request::builder()
			.uri("https://gateway.example.com/.well-known/oauth-authorization-server/mcp")
			.body(Body::empty())
			.expect("request should build");

		let resp = authorization_server_metadata(&mut req, &auth, client)
			.await
			.expect("metadata should build");
		let json = response_body_to_json(resp).await;

		assert_eq!(
			json["authorization_endpoint"],
			format!("{issuer}/protocol/openid-connect/auth")
		);
		assert_eq!(
			json["token_endpoint"],
			format!("{issuer}/protocol/openid-connect/token")
		);
		// Registration is still proxied because Keycloak does not support CORS for it.
		assert_eq!(
			json["registration_endpoint"],
			"https://gateway.example.com/.well-known/oauth-authorization-server/mcp/client-registration"
		);
		assert_eq!(
			json["issuer"], issuer,
			"pure-DCR Keycloak metadata must advertise Keycloak's real issuer, not the gateway's"
		);
	}

	#[tokio::test]
	async fn keycloak_metadata_client_secret_alone_does_not_terminate() {
		// client_secret does not control flow termination; without client_id, keep
		// Keycloak's issuer and authorize/token endpoints unchanged.
		let mock = MockServer::start().await;
		let issuer = format!("{}/auth/realms/example", mock.uri());
		Mock::given(method("GET"))
			.and(path("/auth/realms/example/.well-known/openid-configuration"))
			.respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
				"issuer": issuer,
				"authorization_endpoint": format!("{issuer}/protocol/openid-connect/auth"),
				"token_endpoint": format!("{issuer}/protocol/openid-connect/token"),
				"registration_endpoint": format!("{issuer}/clients-registrations/openid-connect"),
				"jwks_uri": format!("{issuer}/protocol/openid-connect/certs"),
			})))
			.mount(&mock)
			.await;

		let mut auth = keycloak_auth_with_issuer(issuer.clone());
		auth.client_id = None;
		auth.relay_signing_key = None;
		auth.client_secret = Some(secrecy::SecretString::new(
			"kc-client-secret".to_string().into_boxed_str(),
		));
		let client = crate::test_helpers::policy_client();
		let mut req = ::http::Request::builder()
			.uri("https://gateway.example.com/.well-known/oauth-authorization-server/mcp")
			.body(Body::empty())
			.expect("request should build");

		let resp = authorization_server_metadata(&mut req, &auth, client)
			.await
			.expect("metadata should build");
		let json = response_body_to_json(resp).await;

		assert_eq!(
			json["authorization_endpoint"],
			format!("{issuer}/protocol/openid-connect/auth")
		);
		assert_eq!(
			json["token_endpoint"],
			format!("{issuer}/protocol/openid-connect/token")
		);
		assert_eq!(
			json["registration_endpoint"],
			"https://gateway.example.com/.well-known/oauth-authorization-server/mcp/client-registration"
		);
		assert_eq!(
			json["issuer"], issuer,
			"client_secret alone (no client_id) must not make Keycloak metadata advertise the gateway as issuer"
		);
	}

	#[tokio::test]
	async fn protected_resource_metadata_pure_dcr_uses_keycloak_issuer() {
		// For pure DCR, protected-resource metadata must identify the same issuer
		// advertised by the authorization server metadata (RFC 8414 §3.3).
		let issuer = "https://login.example.com/auth/realms/example".to_string();
		let mut auth = keycloak_auth_with_issuer(issuer.clone());
		auth.client_id = None;
		auth.relay_signing_key = None;
		let mut req = ::http::Request::builder()
			.uri("https://gateway.example.com/.well-known/oauth-protected-resource/mcp")
			.body(Body::empty())
			.expect("request should build");

		let resp = protected_resource_metadata(&mut req, &auth).await;
		let json = response_body_to_json(resp).await;

		assert_eq!(
			json["authorization_servers"],
			serde_json::json!([issuer]),
			"pure-DCR Keycloak protected-resource metadata must name Keycloak as the authorization server, not the gateway"
		);
	}

	#[test]
	fn well_known_endpoint_requires_root_and_slash_delimited_suffix() {
		assert!(is_well_known_endpoint(
			"/.well-known/oauth-protected-resource"
		));
		assert!(is_well_known_endpoint(
			"/.well-known/oauth-protected-resource/mcp"
		));
		assert!(is_well_known_endpoint(
			"/.well-known/oauth-authorization-server/tenant"
		));
		assert!(!is_well_known_endpoint(
			"/mcp/.well-known/oauth-protected-resource"
		));
		assert!(!is_well_known_endpoint(
			"/.well-known/oauth-protected-resource-evil"
		));
	}

	#[test]
	fn www_authenticate_resource_metadata_preserves_authority_for_root_path() {
		let req = auth_request("https://example.com/", default_auth());

		assert_eq!(
			www_authenticate_resource_metadata(&req),
			"Bearer resource_metadata=\"https://example.com/.well-known/oauth-protected-resource/\""
		);
	}

	#[test]
	fn www_authenticate_resource_metadata_preserves_authority_when_path_matches_host_prefix() {
		let req = auth_request("https://example.com/example.com", default_auth());

		assert_eq!(
			www_authenticate_resource_metadata(&req),
			"Bearer resource_metadata=\"https://example.com/.well-known/oauth-protected-resource/example.com\""
		);
	}

	#[test]
	fn www_authenticate_resource_metadata_preserves_authority_for_non_matching_path() {
		let req = auth_request("https://example.com/sse", default_auth());

		assert_eq!(
			www_authenticate_resource_metadata(&req),
			"Bearer resource_metadata=\"https://example.com/.well-known/oauth-protected-resource/sse\""
		);
	}

	#[test]
	fn auth_required_response_accepts_configured_resource_with_path() {
		let req = auth_request(
			"http://backend.internal/mcp",
			McpAuthentication {
				issuer: "https://idp.example.com".to_string(),
				audiences: Vec::new(),
				provider: None,
				resource_metadata: crate::types::agent::ResourceMetadata {
					extra: std::collections::BTreeMap::from([(
						"resource".to_string(),
						serde_json::Value::String(
							"https://gateway.example.com/base/path?debug=true".to_string(),
						),
					)]),
				},
				jwt_validator: Arc::new(crate::http::jwt::Jwt::from_providers(
					Vec::new(),
					crate::http::jwt::Mode::Strict,
					crate::http::auth::AuthorizationLocation::default(),
					false,
				)),
				mode: crate::types::agent::McpAuthenticationMode::Strict,
				client_id: None,
				client_secret: None,
				relay_signing_key: None,
			},
		);

		assert_eq!(
			www_authenticate_resource_metadata(&req),
			"Bearer resource_metadata=\"https://gateway.example.com/.well-known/oauth-protected-resource/mcp\""
		);
	}

	fn auth_request(uri: &'static str, auth: McpAuthentication) -> Request {
		let mut req = ::http::Request::builder()
			.uri(uri)
			.body(Body::empty())
			.expect("request should build");
		req.extensions_mut().insert(auth);
		req
	}

	fn default_auth() -> McpAuthentication {
		McpAuthentication {
			issuer: "https://issuer.example.com".to_string(),
			audiences: vec!["mcp".to_string()],
			provider: None,
			resource_metadata: crate::types::agent::ResourceMetadata {
				extra: Default::default(),
			},
			jwt_validator: Arc::new(crate::http::jwt::Jwt::from_providers(
				vec![],
				crate::http::jwt::Mode::Strict,
				crate::http::auth::AuthorizationLocation::bearer_header(),
				false,
			)),
			mode: crate::types::agent::McpAuthenticationMode::Strict,
			client_id: None,
			client_secret: None,
			relay_signing_key: None,
		}
	}

	fn www_authenticate_resource_metadata(req: &Request) -> String {
		let err = create_auth_required_response(
			ProxyError::ProcessingString("test auth failure".to_string()),
			req,
			req
				.extensions()
				.get::<McpAuthentication>()
				.expect("auth should be set"),
		);

		match err {
			ProxyError::McpJwtAuthenticationFailure(_, www_authenticate) => www_authenticate,
			other => panic!("expected MCP JWT authentication failure, got {other:?}"),
		}
	}

	async fn response_body_to_json(resp: Response) -> serde_json::Value {
		let bytes = crate::http::read_resp_body(resp)
			.await
			.expect("response body should read");
		serde_json::from_slice(&bytes).expect("response body should be JSON")
	}

	fn dcr_request(body: &'static str) -> Request {
		::http::Request::builder()
			.method(Method::POST)
			.uri("https://gateway.example.com/client-registration")
			.header(::http::header::CONTENT_TYPE, "application/json")
			.body(Body::from(body))
			.expect("request should build")
	}

	#[tokio::test]
	async fn mock_dcr_echoes_redirect_uris_and_overrides_client_id() {
		let body = r#"{"redirect_uris":["http://localhost:33418/callback"],"grant_types":["authorization_code"],"client_name":"Claude Code"}"#;
		let mut req = dcr_request(body);

		let resp = build_mock_dcr_response(&mut req, "0oa1wcsu7sbWwq3Ht358")
			.await
			.expect("mock should build");

		assert_eq!(resp.status(), ::http::StatusCode::CREATED);
		let json = response_body_to_json(resp).await;
		assert_eq!(json["client_id"], "0oa1wcsu7sbWwq3Ht358");
		assert_eq!(
			json["redirect_uris"],
			serde_json::json!(["http://localhost:33418/callback"])
		);
		assert_eq!(
			json["grant_types"],
			serde_json::json!(["authorization_code"])
		);
		assert_eq!(json["response_types"], serde_json::json!(["code"]));
		assert_eq!(json["token_endpoint_auth_method"], "none");
		assert_eq!(json["client_id_issued_at"], MOCK_DCR_CLIENT_ID_ISSUED_AT);
		assert!(json.get("client_name").is_none());
	}

	#[tokio::test]
	async fn mock_dcr_overrides_client_id_if_client_submitted_one() {
		// If a client submitted its own client_id (unusual but possible),
		// we override it with the operator-configured value rather than
		// honoring what the client sent.
		let body = r#"{"redirect_uris":["http://localhost:1234/cb"],"client_id":"client-supplied-id"}"#;
		let mut req = dcr_request(body);

		let resp = build_mock_dcr_response(&mut req, "operator-id")
			.await
			.expect("mock should build");

		let json = response_body_to_json(resp).await;
		assert_eq!(json["client_id"], "operator-id");
		assert_eq!(
			json["redirect_uris"],
			serde_json::json!(["http://localhost:1234/cb"])
		);
	}

	#[tokio::test]
	async fn mock_dcr_handles_empty_body() {
		let mut req = ::http::Request::builder()
			.method(Method::POST)
			.uri("https://gateway.example.com/client-registration")
			.body(Body::empty())
			.expect("request should build");

		let resp = build_mock_dcr_response(&mut req, "operator-id")
			.await
			.expect("mock should build for empty body");

		let json = response_body_to_json(resp).await;
		assert_eq!(json["client_id"], "operator-id");
		assert_eq!(json["client_id_issued_at"], MOCK_DCR_CLIENT_ID_ISSUED_AT);
		assert_eq!(json["redirect_uris"], serde_json::json!([]));
	}

	#[tokio::test]
	async fn mock_dcr_handles_malformed_json() {
		let mut req = dcr_request("this is not json {{{");

		let resp = build_mock_dcr_response(&mut req, "operator-id")
			.await
			.expect("mock should build for invalid JSON");

		let json = response_body_to_json(resp).await;
		assert_eq!(json["client_id"], "operator-id");
		assert_eq!(json["redirect_uris"], serde_json::json!([]));
	}

	#[tokio::test]
	async fn mock_dcr_handles_non_object_body() {
		let mut req = dcr_request(r#"["not", "an", "object"]"#);

		let resp = build_mock_dcr_response(&mut req, "operator-id")
			.await
			.expect("mock should build for non-object body");

		let json = response_body_to_json(resp).await;
		assert_eq!(json["client_id"], "operator-id");
		assert!(json.is_object());
		assert_eq!(json["redirect_uris"], serde_json::json!([]));
	}

	fn entra_auth() -> McpAuthentication {
		McpAuthentication {
			issuer: "https://login.microsoftonline.com/11111111-2222-3333-4444-555555555555/v2.0"
				.to_string(),
			audiences: vec!["api://client-id-guid".to_string()],
			provider: Some(McpIDP::Entra {}),
			resource_metadata: crate::types::agent::ResourceMetadata {
				extra: Default::default(),
			},
			jwt_validator: Arc::new(crate::http::jwt::Jwt::from_providers(
				vec![],
				crate::http::jwt::Mode::Strict,
				crate::http::auth::AuthorizationLocation::bearer_header(),
				false,
			)),
			mode: crate::types::agent::McpAuthenticationMode::Strict,
			client_id: Some("client-id-guid".to_string()),
			client_secret: None,
			relay_signing_key: None,
		}
	}

	/// Returns a valid 32-byte hex-encoded relay signing key.
	/// `seed` makes keys deterministic and distinct across tests.
	fn relay_signing_key(seed: u8) -> secrecy::SecretString {
		secrecy::SecretString::new(hex::encode([seed; 32]).into_boxed_str())
	}

	/// Creates a Keycloak authentication config for a pre-registered public client
	/// (`client_id` and `relay_signing_key` set, no `client_secret`).
	fn keycloak_auth_with_issuer(issuer: String) -> McpAuthentication {
		McpAuthentication {
			issuer,
			audiences: vec!["mcp".to_string()],
			provider: Some(McpIDP::Keycloak {}),
			resource_metadata: crate::types::agent::ResourceMetadata {
				extra: Default::default(),
			},
			jwt_validator: Arc::new(crate::http::jwt::Jwt::from_providers(
				vec![],
				crate::http::jwt::Mode::Strict,
				crate::http::auth::AuthorizationLocation::bearer_header(),
				false,
			)),
			mode: crate::types::agent::McpAuthenticationMode::Strict,
			client_id: Some("mcp-gateway".to_string()),
			client_secret: None,
			relay_signing_key: Some(relay_signing_key(0x42)),
		}
	}

	fn keycloak_auth() -> McpAuthentication {
		keycloak_auth_with_issuer("https://login.example.com/auth/realms/example".to_string())
	}

	#[test]
	fn entra_authorize_strips_resource_param() {
		// Entra rejects RFC 8707 `resource` with AADSTS9010010; everything else must be preserved.
		let req = ::http::Request::builder()
			.uri("https://gateway.example.com/.well-known/oauth-authorization-server/mcp/authorize?client_id=abc&resource=https%3A%2F%2Fgateway.example.com%2Fmcp&state=xyz&code_challenge=ccc&code_challenge_method=S256")
			.body(Body::empty())
			.expect("request should build");

		let resp = entra_authorize(&req, &entra_auth()).expect("authorize should redirect");

		assert_eq!(resp.status(), StatusCode::FOUND);
		let location = resp
			.headers()
			.get(::http::header::LOCATION)
			.expect("location header")
			.to_str()
			.expect("location should be a string");
		assert!(
			location.starts_with(
				"https://login.microsoftonline.com/11111111-2222-3333-4444-555555555555/oauth2/v2.0/authorize?"
			),
			"unexpected location: {location}"
		);
		assert!(
			!location.contains("resource="),
			"unexpected location: {location}"
		);
		assert!(
			location.contains("client_id=abc"),
			"unexpected location: {location}"
		);
		assert!(
			location.contains("state=xyz"),
			"unexpected location: {location}"
		);
		assert!(
			location.contains("code_challenge_method=S256"),
			"unexpected location: {location}"
		);
	}

	#[test]
	fn entra_authorize_without_query_redirects_to_bare_endpoint() {
		let req = ::http::Request::builder()
			.uri("https://gateway.example.com/.well-known/oauth-authorization-server/mcp/authorize")
			.body(Body::empty())
			.expect("request should build");

		let resp = entra_authorize(&req, &entra_auth()).expect("authorize should redirect");

		assert_eq!(resp.status(), StatusCode::FOUND);
		assert_eq!(
			resp
				.headers()
				.get(::http::header::LOCATION)
				.expect("location header"),
			"https://login.microsoftonline.com/11111111-2222-3333-4444-555555555555/oauth2/v2.0/authorize"
		);
	}

	#[tokio::test]
	async fn entra_token_rejects_non_post_methods() {
		let client = crate::test_helpers::policy_client();
		let mut req = ::http::Request::builder()
			.method(Method::GET)
			.uri("https://gateway.example.com/.well-known/oauth-authorization-server/mcp/token")
			.body(Body::empty())
			.expect("request should build");

		let resp = entra_token(&mut req, &entra_auth(), client)
			.await
			.expect("non-POST should get a response");

		assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
		assert_eq!(
			resp.headers().get(::http::header::ALLOW).expect("allow"),
			"POST"
		);
	}

	#[test]
	fn parse_entra_token_form_removes_resource_and_detects_client_secret() {
		let parsed = parse_entra_token_form(
			b"grant_type=authorization_code&client_id=abc-123&code=abc&resource=https%3A%2F%2Fgw%2Fmcp&code_verifier=v",
		);
		assert!(!parsed.has_client_secret);
		assert_eq!(parsed.grant_type.as_deref(), Some("authorization_code"));
		assert_eq!(parsed.client_id.as_deref(), Some("abc-123"));
		assert!(!parsed.form.contains("resource"));
		assert!(parsed.form.contains("grant_type=authorization_code"));
		assert!(parsed.form.contains("code=abc"));
		assert!(parsed.form.contains("code_verifier=v"));

		let parsed = parse_entra_token_form(b"grant_type=refresh_token&client_secret=s3cret");
		assert!(parsed.has_client_secret);
		assert_eq!(parsed.grant_type.as_deref(), Some("refresh_token"));
		assert!(parsed.form.contains("client_secret=s3cret"));
	}

	#[test]
	fn entra_client_secret_only_attaches_to_user_delegated_grants() {
		assert!(entra_grant_may_use_client_secret(Some(
			"authorization_code"
		)));
		assert!(entra_grant_may_use_client_secret(Some("refresh_token")));
		// A hostile page could POST these pre-auth; the gateway must never attach its secret.
		assert!(!entra_grant_may_use_client_secret(Some(
			"client_credentials"
		)));
		assert!(!entra_grant_may_use_client_secret(Some(
			"urn:ietf:params:oauth:grant-type:jwt-bearer"
		)));
		assert!(!entra_grant_may_use_client_secret(None));
	}

	#[rstest::rstest]
	#[case::bare_metadata("https://gateway.example.com/.well-known/oauth-authorization-server/mcp")]
	#[case::authorize(
		"https://gateway.example.com/.well-known/oauth-authorization-server/mcp/authorize"
	)]
	#[case::token("https://gateway.example.com/.well-known/oauth-authorization-server/mcp/token")]
	#[case::callback(
		"https://gateway.example.com/.well-known/oauth-authorization-server/mcp/callback"
	)]
	fn keycloak_resource_uri_strips_provider_adapter_suffixes(#[case] uri: &'static str) {
		let req = ::http::Request::builder()
			.uri(uri)
			.body(Body::empty())
			.expect("request should build");

		assert_eq!(
			keycloak_resource_uri(&req)
				.expect("resource uri should resolve")
				.to_string(),
			"https://gateway.example.com/mcp"
		);
	}

	#[test]
	fn keycloak_authorize_swaps_redirect_uri_and_state_for_relay() {
		let req = ::http::Request::builder()
			.uri("https://gateway.example.com/.well-known/oauth-authorization-server/mcp/authorize?client_id=abc&redirect_uri=http%3A%2F%2F127.0.0.1%3A33418%2Fcallback&state=client-csrf&code_challenge=ccc&code_challenge_method=S256")
			.body(Body::empty())
			.expect("request should build");

		let resp = keycloak_authorize(&req, &keycloak_auth()).expect("authorize should redirect");

		assert_eq!(resp.status(), StatusCode::FOUND);
		let location = resp
			.headers()
			.get(::http::header::LOCATION)
			.expect("location header")
			.to_str()
			.expect("location should be a string");

		assert!(
			location.starts_with("https://login.example.com/auth/realms/example"),
			"unexpected location: {location}"
		);
		assert!(
			location.contains("client_id=abc"),
			"unexpected location: {location}"
		);
		assert!(
			location.contains("code_challenge_method=S256"),
			"unexpected location: {location}"
		);
		assert!(
			location.contains(
				"redirect_uri=https%3A%2F%2Fgateway.example.com%2F.well-known%2Foauth-authorization-server%2Fmcp%2Fcallback"
			),
			"expected redirect_uri swapped to gateway callback, got: {location}"
		);
		assert!(
			!location.contains("state=client-csrf"),
			"client's real state must not reach Keycloak unwrapped: {location}"
		);

		let query: std::collections::HashMap<_, _> =
			url::form_urlencoded::parse(location.split('?').nth(1).unwrap().as_bytes())
				.into_owned()
				.collect();
		let relay_token = query.get("state").expect("state param present");
		let decoded = crate::mcp::relay_state::decode(
			keycloak_auth()
				.relay_signing_key
				.as_ref()
				.expect("relay_signing_key set"),
			relay_token,
		)
		.expect("relay state should decode");
		assert_eq!(
			decoded.client_redirect_uri,
			"http://127.0.0.1:33418/callback"
		);
		assert_eq!(decoded.client_state.as_deref(), Some("client-csrf"));
	}

	#[test]
	fn keycloak_authorize_forces_response_mode_to_query() {
		// The callback relay only handles query parameters, so force Keycloak to use query mode.
		let req = ::http::Request::builder()
			.uri("https://gateway.example.com/.well-known/oauth-authorization-server/mcp/authorize?client_id=abc&redirect_uri=http%3A%2F%2F127.0.0.1%3A33418%2Fcallback&state=client-csrf&response_mode=form_post")
			.body(Body::empty())
			.expect("request should build");

		let resp = keycloak_authorize(&req, &keycloak_auth()).expect("authorize should redirect");
		let location = resp
			.headers()
			.get(::http::header::LOCATION)
			.expect("location header")
			.to_str()
			.expect("location should be a string");

		assert!(
			location.contains("response_mode=query"),
			"expected response_mode forced to query, got: {location}"
		);
		assert!(
			!location.contains("form_post"),
			"form_post must never reach keycloak: {location}"
		);
	}

	#[test]
	fn keycloak_authorize_requires_relay_signing_key() {
		// The gateway-managed authorization flow requires a relay signing key.
		let mut auth = keycloak_auth();
		auth.relay_signing_key = None;
		let req = ::http::Request::builder()
			.uri("https://gateway.example.com/.well-known/oauth-authorization-server/mcp/authorize?redirect_uri=http%3A%2F%2F127.0.0.1%2Fcb&state=s")
			.body(Body::empty())
			.expect("request should build");

		let err = keycloak_authorize(&req, &auth)
			.expect_err("missing relaySigningKey must be rejected, not silently succeed");
		// Assert on the specific guard so this doesn't pass because of an unrelated validation error.
		assert!(
			err.to_string().contains("relaySigningKey"),
			"expected the missing-relaySigningKey error, got: {err}"
		);
	}

	fn percent_encode(value: &str) -> String {
		percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC).to_string()
	}

	#[rstest::rstest]
	#[case::loopback_v4("http://127.0.0.1:33418/callback", true)]
	#[case::loopback_localhost("http://localhost:4000/cb", true)]
	#[case::loopback_v6("http://[::1]:33418/callback", true)]
	#[case::same_origin("https://gateway.example.com/somewhere", true)]
	#[case::remote_https("https://evil.example/cb", false)]
	#[case::remote_http("http://evil.example/cb", false)]
	#[case::non_http_scheme("custom-scheme://cb", false)]
	// `origin()` ignores userinfo, so `gateway.example.com@evil.example` has
	// origin `https://evil.example` and must be rejected.
	#[case::userinfo_confusion("https://gateway.example.com@evil.example/", false)]
	// WHATWG URL parsing normalizes the decimal IPv4 form to 127.0.0.1.
	#[case::decimal_ip_loopback("http://2130706433/", true)]
	// Must not accept a domain that merely starts with the loopback address.
	#[case::loopback_lookalike_domain("http://127.0.0.1.evil.example/", false)]
	fn is_allowed_client_redirect_uri_cases(#[case] uri: &str, #[case] expected: bool) {
		assert_eq!(
			is_allowed_client_redirect_uri(uri, "https://gateway.example.com"),
			expected,
			"unexpected result for {uri}"
		);
	}

	#[test]
	fn keycloak_authorize_rejects_attacker_controlled_client_redirect_uri() {
		let req = ::http::Request::builder()
			.uri("https://gateway.example.com/.well-known/oauth-authorization-server/mcp/authorize?client_id=abc&redirect_uri=https%3A%2F%2Fevil.example%2Fcb&state=client-csrf")
			.body(Body::empty())
			.expect("request should build");

		assert!(
			keycloak_authorize(&req, &keycloak_auth()).is_err(),
			"attacker-controlled redirect_uri must be rejected before it's encoded into relay state"
		);
	}

	#[test]
	fn keycloak_authorize_accepts_loopback_and_same_origin_client_redirect_uris() {
		for redirect_uri in [
			"http://127.0.0.1:33418/callback",
			"http://localhost:4000/cb",
			"https://gateway.example.com/somewhere",
		] {
			let req = ::http::Request::builder()
				.uri(format!(
					"https://gateway.example.com/.well-known/oauth-authorization-server/mcp/authorize?client_id=abc&redirect_uri={}&state=client-csrf",
					percent_encode(redirect_uri)
				))
				.body(Body::empty())
				.expect("request should build");

			assert!(
				keycloak_authorize(&req, &keycloak_auth()).is_ok(),
				"redirect_uri {redirect_uri:?} should be allowed"
			);
		}
	}

	#[test]
	fn keycloak_callback_rewrites_iss_and_restores_client_state() {
		let auth = keycloak_auth();
		let relay_token = relay_state::encode(
			auth.relay_signing_key.as_ref().unwrap(),
			&relay_state::RelayState {
				client_redirect_uri: "http://127.0.0.1:33418/callback".to_string(),
				client_state: Some("client-csrf".to_string()),
				expires_at_unix: now_unix() + 300,
			},
		)
		.expect("relay state should encode");

		let req = ::http::Request::builder()
			.uri(format!(
				"https://gateway.example.com/.well-known/oauth-authorization-server/mcp/callback?code=abc123&state={}&iss=https%3A%2F%2Flogin.example.com%2Fauth%2Frealms%2Fexample",
				percent_encode(&relay_token)
			))
			.body(Body::empty())
			.expect("request should build");

		let resp = keycloak_callback(&req, &auth).expect("callback should redirect");

		assert_eq!(resp.status(), StatusCode::FOUND);
		let location = resp
			.headers()
			.get(::http::header::LOCATION)
			.expect("location header")
			.to_str()
			.expect("location should be a string");

		assert!(
			location.starts_with("http://127.0.0.1:33418/callback?"),
			"unexpected location: {location}"
		);
		assert!(
			location.contains("code=abc123"),
			"unexpected location: {location}"
		);
		assert!(
			location.contains("state=client-csrf"),
			"unexpected location: {location}"
		);
		// The client must receive the gateway's issuer, not Keycloak's issuer.
		assert!(
			location.contains("iss=https%3A%2F%2Fgateway.example.com%2Fmcp")
				|| location.contains("iss=https://gateway.example.com/mcp"),
			"expected gateway iss, got: {location}"
		);
		assert!(
			!location.contains("login.example.com"),
			"keycloak's real issuer must not leak to the client: {location}"
		);
	}

	#[test]
	fn keycloak_callback_rejects_tampered_state() {
		let auth = keycloak_auth();
		let req = ::http::Request::builder()
			.uri("https://gateway.example.com/.well-known/oauth-authorization-server/mcp/callback?code=abc123&state=not-a-real-relay-token&iss=https%3A%2F%2Flogin.example.com%2Fauth%2Frealms%2Fexample")
			.body(Body::empty())
			.expect("request should build");

		assert!(keycloak_callback(&req, &auth).is_err());
	}

	#[test]
	fn keycloak_callback_relays_access_denied_error_to_client() {
		// OAuth authorization errors are returned to the client's redirect_uri rather than
		// treated as an internal callback failure.
		let auth = keycloak_auth();
		let relay_token = relay_state::encode(
			auth.relay_signing_key.as_ref().unwrap(),
			&relay_state::RelayState {
				client_redirect_uri: "http://127.0.0.1:33418/callback".to_string(),
				client_state: Some("client-csrf".to_string()),
				expires_at_unix: now_unix() + 300,
			},
		)
		.expect("relay state should encode");

		let req = ::http::Request::builder()
			.uri(format!(
				"https://gateway.example.com/.well-known/oauth-authorization-server/mcp/callback?error=access_denied&error_description=user+cancelled&state={}",
				percent_encode(&relay_token)
			))
			.body(Body::empty())
			.expect("request should build");

		let resp = keycloak_callback(&req, &auth).expect("callback should redirect, not 500");

		assert_eq!(resp.status(), StatusCode::FOUND);
		let location = resp
			.headers()
			.get(::http::header::LOCATION)
			.expect("location header")
			.to_str()
			.expect("location should be a string");

		assert!(
			location.starts_with("http://127.0.0.1:33418/callback?"),
			"unexpected location: {location}"
		);
		assert!(
			location.contains("error=access_denied"),
			"unexpected location: {location}"
		);
		assert!(
			location.contains("state=client-csrf"),
			"expected client's original state restored: {location}"
		);
		assert!(
			!location.contains("code="),
			"no code should be present when relaying an error: {location}"
		);
	}

	#[test]
	fn keycloak_callback_requires_code_and_state() {
		let auth = keycloak_auth();
		let req = ::http::Request::builder()
			.uri("https://gateway.example.com/.well-known/oauth-authorization-server/mcp/callback?iss=https%3A%2F%2Flogin.example.com%2Fauth%2Frealms%2Fexample")
			.body(Body::empty())
			.expect("request should build");

		assert!(keycloak_callback(&req, &auth).is_err());
	}

	#[test]
	fn keycloak_callback_requires_relay_signing_key() {
		// The gateway-managed callback requires the relay signing key.
		let mut auth = keycloak_auth();
		auth.relay_signing_key = None;
		let req = ::http::Request::builder()
			.uri("https://gateway.example.com/.well-known/oauth-authorization-server/mcp/callback?code=abc123&state=irrelevant-because-no-key&iss=https%3A%2F%2Flogin.example.com%2Fauth%2Frealms%2Fexample")
			.body(Body::empty())
			.expect("request should build");

		let err = keycloak_callback(&req, &auth)
			.expect_err("missing relaySigningKey must be rejected, not silently succeed");
		// Assert on the specific guard so this doesn't pass because of an unrelated decode error.
		assert!(
			err.to_string().contains("relaySigningKey"),
			"expected the missing-relaySigningKey error, got: {err}"
		);
	}

	#[test]
	fn rewrite_keycloak_token_form_replaces_redirect_uri() {
		let body =
			b"grant_type=authorization_code&client_id=mcp-gateway&code=abc123&redirect_uri=http%3A%2F%2F127.0.0.1%3A33418%2Fcallback&code_verifier=v";

		let rewritten = rewrite_keycloak_token_form(
			body,
			"https://gateway.example.com/.well-known/oauth-authorization-server/mcp/callback",
		);

		assert!(rewritten.contains("grant_type=authorization_code"));
		assert!(rewritten.contains("client_id=mcp-gateway"));
		assert!(rewritten.contains("code=abc123"));
		assert!(rewritten.contains("code_verifier=v"));
		assert!(
			rewritten.contains(
				"redirect_uri=https%3A%2F%2Fgateway.example.com%2F.well-known%2Foauth-authorization-server%2Fmcp%2Fcallback"
			),
			"expected redirect_uri rewritten to gateway callback, got: {rewritten}"
		);
		assert!(
			!rewritten.contains("127.0.0.1"),
			"client's original redirect_uri must not survive: {rewritten}"
		);
	}

	#[test]
	fn rewrite_keycloak_token_form_handles_missing_redirect_uri() {
		let body = b"grant_type=refresh_token&refresh_token=r1";

		let rewritten = rewrite_keycloak_token_form(
			body,
			"https://gateway.example.com/.well-known/oauth-authorization-server/mcp/callback",
		);

		// Refresh-token requests have no redirect_uri to rewrite.
		assert!(rewritten.contains("grant_type=refresh_token"));
		assert!(!rewritten.contains("redirect_uri"));
	}

	#[tokio::test]
	async fn keycloak_token_rejects_non_post_methods() {
		let client = crate::test_helpers::policy_client();
		let mut req = ::http::Request::builder()
			.method(Method::GET)
			.uri("https://gateway.example.com/.well-known/oauth-authorization-server/mcp/token")
			.body(Body::empty())
			.expect("request should build");

		let resp = keycloak_token(&mut req, &keycloak_auth(), client)
			.await
			.expect("non-POST should get a response");

		assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
		assert_eq!(
			resp.headers().get(::http::header::ALLOW).expect("allow"),
			"POST"
		);
	}

	#[tokio::test]
	async fn handle_mcp_request_routes_keycloak_authorize_and_token() {
		let client = crate::test_helpers::policy_client();
		let auth = keycloak_auth();

		let mut authorize_req = ::http::Request::builder()
			.uri("https://gateway.example.com/.well-known/oauth-authorization-server/mcp/authorize?redirect_uri=http%3A%2F%2F127.0.0.1%2Fcb&state=s")
			.body(Body::empty())
			.expect("request should build");
		let resp = handle_mcp_request(&mut authorize_req, &auth, &client)
			.await
			.expect("should not error")
			.expect("should be handled");
		assert_eq!(resp.status(), StatusCode::FOUND);

		let mut token_req = ::http::Request::builder()
			.method(Method::GET)
			.uri("https://gateway.example.com/.well-known/oauth-authorization-server/mcp/token")
			.body(Body::empty())
			.expect("request should build");
		let resp = handle_mcp_request(&mut token_req, &auth, &client)
			.await
			.expect("should not error")
			.expect("should be handled");
		assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
	}

	#[test]
	fn keycloak_authorize_and_keycloak_token_use_same_callback_uri() {
		// The redirect_uri used by /authorize must match the one sent to Keycloak's
		// token endpoint, otherwise Keycloak rejects the token exchange.
		let authorize_req = ::http::Request::builder()
			.uri("https://gateway.example.com/.well-known/oauth-authorization-server/mcp/authorize?client_id=abc&redirect_uri=http%3A%2F%2F127.0.0.1%3A33418%2Fcallback&state=client-csrf")
			.body(Body::empty())
			.expect("request should build");

		let resp =
			keycloak_authorize(&authorize_req, &keycloak_auth()).expect("authorize should redirect");
		let location = resp
			.headers()
			.get(::http::header::LOCATION)
			.expect("location header")
			.to_str()
			.expect("location should be a string");
		let query: std::collections::HashMap<String, String> =
			url::form_urlencoded::parse(location.split('?').nth(1).unwrap().as_bytes())
				.into_owned()
				.collect();
		let authorize_callback = query
			.get("redirect_uri")
			.expect("authorize response should carry redirect_uri")
			.clone();

		let token_req = ::http::Request::builder()
			.uri("https://gateway.example.com/.well-known/oauth-authorization-server/mcp/token")
			.body(Body::empty())
			.expect("request should build");
		let token_callback = keycloak_callback_uri(&token_req).expect("callback uri should resolve");

		assert_eq!(authorize_callback, token_callback);
		assert_eq!(
			authorize_callback,
			"https://gateway.example.com/.well-known/oauth-authorization-server/mcp/callback"
		);
	}

	async fn keycloak_token_echo(
		mock: &MockServer,
		auth: &McpAuthentication,
		req: &mut Request,
	) -> String {
		Mock::given(method("POST"))
			.and(path("/auth/realms/example/protocol/openid-connect/token"))
			.respond_with(|request: &wiremock::Request| {
				ResponseTemplate::new(200).set_body_bytes(request.body.clone())
			})
			.mount(mock)
			.await;

		let client = crate::test_helpers::policy_client();
		let resp = keycloak_token(req, auth, client)
			.await
			.expect("token exchange should succeed");
		let bytes = crate::http::read_resp_body(resp)
			.await
			.expect("response body should read");
		String::from_utf8(bytes.to_vec()).expect("echoed body should be utf8")
	}

	fn keycloak_auth_for_mock(mock: &MockServer) -> McpAuthentication {
		let issuer = format!("{}/auth/realms/example", mock.uri());
		let mut auth = keycloak_auth_with_issuer(issuer);
		auth.client_secret = Some(secrecy::SecretString::new(
			"keycloak-client-secret".to_string().into_boxed_str(),
		));
		auth
	}

	fn keycloak_token_request(body: &'static str) -> Request {
		::http::Request::builder()
			.method(Method::POST)
			.uri("https://gateway.example.com/.well-known/oauth-authorization-server/mcp/token")
			.header(
				::http::header::CONTENT_TYPE,
				"application/x-www-form-urlencoded",
			)
			.body(Body::from(body))
			.expect("request should build")
	}

	#[tokio::test]
	async fn keycloak_token_injects_client_secret_for_matching_authorization_code_grant() {
		let mock = MockServer::start().await;
		let auth = keycloak_auth_for_mock(&mock);
		let mut req = keycloak_token_request(
			"grant_type=authorization_code&client_id=mcp-gateway&code=abc123&redirect_uri=http%3A%2F%2F127.0.0.1%3A33418%2Fcallback&code_verifier=v",
		);

		let body = keycloak_token_echo(&mock, &auth, &mut req).await;

		assert!(
			body.contains("client_secret=keycloak-client-secret"),
			"expected configured secret injected, got: {body}"
		);
	}

	#[tokio::test]
	async fn keycloak_token_does_not_inject_client_secret_for_mismatched_client_id() {
		let mock = MockServer::start().await;
		let auth = keycloak_auth_for_mock(&mock);
		let mut req = keycloak_token_request(
			"grant_type=authorization_code&client_id=some-other-client&code=abc123",
		);

		let body = keycloak_token_echo(&mock, &auth, &mut req).await;

		assert!(
			!body.contains("client_secret="),
			"must not attach the gateway's secret to a request for a different client_id: {body}"
		);
	}

	#[tokio::test]
	async fn keycloak_token_does_not_inject_client_secret_for_client_credentials_grant() {
		let mock = MockServer::start().await;
		let auth = keycloak_auth_for_mock(&mock);
		let mut req = keycloak_token_request("grant_type=client_credentials&client_id=mcp-gateway");

		let body = keycloak_token_echo(&mock, &auth, &mut req).await;

		assert!(
			!body.contains("client_secret="),
			"pre-auth client_credentials must never get the gateway's secret attached: {body}"
		);
	}

	#[tokio::test]
	async fn keycloak_token_does_not_overwrite_existing_client_secret() {
		let mock = MockServer::start().await;
		let auth = keycloak_auth_for_mock(&mock);
		let mut req = keycloak_token_request(
			"grant_type=authorization_code&client_id=mcp-gateway&code=abc123&client_secret=caller-supplied-secret",
		);

		let body = keycloak_token_echo(&mock, &auth, &mut req).await;

		assert!(
			body.contains("client_secret=caller-supplied-secret"),
			"caller-supplied client_secret must survive: {body}"
		);
		assert!(
			!body.contains("keycloak-client-secret"),
			"must not attach a second, configured secret on top of the caller's own: {body}"
		);
	}

	#[tokio::test]
	async fn keycloak_token_does_not_inject_client_secret_when_grant_type_duplicated() {
		// Duplicate security-sensitive fields make the value used by the injection gate
		// potentially differ from the value Keycloak uses. Fail closed by skipping injection.
		let mock = MockServer::start().await;
		let auth = keycloak_auth_for_mock(&mock);
		let mut req = keycloak_token_request(
			"grant_type=client_credentials&grant_type=authorization_code&client_id=mcp-gateway&code=abc123",
		);

		let body = keycloak_token_echo(&mock, &auth, &mut req).await;

		assert!(
			!body.contains("client_secret="),
			"duplicate grant_type must disable secret injection even though one of the values looks like authorization_code: {body}"
		);
	}

	#[tokio::test]
	async fn keycloak_token_does_not_inject_client_secret_when_client_id_duplicated() {
		let mock = MockServer::start().await;
		let auth = keycloak_auth_for_mock(&mock);
		let mut req = keycloak_token_request(
			"grant_type=authorization_code&client_id=mcp-gateway&client_id=some-other-client&code=abc123",
		);

		let body = keycloak_token_echo(&mock, &auth, &mut req).await;

		assert!(
			!body.contains("client_secret="),
			"duplicate client_id must disable secret injection: {body}"
		);
	}

	#[tokio::test]
	async fn keycloak_token_does_not_inject_client_secret_when_authorization_header_present() {
		let mock = MockServer::start().await;
		let auth = keycloak_auth_for_mock(&mock);
		let mut req =
			keycloak_token_request("grant_type=authorization_code&client_id=mcp-gateway&code=abc123");
		req.headers_mut().insert(
			::http::header::AUTHORIZATION,
			"Basic bWNwLWdhdGV3YXk6c2VjcmV0".parse().unwrap(),
		);

		let body = keycloak_token_echo(&mock, &auth, &mut req).await;

		assert!(
			!body.contains("client_secret="),
			"must not attach a client_secret on top of an existing Authorization header: {body}"
		);
	}
}
