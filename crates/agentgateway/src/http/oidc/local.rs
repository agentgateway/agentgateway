use std::sync::Arc;

use jsonwebtoken::jwk::JwkSet;
use secrecy::SecretString;

use super::{
	ClientConfig, CookieSecureMode, Error, OidcPolicy, PolicyId, Provider, ProviderEndpoint,
	RedirectUri, SameSiteMode, SessionConfig, dedupe_scopes, session,
};
use crate::http::oauth::{
	TokenEndpointAuth, openid_configuration_metadata_url, parse_token_endpoint_auth_methods,
};
use crate::serdes::FileInlineOrRemote;
use crate::{apply, schema, schema_de};

#[derive(Debug, serde::Deserialize)]
struct OidcDiscoveryDocument {
	issuer: String,
	authorization_endpoint: String,
	token_endpoint: String,
	jwks_uri: String,
	#[serde(default)]
	token_endpoint_auth_methods_supported: Option<Vec<String>>,
}

struct PreparedOidcProvider {
	issuer: String,
	authorization_endpoint: ProviderEndpoint,
	token_endpoint: ProviderEndpoint,
	token_endpoint_auth: TokenEndpointAuth,
	id_token_jwks: JwkSet,
}

struct PreparedOidcPolicy {
	provider: PreparedOidcProvider,
	client_id: String,
	client_secret: SecretString,
	redirect_uri: RedirectUri,
	scopes: Vec<String>,
	login: Option<OidcLogin>,
	logout: Option<OidcLogout>,
	outbound_tunnel: Option<Arc<crate::client::TunnelSpec>>,
}

/// Optional browser login entry point and unauthenticated redirect destination.
#[apply(schema!)]
pub struct OidcLogin {
	/// Local endpoint that starts OAuth, for example `/auth/login`. Point your sign-in
	/// link here, optionally with `?returnTo=/app` to choose the destination after login.
	/// `returnTo` must be a safe local path and defaults to `/`.
	/// This endpoint is handled by the policy, not forwarded to your application.
	pub path: String,
	/// Local page to redirect unauthenticated browser navigations to, for example
	/// `/login`. This happens BEFORE authentication; it is not the OAuth callback or
	/// the destination after successful login. The gateway appends `returnTo` so your
	/// page can preserve it in its sign-in link to `login.path`.
	/// Serve this page and its assets on routes that bypass authentication, using a
	/// conditional policy or separate routes. This setting does not make them public.
	/// Fetch requests receive 401 with this destination in the Location header.
	/// If omitted, unauthenticated navigations start OAuth directly.
	#[serde(default)]
	pub redirect: Option<String>,
}

/// Optional local-session logout endpoint and destination.
#[apply(schema!)]
pub struct OidcLogout {
	/// Local endpoint that clears this policy's session and login transaction cookies,
	/// for example `/auth/logout`. Submit a POST from the callback URI's origin;
	/// requests without a matching Origin header are rejected. The policy handles
	/// this endpoint even when there is no valid session. This does not log out of
	/// the identity provider or revoke tokens.
	pub path: String,
	/// Local destination for the 303 redirect AFTER logout, for example `/signed-out`.
	/// Defaults to `login.redirect` if configured, otherwise `/`. Make the destination
	/// public through routing or a conditional policy; a protected destination can
	/// immediately start another OAuth login using the existing identity-provider session.
	#[serde(default)]
	pub redirect: Option<String>,
}

/// Browser-based OIDC authentication policy.
///
/// Explicit mode is still OIDC: it supplies provider metadata manually instead of using discovery.
/// Unauthenticated document navigations redirect to the provider login flow. Browser requests
/// positively identified as non-navigation requests return 401 so the caller can initiate a
/// document navigation.
#[apply(schema_de!)]
pub struct LocalOidcConfig {
	/// Issuer used for discovery and ID token validation.
	pub issuer: String,

	/// Optional discovery document override. If omitted, discovery uses
	/// `${issuer}/.well-known/openid-configuration`.
	#[serde(default)]
	pub discovery: Option<FileInlineOrRemote>,

