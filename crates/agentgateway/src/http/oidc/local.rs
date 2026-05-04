use std::sync::Arc;

use jsonwebtoken::jwk::JwkSet;
use macro_rules_attribute::apply;
use secrecy::SecretString;

use super::{
	ClientConfig, ClientCredentials, CookieSecureMode, Error, OidcCookieEncoder, OidcPolicy,
	PolicyId, Provider, ProviderEndpoint, RedirectUri, SameSiteMode, SessionConfig, normalize_scopes,
	session,
};
use crate::client::Client;
use crate::http::oauth::{
	TokenEndpointAuth, openid_configuration_metadata_url, parse_token_endpoint_auth_methods,
};
use crate::schema_de;
use crate::serdes::FileInlineOrRemote;

#[derive(Debug, serde::Deserialize)]
struct OidcDiscoveryDocument {
	issuer: String,
	authorization_endpoint: String,
	token_endpoint: String,
	jwks_uri: String,
	#[serde(default)]
	token_endpoint_auth_methods_supported: Option<Vec<String>>,
}

pub(crate) struct PreparedOidcPolicy {
	policy_id: PolicyId,
	issuer: String,
	authorization_endpoint: ProviderEndpoint,
	token_endpoint: ProviderEndpoint,
	id_token_jwks: JwkSet,
	client_id: String,
	credentials: ClientCredentials,
	redirect_uri: RedirectUri,
	scopes: Vec<String>,
}

/// Decode a fully-resolved OIDC policy delivered over xDS into typed config.
/// Mirrors the JWT path: the controller has already resolved provider metadata
/// and JWKS, while runtime cookie crypto is applied later at store ingestion.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_oidc_policy_from_xds(
	policy_id: PolicyId,
	issuer: String,
	authorization_endpoint: ProviderEndpoint,
	token_endpoint: ProviderEndpoint,
	jwks_inline: &str,
	client_id: String,
	credentials: ClientCredentials,
	redirect_uri: RedirectUri,
	scopes: Vec<String>,
) -> Result<PreparedOidcPolicy, Error> {
	let id_token_jwks: JwkSet = serde_json::from_str(jwks_inline).map_err(|e| {
		Error::Config(format!(
			"failed to parse inline oidc jwks delivered by xds: {e}"
		))
	})?;
	Ok(PreparedOidcPolicy {
		policy_id,
		issuer,
		authorization_endpoint,
		token_endpoint,
		id_token_jwks,
		client_id,
		credentials,
		redirect_uri,
		scopes,
	})
}

/// Browser-based OIDC authentication policy.
///
/// Explicit mode is still OIDC: it supplies provider metadata manually instead of using discovery.
/// Unauthenticated non-callback requests always redirect to the provider login flow. Routes that
/// need non-redirect authentication behavior should use a different auth policy.
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

	/// OAuth2 client secret used for token exchange. Omit for a public
	/// client (`tokenEndpointAuth: none`).
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		serialize_with = "crate::serdes::ser_redact"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub client_secret: Option<SecretString>,

	/// Absolute callback URI handled by the gateway.
	/// This policy always redirects unauthenticated non-callback requests back through this login
	/// flow.
	#[serde(rename = "redirectURI")]
	pub redirect_uri: String,

	/// Additional OAuth2 scopes to request. `openid` is always included.
	#[serde(default)]
	pub scopes: Vec<String>,
}

struct DiscoveredProviderMetadata {
	authorization_endpoint: ProviderEndpoint,
	token_endpoint: ProviderEndpoint,
	token_endpoint_auth: TokenEndpointAuth,
	jwks: FileInlineOrRemote,
}

struct ResolvedProvider {
	authorization_endpoint: ProviderEndpoint,
	token_endpoint: ProviderEndpoint,
	id_token_jwks: JwkSet,
	token_endpoint_auth: TokenEndpointAuth,
}

impl LocalOidcConfig {
	pub(crate) async fn compile(
		self,
		client: Client,
		policy_id: PolicyId,
		oidc_cookie_encoder: &OidcCookieEncoder,
	) -> Result<OidcPolicy, Error> {
		self
			.resolve(client, policy_id)
			.await?
			.compile(oidc_cookie_encoder)
	}

