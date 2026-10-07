use std::collections::BTreeMap;

use tracing::warn;

#[cfg(feature = "schema")]
use super::TokenCacheConfig;
use super::cache::InMemoryTokenCache;
use super::{
	ChainedExchange, OAuthClientAuth, OAuthGrantType, OAuthTokenExchangeAuth, OAuthTokenType,
	TokenSpec, default_token_cache, deserialize_token_cache, proto_token_type,
	token_cache_from_proto,
};
use crate::http::auth::AuthorizationLocation;
use crate::types::agent::SimpleBackendReferenceWithPolicies;
use crate::types::agent_xds::{Diagnostics, authorization_location, resolve_simple_reference};
use crate::types::proto::{ProtoError, agent};
use crate::{apply, schema};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(from = "CrossAppAccessAuthConfig")]
pub struct CrossAppAccessAuth {
	oauth: OAuthTokenExchangeAuth,
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for CrossAppAccessAuth {
	fn schema_name() -> std::borrow::Cow<'static, str> {
		"CrossAppAccessAuth".into()
	}

	fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
		CrossAppAccessAuthConfig::json_schema(generator)
	}
}

impl serde::Serialize for CrossAppAccessAuth {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		serde::Serialize::serialize(
			&self
				.config_for_serialize()
				.map_err(serde::ser::Error::custom)?,
			serializer,
		)
	}
}

#[apply(schema!)]
pub(super) struct CrossAppAccessAuthConfig {
	/// The user's IdP authorization server, used for the RFC 8693 token exchange.
	pub(super) identity_provider: CrossAppAccessEndpoint,
	/// The resource authorization server, which exchanges the ID-JAG for an access token.
	pub(super) resource_authorization_server: CrossAppAccessEndpoint,
	/// Identifier of the resource authorization server. The issued ID-JAG is bound to this audience.
	pub(super) audience: String,
	/// `resource` parameters naming the protected resource APIs.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub(super) resources: Vec<String>,
	/// `scope` values requested when obtaining the ID-JAG from the identity provider, sent
	/// space-delimited.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub(super) scopes: Vec<String>,
	/// `scope` values requested when exchanging the ID-JAG for an access token. When unset,
	/// inherits `scopes`. When empty, omits `scope`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub(super) access_token_scopes: Option<Vec<String>>,
	/// Subject token sent to the identity provider. Defaults to an OpenID Connect ID token read
	/// from the Authorization Bearer header.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub(super) subject_token: Option<CrossAppAccessSubjectToken>,
	/// Response cache configuration. Defaults to an in-memory cache with 8192 entries and a 300s
	/// TTL when the token endpoint omits `expires_in`. Set `maxEntries` to 0 to disable.
	#[serde(
		default = "default_token_cache",
		deserialize_with = "deserialize_token_cache",
		skip_serializing
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<TokenCacheConfig>"))]
	pub(super) cache: Option<InMemoryTokenCache>,
}

#[apply(schema!)]
pub(super) struct CrossAppAccessSubjectToken {
	/// Where to read the subject token. Defaults to the Authorization Bearer header.
	#[serde(default)]
	pub(super) source: AuthorizationLocation,
	/// RFC 8693 subject token type URI. Defaults to an OpenID Connect ID token.
	#[serde(
		default = "default_subject_token_type",
		skip_serializing_if = "is_default_subject_token_type"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	pub(super) token_type: OAuthTokenType,
}

impl Default for CrossAppAccessSubjectToken {
	fn default() -> Self {
		Self {
			source: AuthorizationLocation::default(),
			token_type: default_subject_token_type(),
		}
	}
}

fn default_subject_token_type() -> OAuthTokenType {
	OAuthTokenType::IdToken
}

fn is_default_subject_token_type(token_type: &OAuthTokenType) -> bool {
	*token_type == OAuthTokenType::IdToken
}

