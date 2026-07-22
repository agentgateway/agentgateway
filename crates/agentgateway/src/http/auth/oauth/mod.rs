use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use tracing::{debug, trace, warn};

use super::AuthorizationLocation;
use crate::http::Request;
use crate::http::jwt::Claims;
use crate::http::oauth::{TOKEN_TYPE_ACCESS, TOKEN_TYPE_ID, TOKEN_TYPE_ID_JAG, TOKEN_TYPE_JWT};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::serdes::schema;
use crate::types::agent::SimpleBackendReferenceWithPolicies;
use crate::types::agent_xds::{
	Diagnostics, authorization_location, optional_authorization_location,
	permissive_cel_expression_arc, resolve_simple_reference,
};
use crate::types::proto::{ProtoError, agent as proto};
use crate::{apply, cel, schema_enum};

mod cache;
mod client_auth;
mod cross_app_access;
mod transport;

use cache::{InMemoryTokenCache, TokenCacheResult};
use client_auth::sign_client_assertion;
pub use client_auth::{OAuthClientAuth, OAuthClientAuthMethod, PrivateKeyJwt, SigningAlg};
pub use cross_app_access::CrossAppAccessAuth;
pub(super) use transport::FetchError;

#[apply(schema!)]
pub struct OAuthTokenExchangeAuth {
	// ----- Token endpoint -----
	/// Backend serving the RFC 8693 token endpoint and policies used when connecting to it.
	#[serde(flatten)]
	target: SimpleBackendReferenceWithPolicies,
	/// Token endpoint path on the backend; defaults to "/".
	#[serde(default, skip_serializing_if = "String::is_empty")]
	path: String,

	// ----- Grant and incoming tokens -----
	/// Selects which RFC the request follows; defaults to token exchange (RFC 8693).
	#[serde(default)]
	grant_type: OAuthGrantType,
	/// Where the subject token is read from, and its token type. Defaults to the
	/// Authorization Bearer header with token type access_token.
	#[serde(default)]
	subject_token: TokenSpec,
	/// RFC 8693 delegation actor token. Token-exchange grant only.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	actor_token: Option<ActorTokenSpec>,

	// ----- Token request parameters -----
	/// `audience` parameters naming the target services at the authorization server.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	audiences: Vec<String>,
	/// `scope` values for the requested token, sent space-delimited.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	scopes: Vec<String>,
	/// `resource` parameters with the target service URIs.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	resources: Vec<String>,
	/// `requested_token_type` parameter. Under token exchange, unset defaults to
	/// access_token because this policy forwards bearer access tokens.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	requested_token_type: Option<OAuthTokenType>,
	/// Client authentication used when calling the token endpoint.
	/// When unset, no client authentication fields are sent.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	client_auth: Option<OAuthClientAuth>,
	/// Extra form parameters appended to the token request.
	/// Values are CEL expressions evaluated against the incoming request.
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	additional_params: BTreeMap<String, Arc<cel::Expression>>,

	// ----- Output and runtime behavior -----
	/// Where to place the exchanged token in the backend request. Defaults to the
	/// Authorization header with a "Bearer " prefix. The CEL `expression` source is
	/// not valid here (it cannot insert).
	#[serde(default)]
	authorization_location: AuthorizationLocation,

	/// Response cache configuration. Defaults to an in-memory cache with 8192 entries and a 300s
	/// TTL when the token endpoint omits `expires_in`. Set `maxEntries` to 0 to disable.
	#[serde(
		default = "default_token_cache",
		deserialize_with = "deserialize_token_cache",
		skip_serializing
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<TokenCacheConfig>"))]
	cache: Option<InMemoryTokenCache>,

	// --- internal ---
	// Optional RFC 7523 jwt-bearer hop used internally by ID-JAG.
	#[serde(skip)]
	chained_exchange: Option<ChainedExchange>,
}

