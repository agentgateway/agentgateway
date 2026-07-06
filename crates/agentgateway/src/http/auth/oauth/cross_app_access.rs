use std::collections::BTreeMap;
use std::sync::Arc;

#[cfg(feature = "schema")]
use super::TokenCacheConfig;
use super::cache::InMemoryTokenCache;
use super::{
	ChainedExchange, OAuthClientAuth, OAuthGrantType, OAuthTokenExchangeAuth, OAuthTokenType,
	TokenSpec, default_token_cache, deserialize_token_cache,
};
use crate::http::auth::AuthorizationLocation;
use crate::types::agent::{BackendTrafficPolicy, SimpleBackendReference};
use crate::{apply, schema};

#[apply(schema!)]
pub struct CrossAppAccessAuth {
	/// The user's IdP authorization server, used for the RFC 8693 token exchange.
	pub(super) identity_provider: CrossAppAccessEndpoint,
	/// The resource authorization server, which exchanges the ID-JAG for an access token.
	pub(super) resource_authorization_server: CrossAppAccessEndpoint,
	/// Identifier of the resource authorization server. The issued ID-JAG is bound to this audience.
	pub(super) audience: String,
	/// `resource` parameters naming the protected resource APIs.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub(super) resources: Vec<String>,
	/// `scope` values for the requested token, sent space-delimited.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub(super) scopes: Vec<String>,
	/// Response cache configuration. Defaults to an in-memory cache with 8192 entries and a 300s
	/// TTL when the token endpoint omits `expires_in`. Set `maxEntries` to 0 to disable.
	#[serde(
		default = "default_token_cache",
		deserialize_with = "deserialize_token_cache",
		skip_serializing
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<TokenCacheConfig>"))]
	pub(super) cache: Option<InMemoryTokenCache>,
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	pub(super) oauth: Option<OAuthTokenExchangeAuth>,
}

impl CrossAppAccessAuth {
	pub(crate) fn validate_load(&self) -> Result<(), String> {
		if self.audience.is_empty() {
			return Err("crossAppAccess audience must not be empty".into());
		}
		self
			.identity_provider
			.validate_load("crossAppAccess.identityProvider")?;
		self
			.resource_authorization_server
			.validate_load("crossAppAccess.resourceAuthorizationServer")?;
		let oauth = self.oauth.as_ref().ok_or_else(|| {
			"crossAppAccess derived oauth config must be initialized by apply_local_defaults".to_string()
		})?;
		oauth.validate_load()?;
		Ok(())
	}

	pub(crate) fn apply_local_defaults(&mut self) -> Result<(), String> {
		self.oauth = Some(OAuthTokenExchangeAuth {
			target: self.identity_provider.target.clone(),
			policies: self.identity_provider.policies.clone(),
			token_endpoint_path: self.identity_provider.token_endpoint_path.clone(),
			grant_type: OAuthGrantType::TokenExchange,
			subject_token: TokenSpec {
				source: AuthorizationLocation::default(),
				token_type: OAuthTokenType::IdToken,
			},
			actor_token: None,
			audiences: vec![self.audience.clone()],
			scopes: self.scopes.clone(),
			resources: self.resources.clone(),
			requested_token_type: Some(OAuthTokenType::IdJag),
			client_auth: Some(self.identity_provider.client_auth.clone()),
			additional_params: BTreeMap::new(),
			chained_exchange: Some(
				self
					.resource_authorization_server
					.as_chained_exchange(&self.scopes),
			),
			authorization_location: AuthorizationLocation::default(),
			cache: self.cache.clone(),
		});
		Ok(())
	}

	pub(super) fn oauth_token_exchange(&self) -> &OAuthTokenExchangeAuth {
		self
			.oauth
			.as_ref()
			.expect("Cross App Access derived OAuth config must be initialized by apply_local_defaults")
	}
}

#[serde_with::serde_as]
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub(super) struct CrossAppAccessEndpoint {
	/// Token endpoint backend.
	#[serde(flatten)]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "crate::types::local::SimpleLocalBackend")
	)]
	pub(super) target: Arc<SimpleBackendReference>,
	/// Backend policies (TLS, request timeout, ...) used when connecting to the token endpoint.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	#[serde(deserialize_with = "crate::types::local::de_from_local_backend_policy")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "Option<crate::types::local::SimpleLocalBackendPolicies>")
	)]
	pub(super) policies: Vec<BackendTrafficPolicy>,
	/// Token endpoint path on the backend; defaults to "/".
	#[serde(default, skip_serializing_if = "String::is_empty")]
	pub(super) token_endpoint_path: String,
	/// Client authentication used when calling the token endpoint.
	pub(super) client_auth: OAuthClientAuth,
}

impl CrossAppAccessEndpoint {
	fn validate_load(&self, prefix: &str) -> Result<(), String> {
		if !self.token_endpoint_path.is_empty() && !self.token_endpoint_path.starts_with('/') {
			return Err(format!(
				"{prefix}.token_endpoint_path {:?} must start with /",
				self.token_endpoint_path
			));
		}
		self.client_auth.validate_load()
	}

	// The root ID-JAG exchange sends configured resources to the IdP; the resulting
	// assertion binds the resource, so the chained jwt-bearer leg omits `resource`.
	// It still sends `scope`: RFC 7523 uses it to select the access-token scopes, and
	// resource ASs (Okta, xaa.dev) issue an unscoped token without it. The draft's
	// minimal example omits scope, but the ID-JAG's `scope` claim is only the ceiling.
	fn as_chained_exchange(&self, scopes: &[String]) -> ChainedExchange {
		ChainedExchange {
			target: self.target.clone(),
			policies: self.policies.clone(),
			token_endpoint_path: self.token_endpoint_path.clone(),
			client_auth: Some(self.client_auth.clone()),
			audiences: Vec::new(),
			scopes: scopes.to_vec(),
			resources: Vec::new(),
			additional_params: BTreeMap::new(),
		}
	}
}
