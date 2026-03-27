use std::sync::Arc;
use std::time::Duration;

use macro_rules_attribute::apply;
use secrecy::SecretString;

use super::provider::{
	ResolvedProvider, build_explicit_provider, default_discovery_url, discover_provider_metadata,
	load_jwks, normalize_token_endpoint_auth_methods,
};
use super::session::derive_cookie_names;
use super::{
	ClientConfig, CookieSecureMode, Error, OidcPolicy, PolicyId, RedirectUri, SameSiteMode,
	SessionConfig, TokenEndpointAuth, dedupe_scopes,
};
use crate::client::Client;
use crate::http::{Uri, sessionpersistence};
use crate::schema_de;
use crate::serdes::FileInlineOrRemote;

/// Browser-based OIDC authentication policy.
#[apply(schema_de!)]
pub struct LocalOidcConfig {
	/// Issuer used for discovery and ID token validation.
	pub issuer: String,

	/// Optional discovery document override. If omitted, discovery uses
	/// `${issuer}/.well-known/openid-configuration`.
	#[serde(default)]
	pub discovery: Option<FileInlineOrRemote>,

	/// Authorization endpoint used to start the browser login flow.
	#[serde(default, with = "http_serde::option::uri")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub authorization_endpoint: Option<Uri>,

	/// Token endpoint used to exchange the authorization code.
	#[serde(default, with = "http_serde::option::uri")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub token_endpoint: Option<Uri>,

	/// JWKS source used to validate returned ID tokens.
	#[serde(default)]
	pub jwks: Option<FileInlineOrRemote>,

	/// Supported client authentication methods for the token endpoint.
	///
	/// When omitted and discovery is used, the discovery document decides.
	/// When omitted and the provider is fully explicit, this defaults to
	/// `["clientSecretBasic"]`.
	#[serde(default)]
	pub token_endpoint_auth_methods_supported: Vec<TokenEndpointAuth>,

	/// OAuth2 client identifier used for authorization and token exchange.
	pub client_id: String,

	/// OAuth2 client secret used for token exchange.
	#[serde(serialize_with = "crate::serdes::ser_redact")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	pub client_secret: SecretString,

	/// Absolute callback URI handled by the gateway.
	#[serde(rename = "redirectURI")]
	pub redirect_uri: String,

	/// Additional OAuth2 scopes to request. `openid` is always included.
	#[serde(default)]
	pub scopes: Vec<String>,
}

#[derive(Debug, Clone)]
struct ResolvedOidcPolicy {
	provider: ResolvedProvider,
	client_id: String,
	client_secret: SecretString,
	redirect_uri: RedirectUri,
	scopes: Vec<String>,
}

impl LocalOidcConfig {
	pub(crate) async fn translate(
		self,
		client: Client,
		oidc_cookie_encoder: &sessionpersistence::Encoder,
		policy_id: PolicyId,
	) -> Result<OidcPolicy, Error> {
		self
			.resolve(client)
			.await?
			.compile(oidc_cookie_encoder, policy_id)
	}

	async fn resolve(self, client: Client) -> Result<ResolvedOidcPolicy, Error> {
		let LocalOidcConfig {
			issuer,
			discovery,
			authorization_endpoint,
			token_endpoint,
			jwks,
			token_endpoint_auth_methods_supported,
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
				let discovered =
					discover_provider_metadata(client.clone(), &issuer, Some(discovery)).await?;
				let token_endpoint_auth = if token_endpoint_auth_methods_supported.is_empty() {
					discovered.token_endpoint_auth
				} else {
					normalize_token_endpoint_auth_methods(token_endpoint_auth_methods_supported)?
				};
				let jwks = load_jwks(
					client.clone(),
					discovered.jwks,
					super::provider::JwksLoadSource::Discovered,
				)
				.await?;

				ResolvedProvider {
					issuer,
					authorization_endpoint: discovered.authorization_endpoint,
					token_endpoint: discovered.token_endpoint,
					token_endpoint_auth,
					id_token_jwks: jwks,
				}
			},
			3 => {
				if discovery.is_some() {
					return Err(Error::Config(
							"oidc discovery must be omitted when authorizationEndpoint, tokenEndpoint, and jwks are configured explicitly".into(),
						));
				}
				let token_endpoint_auth_methods_supported =
					if token_endpoint_auth_methods_supported.is_empty() {
						vec![TokenEndpointAuth::ClientSecretBasic]
					} else {
						token_endpoint_auth_methods_supported
					};
				build_explicit_provider(
					client.clone(),
					issuer,
					authorization_endpoint.expect("checked above"),
					token_endpoint.expect("checked above"),
					jwks.expect("checked above"),
					token_endpoint_auth_methods_supported,
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

		Ok(ResolvedOidcPolicy {
			provider,
			client_id,
			client_secret,
			redirect_uri,
			scopes,
		})
	}
}

impl ResolvedOidcPolicy {
	fn compile(
		self,
		oidc_cookie_encoder: &sessionpersistence::Encoder,
		policy_id: PolicyId,
	) -> Result<OidcPolicy, Error> {
		let scopes = dedupe_scopes(self.scopes);
		let (cookie_name, transaction_cookie_name) = derive_cookie_names(&policy_id);
		let token_endpoint_auth = self.provider.token_endpoint_auth;
		let provider = Arc::new(self.provider.compile(vec![self.client_id.clone()])?);

		Ok(OidcPolicy {
			policy_id,
			provider,
			client: ClientConfig {
				client_id: self.client_id,
				client_secret: self.client_secret,
				token_endpoint_auth,
			},
			redirect_uri: self.redirect_uri,
			session: SessionConfig {
				cookie_name,
				transaction_cookie_name,
				same_site: SameSiteMode::Lax,
				secure: CookieSecureMode::Auto,
				ttl: default_session_ttl(),
				transaction_ttl: default_transaction_ttl(),
				encoder: oidc_cookie_encoder.clone(),
			},
			scopes,
		})
	}
}

pub(crate) fn default_session_ttl() -> std::time::Duration {
	Duration::from_secs(60 * 60)
}

pub(crate) fn default_transaction_ttl() -> std::time::Duration {
	Duration::from_secs(5 * 60)
}