#[serde_with::serde_as]
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ChainedExchange {
	/// Backend serving the chained RFC 7523 token endpoint and policies used when connecting to it.
	#[serde(flatten)]
	target: SimpleBackendReferenceWithPolicies,
	/// Token endpoint path on the backend; defaults to "/".
	#[serde(default, skip_serializing_if = "String::is_empty")]
	path: String,
	/// Client authentication used when calling the chained token endpoint.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	client_auth: Option<OAuthClientAuth>,
	/// `audience` parameters naming the target services at the authorization server.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	audiences: Vec<String>,
	/// `scope` values for the requested token, sent space-delimited.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	scopes: Vec<String>,
	/// `resource` parameters with the target service URIs.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	resources: Vec<String>,
	/// Extra form parameters appended to the chained token request.
	/// Values are CEL expressions evaluated against the incoming request.
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	additional_params: BTreeMap<String, Arc<cel::Expression>>,
}

impl ChainedExchange {
	fn validate_load(&self) -> Result<(), String> {
		if !self.path.is_empty() && !self.path.starts_with('/') {
			return Err(format!(
				"chained_exchange.path {:?} must start with /",
				self.path
			));
		}
		if let Some(client_auth) = &self.client_auth {
			client_auth.validate_load()?;
		}
		warn_on_invalid_resources("chained oauth token exchange", &self.resources);
		warn_on_invalid_scopes("chained oauth token exchange scopes", &self.scopes);
		validate_additional_params(&self.additional_params)?;
		Ok(())
	}

	fn evaluate_additional_params(&self, req: &Request) -> anyhow::Result<Vec<(String, String)>> {
		evaluate_additional_params(&self.additional_params, req)
	}
}

/// Spec-defined form parameters that `additional_params` must not override.
const RESERVED_FORM_PARAMS: &[&str] = &[
	"grant_type",
	"subject_token",
	"subject_token_type",
	"actor_token",
	"actor_token_type",
	"assertion",
	"audience",
	"resource",
	"scope",
	"requested_token_type",
	"client_id",
	"client_secret",
	"client_assertion",
	"client_assertion_type",
];

impl OAuthTokenExchangeAuth {
	pub(crate) fn validate_load(&self) -> Result<(), String> {
		if !self.path.is_empty() && !self.path.starts_with('/') {
			return Err(format!("path {:?} must start with /", self.path));
		}
		if self.grant_type == OAuthGrantType::JwtBearer {
			if self.requested_token_type.is_some() {
				return Err("requested_token_type is only valid with the token-exchange grant".into());
			}
			if self.actor_token.is_some() {
				return Err("actor_token is only valid with the token-exchange grant".into());
			}
		}
		if let Some(actor_token) = &self.actor_token {
			actor_token.validate_load()?;
		}
		if let Some(client_auth) = &self.client_auth {
			client_auth.validate_load()?;
		}
		if let Some(OAuthTokenType::Custom(token_type)) = &self.requested_token_type {
			return Err(format!(
				"unsupported requested_token_type {token_type:?}; custom token types are only supported for subject_token and actor_token"
			));
		}

		warn_on_invalid_resources("oauth token exchange", &self.resources);
		warn_on_invalid_scopes("oauth token exchange scopes", &self.scopes);
		validate_additional_params(&self.additional_params)?;

		if let Some(chained_exchange) = &self.chained_exchange {
			chained_exchange.validate_load()?;
			if self.grant_type != OAuthGrantType::TokenExchange {
				return Err("chained_exchange is only valid with the token-exchange grant".into());
			}
			if self.requested_token_type != Some(OAuthTokenType::IdJag) {
				return Err("chained_exchange currently requires requested_token_type id-jag".into());
			}
		}
		if self.requested_token_type == Some(OAuthTokenType::IdJag) {
			if self.chained_exchange.is_none() {
				return Err(
					"requested_token_type id-jag is only supported by backendAuth.crossAppAccess".into(),
				);
			}
			if self.audiences.is_empty() {
				return Err("requested_token_type id-jag requires at least one audience".into());
			}
			if self.subject_token.token_type == OAuthTokenType::AccessToken {
				warn!(
					"oauth token exchange requested_token_type id-jag is configured with an access_token subject; the ID-JAG draft expects an ID token subject"
				);
			}
		}

		if matches!(
			self.authorization_location,
			AuthorizationLocation::Expression { .. }
		) {
			return Err("expression auth location is only supported for credential extraction".into());
		}
		if matches!(
			self.authorization_location,
			AuthorizationLocation::QueryParameter { .. }
		) {
			warn!(
				"oauth token exchange is configured to forward the exchanged bearer token in a URI query parameter; OAuth 2.1 omits this bearer-token usage and future versions may reject it"
			);
		}
		Ok(())
	}