	/// Authorization endpoint used to start the browser login flow.
	#[serde(default)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub authorization_endpoint: Option<ProviderEndpoint>,

	/// Token endpoint used to exchange the authorization code.
	#[serde(default)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub token_endpoint: Option<ProviderEndpoint>,

	/// Token endpoint client authentication method for explicit provider configuration.
	///
	/// Discovery mode derives this from provider metadata. Explicit mode defaults to
	/// `clientSecretBasic` when omitted.
	#[serde(default)]
	pub token_endpoint_auth: Option<TokenEndpointAuth>,

	/// JWKS source used to validate returned ID tokens.
	#[serde(default)]
	pub jwks: Option<FileInlineOrRemote>,

	/// OAuth2 client identifier used for authorization and token exchange.
	pub client_id: String,

	/// OAuth2 client secret used for token exchange.
	#[serde(serialize_with = "crate::serdes::ser_redact")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	pub client_secret: SecretString,

	/// Absolute callback URI handled by the gateway.
	/// Unauthenticated document navigations are redirected back through this login flow.
	#[serde(rename = "redirectURI")]
	pub redirect_uri: String,

	/// Additional OAuth2 scopes to request. `openid` is always included.
	#[serde(default)]
	pub scopes: Vec<String>,

	/// Optional explicit login endpoint and pre-login redirect. Omit for automatic OAuth login.
	#[serde(default)]
	pub login: Option<OidcLogin>,

	/// Optional logout endpoint. Independent of login; omit to disable the logout endpoint.
	#[serde(default)]
	pub logout: Option<OidcLogout>,

	/// Optional outbound proxy backend to tunnel this policy's own egress through
	/// (OIDC discovery, JWKS, and token exchange). Mirrors `backendTunnel` on LLM
	/// providers. Use when the identity provider is only reachable through a forward
	/// proxy (e.g. a corp egress proxy) that blocks direct outbound HTTPS. Set the
	/// proxy inline via `{host, port}` so no separate named backend is required.
	#[serde(rename = "backendTunnel", default)]
	pub backend_tunnel: Option<crate::types::backend::Tunnel>,
}

struct DiscoveredProviderMetadata {
	authorization_endpoint: ProviderEndpoint,
	token_endpoint: ProviderEndpoint,
	token_endpoint_auth: TokenEndpointAuth,
	jwks: FileInlineOrRemote,
}

impl LocalOidcConfig {
	pub(crate) async fn compile(
		self,
		resources: &crate::resource_manager::ResourceFetcher,
		policy_id: PolicyId,
		oidc_cookie_encoder: &crate::http::sessionpersistence::Encoder,
	) -> Result<OidcPolicy, Error> {
		self
			.resolve(resources)
			.await?
			.compile(policy_id, oidc_cookie_encoder)
	}

