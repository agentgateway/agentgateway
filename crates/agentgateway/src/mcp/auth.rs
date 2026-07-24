use axum::http::StatusCode;
use axum::response::Response;
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
				.into_response(),
		)),
		path
			if path == "/.well-known/oauth-protected-resource"
				|| path.starts_with("/.well-known/oauth-protected-resource/") =>
		{
			Ok(Some(
				protected_resource_metadata(req, auth).await.into_response(),
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
					.into_response(),
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
					.into_response(),
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
					.into_response(),
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
		.body(Body::empty())?;
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
			let Some(serde_json::Value::String(re)) =
				json::traverse_mut(&mut resp, &["registration_endpoint"])
			else {
				return Err(ProxyError::ProcessingString(
					"registration_endpoint missing".to_string(),
				));
			};
			*re = format!("{current_uri}/client-registration");
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
	if let Some(client_id) = &auth.client_id {
		return build_mock_dcr_response(req, client_id).await;
	}

	// Normalize issuer URL by removing trailing slashes to avoid double-slash in path
	let issuer = auth.issuer.trim_end_matches('/');
	let body = std::mem::take(req.body_mut());
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
		.body(body)?;

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
		Response::builder()
			.status(StatusCode::FOUND)
			.header(::http::header::LOCATION, location.to_string())
			.body(axum::body::Body::empty())?,
	)
}

/// Proxy an OAuth token request to Entra, stripping the RFC 8707 `resource` parameter
/// (see [`entra_authorize`]) and injecting the configured client secret when the client did
/// not supply one. Entra app registrations under the Web platform are confidential clients
/// and require the secret at the token endpoint, while public clients (PKCE-only) do not.
///
/// The secret is only attached when the request is for the configured `clientId` (the app
/// registration the secret belongs to) and uses a user-delegated grant (`authorization_code`,
/// `refresh_token`). This endpoint is reachable pre-authentication, so injecting the secret
/// into other grant types — notably `client_credentials` — would let any caller mint
/// app-level tokens with the gateway's credential.
pub(super) async fn entra_token(
	req: &mut Request,
	auth: &McpAuthentication,
	client: PolicyClient,
) -> Result<Response, ProxyError> {
	// CORS (including preflight) is the responsibility of the route's cors policy.
	if req.method() != Method::POST {
		return Ok(
			Response::builder()
				.status(StatusCode::METHOD_NOT_ALLOWED)
				.header(::http::header::ALLOW, "POST")
				.body(axum::body::Body::empty())?,
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
		Response::builder()
			.status(::http::StatusCode::CREATED)
			.header(::http::header::CONTENT_TYPE, "application/json")
			.body(body_bytes.into())?,
	)
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

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
				)),
				mode: crate::types::agent::McpAuthenticationMode::Strict,
				client_id: None,
				client_secret: None,
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
			)),
			mode: crate::types::agent::McpAuthenticationMode::Strict,
			client_id: None,
			client_secret: None,
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
			)),
			mode: crate::types::agent::McpAuthenticationMode::Strict,
			client_id: Some("client-id-guid".to_string()),
			client_secret: None,
		}
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
}
