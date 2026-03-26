use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use jsonwebtoken::jwk::JwkSet;
use macro_rules_attribute::apply;
use secrecy::ExposeSecret;
use secrecy::SecretString;
use serde::Deserialize;

use crate::client::Client;
use crate::http::{Uri, jwt, sessionpersistence};
use crate::schema_de;
use crate::serdes::FileInlineOrRemote;
use agent_core::prelude::Strng;

use super::provider::{
	build_explicit_provider, discover_provider_metadata, load_jwks,
	normalize_token_endpoint_auth_methods,
};
use super::session::derive_cookie_names;
use super::{
	ClientConfig, CookieSecureMode, Error, NamedOidcProvider, OidcPolicy, PolicyId, Provider,
	RedirectUri, SameSiteMode, SessionConfig, TokenEndpointAuth, dedupe_scopes,
};

#[apply(schema_de!)]
#[derive(Default)]
pub struct LocalOidcListenerConfig {
	#[serde(default)]
	pub providers: Vec<LocalOidcProvider>,
}

#[apply(schema_de!)]
pub struct LocalOidcProvider {
	pub name: Strng,
	#[serde(flatten)]
	pub config: LocalOidcConfig,
}

/// Browser-based OIDC authentication policy.
///
/// Requires `OIDC_COOKIE_SECRET` to be present in the environment when the
/// config is translated into the runtime policy, because session cookie
/// encoders are derived during policy compilation rather than lazily at
/// request time.
#[apply(schema_de!)]
pub struct LocalOidcConfig {
	/// Issuer used for discovery and ID token validation.
	pub issuer: String,

	/// Optional discovery document override. If omitted, discovery uses
	/// `${issuer}/.well-known/openid-configuration`.
	#[serde(default)]
	pub discovery: Option<FileInlineOrRemote>,

	/// Authorization endpoint used to start the browser login flow.
	#[serde(default, deserialize_with = "de_optional_uri")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub authorization_endpoint: Option<Uri>,

	/// Token endpoint used to exchange the authorization code.
	#[serde(default, deserialize_with = "de_optional_uri")]
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct OidcProviderRef {
	pub provider: Strng,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResolvedOidcPolicy {
	provider: ResolvedProvider,
	client_id: String,
	#[serde(serialize_with = "crate::serdes::ser_redact")]
	client_secret: SecretString,
	redirect_uri: RedirectUri,
	#[serde(default)]
	scopes: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ResolvedProvider {
	pub(super) issuer: String,
	#[serde(
		serialize_with = "crate::serdes::ser_display",
		deserialize_with = "crate::serdes::de_parse"
	)]
	pub(super) authorization_endpoint: Uri,
	#[serde(
		serialize_with = "crate::serdes::ser_display",
		deserialize_with = "crate::serdes::de_parse"
	)]
	pub(super) token_endpoint: Uri,
	pub(super) token_endpoint_auth_methods_supported: Vec<TokenEndpointAuth>,
	pub(super) token_endpoint_auth: TokenEndpointAuth,
	pub(super) id_token_audiences: Vec<String>,
	#[serde(default)]
	pub(super) jwt_validation_options: jwt::JWTValidationOptions,
	pub(super) id_token_jwks: JwkSet,
}