impl From<CrossAppAccessAuthConfig> for CrossAppAccessAuth {
	fn from(config: CrossAppAccessAuthConfig) -> Self {
		// Cross App Access is the RFC 8693 ID-JAG leg (to the IdP) chained into the RFC 7523
		// jwt-bearer leg (to the resource AS).
		let CrossAppAccessAuthConfig {
			identity_provider,
			resource_authorization_server,
			audience,
			resources,
			scopes,
			access_token_scopes,
			subject_token,
			cache,
		} = config;
		let CrossAppAccessEndpoint {
			target,
			path,
			client_auth,
		} = identity_provider;
		let CrossAppAccessSubjectToken { source, token_type } = subject_token.unwrap_or_default();
		let chained_scopes = access_token_scopes.unwrap_or_else(|| scopes.clone());
		let chained_exchange = resource_authorization_server.into_chained_exchange(chained_scopes);
		let oauth = OAuthTokenExchangeAuth {
			target,
			path,
			grant_type: OAuthGrantType::TokenExchange,
			subject_token: TokenSpec { source, token_type },
			actor_token: None,
			audiences: vec![audience],
			scopes,
			resources,
			requested_token_type: Some(OAuthTokenType::IdJag),
			client_auth: Some(client_auth),
			additional_params: BTreeMap::new(),
			chained_exchange: Some(chained_exchange),
			authorization_location: AuthorizationLocation::default(),
			cache,
		};
		Self { oauth }
	}
}

impl CrossAppAccessAuth {
	pub(crate) fn validate_load(&self) -> Result<(), String> {
		if self.audience().is_empty() {
			return Err("crossAppAccess audience must not be empty".into());
		}
		if self.oauth.subject_token.token_type == OAuthTokenType::IdJag {
			return Err("crossAppAccess subjectToken tokenType id-jag is not supported".into());
		}
		self.validate_endpoint_paths()?;
		self.warn_on_unreachable_access_token_scopes();
		self.oauth.validate_load()
	}

	// The ID-JAG's `scope` claim is the ceiling for the chained leg; asking for more invites
	// `invalid_scope`. Warn only: the IdP may grant broader scopes than requested.
	// TODO(mk): report via `Diagnostics` so the control plane can map this back to the source
	// resource, alongside the rest of the oauth validation.
	fn warn_on_unreachable_access_token_scopes(&self) {
		let Some(chained_exchange) = &self.oauth.chained_exchange else {
			return;
		};
		if self.oauth.scopes.is_empty() {
			return;
		}
		let unreachable = chained_exchange
			.scopes
			.iter()
			.filter(|scope| !self.oauth.scopes.contains(scope))
			.map(String::as_str)
			.collect::<Vec<_>>();
		if !unreachable.is_empty() {
			warn!(
				scopes = unreachable.join(" "),
				"crossAppAccess accessTokenScopes requests scopes absent from scopes; the ID-JAG is granted only scopes, so the resource authorization server is likely to reject the exchange with invalid_scope"
			);
		}
	}

	pub(super) fn audience(&self) -> &str {
		self
			.oauth
			.audiences
			.first()
			.map(String::as_str)
			.unwrap_or_default()
	}

	pub(super) fn oauth_token_exchange(&self) -> &OAuthTokenExchangeAuth {
		&self.oauth
	}

	fn config_for_serialize(&self) -> Result<CrossAppAccessAuthConfig, String> {
		let chained_exchange = self
			.oauth
			.chained_exchange
			.as_ref()
			.ok_or_else(|| "cross app access auth must have a chained token exchange".to_string())?;
		let client_auth = self
			.oauth
			.client_auth
			.as_ref()
			.ok_or_else(|| "cross app access identity provider must have client auth".to_string())?;
		let chained_client_auth = chained_exchange.client_auth.as_ref().ok_or_else(|| {
			"cross app access resource authorization server must have client auth".to_string()
		})?;
		let audience = match self.oauth.audiences.as_slice() {
			[audience] => audience.clone(),
			audiences => {
				return Err(format!(
					"cross app access auth must have exactly one audience, got {}",
					audiences.len()
				));
			},
		};

		Ok(CrossAppAccessAuthConfig {
			identity_provider: CrossAppAccessEndpoint {
				target: self.oauth.target.clone(),
				path: self.oauth.path.clone(),
				client_auth: client_auth.clone(),
			},
			resource_authorization_server: CrossAppAccessEndpoint {
				target: chained_exchange.target.clone(),
				path: chained_exchange.path.clone(),
				client_auth: chained_client_auth.clone(),
			},
			audience,
			subject_token: Some(CrossAppAccessSubjectToken {
				source: self.oauth.subject_token.source.clone(),
				token_type: self.oauth.subject_token.token_type.clone(),
			}),
			resources: self.oauth.resources.clone(),
			scopes: self.oauth.scopes.clone(),
			access_token_scopes: (chained_exchange.scopes != self.oauth.scopes)
				.then(|| chained_exchange.scopes.clone()),
			cache: self.oauth.cache.clone(),
		})
	}