	pub(crate) fn from_proto(
		t: proto::OAuthTokenExchange,
		diagnostics: &mut Diagnostics,
	) -> Result<Self, ProtoError> {
		use proto::o_auth_token_exchange::GrantType;

		let target = resolve_simple_reference(t.token_endpoint.as_ref());
		let path = t.token_endpoint_path.unwrap_or_default();

		let grant_type = match GrantType::try_from(t.grant_type) {
			Ok(GrantType::Unspecified | GrantType::TokenExchange) => OAuthGrantType::TokenExchange,
			Ok(GrantType::JwtBearer) => OAuthGrantType::JwtBearer,
			Err(_) => return Err(ProtoError::EnumParse("unknown oauth grant type".into())),
		};

		let subject_token = t
			.subject_token
			.map(|s| token_spec_from_proto(s, diagnostics))
			.transpose()?
			.unwrap_or_default();

		let actor_token = t
			.actor_token
			.map(|s| actor_token_from_proto(s, diagnostics))
			.transpose()?;

		let requested_token_type = match t.requested_token_type {
			Some(token_type) if !token_type.is_empty() => Some(proto_requested_token_type(
				"requested_token_type",
				&token_type,
			)?),
			_ => None,
		};
		if requested_token_type == Some(OAuthTokenType::IdJag) {
			return Err(ProtoError::Generic(
				"requested_token_type id-jag is only supported by local backendAuth.crossAppAccess".into(),
			));
		}

		let client_auth = t.client_auth.map(OAuthClientAuth::try_from).transpose()?;

		let authorization_location =
			optional_authorization_location(t.authorization_location.as_ref())?.unwrap_or_default();

		let additional_params = t
			.additional_params
			.into_iter()
			.map(|(k, v)| {
				let expr = permissive_cel_expression_arc(
					diagnostics,
					format!("backendAuth.oauth.additionalParams.{k}"),
					v,
				);
				(k, expr)
			})
			.collect::<BTreeMap<_, _>>();

		let cache = token_cache_from_proto(t.cache)?;

		let auth = Self {
			target: SimpleBackendReferenceWithPolicies {
				target: Arc::new(target),
				// Inline connection policies are not supported from xDS;
				// the backend resource carries its own policies there.
				policies: Vec::new(),
			},
			path,
			grant_type,
			subject_token,
			actor_token,
			audiences: t.audiences,
			scopes: t.scopes,
			resources: t.resources,
			requested_token_type,
			client_auth,
			additional_params,
			chained_exchange: None,
			authorization_location,
			cache,
		};
		auth.validate_load().map_err(ProtoError::Generic)?;
		Ok(auth)
	}

	fn requested_token_type_param(&self) -> Option<OAuthTokenType> {
		match self.grant_type {
			OAuthGrantType::TokenExchange => Some(self.requested_token_type.clone().unwrap_or_default()),
			OAuthGrantType::JwtBearer => None,
		}
	}

	// We expect the authorization server to issue exactly the token type we
	// requested, so the expected issued type mirrors the requested param.
	fn expected_issued_token_type(&self) -> Option<OAuthTokenType> {
		self.requested_token_type_param()
	}