	async fn resolve(
		self,
		resources: &crate::resource_manager::ResourceFetcher,
	) -> Result<PreparedOidcPolicy, Error> {
		let LocalOidcConfig {
			issuer,
			discovery,
			authorization_endpoint,
			token_endpoint,
			token_endpoint_auth,
			jwks,
			client_id,
			client_secret,
			redirect_uri,
			scopes,
			login,
			logout,
			backend_tunnel,
		} = self;
		let redirect_uri = RedirectUri::parse(redirect_uri)?;
		let mut endpoints = vec![redirect_uri.callback_path.as_str()];
		let mut login_destination = None;
		for (name, endpoint, destination) in [
			(
				"login",
				login.as_ref().map(|v| &v.path),
				login.as_ref().and_then(|v| v.redirect.as_ref()),
			),
			(
				"logout",
				logout.as_ref().map(|v| &v.path),
				logout.as_ref().and_then(|v| v.redirect.as_ref()),
			),
		] {
			if let Some(endpoint) = endpoint {
				let parsed = endpoint.parse::<http::uri::PathAndQuery>().ok();
				if session::normalize_original_uri(parsed.as_ref()) != *endpoint
					|| parsed.as_ref().is_none_or(|v| v.query().is_some())
					|| endpoint.contains('#')
					|| endpoints.contains(&endpoint.as_str())
				{
					return Err(Error::Config(format!(
						"{name}.path must be a distinct local path without a query"
					)));
				}
				endpoints.push(endpoint);
			}
			if let Some(destination) = destination {
				let parsed = destination.parse::<http::uri::PathAndQuery>().ok();
				if session::normalize_original_uri(parsed.as_ref()) != *destination
					|| destination.contains('#')
				{
					return Err(Error::Config(format!(
						"{name}.redirect must be a safe local path"
					)));
				}
				if name == "login" {
					login_destination = parsed;
				}
			}
		}
		// All endpoints are now known, including logout, which is validated after login.
		if let Some(path) = login_destination
			&& endpoints.contains(&path.path())
		{
			return Err(Error::Config(
				"login.redirect must differ from the login, logout, and callback paths".into(),
			));
		}
		let explicit_field_count = usize::from(authorization_endpoint.is_some())
			+ usize::from(token_endpoint.is_some())
			+ usize::from(jwks.is_some());
		if token_endpoint_auth.is_some() && explicit_field_count != 3 {
			return Err(Error::Config(
				"tokenEndpointAuth must be omitted unless authorizationEndpoint, tokenEndpoint, and jwks are configured explicitly".into(),
			));
		}
		let outbound_tunnel = build_oidc_tunnel(backend_tunnel)?;
		// When tunneling is configured, discovery and JWKS fetches must also go
		// through the proxy. Derive a dedicated tunneled fetcher and reuse it for
		// the rest of this resolve call.
		let tunneled_resources;
		let resources = match outbound_tunnel {
			Some(ref tunnel) => {
				tunneled_resources = resources.with_outbound_tunnel((**tunnel).clone());
				&tunneled_resources
			},
			None => resources,
		};
		let provider = match explicit_field_count {
			0 => {
				let discovery = match discovery {
					Some(discovery) => discovery,
					None => FileInlineOrRemote::Remote {
						url: default_discovery_url(&issuer)?,
					},
				};
				let discovered = discover_provider_metadata(resources, &issuer, discovery).await?;
				let id_token_jwks = load_jwks(resources, discovered.jwks, "discovered jwks source").await?;

				PreparedOidcProvider {
					issuer,
					authorization_endpoint: discovered.authorization_endpoint,
					token_endpoint: discovered.token_endpoint,
					token_endpoint_auth: discovered.token_endpoint_auth,
					id_token_jwks,
				}
			},
			3 => {
				if discovery.is_some() {
					return Err(Error::Config(
						"oidc discovery must be omitted when authorizationEndpoint, tokenEndpoint, and jwks are configured explicitly".into(),
					));
				}
				resolve_explicit_provider(
					resources,
					issuer,
					authorization_endpoint.expect("checked above"),
					token_endpoint.expect("checked above"),
					token_endpoint_auth.unwrap_or_default(),
					jwks.expect("checked above"),
				)
				.await?
			},
			_ => {
				return Err(Error::Config(
					"authorizationEndpoint, tokenEndpoint, and jwks must either all be set or all be omitted"
						.into(),
				));
			},
		};

		Ok(PreparedOidcPolicy {
			provider,
			client_id,
			client_secret,
			redirect_uri,
			scopes,
			login,
			logout,
			outbound_tunnel,
		})
	}
}

