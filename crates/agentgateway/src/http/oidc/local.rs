use std::sync::Arc;

use jsonwebtoken::jwk::JwkSet;
use macro_rules_attribute::apply;
use secrecy::SecretString;

use super::{
	ClientConfig, CookieSecureMode, Error, OidcPolicy, PolicyId, Provider, ProviderEndpoint,
	RedirectUri, SameSiteMode, SessionConfig, dedupe_scopes, session,
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

struct ResolvedOidcProviderSpec {
	issuer: String,
	authorization_endpoint: ProviderEndpoint,
	token_endpoint: ProviderEndpoint,
	token_endpoint_auth: TokenEndpointAuth,
	id_token_jwks: JwkSet,
}

struct ResolvedOidcPolicySpec {
	provider: ResolvedOidcProviderSpec,
	client_id: String,
	client_secret: SecretString,
	redirect_uri: RedirectUri,
	scopes: Vec<String>,
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

#[derive(Debug, Clone, Copy)]
enum JwksLoadSource {
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

impl LocalOidcConfig {
	pub(crate) async fn compile(
		self,
		client: Client,
		policy_id: PolicyId,
		oidc_cookie_encoder: &crate::http::sessionpersistence::Encoder,
	) -> Result<OidcPolicy, Error> {
		self
			.resolve(client)
			.await?
			.compile(policy_id, oidc_cookie_encoder)
	}

	async fn resolve(self, client: Client) -> Result<ResolvedOidcPolicySpec, Error> {
		let LocalOidcConfig {
			issuer,
			discovery,
			authorization_endpoint,
			token_endpoint,
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
		let provider = match explicit_field_count {
			0 => {
				let discovery = match discovery {
					Some(discovery) => discovery,
					None => FileInlineOrRemote::Remote {
						url: default_discovery_url(&issuer)?,
					},
				};
				let discovered = discover_provider_metadata(client.clone(), &issuer, discovery).await?;
				let id_token_jwks = load_jwks(client, discovered.jwks, JwksLoadSource::Discovered).await?;

				ResolvedOidcProviderSpec {
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
					client,
					issuer,
					authorization_endpoint.expect("checked above"),
					token_endpoint.expect("checked above"),
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

		Ok(ResolvedOidcPolicySpec {
			provider,
			client_id,
			client_secret,
			redirect_uri,
			scopes,
		})
	}
}

async fn discover_provider_metadata(
	client: Client,
	issuer: &str,
	discovery: FileInlineOrRemote,
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
	client: Client,
	issuer: String,
	authorization_endpoint: ProviderEndpoint,
	token_endpoint: ProviderEndpoint,
	jwks: FileInlineOrRemote,
) -> Result<ResolvedOidcProviderSpec, Error> {
	let id_token_jwks = load_jwks(client, jwks, JwksLoadSource::Explicit).await?;

	Ok(ResolvedOidcProviderSpec {
		issuer,
		authorization_endpoint,
		token_endpoint,
		token_endpoint_auth: TokenEndpointAuth::ClientSecretBasic,
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

impl ResolvedOidcProviderSpec {
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
			),
		})
	}
}

impl ResolvedOidcPolicySpec {
	fn compile(
		self,
		policy_id: PolicyId,
		oidc_cookie_encoder: &crate::http::sessionpersistence::Encoder,
	) -> Result<OidcPolicy, Error> {
		let (cookie_name, transaction_cookie_name) = session::derive_cookie_names(&policy_id);
		let ResolvedOidcPolicySpec {
			provider,
			client_id,
			client_secret,
			redirect_uri,
			scopes,
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
				transaction_cookie_name,
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