	/// Evaluate the configured `additional_params` CEL expressions against the
	/// incoming request. Fails closed if any expression errors or is not a string.
	fn evaluate_additional_params(&self, req: &Request) -> anyhow::Result<Vec<(String, String)>> {
		evaluate_additional_params(&self.additional_params, req)
	}

	fn build_exchange_request(&self, req: &Request) -> Result<ExchangeRequest, ProxyError> {
		// Extract everything up front so a bad request fails before we touch it.
		let subject_token =
			extract_subject_token(&self.subject_token.source, req).ok_or_else(|| {
				debug!("oauth token exchange subject token missing");
				ProxyError::InvalidRequest
			})?;
		let actor = self
			.actor_token
			.as_ref()
			.map(|spec| actor_token_from_request(spec, req, &subject_token))
			.transpose()?;
		let extra_params = self.evaluate_additional_params(req).map_err(|e| {
			debug!("oauth token exchange additional parameter evaluation failed: {e}");
			ProxyError::InvalidRequest
		})?;
		let chained_extra_params = self
			.chained_exchange
			.as_ref()
			.map(|chained_exchange| chained_exchange.evaluate_additional_params(req))
			.transpose()
			.map_err(|e| {
				debug!("oauth chained token exchange additional parameter evaluation failed: {e}");
				ProxyError::InvalidRequest
			})?
			.unwrap_or_default();

		Ok(ExchangeRequest {
			subject_token: subject_token.into(),
			subject_token_type: self.subject_token.token_type.clone(),
			actor,
			extra_params,
			chained_extra_params,
		})
	}

	fn insert_exchanged_token(
		&self,
		req: &mut Request,
		access_token: &str,
	) -> Result<bool, ProxyError> {
		// Replace the original credentials with the backend's.
		self.subject_token.source.remove(req)?;

		if let Some(actor) = &self.actor_token {
			actor.source.remove(req)?;
		}

		self.authorization_location.insert(req, access_token)?;

		Ok(true)
	}
}

#[apply(schema_enum!)]
#[derive(Default)]
pub enum OAuthGrantType {
	/// RFC 8693 token exchange; the subject token is sent as `subject_token`.
	#[default]
	TokenExchange,
	/// RFC 7523; the subject token is sent as the `assertion`.
	JwtBearer,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
enum OAuthTokenType {
	#[default]
	AccessToken,
	Jwt,
	IdToken,
	IdJag,
	Custom(String),
}

impl OAuthTokenType {
	fn from_urn(token_type: &str) -> Option<Self> {
		match token_type {
			TOKEN_TYPE_ACCESS => Some(Self::AccessToken),
			TOKEN_TYPE_JWT => Some(Self::Jwt),
			TOKEN_TYPE_ID => Some(Self::IdToken),
			TOKEN_TYPE_ID_JAG => Some(Self::IdJag),
			_ if is_oauth_absolute_uri(token_type) => Some(Self::Custom(token_type.into())),
			_ => None,
		}
	}

	fn supported_requested_from_urn(token_type: &str) -> Option<Self> {
		match token_type {
			TOKEN_TYPE_ACCESS => Some(Self::AccessToken),
			TOKEN_TYPE_JWT => Some(Self::Jwt),
			TOKEN_TYPE_ID => Some(Self::IdToken),
			TOKEN_TYPE_ID_JAG => Some(Self::IdJag),
			_ => None,
		}
	}

	fn as_str(&self) -> &str {
		match self {
			Self::AccessToken => TOKEN_TYPE_ACCESS,
			Self::Jwt => TOKEN_TYPE_JWT,
			Self::IdToken => TOKEN_TYPE_ID,
			Self::IdJag => TOKEN_TYPE_ID_JAG,
			Self::Custom(token_type) => token_type,
		}
	}
}

impl serde::Serialize for OAuthTokenType {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		serializer.serialize_str(self.as_str())
	}
}