/// Resolve the OIDC policy's optional `backendTunnel` into an outbound proxy hop.
/// Only the self-contained inline `{host, port}` proxy form is supported here:
/// the compile-time OIDC fetch path has no access to the named-backend store, so
/// named/service references cannot be resolved. The proxy is reached with a plain
/// HTTP CONNECT (no TLS to the proxy); TLS to the origin is unchanged.
fn build_oidc_tunnel(
	tunnel: Option<crate::types::backend::Tunnel>,
) -> Result<Option<Arc<crate::client::TunnelSpec>>, Error> {
	let Some(tunnel) = tunnel else {
		return Ok(None);
	};
	use crate::types::agent::{BackendTrafficPolicy, SimpleBackendReference, Target};
	let (host, port) = match tunnel.proxy.as_ref() {
		SimpleBackendReference::InlineBackend(Target::Hostname(host, port)) => {
			(host.to_string(), *port)
		},
		SimpleBackendReference::InlineBackend(Target::Address(addr)) => {
			(addr.ip().to_string(), addr.port())
		},
		SimpleBackendReference::Invalid => {
			return Err(Error::Config("invalid backendTunnel.proxy".into()));
		},
		SimpleBackendReference::InlineBackend(Target::UnixSocket(_)) => {
			return Err(Error::Config(
				"backendTunnel.proxy.host must be a host:port, not a unix socket".into(),
			));
		},
		other => {
			return Err(Error::Config(format!(
				"ui.policies.oidc.backendTunnel.proxy must be an inline {{host, port}} (named/service refs aren't resolvable in this path), got {other:?}"
			)));
		},
	};
	let target = Target::from((host.as_str(), port));
	let connection = crate::client::ConnectionConfig {
		transport: crate::client::Transport::Plain(crate::client::ApplicationTransport::Plaintext),
		tcp: tunnel.policies.iter().find_map(|p| match p {
			BackendTrafficPolicy::TCP(t) => Some(t.clone()),
			_ => None,
		}),
		max_connection_duration: None,
	};
	// Follow the backend tunnel pattern (build_transport): an optional
	// `backendAuth` on the proxy backend becomes the CONNECT `Proxy-Authorization`
	// token, so an egress proxy that requires auth can be used.
	let token = tunnel
		.policies
		.iter()
		.find_map(|p| match p {
			BackendTrafficPolicy::BackendAuth(auth) => Some(auth),
			_ => None,
		})
		.map(|auth| {
			crate::http::auth::apply_tunnel_auth(auth)
				.map_err(|e| Error::Config(format!("backendTunnel proxy backendAuth: {e}")))
		})
		.transpose()?;
	// Only `TCP` (connection tuning) and `backendAuth` (proxy auth) are honored
	// for the OIDC tunnel. TLS-to-the-proxy, HTTP transforms, authz, etc. are
	// not representable in this plain-CONNECT path, so reject them up front
	// rather than silently ignoring a policy the user configured.
	for policy in &tunnel.policies {
		let supported = matches!(
			policy,
			BackendTrafficPolicy::TCP(_) | BackendTrafficPolicy::BackendAuth(_)
		);
		if !supported {
			return Err(Error::Config(format!(
				"ui.policies.oidc.backendTunnel does not support policy {policy:?}; only `tcp` and `backendAuth` are honored on the OIDC tunnel"
			)));
		}
	}
	Ok(Some(Arc::new(crate::client::TunnelSpec {
		target,
		connection,
		connect: tunnel.mode == crate::types::backend::TunnelMode::Connect,
		connect_headers: Vec::new(),
		token,
	})))
}

async fn discover_provider_metadata(
	resources: &crate::resource_manager::ResourceFetcher,
	issuer: &str,
	discovery: FileInlineOrRemote,
) -> Result<DiscoveredProviderMetadata, Error> {
	let document = discovery
		.load::<OidcDiscoveryDocument>(
			resources,
			crate::resource_manager::ResourceKind::OidcDiscovery,
		)
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
		parse_token_endpoint_auth_methods(document.token_endpoint_auth_methods_supported)
			.map_err(Error::Config)?;
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
			.map_err(|e| Error::Config(format!("invalid authorization endpoint: {e}")))?,
		token_endpoint: document
			.token_endpoint
			.parse()
			.map_err(|e| Error::Config(format!("invalid token endpoint: {e}")))?,
		token_endpoint_auth,
		jwks,
	})
}