impl LocalOidcConfig {
	pub(crate) async fn translate(
		self,
		client: Client,
		oidc_cookie_secret: &SecretString,
	) -> Result<OidcPolicy, Error> {
		self.resolve(client).await?.compile(oidc_cookie_secret)
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
		let effective_audiences = vec![client_id.clone()];

		let provider = match (authorization_endpoint, token_endpoint, jwks) {
			(Some(authorization_endpoint), Some(token_endpoint), Some(jwks)) => {
				let token_endpoint_auth_methods_supported =
					if token_endpoint_auth_methods_supported.is_empty() {
						default_explicit_token_endpoint_auth_methods()
					} else {
						token_endpoint_auth_methods_supported
					};
				build_explicit_provider(
					client.clone(),
					issuer,
					authorization_endpoint,
					token_endpoint,
					jwks,
					effective_audiences,
					token_endpoint_auth_methods_supported,
				)
				.await?
			},
			(authorization_endpoint, token_endpoint, jwks) => {
				let discovered = discover_provider_metadata(client.clone(), &issuer, discovery).await?;
				let (token_endpoint_auth_methods_supported, token_endpoint_auth) =
					if token_endpoint_auth_methods_supported.is_empty() {
						(
							discovered.token_endpoint_auth_methods_supported,
							discovered.token_endpoint_auth,
						)
					} else {
						let methods =
							normalize_token_endpoint_auth_methods(token_endpoint_auth_methods_supported)?;
						(methods.supported, methods.selected)
					};
				let jwks = load_jwks(
					client.clone(),
					jwks.unwrap_or(discovered.jwks),
					"failed to load provider jwks",
				)
				.await?;

				ResolvedProvider {
					issuer,
					authorization_endpoint: authorization_endpoint
						.unwrap_or(discovered.authorization_endpoint),
					token_endpoint: token_endpoint.unwrap_or(discovered.token_endpoint),
					token_endpoint_auth_methods_supported,
					token_endpoint_auth,
					id_token_audiences: effective_audiences,
					jwt_validation_options: jwt::JWTValidationOptions::default(),
					id_token_jwks: jwks,
				}
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

impl LocalOidcProvider {
	pub async fn translate(
		self,
		client: Client,
		oidc_cookie_secret: &SecretString,
	) -> Result<NamedOidcProvider, Error> {
		Ok(NamedOidcProvider {
			name: self.name,
			policy: self.config.translate(client, oidc_cookie_secret).await?,
		})
	}
}

impl ResolvedOidcPolicy {
	fn compile(self, oidc_cookie_secret: &SecretString) -> Result<OidcPolicy, Error> {
		let scopes = dedupe_scopes(self.scopes);
		let policy_id = derive_policy_id(&self.provider, &self.client_id, &self.redirect_uri, &scopes);
		let (cookie_name, transaction_cookie_name) = derive_cookie_names(&policy_id);
		let encoder = sessionpersistence::Encoder::aes(oidc_cookie_secret.expose_secret().trim())
			.map_err(|e| Error::Config(format!("invalid OIDC_COOKIE_SECRET: {e}")))?;
		let token_endpoint_auth = self.provider.token_endpoint_auth;
		let provider = Arc::new(self.provider.compile()?);

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
				encoder,
			},
			scopes,
		})
	}
}

pub(crate) fn load_oidc_cookie_secret() -> Result<SecretString, Error> {
	let oidc_cookie_secret = std::env::var("OIDC_COOKIE_SECRET")
		.map_err(|_| Error::Config("OIDC_COOKIE_SECRET is required when oidc is configured".into()))?;
	Ok(SecretString::new(
		oidc_cookie_secret.trim().to_owned().into_boxed_str(),
	))
}

impl ResolvedProvider {
	fn compile(self) -> Result<Provider, Error> {
		let provider = jwt::Provider::from_jwks(
			self.id_token_jwks,
			self.issuer.clone(),
			Some(self.id_token_audiences.clone()),
			self.jwt_validation_options.clone(),
		)
		.map_err(|e| Error::Config(format!("failed to create id token validator: {e}")))?;

		Ok(Provider {
			issuer: self.issuer,
			authorization_endpoint: self.authorization_endpoint,
			token_endpoint: self.token_endpoint,
			token_endpoint_auth_methods_supported: self.token_endpoint_auth_methods_supported,
			id_token_validator: jwt::Jwt::from_providers(vec![provider], jwt::Mode::Strict),
		})
	}
}

pub(crate) fn default_session_ttl() -> std::time::Duration {
	Duration::from_secs(60 * 60)
}

pub(crate) fn default_transaction_ttl() -> std::time::Duration {
	Duration::from_secs(5 * 60)
}

fn default_explicit_token_endpoint_auth_methods() -> Vec<TokenEndpointAuth> {
	vec![TokenEndpointAuth::ClientSecretBasic]
}

fn de_optional_uri<'de, D>(deserializer: D) -> Result<Option<Uri>, D::Error>
where
	D: serde::Deserializer<'de>,
{
	let value = Option::<String>::deserialize(deserializer)?;
	value
		.map(|value| value.parse().map_err(serde::de::Error::custom))
		.transpose()
}

fn derive_policy_id(
	provider: &ResolvedProvider,
	client_id: &str,
	redirect_uri: &RedirectUri,
	scopes: &[String],
) -> PolicyId {
	let mut seed = String::new();
	append_seed_field(&mut seed, "issuer", &provider.issuer);
	append_seed_field(
		&mut seed,
		"authorizationEndpoint",
		&provider.authorization_endpoint.to_string(),
	);
	append_seed_field(
		&mut seed,
		"tokenEndpoint",
		&provider.token_endpoint.to_string(),
	);
	for method in &provider.token_endpoint_auth_methods_supported {
		append_seed_field(&mut seed, "tokenEndpointAuthMethod", method.as_str());
	}
	append_seed_field(&mut seed, "clientId", client_id);
	append_seed_field(&mut seed, "redirectURI", &redirect_uri.canonical_uri());
	for scope in scopes {
		append_seed_field(&mut seed, "scope", scope);
	}
	let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, seed.as_bytes());
	PolicyId::from(hex::encode(digest.as_ref()))
}

fn append_seed_field(seed: &mut String, key: &str, value: &str) {
	let _ = writeln!(seed, "{key}:{}:{value}", value.len());
}