impl<'de> serde::Deserialize<'de> for OAuthTokenType {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		let token_type = <String as serde::Deserialize>::deserialize(deserializer)?;
		Self::from_urn(&token_type).ok_or_else(|| {
			serde::de::Error::custom(format!(
				"OAuth token type must be an absolute URI without a fragment, got {token_type:?}"
			))
		})
	}
}

#[derive(Default)]
#[apply(schema!)]
pub struct TokenSpec {
	/// Where the token is read from in the incoming request. The CEL `expression`
	/// source is permitted (extraction only).
	#[serde(default)]
	source: AuthorizationLocation,
	/// RFC 8693 token type URN; when omitted defaults to access_token
	#[serde(default)]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	token_type: OAuthTokenType,
}

#[apply(schema!)]
pub struct ActorTokenSpec {
	/// Where the actor token is read from in the incoming request. The CEL
	/// `expression` source is permitted (extraction only). Unlike subject tokens,
	/// actor tokens have no default source.
	source: AuthorizationLocation,
	/// RFC 8693 actor token type URN; when omitted defaults to access_token and is still sent
	#[serde(default)]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	token_type: OAuthTokenType,
	/// Enforce that the subject's `may_act` claim authorizes the actor before exchanging.
	#[serde(default)]
	enforce_may_act: bool,
}

impl ActorTokenSpec {
	fn validate_load(&self) -> Result<(), String> {
		if self.enforce_may_act && self.token_type != OAuthTokenType::Jwt {
			return Err(format!(
				"actor_token.enforce_may_act requires actor_token.token_type {TOKEN_TYPE_JWT}"
			));
		}
		Ok(())
	}
}

#[derive(Default)]
#[apply(schema!)]
struct TokenCacheConfig {
	/// Maximum number of token exchange responses to keep in the cache. Set to 0 to disable.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	max_entries: Option<usize>,
	/// TTL used when the token endpoint omits `expires_in`. Defaults to 300s.
	#[serde(
		default,
		with = "crate::serdes::serde_dur_option",
		skip_serializing_if = "Option::is_none"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	default_ttl: Option<Duration>,
}

impl TokenCacheConfig {
	fn into_cache(self) -> Option<InMemoryTokenCache> {
		let max_entries = match self.max_entries {
			Some(0) => return None,
			Some(max_entries) => max_entries,
			None => cache::DEFAULT_CACHE_CAPACITY,
		};
		let default_ttl = self
			.default_ttl
			.filter(|d| !d.is_zero())
			.unwrap_or(cache::DEFAULT_CACHE_TTL);
		Some(InMemoryTokenCache::new(max_entries, default_ttl))
	}
}

fn default_token_cache() -> Option<InMemoryTokenCache> {
	Some(InMemoryTokenCache::default())
}

fn deserialize_token_cache<'de, D>(deserializer: D) -> Result<Option<InMemoryTokenCache>, D::Error>
where
	D: serde::Deserializer<'de>,
{
	use serde::Deserialize;

	let cache = Option::<TokenCacheConfig>::deserialize(deserializer)?;
	Ok(cache.unwrap_or_default().into_cache())
}

fn token_cache_config_from_proto(
	cache: Option<proto::o_auth_token_exchange::TokenCache>,
) -> Result<TokenCacheConfig, ProtoError> {
	let Some(in_memory) = cache.and_then(|c| c.in_memory) else {
		return Ok(TokenCacheConfig::default());
	};

	Ok(TokenCacheConfig {
		max_entries: in_memory
			.max_entries
			.map(|max_entries| max_entries as usize),
		default_ttl: in_memory
			.default_ttl
			.and_then(|d| Duration::try_from(d).ok()),
	})
}

fn token_cache_from_proto(
	cache: Option<proto::o_auth_token_exchange::TokenCache>,
) -> Result<Option<InMemoryTokenCache>, ProtoError> {
	Ok(token_cache_config_from_proto(cache)?.into_cache())
}