	async fn resolve(self, client: Client, policy_id: PolicyId) -> Result<PreparedOidcPolicy, Error> {
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
		} = self;
		let redirect_uri = RedirectUri::parse(redirect_uri)?;
		let explicit_field_count = usize::from(authorization_endpoint.is_some())
			+ usize::from(token_endpoint.is_some())
			+ usize::from(jwks.is_some());
		if token_endpoint_auth.is_some() && explicit_field_count != 3 {
			return Err(Error::Config(
				"tokenEndpointAuth must be omitted unless authorizationEndpoint, tokenEndpoint, and jwks are configured explicitly".into(),
			));
		}
		let resolved = match explicit_field_count {
			0 => {
				let discovery = match discovery {
					Some(discovery) => discovery,
					None => FileInlineOrRemote::Remote {
						url: default_discovery_url(&issuer)?,
					},
				};
				let discovered =
					discover_provider_metadata(client.clone(), &issuer, discovery, client_secret.as_ref())
						.await?;
				let id_token_jwks = load_jwks(client, discovered.jwks, "discovered jwks source").await?;

				ResolvedProvider {
					authorization_endpoint: discovered.authorization_endpoint,
					token_endpoint: discovered.token_endpoint,
					id_token_jwks,
					token_endpoint_auth: discovered.token_endpoint_auth,
				}
			},
			3 => {
				if discovery.is_some() {
					return Err(Error::Config(
						"oidc discovery must be omitted when authorizationEndpoint, tokenEndpoint, and jwks are configured explicitly".into(),
					));
				}
				let (Some(authorization_endpoint), Some(token_endpoint), Some(jwks)) =
					(authorization_endpoint, token_endpoint, jwks)
				else {
					unreachable!("explicit_field_count == 3 requires all explicit provider fields");
				};
				let id_token_jwks = load_jwks(client, jwks, "explicit jwks source").await?;
				ResolvedProvider {
					authorization_endpoint,
					token_endpoint,
					id_token_jwks,
					token_endpoint_auth: token_endpoint_auth.unwrap_or_default(),
				}
			},
			_ => {
				return Err(Error::Config(
					"authorizationEndpoint, tokenEndpoint, and jwks must either all be set or all be omitted"
						.into(),
				));
			},
		};
		let credentials = ClientCredentials::from_parts(resolved.token_endpoint_auth, client_secret)?;

		Ok(PreparedOidcPolicy {
			policy_id,
			issuer,
			authorization_endpoint: resolved.authorization_endpoint,
			token_endpoint: resolved.token_endpoint,
			id_token_jwks: resolved.id_token_jwks,
			client_id,
			credentials,
			redirect_uri,
			scopes,
		})
	}
}

async fn discover_provider_metadata(
	client: Client,
	issuer: &str,
	discovery: FileInlineOrRemote,
	client_secret: Option<&SecretString>,
) -> Result<DiscoveredProviderMetadata, Error> {
	let document = discovery
		.load::<OidcDiscoveryDocument>(client)
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

	let token_endpoint_auth = parse_token_endpoint_auth_methods(
		document.token_endpoint_auth_methods_supported,
		client_secret,
	)
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
	client: Client,
	jwks: FileInlineOrRemote,
	source: &'static str,
) -> Result<JwkSet, Error> {
	let jwks = jwks.load::<JwkSet>(client).await.map_err(|e| {
		Error::Config(format!(
			"failed to load oidc jwks from {} {}: {e}",
			source,
			describe_file_inline_or_remote(&jwks)
		))
	})?;
	Ok(jwks)
}

impl PreparedOidcPolicy {
	pub(crate) fn compile(
		self,
		oidc_cookie_encoder: &OidcCookieEncoder,
	) -> Result<OidcPolicy, Error> {
		let (cookie_name, transaction_cookie_prefix) = session::derive_cookie_names(&self.policy_id);
		let PreparedOidcPolicy {
			policy_id,
			issuer,
			authorization_endpoint,
			token_endpoint,
			id_token_jwks,
			client_id,
			credentials,
			redirect_uri,
			scopes,
		} = self;
		let scopes = normalize_scopes(scopes);
		let id_token_validator = crate::http::jwt::Provider::from_jwks(
			id_token_jwks,
			issuer.clone(),
			Some(vec![client_id.clone()]),
			crate::http::jwt::JWTValidationOptions::default(),
		)
		.map_err(|e| Error::Config(format!("failed to create id token validator: {e}")))?;
		let provider = Arc::new(Provider {
			issuer,
			authorization_endpoint,
			token_endpoint,
			id_token_validator: crate::http::jwt::Jwt::from_providers(
				vec![id_token_validator],
				crate::http::jwt::Mode::Strict,
				crate::http::auth::AuthorizationLocation::bearer_header(),
			),
		});

		Ok(OidcPolicy {
			policy_id,
			provider,
			client: ClientConfig {
				client_id,
				credentials,
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