async fn resolve_explicit_provider(
	resources: &crate::resource_manager::ResourceFetcher,
	issuer: String,
	authorization_endpoint: ProviderEndpoint,
	token_endpoint: ProviderEndpoint,
	token_endpoint_auth: TokenEndpointAuth,
	jwks: FileInlineOrRemote,
) -> Result<PreparedOidcProvider, Error> {
	let id_token_jwks = load_jwks(resources, jwks, "explicit jwks source").await?;

	Ok(PreparedOidcProvider {
		issuer,
		authorization_endpoint,
		token_endpoint,
		token_endpoint_auth,
		id_token_jwks,
	})
}

fn default_discovery_url(issuer: &str) -> Result<http::Uri, Error> {
	openid_configuration_metadata_url(issuer)
		.parse()
		.map_err(|e| {
			Error::Config(format!(
				"invalid discovery uri derived from issuer '{issuer}': {e}"
			))
		})
}

async fn load_jwks(
	resources: &crate::resource_manager::ResourceFetcher,
	jwks: FileInlineOrRemote,
	source: &'static str,
) -> Result<JwkSet, Error> {
	let jwks = jwks
		.load::<JwkSet>(resources, crate::resource_manager::ResourceKind::Jwks)
		.await
		.map_err(|e| {
			Error::Config(format!(
				"failed to load oidc jwks from {} {}: {e}",
				source,
				describe_file_inline_or_remote(&jwks)
			))
		})?;
	Ok(jwks)
}

impl PreparedOidcProvider {
	fn compile(self, client_id: String) -> Result<Provider, Error> {
		let provider = crate::http::jwt::Provider::from_jwks(
			self.id_token_jwks,
			self.issuer.clone(),
			Some(vec![client_id]),
			crate::http::jwt::JWTValidationOptions::default(),
		)
		.map_err(|e| Error::Config(format!("failed to create id token validator: {e}")))?;

		Ok(Provider {
			issuer: self.issuer,
			authorization_endpoint: self.authorization_endpoint,
			token_endpoint: self.token_endpoint,
			id_token_validator: crate::http::jwt::Jwt::from_providers(
				vec![provider],
				crate::http::jwt::Mode::Strict,
				crate::http::auth::AuthorizationLocation::bearer_header(),
				false,
			),
		})
	}
}