fn token_spec_from_proto(
	spec: proto::o_auth_token_exchange::TokenSpec,
	diagnostics: &mut Diagnostics,
) -> Result<TokenSpec, ProtoError> {
	Ok(TokenSpec {
		source: authorization_location(
			diagnostics,
			"backendAuth.oauth.subjectToken.source",
			spec.source.as_ref(),
			AuthorizationLocation::default(),
		)?,
		token_type: if spec.token_type.is_empty() {
			OAuthTokenType::default()
		} else {
			proto_token_type("subject_token.token_type", &spec.token_type)?
		},
	})
}

fn actor_token_from_proto(
	spec: proto::o_auth_token_exchange::ActorToken,
	diagnostics: &mut Diagnostics,
) -> Result<ActorTokenSpec, ProtoError> {
	// Unlike the subject token, the actor token has no default source: it must be
	// explicit so actor and subject can't accidentally be the same credential.
	if spec.source.is_none() {
		return Err(ProtoError::Generic(
			"oauth token exchange actor_token.source must be set".into(),
		));
	}
	Ok(ActorTokenSpec {
		source: authorization_location(
			diagnostics,
			"backendAuth.oauth.actorToken.source",
			spec.source.as_ref(),
			AuthorizationLocation::default(),
		)?,
		token_type: if spec.token_type.is_empty() {
			OAuthTokenType::default()
		} else {
			proto_token_type("actor_token.token_type", &spec.token_type)?
		},
		enforce_may_act: spec.enforce_may_act,
	})
}

fn validate_additional_params(
	params: &BTreeMap<String, Arc<cel::Expression>>,
) -> Result<(), String> {
	for key in params.keys() {
		if RESERVED_FORM_PARAMS
			.iter()
			.any(|reserved| reserved.eq_ignore_ascii_case(key))
		{
			return Err(format!(
				"additional parameter {key:?} overrides a reserved OAuth parameter"
			));
		}
	}
	Ok(())
}

fn warn_on_invalid_resources(context: &str, resources: &[String]) {
	for resource in resources {
		if !is_oauth_absolute_uri(resource) {
			warn!(
				resource,
				"{context} resource is not an absolute URI without a fragment; future OAuth 2.1 compliance enforcement may reject it"
			);
		}
	}
}

// RFC 8707 resource identifiers and RFC 8693 token type identifiers must both be
// absolute URIs without a fragment.
fn is_oauth_absolute_uri(value: &str) -> bool {
	url::Url::parse(value)
		.map(|url| url.fragment().is_none())
		.unwrap_or(false)
}

fn warn_on_invalid_scopes(context: &str, scopes: &[String]) {
	for scope in scopes {
		if !is_valid_scope_token(scope) {
			warn!(
				scope,
				"{context} contains an invalid OAuth scope-token; scopes must be non-empty and free of spaces, quotes, backslashes, control characters, and non-ASCII characters; future OAuth 2.1 compliance enforcement may reject it"
			);
		}
	}
}

fn is_valid_scope_token(scope: &str) -> bool {
	!scope.is_empty()
		&& scope
			.bytes()
			.all(|b| b == 0x21 || (0x23..=0x5b).contains(&b) || (0x5d..=0x7e).contains(&b))
}

fn evaluate_additional_params(
	params: &BTreeMap<String, Arc<cel::Expression>>,
	req: &Request,
) -> anyhow::Result<Vec<(String, String)>> {
	let exec = cel::Executor::new_request(req);
	params
		.iter()
		.map(|(k, expr)| {
			let value = exec
				.eval(expr)
				.ok()
				.ok_or_else(|| anyhow::anyhow!("additional parameter {k} CEL evaluation failed"))?;
			let value = value
				.as_str()
				.ok()
				.ok_or_else(|| anyhow::anyhow!("additional parameter {k} did not evaluate to a string"))?
				.into_owned();
			Ok((k.clone(), value))
		})
		.collect()
}