	fn validate_endpoint_paths(&self) -> Result<(), String> {
		if !self.oauth.path.is_empty() && !self.oauth.path.starts_with('/') {
			return Err(format!(
				"crossAppAccess.identityProvider.path {:?} must start with /",
				self.oauth.path
			));
		}
		match &self.oauth.chained_exchange {
			Some(chained_exchange)
				if !chained_exchange.path.is_empty() && !chained_exchange.path.starts_with('/') =>
			{
				return Err(format!(
					"crossAppAccess.resourceAuthorizationServer.path {:?} must start with /",
					chained_exchange.path
				));
			},
			_ => {},
		}
		Ok(())
	}

	pub(crate) fn from_proto(
		t: agent::CrossAppAccessAuth,
		diagnostics: &mut Diagnostics,
	) -> Result<Self, ProtoError> {
		let subject_token = match t.subject_token.as_ref() {
			Some(subject_token) => Some(CrossAppAccessSubjectToken {
				source: authorization_location(
					diagnostics,
					"crossAppAccess.subjectToken.source",
					subject_token.source.as_ref(),
					AuthorizationLocation::default(),
				)?,
				token_type: if subject_token.token_type.is_empty() {
					default_subject_token_type()
				} else {
					proto_token_type(
						"crossAppAccess.subjectToken.tokenType",
						&subject_token.token_type,
					)?
				},
			}),
			None => None,
		};
		let config = CrossAppAccessAuthConfig {
			identity_provider: CrossAppAccessEndpoint::from_proto(t.identity_provider, diagnostics)?,
			resource_authorization_server: CrossAppAccessEndpoint::from_proto(
				t.resource_authorization_server,
				diagnostics,
			)?,
			audience: t.audience,
			resources: t.resources,
			scopes: t.scopes,
			access_token_scopes: t.access_token_scopes.map(|scopes| scopes.values),
			subject_token,
			cache: token_cache_from_proto(t.cache)?,
		};
		let auth = Self::from(config);
		auth.validate_load().map_err(ProtoError::Generic)?;
		Ok(auth)
	}
}

#[serde_with::serde_as]
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub(super) struct CrossAppAccessEndpoint {
	/// Token endpoint backend and policies used when connecting to it.
	#[serde(flatten)]
	pub(super) target: SimpleBackendReferenceWithPolicies,
	/// Token endpoint path on the backend; defaults to "/".
	#[serde(default, skip_serializing_if = "String::is_empty")]
	pub(super) path: String,
	/// Client authentication used when calling the token endpoint.
	pub(super) client_auth: OAuthClientAuth,
}

impl CrossAppAccessEndpoint {
	fn from_proto(
		t: Option<agent::cross_app_access_auth::Endpoint>,
		diagnostics: &mut Diagnostics,
	) -> Result<Self, ProtoError> {
		let t = t.ok_or(ProtoError::MissingRequiredField)?;
		let token_endpoint = t
			.token_endpoint
			.as_ref()
			.ok_or(ProtoError::MissingRequiredField)?;
		let client_auth = t
			.client_auth
			.ok_or(ProtoError::MissingRequiredField)?
			.try_into()?;
		let policies =
			crate::types::agent_xds::backend_policies_from_proto(&t.inline_policies, diagnostics)?;
		Ok(Self {
			target: SimpleBackendReferenceWithPolicies {
				target: std::sync::Arc::new(resolve_simple_reference(Some(token_endpoint))),
				policies,
			},
			path: t.token_endpoint_path.unwrap_or_default(),
			client_auth,
		})
	}

	// The root ID-JAG exchange sends configured resources to the IdP; the resulting
	// assertion binds the resource, so the chained jwt-bearer leg omits `resource`.
	fn into_chained_exchange(self, scopes: Vec<String>) -> ChainedExchange {
		ChainedExchange {
			target: self.target,
			path: self.path,
			client_auth: Some(self.client_auth),
			audiences: Vec::new(),
			scopes,
			resources: Vec::new(),
			additional_params: BTreeMap::new(),
		}
	}
}