impl PreparedOidcPolicy {
	fn compile(
		self,
		policy_id: PolicyId,
		oidc_cookie_encoder: &crate::http::sessionpersistence::Encoder,
	) -> Result<OidcPolicy, Error> {
		let (cookie_name, transaction_cookie_prefix) = session::derive_cookie_names(&policy_id);
		let PreparedOidcPolicy {
			provider,
			client_id,
			client_secret,
			redirect_uri,
			scopes,
			login,
			logout,
			outbound_tunnel,
		} = self;
		let scopes = dedupe_scopes(scopes);
		let token_endpoint_auth = provider.token_endpoint_auth;
		let provider = Arc::new(provider.compile(client_id.clone())?);

		Ok(OidcPolicy {
			policy_id,
			provider,
			client: ClientConfig {
				client_id,
				client_secret,
				token_endpoint_auth,
			},
			redirect_uri,
			session: SessionConfig {
				cookie_name,
				transaction_cookie_prefix,
				same_site: SameSiteMode::Lax,
				secure: CookieSecureMode::Auto,
				ttl: session::default_session_ttl(),
				transaction_ttl: session::default_transaction_ttl(),
				encoder: oidc_cookie_encoder.clone(),
			},
			scopes,
			login,
			logout,
			outbound_tunnel,
		})
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
	use super::*;
	use crate::types::agent::{BackendTrafficPolicy, Target};

	fn inline_tunnel(
		host: &str,
		port: u16,
		mode: crate::types::backend::TunnelMode,
	) -> crate::types::backend::Tunnel {
		crate::types::backend::Tunnel {
			proxy: Arc::new(crate::types::agent::SimpleBackendReference::InlineBackend(
				crate::types::agent::Target::from((host, port)),
			)),
			mode,
			policies: Vec::new(),
		}
	}

	#[test]
	fn no_tunnel_returns_none() {
		assert!(build_oidc_tunnel(None).unwrap().is_none());
	}

	#[test]
	fn inline_hostname_proxy_resolves_target() {
		let spec = build_oidc_tunnel(Some(inline_tunnel(
			"genproxy.corp.example.com",
			8080,
			crate::types::backend::TunnelMode::Connect,
		)))
		.unwrap()
		.unwrap();
		let Target::Hostname(host, port) = &spec.target else {
			panic!("expected hostname target");
		};
		assert_eq!(host.as_str(), "genproxy.corp.example.com");
		assert_eq!(*port, 8080);
		assert!(spec.connect);
		// Proxy hop is a plain (non-TLS) transport: a forward proxy spoken to over
		// HTTP CONNECT.
		assert!(matches!(
			spec.connection.transport,
			crate::client::Transport::Plain(crate::client::ApplicationTransport::Plaintext)
		));
	}

	#[test]
	fn inline_ip_proxy_resolves_address() {
		let spec = build_oidc_tunnel(Some(inline_tunnel(
			"192.168.1.169",
			3128,
			crate::types::backend::TunnelMode::Auto,
		)))
		.unwrap()
		.unwrap();
		assert!(matches!(spec.target, Target::Address(_)));
		assert!(!spec.connect);
	}

	#[test]
	fn named_backend_reference_is_rejected() {
		let tunnel = crate::types::backend::Tunnel {
			proxy: Arc::new(crate::types::agent::SimpleBackendReference::Backend(
				"ns/backend".into(),
			)),
			mode: crate::types::backend::TunnelMode::Connect,
			policies: Vec::new(),
		};
		assert!(build_oidc_tunnel(Some(tunnel)).is_err());
	}

	#[test]
	fn deserializes_backend_tunnel_from_json() {
		let raw = serde_json::json!({
			"issuer": "https://issuer.example.com",
			"clientId": "id",
			"clientSecret": "secret",
			"redirectURI": "http://localhost:4000/oauth/callback",
			"backendTunnel": {
				"proxy": { "host": "genproxy.corp.example.com:8080" },
				"mode": "connect"
			}
		});
		let cfg: LocalOidcConfig = serde_json::from_value(raw).expect("parse LocalOidcConfig");
		let Some(tunnel) = cfg.backend_tunnel else {
			panic!("expected backend_tunnel");
		};
		let spec = build_oidc_tunnel(Some(tunnel)).unwrap().unwrap();
		let Target::Hostname(host, _) = &spec.target else {
			panic!("expected hostname");
		};
		assert_eq!(host.as_str(), "genproxy.corp.example.com");
	}

	#[test]
	fn backend_auth_becomes_proxy_authorization_token() {
		let mut tunnel = inline_tunnel(
			"proxy.example.com",
			8080,
			crate::types::backend::TunnelMode::Connect,
		);
		tunnel.policies.push(BackendTrafficPolicy::BackendAuth(
			crate::http::auth::BackendAuth::new(crate::http::auth::BackendAuthKind::Key {
				value: secrecy::SecretString::new("my-key".into()),
				location: None,
			}),
		));
		let spec = build_oidc_tunnel(Some(tunnel)).unwrap().unwrap();
		let token = spec.token.clone().expect("proxy-auth token should be set");
		assert_eq!(token.to_str().unwrap(), "Bearer my-key");
	}

	#[test]
	fn unsupported_policies_are_rejected() {
		let mut tunnel = inline_tunnel(
			"proxy.example.com",
			8080,
			crate::types::backend::TunnelMode::Connect,
		);
		// A policy other than `tcp`/`backendAuth` (here `http`) is not
		// representable on the plain-CONNECT OIDC tunnel and must be rejected,
		// not silently ignored.
		tunnel.policies.push(BackendTrafficPolicy::HTTP(
			crate::types::backend::HTTP::default(),
		));
		let err = build_oidc_tunnel(Some(tunnel)).unwrap_err();
		let msg = err.to_string();
		assert!(
			msg.contains("does not support policy"),
			"expected an unsupported-policy config error, got: {msg}"
		);
	}
}