/// Per-request inputs to a token exchange, assembled by the dispatch layer so the
/// exchange itself stays request-free.
#[derive(Clone, Default)]
struct ExchangeRequest {
	subject_token: SecretString,
	subject_token_type: OAuthTokenType,
	/// RFC 8693 delegation actor token and its token type, when configured.
	actor: Option<(SecretString, OAuthTokenType)>,
	extra_params: Vec<(String, String)>,
	chained_extra_params: Vec<(String, String)>,
}

impl ExchangeRequest {
	fn jwt_bearer_assertion(assertion: SecretString, extra_params: Vec<(String, String)>) -> Self {
		Self {
			subject_token: assertion,
			extra_params,
			..Default::default()
		}
	}
}

pub(super) async fn apply_token_exchange(
	inputs: &Arc<crate::ProxyInputs>,
	auth: &OAuthTokenExchangeAuth,
	req: &mut Request,
) -> Result<bool, ProxyError> {
	let client = PolicyClient::new(inputs.clone());

	let access_token = fetch_token(&client, auth, auth.build_exchange_request(req)?)
		.await
		.map_err(FetchError::into_proxy_error)?;

	let explicit = auth.insert_exchanged_token(req, access_token.expose_secret())?;
	trace!("attached oauth exchanged access token");
	Ok(explicit)
}

pub(super) async fn apply_identity_assertion(
	inputs: &Arc<crate::ProxyInputs>,
	auth: &CrossAppAccessAuth,
	req: &mut Request,
) -> Result<bool, ProxyError> {
	let oauth = auth.oauth_token_exchange();
	let client = PolicyClient::new(inputs.clone());

	trace!(audience = %auth.audience(), "performing ID-JAG identity assertion exchange");
	let access_token = fetch_token(&client, oauth, oauth.build_exchange_request(req)?)
		.await
		.map_err(FetchError::into_proxy_error)?;

	let explicit = oauth.insert_exchanged_token(req, access_token.expose_secret())?;
	trace!("attached ID-JAG exchanged access token");
	Ok(explicit)
}

/// Read a subject token for exchange. A JWT auth policy may have already stripped
/// the configured credential after validation, so fall back to populated Claims.
pub(super) fn extract_subject_token(
	source: &AuthorizationLocation,
	req: &Request,
) -> Option<String> {
	source
		.extract(req)
		.map(|token| token.into_owned())
		.filter(|token| !token.trim().is_empty())
		.or_else(|| extract_validated_claims_token(req))
		.filter(|token| !token.trim().is_empty())
}

fn extract_validated_claims_token(req: &Request) -> Option<String> {
	req
		.extensions()
		.get::<Claims>()
		.map(|claims| claims.jwt.expose_secret().to_string())
}

fn actor_token_from_request(
	spec: &ActorTokenSpec,
	req: &Request,
	subject_token: &str,
) -> Result<(SecretString, OAuthTokenType), ProxyError> {
	let token = spec
		.source
		.extract(req)
		.map(|token| token.into_owned())
		.ok_or_else(|| {
			debug!("oauth token exchange actor token missing");
			ProxyError::InvalidRequest
		})?;
	if spec.enforce_may_act && !may_act_authorizes(req, subject_token, &token) {
		debug!("oauth token exchange actor is not authorized by the subject's may_act claim");
		return Err(ProxyError::AuthorizationFailed);
	}
	Ok((SecretString::from(token), spec.token_type.clone()))
}

fn may_act_authorizes(req: &Request, subject_token: &str, actor_token: &str) -> bool {
	let Some(may_act) = subject_may_act_claim(req, subject_token) else {
		return false;
	};
	if may_act.is_empty() {
		// vacuously true otherwise: `all()` over an empty map would authorize any actor.
		return false;
	}
	let Some(actor_claims) = decode_unverified_jwt_claims::<Map<String, Value>>(actor_token) else {
		return false;
	};
	may_act
		.iter()
		.all(|(k, expected)| claim_satisfies(actor_claims.get(k), expected))
}

fn subject_may_act_claim<'a>(
	req: &'a Request,
	subject_token: &str,
) -> Option<Cow<'a, Map<String, Value>>> {
	let validated_claims = req
		.extensions()
		.get::<Claims>()
		.filter(|claims| claims.jwt.expose_secret() == subject_token);
	if let Some(claims) = validated_claims {
		return may_act_claim_from_value(claims.inner.get("may_act")).map(Cow::Borrowed);
	}

	#[derive(serde::Deserialize)]
	struct SubjectMayActClaim {
		may_act: Option<Value>,
	}
	let claims = decode_unverified_jwt_claims::<SubjectMayActClaim>(subject_token)?;
	may_act_claim_from_value(claims.may_act.as_ref()).map(|may_act| Cow::Owned(may_act.clone()))
}

fn may_act_claim_from_value(value: Option<&Value>) -> Option<&Map<String, Value>> {
	match value? {
		Value::Object(may_act) => Some(may_act),
		_ => {
			debug!("oauth token exchange subject may_act claim must be an object");
			None
		},
	}
}

// Decodes JWT-shaped tokens without signature or expiry validation. This is not
// a trust boundary; callers use it only for best-effort local checks.
fn decode_unverified_jwt_claims<T: DeserializeOwned>(token: &str) -> Option<T> {
	jsonwebtoken::dangerous::insecure_decode::<T>(token)
		.ok()
		.map(|decoded| decoded.claims)
}

fn claim_satisfies(actor_value: Option<&Value>, expected: &Value) -> bool {
	match expected {
		Value::Array(allowed) => actor_value.is_some_and(|v| allowed.contains(v)),
		_ => actor_value == Some(expected),
	}
}

fn proto_token_type(field: &str, token_type: &str) -> Result<OAuthTokenType, ProtoError> {
	OAuthTokenType::from_urn(token_type)
		.ok_or_else(|| ProtoError::Generic(format!("unsupported {field} {token_type:?}")))
}

fn proto_requested_token_type(field: &str, token_type: &str) -> Result<OAuthTokenType, ProtoError> {
	OAuthTokenType::supported_requested_from_urn(token_type)
		.ok_or_else(|| ProtoError::Generic(format!("unsupported {field} {token_type:?}")))
}

async fn fetch_token(
	client: &PolicyClient,
	auth: &OAuthTokenExchangeAuth,
	req: ExchangeRequest,
) -> Result<SecretString, FetchError> {
	let result = match auth.cache.as_ref() {
		Some(cache) => {
			cache
				.get_or_insert_with(&req, || fetch_token_uncached(client, auth, &req))
				.await?
		},
		None => {
			let transport::TokenEndpointResponse { access_token, .. } =
				fetch_token_uncached(client, auth, &req).await?;
			TokenCacheResult::Miss(access_token)
		},
	};

	// TODO: export metrics
	match &result {
		TokenCacheResult::Hit(_) => trace!("token exchange cache hit"),
		TokenCacheResult::Miss(_) => trace!("token exchange succeeded"),
	}
	Ok(result.into_token())
}

async fn fetch_token_uncached(
	client: &PolicyClient,
	auth: &OAuthTokenExchangeAuth,
	req: &ExchangeRequest,
) -> Result<transport::TokenEndpointResponse, FetchError> {
	let first =
		transport::request_token(client, &transport::TokenRequestSpec::from(auth), req).await?;
	let Some(chained_exchange) = &auth.chained_exchange else {
		return Ok(first);
	};
	let chained_req =
		ExchangeRequest::jwt_bearer_assertion(first.access_token, req.chained_extra_params.clone());
	transport::request_token(
		client,
		&transport::TokenRequestSpec::from(chained_exchange),
		&chained_req,
	)
	.await
	.map_err(FetchError::chained_exchange)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
