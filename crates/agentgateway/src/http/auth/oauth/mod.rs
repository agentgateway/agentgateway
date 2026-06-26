use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use quick_cache::sync::{EntryAction, EntryResult};
use secrecy::{ExposeSecret, SecretString};
use tracing::{debug, trace};

use super::AuthorizationLocation;
use crate::http::Request;
use crate::http::jwt::Claims;
use crate::http::oauth::{TOKEN_TYPE_ACCESS, supported_oauth_token_type};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::serdes::schema;
use crate::types::agent::SimpleBackendReference;
use crate::types::discovery::NamespacedHostname;
use crate::types::proto::{ProtoError, agent as proto};
use crate::{apply, cel, schema_enum, ser_redact};

mod cache;
mod transport;

use cache::{
	CachedToken, TokenCacheConfig, TokenCacheKey, TokenExchangeCache, cache_expiry,
	cached_token_valid,
};
pub(super) use transport::FetchError;

#[apply(schema!)]
pub struct OAuthTokenExchangeAuth {
	// ----- Token endpoint -----
	/// Backend serving the RFC 8693 token endpoint.
	token_endpoint: Arc<SimpleBackendReference>,
	/// Token endpoint path on the backend; defaults to "/".
	#[serde(default, skip_serializing_if = "String::is_empty")]
	token_endpoint_path: String,
	/// Max time to wait for the token endpoint; defaults to 10s.
	#[serde(
		default = "default_token_endpoint_timeout",
		with = "crate::serdes::serde_dur"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	token_endpoint_timeout: Duration,

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
	/// `requested_token_type` parameter. When unset, the form field is omitted
	/// and a declared response type is expected to be access_token.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	requested_token_type: Option<String>,
	/// Extra form parameters appended to the token request. Values are CEL
	/// expressions evaluated against the incoming request.
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	additional_params: BTreeMap<String, Arc<cel::Expression>>,

	// ----- Authorization server client authentication -----
	/// Client authentication used when calling the token endpoint. When unset,
	/// no client authentication fields are sent.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	client_auth: Option<OAuthClientAuth>,

	// ----- Output and runtime behavior -----
	/// Where to place the exchanged token in the backend request. Defaults to the
	/// Authorization header with a "Bearer " prefix. The CEL `expression` source is
	/// not valid here (it cannot insert).
	#[serde(default, skip_serializing_if = "Option::is_none")]
	authorization_location: Option<AuthorizationLocation>,
	// Runtime cache configuration. Local YAML currently uses internal defaults
	// and does not expose this field.
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	cache: TokenExchangeCache,
}

const TOKEN_ENDPOINT_TIMEOUT: Duration = Duration::from_secs(10);

fn default_token_endpoint_timeout() -> Duration {
	TOKEN_ENDPOINT_TIMEOUT
}

impl OAuthTokenExchangeAuth {
	pub(crate) fn validate_load(&self) -> Result<(), String> {
		if !self.token_endpoint_path.is_empty() && !self.token_endpoint_path.starts_with('/') {
			return Err(format!(
				"token_endpoint_path {} must start with /",
				self.token_endpoint_path
			));
		}
		if self.token_endpoint_timeout.is_zero() {
			return Err("token_endpoint_timeout must be greater than zero".into());
		}
		if self.grant_type == OAuthGrantType::JwtBearer && self.requested_token_type.is_some() {
			return Err("requested_token_type is only valid with the token-exchange grant".into());
		}
		if self.actor_token.is_some() && self.grant_type == OAuthGrantType::JwtBearer {
			return Err("actor_token is only valid with the token-exchange grant".into());
		}
		if let Some(requested) = &self.requested_token_type {
			validate_requested_token_type(requested)?;
		}
		if let Some(client_auth) = &self.client_auth {
			client_auth.validate_load()?;
		}
		for key in self.additional_params.keys() {
			if RESERVED_FORM_PARAMS
				.iter()
				.any(|reserved| reserved.eq_ignore_ascii_case(key))
			{
				return Err(format!(
					"additional parameter {key} overrides a reserved OAuth parameter"
				));
			}
		}
		if matches!(
			self.authorization_location,
			Some(AuthorizationLocation::Expression { .. })
		) {
			return Err("expression auth location is only supported for credential extraction".into());
		}
		if matches!(
			self.authorization_location,
			Some(AuthorizationLocation::QueryParameter { .. })
		) {
			return Err(
				"query-parameter auth location is not supported for token exchange output".into(),
			);
		}
		Ok(())
	}

	fn expected_issued_token_type(&self) -> Option<&str> {
		match self.grant_type {
			OAuthGrantType::TokenExchange => Some(
				self
					.requested_token_type
					.as_deref()
					.unwrap_or(TOKEN_TYPE_ACCESS),
			),
			OAuthGrantType::JwtBearer => None,
		}
	}

	/// Evaluate the configured `additional_params` CEL expressions against the
	/// incoming request. Fails closed if any expression errors or is not a string.
	fn evaluate_additional_params(&self, req: &Request) -> anyhow::Result<Vec<(String, String)>> {
		self
			.additional_params
			.iter()
			.map(|(k, expr)| {
				let exec = cel::Executor::new_request(req);
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

	fn build_exchange_request(&self, req: &Request) -> Result<ExchangeRequest, ProxyError> {
		// Extract everything up front so a bad request fails before we touch it.
		let subject_token =
			extract_exchange_token(&self.subject_token.source, req).ok_or(ProxyError::InvalidRequest)?;
		let actor = self
			.actor_token
			.as_ref()
			.map(|spec| -> Result<_, ProxyError> {
				let token = extract_exchange_token(&spec.source, req).ok_or(ProxyError::InvalidRequest)?;
				Ok((SecretString::from(token), spec.token_type.clone()))
			})
			.transpose()?;
		let extra_params = self.evaluate_additional_params(req).map_err(|e| {
			debug!("oauth token exchange additional parameter evaluation failed: {e}");
			ProxyError::InvalidRequest
		})?;

		Ok(ExchangeRequest {
			subject_token: subject_token.into(),
			subject_token_type: self.subject_token.token_type.clone(),
			actor,
			extra_params,
		})
	}

	fn output_location(&self) -> (&AuthorizationLocation, bool) {
		// TODO(mk): `AppliedBackendAuthLocation::explicit` currently means both
		// "user configured this" and "downstream must not rewrite Authorization".
		// Keep OAuth's old `true` behavior until those meanings are separated.
		let explicit = true;
		let resolved = self
			.authorization_location
			.as_ref()
			.unwrap_or(&super::DEFAULT_AUTHORIZATION_LOCATION);

		(resolved, explicit)
	}

	fn remove_input_tokens(&self, req: &mut Request) -> Result<(), ProxyError> {
		self.subject_token.source.remove(req)?;
		if let Some(actor) = &self.actor_token {
			actor.source.remove(req)?;
		}
		Ok(())
	}
}

#[apply(schema!)]
pub struct OAuthClientAuth {
	/// `client_id` parameter identifying the gateway at the authorization server.
	client_id: String,
	/// Client secret. When absent the client is public, which is only valid with
	/// `ClientSecretPost`.
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		serialize_with = "ser_redact"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	client_secret: Option<SecretString>,
	/// RFC 6749 §2.3 client authentication method.
	#[serde(default)]
	method: OAuthClientAuthMethod,
}

impl OAuthClientAuth {
	pub fn new(
		client_id: String,
		client_secret: Option<SecretString>,
		method: OAuthClientAuthMethod,
	) -> Self {
		Self {
			client_id,
			client_secret,
			method,
		}
	}

	fn validate_load(&self) -> Result<(), String> {
		if self.client_id.is_empty() {
			return Err("oauth token exchange client_id must not be empty".into());
		}
		if self
			.client_secret
			.as_ref()
			.is_some_and(|secret| secret.expose_secret().is_empty())
		{
			return Err("oauth token exchange client_secret must not be empty".into());
		}
		if self.client_secret.is_none() && self.method == OAuthClientAuthMethod::ClientSecretBasic {
			return Err(
				"oauth token exchange client_secret is required with the client_secret_basic method".into(),
			);
		}
		Ok(())
	}
}

impl TryFrom<proto::o_auth_token_exchange::ClientAuth> for OAuthClientAuth {
	type Error = ProtoError;

	fn try_from(c: proto::o_auth_token_exchange::ClientAuth) -> Result<Self, Self::Error> {
		use proto::o_auth_token_exchange::client_auth::Method;
		let method = match Method::try_from(c.method) {
			Ok(Method::Unspecified | Method::ClientSecretBasic) => {
				OAuthClientAuthMethod::ClientSecretBasic
			},
			Ok(Method::ClientSecretPost) => OAuthClientAuthMethod::ClientSecretPost,
			Err(_) => {
				return Err(ProtoError::EnumParse(
					"unknown oauth client auth method".into(),
				));
			},
		};
		let auth = Self {
			client_id: c.client_id,
			client_secret: c.client_secret.map(Into::into),
			method,
		};
		auth.validate_load().map_err(ProtoError::Generic)?;
		Ok(auth)
	}
}

#[apply(schema_enum!)]
#[derive(Default)]
pub enum OAuthClientAuthMethod {
	/// `client_id`/`client_secret` sent in the HTTP Basic Authorization header (RFC 6749 §2.3.1).
	#[default]
	ClientSecretBasic,
	/// `client_id`/`client_secret` sent in the request form body.
	ClientSecretPost,
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

#[apply(schema!)]
pub struct TokenSpec {
	/// Where the token is read from in the incoming request. The CEL `expression`
	/// source is permitted (extraction only).
	#[serde(default)]
	source: AuthorizationLocation,
	/// RFC 8693 token type URN; empty defaults to access_token.
	#[serde(default = "default_token_type")]
	token_type: String,
}

#[apply(schema!)]
pub struct ActorTokenSpec {
	/// Where the actor token is read from in the incoming request. The CEL
	/// `expression` source is permitted (extraction only). Unlike subject tokens,
	/// actor tokens have no default source.
	source: AuthorizationLocation,
	/// RFC 8693 actor token type URN; empty defaults to access_token and is still sent.
	#[serde(default = "default_token_type")]
	token_type: String,
}

impl Default for TokenSpec {
	fn default() -> Self {
		Self {
			source: AuthorizationLocation::default(),
			token_type: default_token_type(),
		}
	}
}

fn default_token_type() -> String {
	TOKEN_TYPE_ACCESS.to_string()
}

fn positive_duration_from_proto(
	field: &str,
	d: prost_types::Duration,
) -> Result<Duration, ProtoError> {
	if d.seconds < 0 || d.nanos < 0 {
		return Err(ProtoError::Generic(format!("{field} must not be negative")));
	}
	if d.nanos >= 1_000_000_000 {
		return Err(ProtoError::Generic(format!(
			"{field} nanos must be less than 1000000000"
		)));
	}
	let duration = Duration::from_secs(d.seconds as u64) + Duration::from_nanos(d.nanos as u64);
	if duration.is_zero() {
		return Err(ProtoError::Generic(format!(
			"{field} must be greater than zero"
		)));
	}
	Ok(duration)
}

fn token_cache_config_from_proto(
	cache: Option<proto::o_auth_token_exchange::TokenCache>,
) -> Result<TokenCacheConfig, ProtoError> {
	use proto::o_auth_token_exchange::token_cache::Backend;

	let defaults = TokenCacheConfig::default();
	let Some(cache) = cache else {
		return Ok(defaults);
	};

	match cache.backend {
		Some(Backend::Disabled(_)) => Ok(TokenCacheConfig {
			enabled: false,
			..defaults
		}),
		Some(Backend::InMemory(in_memory)) => Ok(TokenCacheConfig {
			max_entries: match in_memory.max_entries {
				Some(0) | None => defaults.max_entries,
				Some(max_entries) => max_entries as usize,
			},
			default_ttl: in_memory
				.default_ttl
				.map(|d| positive_duration_from_proto("cache.in_memory.default_ttl", d))
				.transpose()?
				.unwrap_or(defaults.default_ttl),
			..defaults
		}),
		None => Ok(defaults),
	}
}

fn token_source_from_proto(
	loc: Option<proto::AuthorizationLocation>,
) -> Result<AuthorizationLocation, ProtoError> {
	use proto::authorization_location::Kind;
	let Some(loc) = loc else {
		return Ok(AuthorizationLocation::default());
	};
	Ok(match loc.kind {
		Some(Kind::Header(h)) => AuthorizationLocation::Header {
			name: h
				.name
				.parse()
				.map_err(|e| ProtoError::Generic(format!("invalid header name {}: {e}", h.name)))?,
			prefix: h.prefix.map(Into::into),
		},
		Some(Kind::QueryParameter(q)) => AuthorizationLocation::QueryParameter {
			name: q.name.into(),
		},
		Some(Kind::Cookie(c)) => AuthorizationLocation::Cookie {
			name: c.name.into(),
		},
		Some(Kind::Expression(expression)) => {
			// TODO(mk): hard-errors on invalid CEL; see additional_params for rationale.
			let (expr, err) = cel::Expression::new_permissive(expression);
			if let Some(err) = err {
				return Err(ProtoError::Generic(format!(
					"invalid CEL expression for token source: {err}"
				)));
			}
			AuthorizationLocation::Expression {
				expression: Arc::new(expr),
			}
		},
		None => AuthorizationLocation::default(),
	})
}

fn token_spec_from_proto(
	spec: proto::o_auth_token_exchange::TokenSpec,
) -> Result<TokenSpec, ProtoError> {
	let token_type = if spec.token_type.is_empty() {
		default_token_type()
	} else {
		spec.token_type
	};
	Ok(TokenSpec {
		source: token_source_from_proto(spec.source)?,
		token_type,
	})
}

fn actor_token_from_proto(
	spec: proto::o_auth_token_exchange::ActorToken,
) -> Result<ActorTokenSpec, ProtoError> {
	let Some(source) = spec.source else {
		return Err(ProtoError::Generic(
			"oauth token exchange actor_token.source must be set".into(),
		));
	};
	let token_type = if spec.token_type.is_empty() {
		default_token_type()
	} else {
		spec.token_type
	};
	Ok(ActorTokenSpec {
		source: token_source_from_proto(Some(source))?,
		token_type,
	})
}

fn token_endpoint_from_proto(target: Option<proto::BackendReference>) -> SimpleBackendReference {
	let Some(target) = target else {
		return SimpleBackendReference::Invalid;
	};

	match target.kind {
		None => SimpleBackendReference::Invalid,
		Some(proto::backend_reference::Kind::Service(svc)) => {
			let name = NamespacedHostname {
				namespace: svc.namespace.into(),
				hostname: svc.hostname.into(),
			};
			SimpleBackendReference::Service {
				name,
				port: target.port as u16,
			}
		},
		Some(proto::backend_reference::Kind::Backend(name)) => {
			SimpleBackendReference::Backend((&name).into())
		},
	}
}

fn output_location_from_proto(
	location: Option<proto::AuthorizationLocation>,
) -> Result<Option<AuthorizationLocation>, ProtoError> {
	use proto::authorization_location::Kind;

	let Some(location) = location else {
		return Ok(None);
	};

	match location.kind {
		Some(Kind::Header(header)) => Ok(Some(AuthorizationLocation::Header {
			name: header.name.parse()?,
			prefix: header.prefix.map(Into::into),
		})),
		Some(Kind::QueryParameter(query)) => Ok(Some(AuthorizationLocation::QueryParameter {
			name: query.name.into(),
		})),
		Some(Kind::Cookie(cookie)) => Ok(Some(AuthorizationLocation::Cookie {
			name: cookie.name.into(),
		})),
		Some(Kind::Expression(_)) => Err(ProtoError::Generic(
			"expression auth location is only supported for credential extraction".into(),
		)),
		None => Ok(None),
	}
}

impl TryFrom<proto::OAuthTokenExchange> for OAuthTokenExchangeAuth {
	type Error = ProtoError;

	fn try_from(t: proto::OAuthTokenExchange) -> Result<Self, Self::Error> {
		use proto::o_auth_token_exchange::GrantType;
		let opt = |s: String| (!s.is_empty()).then_some(s);

		let grant_type = match GrantType::try_from(t.grant_type) {
			Ok(GrantType::Unspecified | GrantType::TokenExchange) => OAuthGrantType::TokenExchange,
			Ok(GrantType::JwtBearer) => OAuthGrantType::JwtBearer,
			Err(_) => return Err(ProtoError::EnumParse("unknown oauth grant type".into())),
		};
		let requested_token_type = t.requested_token_type.and_then(opt);

		let subject_token = t
			.subject_token
			.map(token_spec_from_proto)
			.transpose()?
			.unwrap_or_default();
		let actor_token = t.actor_token.map(actor_token_from_proto).transpose()?;

		let token_endpoint_timeout = t
			.token_endpoint_timeout
			.map(|d| positive_duration_from_proto("token_endpoint_timeout", d))
			.transpose()?
			.unwrap_or(TOKEN_ENDPOINT_TIMEOUT);

		let token_endpoint_path = t.token_endpoint_path.unwrap_or_default();

		let additional_params = t
			.additional_params
			.into_iter()
			.map(|(k, v)| {
				// TODO(mk): token exchange hard-errors on invalid CEL, unlike the rest of
				// the codebase which compiles permissively (always-fails expr + a warning).
				// Revisit if xDS-push resilience is preferred over fail-fast here.
				let (expr, err) = cel::Expression::new_permissive(v);
				if let Some(err) = err {
					return Err(ProtoError::Generic(format!(
						"invalid CEL expression for additional parameter {k}: {err}"
					)));
				}
				Ok((k, Arc::new(expr)))
			})
			.collect::<Result<BTreeMap<_, _>, _>>()?;

		let cache_config = token_cache_config_from_proto(t.cache)?;

		let auth = Self {
			token_endpoint: Arc::new(token_endpoint_from_proto(t.token_endpoint)),
			token_endpoint_path,
			token_endpoint_timeout,
			grant_type,
			subject_token,
			actor_token,
			audiences: t.audiences,
			scopes: t.scopes,
			resources: t.resources,
			requested_token_type,
			client_auth: t.client_auth.map(OAuthClientAuth::try_from).transpose()?,
			additional_params,
			authorization_location: output_location_from_proto(t.authorization_location)?,
			cache: TokenExchangeCache::new(&cache_config),
		};
		auth.validate_load().map_err(ProtoError::Generic)?;
		Ok(auth)
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

/// Per-request inputs to a token exchange, assembled by the dispatch layer so the
/// exchange itself stays request-free.
struct ExchangeRequest {
	subject_token: SecretString,
	subject_token_type: String,
	/// RFC 8693 delegation actor token and its token type, when configured.
	actor: Option<(SecretString, String)>,
	extra_params: Vec<(String, String)>,
}

pub(super) async fn apply_token_exchange(
	inputs: &Arc<crate::ProxyInputs>,
	auth: &OAuthTokenExchangeAuth,
	req: &mut Request,
) -> Result<bool, ProxyError> {
	let client = PolicyClient::new(inputs.clone());
	let exchange = auth.build_exchange_request(req)?;
	let access_token = fetch_token(&client, auth, &exchange)
		.await
		.map_err(map_fetch_error)?;
	let (output_location, explicit) = auth.output_location();

	// Only remove the incoming credentials after the exchange succeeds.
	auth.remove_input_tokens(req)?;
	output_location.insert(req, access_token.expose_secret())?;

	Ok(explicit)
}

/// Read a token for exchange from its configured source. For an Authorization
/// Bearer source, a JWT auth policy may have already stripped the header after
/// validation, so fall back to the populated Claims extension.
pub(super) fn extract_exchange_token(
	source: &AuthorizationLocation,
	req: &Request,
) -> Option<String> {
	source
		.extract(req)
		.map(|token| token.into_owned())
		.or_else(|| extract_bearer_claims_token(source, req))
}

fn extract_bearer_claims_token(source: &AuthorizationLocation, req: &Request) -> Option<String> {
	let AuthorizationLocation::Header { name, prefix } = source else {
		return None;
	};
	if *name != http::header::AUTHORIZATION || !prefix.as_ref()?.eq_ignore_ascii_case("Bearer ") {
		return None;
	}

	req
		.extensions()
		.get::<Claims>()
		.map(|claims| claims.jwt.expose_secret().to_string())
}

fn map_fetch_error(err: FetchError) -> ProxyError {
	match err {
		FetchError::Client(_) => {
			// The authorization server rejected the request/subject token; surface
			// as a client error (4xx), not a gateway fault. Keep proxy logs terse here:
			// the upstream error body may contain provider-specific detail.
			debug!("oauth token exchange rejected by authorization server");
			ProxyError::InvalidRequest
		},
		FetchError::Upstream(e) => ProxyError::BackendAuthenticationFailed(e),
	}
}

fn validate_requested_token_type(requested: &str) -> Result<(), String> {
	if supported_oauth_token_type(requested) {
		return Ok(());
	}
	Err(format!(
		"unsupported requested_token_type {requested}; supported values are {TOKEN_TYPE_ACCESS}, {}, and {}",
		crate::http::oauth::TOKEN_TYPE_JWT,
		crate::http::oauth::TOKEN_TYPE_ID,
	))
}

async fn fetch_token(
	client: &PolicyClient,
	auth: &OAuthTokenExchangeAuth,
	request: &ExchangeRequest,
) -> Result<SecretString, FetchError> {
	let subject_token = request.subject_token.expose_secret();
	let cache_key = TokenCacheKey::new(request);
	let cache = auth.cache.entries.as_ref();
	let guard = match cache {
		Some(cache) => {
			// Retain a fresh cached token, or atomically replace a stale one with a
			// guard so only one request refreshes this exchange.
			match cache
				.entry_async(&cache_key, |_key, cached| {
					if cached_token_valid(cached, SystemTime::now()) {
						EntryAction::Retain(cached.access_token.clone())
					} else {
						EntryAction::ReplaceWithGuard
					}
				})
				.await
			{
				EntryResult::Retained(access_token) => {
					trace!("token exchange cache hit");
					return Ok(access_token);
				},
				EntryResult::Vacant(guard) | EntryResult::Replaced(guard, _) => Some(guard),
				EntryResult::Removed(_, _) | EntryResult::Timeout => unreachable!(),
			}
		},
		None => None,
	};

	let response = transport::request_token(client, auth, request).await?;
	let access_token = response.access_token;

	if response.expires_in.is_none() {
		trace!(
			"token exchange response omitted expires_in; skipping cache insert despite configured default_ttl {:?}",
			auth.cache.default_ttl
		);
	}

	if let Some(guard) = guard
		&& let Some(expires_at) = cache_expiry(response.expires_in, subject_token)
	{
		let _ = guard.insert(CachedToken {
			access_token: access_token.clone(),
			expires_at,
		});
	}

	trace!("token exchange succeeded");
	Ok(access_token)
}

#[cfg(test)]
mod tests {
	use std::collections::HashMap;

	use base64::Engine;
	use base64::prelude::BASE64_STANDARD;
	use rstest::rstest;
	use secrecy::ExposeSecret;
	use serde_json::json;
	use url::form_urlencoded;
	use wiremock::matchers::{method, path};
	use wiremock::{Mock, MockServer, ResponseTemplate};

	use super::*;
	use crate::http::Body;
	use crate::http::oauth::{
		GRANT_TYPE_JWT_BEARER, GRANT_TYPE_TOKEN_EXCHANGE, TOKEN_TYPE_ID, TOKEN_TYPE_JWT,
	};
	use crate::types::agent::Target;

	fn policy_client() -> PolicyClient {
		PolicyClient::new(
			crate::test_helpers::proxymock::setup_proxy_test("{}")
				.unwrap()
				.inputs(),
		)
	}

	fn token_body() -> serde_json::Value {
		json!({
			"access_token": "upstream-token",
			"token_type": "Bearer",
			"issued_token_type": TOKEN_TYPE_ACCESS,
			"expires_in": 3600,
		})
	}

	fn token_body_without_expiry() -> serde_json::Value {
		json!({
			"access_token": "upstream-token",
			"token_type": "Bearer",
			"issued_token_type": TOKEN_TYPE_ACCESS,
		})
	}

	async fn mock_token_endpoint(body: ResponseTemplate) -> MockServer {
		let mock = MockServer::start().await;
		Mock::given(method("POST"))
			.and(path("/token"))
			.respond_with(body)
			.mount(&mock)
			.await;
		mock
	}

	fn endpoint(mock: &MockServer) -> Arc<SimpleBackendReference> {
		Arc::new(SimpleBackendReference::InlineBackend(Target::Address(
			*mock.address(),
		)))
	}

	fn base_auth(endpoint: Arc<SimpleBackendReference>) -> OAuthTokenExchangeAuth {
		OAuthTokenExchangeAuth {
			token_endpoint: endpoint,
			token_endpoint_path: "/token".into(),
			token_endpoint_timeout: TOKEN_ENDPOINT_TIMEOUT,
			grant_type: OAuthGrantType::TokenExchange,
			subject_token: TokenSpec::default(),
			actor_token: None,
			audiences: vec![],
			scopes: vec![],
			resources: vec![],
			requested_token_type: None,
			client_auth: None,
			additional_params: BTreeMap::new(),
			authorization_location: None,
			cache: TokenExchangeCache::default(),
		}
	}

	fn auth(endpoint: Arc<SimpleBackendReference>) -> OAuthTokenExchangeAuth {
		OAuthTokenExchangeAuth {
			audiences: vec!["https://upstream.example".into()],
			..base_auth(endpoint)
		}
	}

	fn exchange(subject: &str) -> ExchangeRequest {
		exchange_typed(subject, TOKEN_TYPE_ACCESS)
	}

	fn exchange_typed(subject: &str, token_type: &str) -> ExchangeRequest {
		ExchangeRequest {
			subject_token: subject.to_string().into(),
			subject_token_type: token_type.to_string(),
			actor: None,
			extra_params: vec![],
		}
	}

	fn backend_info() -> crate::http::auth::BackendInfo {
		crate::http::auth::BackendInfo {
			target: crate::types::agent::BackendTarget::Invalid,
			call_target: Target::Hostname(crate::strng::new("unused"), 0),
			inputs: crate::test_helpers::proxymock::setup_proxy_test("{}")
				.unwrap()
				.inputs(),
		}
	}

	fn incoming_request() -> crate::http::Request {
		::http::Request::builder()
			.method(::http::Method::GET)
			.uri("http://upstream/")
			.header(::http::header::AUTHORIZATION, "Bearer subj")
			.body(Body::empty())
			.unwrap()
	}

	async fn sent_form_params(mock: &MockServer) -> HashMap<String, String> {
		let req = &mock.received_requests().await.unwrap()[0];
		form_urlencoded::parse(&req.body).into_owned().collect()
	}

	fn assert_proto_err_contains(proto: proto::OAuthTokenExchange, expected: &str) {
		let err = OAuthTokenExchangeAuth::try_from(proto).unwrap_err();
		assert!(
			matches!(err, ProtoError::Generic(ref m) if m.contains(expected)),
			"expected error containing {expected:?}, got {err:?}"
		);
	}

	#[test]
	fn deserializes_minimal_config() {
		let a: OAuthTokenExchangeAuth =
			serde_json::from_str(r#"{"tokenEndpoint": {"host": "localhost:8089"}}"#).unwrap();
		assert!(matches!(
			a.token_endpoint.as_ref(),
			SimpleBackendReference::InlineBackend(_)
		));
		assert!(a.token_endpoint_path.is_empty());
	}

	#[tokio::test]
	async fn sends_form_params() {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
		let a = auth(endpoint(&mock));

		let tok = fetch_token(&policy_client(), &a, &exchange("subj-jwt"))
			.await
			.expect("exchange succeeds");
		assert_eq!(tok.expose_secret(), "upstream-token");

		let pairs = sent_form_params(&mock).await;
		assert_eq!(pairs["grant_type"], GRANT_TYPE_TOKEN_EXCHANGE);
		assert_eq!(pairs["subject_token"], "subj-jwt");
		assert_eq!(pairs["subject_token_type"], TOKEN_TYPE_ACCESS);
		assert_eq!(pairs["audience"], "https://upstream.example");
		for k in ["scope", "resource", "requested_token_type", "client_id"] {
			assert!(!pairs.contains_key(k), "unset param {k} must not be sent");
		}
	}

	#[tokio::test]
	async fn sends_optional_params() {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
		let a = OAuthTokenExchangeAuth {
			scopes: vec!["read".into(), "write".into()],
			resources: vec!["https://upstream.example/api".into()],
			requested_token_type: Some(TOKEN_TYPE_ACCESS.into()),
			client_auth: Some(OAuthClientAuth::new(
				"gateway-client".into(),
				None,
				OAuthClientAuthMethod::ClientSecretPost,
			)),
			..base_auth(endpoint(&mock))
		};

		fetch_token(
			&policy_client(),
			&a,
			&exchange_typed("subj", TOKEN_TYPE_JWT),
		)
		.await
		.unwrap();
		let pairs = sent_form_params(&mock).await;
		assert!(!pairs.contains_key("audience"));
		assert_eq!(pairs["subject_token_type"], TOKEN_TYPE_JWT);
		assert_eq!(pairs["scope"], "read write");
		assert_eq!(pairs["resource"], "https://upstream.example/api");
		assert_eq!(pairs["requested_token_type"], TOKEN_TYPE_ACCESS);
		assert_eq!(pairs["client_id"], "gateway-client");
		assert!(
			!pairs.contains_key("client_secret"),
			"public client sends no secret"
		);
	}

	#[tokio::test]
	async fn sends_google_sts_workload_identity_form_without_authorization_header() {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
		let audience = "//iam.googleapis.com/projects/123456789012/locations/global/workloadIdentityPools/pool/providers/provider";
		let a = OAuthTokenExchangeAuth {
			audiences: vec![audience.into()],
			scopes: vec!["https://www.googleapis.com/auth/cloud-platform".into()],
			requested_token_type: Some(TOKEN_TYPE_ACCESS.into()),
			..base_auth(endpoint(&mock))
		};

		fetch_token(
			&policy_client(),
			&a,
			&exchange_typed("external-id-token", TOKEN_TYPE_ID),
		)
		.await
		.unwrap();

		let req = &mock.received_requests().await.unwrap()[0];
		assert!(
			req.headers.get("authorization").is_none(),
			"Google STS requests should not send client auth when client_auth is unset"
		);
		let pairs = sent_form_params(&mock).await;
		assert_eq!(pairs["grant_type"], GRANT_TYPE_TOKEN_EXCHANGE);
		assert_eq!(pairs["audience"], audience);
		assert_eq!(
			pairs["scope"],
			"https://www.googleapis.com/auth/cloud-platform"
		);
		assert_eq!(pairs["requested_token_type"], TOKEN_TYPE_ACCESS);
		assert_eq!(pairs["subject_token"], "external-id-token");
		assert_eq!(pairs["subject_token_type"], TOKEN_TYPE_ID);
	}

	#[rstest]
	#[case(TOKEN_TYPE_JWT, "upstream-jwt")]
	#[case(TOKEN_TYPE_ID, "upstream-id-token")]
	#[tokio::test]
	async fn accepts_requested_response_type(
		#[case] requested_token_type: &str,
		#[case] access_token: &str,
	) {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(json!({
			"access_token": access_token,
			"token_type": "Bearer",
			"issued_token_type": requested_token_type,
		})))
		.await;
		let a = OAuthTokenExchangeAuth {
			requested_token_type: Some(requested_token_type.into()),
			..base_auth(endpoint(&mock))
		};

		let tok = fetch_token(&policy_client(), &a, &exchange("subj"))
			.await
			.expect("requested response type should be accepted");
		assert_eq!(tok.expose_secret(), access_token);
	}

	#[tokio::test]
	async fn client_secret_basic_uses_authorization_header() {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
		let a = OAuthTokenExchangeAuth {
			client_auth: Some(OAuthClientAuth::new(
				"gw client".into(),
				Some("s3cr3t".into()),
				OAuthClientAuthMethod::ClientSecretBasic,
			)),
			..base_auth(endpoint(&mock))
		};

		fetch_token(&policy_client(), &a, &exchange("subj"))
			.await
			.unwrap();

		let req = &mock.received_requests().await.unwrap()[0];
		let header = req.headers["authorization"].to_str().unwrap();
		assert_eq!(
			header,
			format!("Basic {}", BASE64_STANDARD.encode("gw+client:s3cr3t"))
		);
		let pairs = sent_form_params(&mock).await;
		assert!(
			!pairs.contains_key("client_id"),
			"basic auth keeps creds out of the body"
		);
		assert!(!pairs.contains_key("client_secret"));
	}

	#[tokio::test]
	async fn client_secret_post_uses_form_body() {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
		let a = OAuthTokenExchangeAuth {
			client_auth: Some(OAuthClientAuth::new(
				"gateway-client".into(),
				Some("s3cr3t".into()),
				OAuthClientAuthMethod::ClientSecretPost,
			)),
			..base_auth(endpoint(&mock))
		};

		fetch_token(&policy_client(), &a, &exchange("subj"))
			.await
			.unwrap();

		let req = &mock.received_requests().await.unwrap()[0];
		assert!(req.headers.get("authorization").is_none());
		let pairs = sent_form_params(&mock).await;
		assert_eq!(pairs["client_id"], "gateway-client");
		assert_eq!(pairs["client_secret"], "s3cr3t");
	}

	#[tokio::test]
	async fn jwt_bearer_sends_assertion() {
		// RFC 7523 response: a plain RFC 6749 body with no issued_token_type.
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(json!({
			"access_token": "upstream-token",
			"token_type": "Bearer",
		})))
		.await;
		let a = OAuthTokenExchangeAuth {
			grant_type: OAuthGrantType::JwtBearer,
			..base_auth(endpoint(&mock))
		};

		let tok = fetch_token(&policy_client(), &a, &exchange("the-jwt"))
			.await
			.expect("jwt-bearer exchange succeeds");
		assert_eq!(tok.expose_secret(), "upstream-token");

		let pairs = sent_form_params(&mock).await;
		assert_eq!(pairs["grant_type"], GRANT_TYPE_JWT_BEARER);
		assert_eq!(pairs["assertion"], "the-jwt");
		for k in [
			"subject_token",
			"subject_token_type",
			"requested_token_type",
		] {
			assert!(!pairs.contains_key(k), "jwt-bearer must not send {k}");
		}
	}

	#[rstest]
	#[case::missing_token_type(
		json!({
			"access_token": "upstream-token",
			"issued_token_type": TOKEN_TYPE_ACCESS,
			"expires_in": 3600,
		}),
		"missing token_type"
	)]
	#[case::empty_access_token(
		json!({
			"access_token": "",
			"token_type": "Bearer",
			"issued_token_type": TOKEN_TYPE_ACCESS,
			"expires_in": 3600,
		}),
		"empty access_token"
	)]
	#[tokio::test]
	async fn rejects_invalid_token_response(
		#[case] response_body: serde_json::Value,
		#[case] expected: &str,
	) {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(response_body)).await;
		let a = auth(endpoint(&mock));

		let err = fetch_token(&policy_client(), &a, &exchange("subj"))
			.await
			.unwrap_err();
		assert!(err.to_string().contains(expected), "got: {err}");
	}

	#[tokio::test]
	async fn rejects_unusable_issued_token_type() {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(json!({
			"access_token": "t",
			"token_type": "Bearer",
			"issued_token_type": "urn:ietf:params:oauth:token-type:saml2",
		})))
		.await;
		let a = OAuthTokenExchangeAuth {
			grant_type: OAuthGrantType::JwtBearer,
			..base_auth(endpoint(&mock))
		};

		let err = fetch_token(&policy_client(), &a, &exchange("subj"))
			.await
			.unwrap_err();
		assert!(
			err.to_string().contains("unusable issued_token_type"),
			"got: {err}"
		);
	}

	#[tokio::test]
	async fn fails_closed_on_slow_endpoint() {
		let mock = mock_token_endpoint(
			ResponseTemplate::new(200)
				.set_body_json(token_body())
				.set_delay(Duration::from_secs(2)),
		)
		.await;
		let a = OAuthTokenExchangeAuth {
			token_endpoint_timeout: Duration::from_millis(50),
			..base_auth(endpoint(&mock))
		};

		let err = fetch_token(&policy_client(), &a, &exchange("subj"))
			.await
			.unwrap_err();
		assert!(err.to_string().contains("timed out"), "got: {err}");
	}

	#[rstest]
	#[case(400, true)]
	#[case(503, false)]
	#[tokio::test]
	async fn maps_error_status_by_class(#[case] status: u16, #[case] expect_client_error: bool) {
		let response = if expect_client_error {
			ResponseTemplate::new(status).set_body_string(r#"{"error":"invalid_grant"}"#)
		} else {
			ResponseTemplate::new(status)
		};
		let mock = mock_token_endpoint(response).await;
		let a = auth(endpoint(&mock));

		let err = fetch_token(&policy_client(), &a, &exchange("subj"))
			.await
			.unwrap_err();
		if expect_client_error {
			assert!(matches!(err, FetchError::Client(_)), "got: {err:?}");
		} else {
			assert!(matches!(err, FetchError::Upstream(_)), "got: {err:?}");
		}
	}

	#[tokio::test]
	async fn rejects_issued_token_type_that_does_not_match_request() {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(json!({
			"access_token": "t",
			"token_type": "Bearer",
			"issued_token_type": TOKEN_TYPE_ACCESS,
		})))
		.await;
		let a = OAuthTokenExchangeAuth {
			requested_token_type: Some(TOKEN_TYPE_JWT.into()),
			..auth(endpoint(&mock))
		};

		let err = fetch_token(&policy_client(), &a, &exchange("subj"))
			.await
			.unwrap_err();
		assert!(
			err.to_string().contains("expected"),
			"got unexpected error: {err}"
		);
	}

	#[tokio::test]
	async fn token_exchange_without_requested_type_defaults_to_access_token() {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(json!({
			"access_token": "t",
			"token_type": "Bearer",
			"issued_token_type": TOKEN_TYPE_JWT,
		})))
		.await;
		let a = auth(endpoint(&mock));

		let err = fetch_token(&policy_client(), &a, &exchange("subj"))
			.await
			.unwrap_err();
		assert!(
			err.to_string().contains(TOKEN_TYPE_ACCESS),
			"got unexpected error: {err}"
		);
	}

	#[tokio::test]
	async fn caches_per_subject() {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
		let a = auth(endpoint(&mock));
		let client = policy_client();

		let t1 = fetch_token(&client, &a, &exchange("subj")).await.unwrap();
		let t2 = fetch_token(&client, &a, &exchange("subj")).await.unwrap();
		assert_eq!(t1.expose_secret(), t2.expose_secret());
		assert_eq!(mock.received_requests().await.unwrap().len(), 1);
	}

	#[tokio::test]
	async fn response_without_expires_in_is_not_cached() {
		let mock =
			mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body_without_expiry()))
				.await;
		let a = OAuthTokenExchangeAuth {
			cache: TokenExchangeCache::new(&TokenCacheConfig {
				default_ttl: Duration::from_secs(120),
				..Default::default()
			}),
			..auth(endpoint(&mock))
		};
		let client = policy_client();

		let t1 = fetch_token(&client, &a, &exchange("subj")).await.unwrap();
		let t2 = fetch_token(&client, &a, &exchange("subj")).await.unwrap();
		assert_eq!(t1.expose_secret(), t2.expose_secret());
		assert_eq!(mock.received_requests().await.unwrap().len(), 2);
	}

	#[tokio::test]
	async fn disabled_cache_always_misses() {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
		let a = OAuthTokenExchangeAuth {
			cache: TokenExchangeCache::new(&TokenCacheConfig {
				enabled: false,
				..Default::default()
			}),
			..auth(endpoint(&mock))
		};
		let client = policy_client();

		let t1 = fetch_token(&client, &a, &exchange("subj")).await.unwrap();
		let t2 = fetch_token(&client, &a, &exchange("subj")).await.unwrap();
		assert_eq!(t1.expose_secret(), t2.expose_secret());
		assert_eq!(mock.received_requests().await.unwrap().len(), 2);
	}

	#[tokio::test]
	async fn expired_subject_token_is_not_cached() {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
		let a = auth(endpoint(&mock));
		let client = policy_client();
		let expired_subject = {
			let header = base64::prelude::BASE64_URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT"}"#);
			let body = base64::prelude::BASE64_URL_SAFE_NO_PAD.encode(
				format!(
					r#"{{"exp":{}}}"#,
					std::time::SystemTime::now()
						.duration_since(std::time::UNIX_EPOCH)
						.unwrap()
						.as_secs()
						.saturating_sub(10)
				)
				.as_bytes(),
			);
			format!("{header}.{body}.")
		};

		let t1 = fetch_token(&client, &a, &exchange(&expired_subject))
			.await
			.unwrap();
		let t2 = fetch_token(&client, &a, &exchange(&expired_subject))
			.await
			.unwrap();
		assert_eq!(t1.expose_secret(), t2.expose_secret());
		assert_eq!(mock.received_requests().await.unwrap().len(), 2);
	}

	#[tokio::test]
	async fn appends_additional_params() {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
		let a = auth(endpoint(&mock));
		let request = ExchangeRequest {
			subject_token: "subj".to_string().into(),
			subject_token_type: TOKEN_TYPE_ACCESS.to_string(),
			actor: None,
			extra_params: vec![
				("vendor_id".into(), "v1".into()),
				("org".into(), "o2".into()),
			],
		};

		fetch_token(&policy_client(), &a, &request).await.unwrap();

		let pairs = sent_form_params(&mock).await;
		assert_eq!(pairs["vendor_id"], "v1");
		assert_eq!(pairs["org"], "o2");
	}

	#[test]
	fn evaluates_additional_params() {
		let (expr, err) = cel::Expression::new_permissive("\"static-value\"".to_string());
		assert!(err.is_none(), "{err:?}");
		let a = OAuthTokenExchangeAuth {
			additional_params: BTreeMap::from([("p".to_string(), Arc::new(expr))]),
			..base_auth(Arc::new(SimpleBackendReference::Invalid))
		};
		let req = ::http::Request::builder()
			.method(::http::Method::GET)
			.uri("http://example/")
			.body(Body::empty())
			.unwrap();

		let params = a.evaluate_additional_params(&req).unwrap();
		assert_eq!(params, vec![("p".to_string(), "static-value".to_string())]);
	}

	#[test]
	fn rejects_reserved_additional_param() {
		let proto = proto::OAuthTokenExchange {
			additional_params: std::collections::HashMap::from([(
				"client_assertion".to_string(),
				"x".to_string(),
			)]),
			..Default::default()
		};
		let err = OAuthTokenExchangeAuth::try_from(proto).unwrap_err();
		assert!(
			matches!(err, ProtoError::Generic(ref m) if m.contains("reserved")),
			"got: {err:?}"
		);
	}

	#[test]
	fn rejects_invalid_cel_additional_param() {
		let proto = proto::OAuthTokenExchange {
			additional_params: HashMap::from([("p".to_string(), "((".to_string())]),
			..Default::default()
		};
		let err = OAuthTokenExchangeAuth::try_from(proto).unwrap_err();
		assert!(
			matches!(err, ProtoError::Generic(ref m) if m.contains("CEL")),
			"got: {err:?}"
		);
	}

	fn assert_load_err(auth: OAuthTokenExchangeAuth, expected: &str) {
		let err = auth
			.validate_load()
			.expect_err("invalid local config should fail validation");
		assert!(
			err.contains(expected),
			"expected error containing {expected:?}, got {err:?}"
		);
	}

	#[rstest]
	#[case::token_endpoint_path(
		OAuthTokenExchangeAuth {
			token_endpoint_path: "token".into(),
			..base_auth(Arc::new(SimpleBackendReference::Invalid))
		},
		"must start with /"
	)]
	#[case::zero_timeout(
		OAuthTokenExchangeAuth {
			token_endpoint_timeout: Duration::ZERO,
			..base_auth(Arc::new(SimpleBackendReference::Invalid))
		},
		"greater than zero"
	)]
	#[case::jwt_bearer_actor_token(
		OAuthTokenExchangeAuth {
			grant_type: OAuthGrantType::JwtBearer,
			actor_token: Some(ActorTokenSpec {
				source: AuthorizationLocation::default(),
				token_type: default_token_type(),
			}),
			..base_auth(Arc::new(SimpleBackendReference::Invalid))
		},
		"actor_token"
	)]
	#[case::basic_without_secret(
		OAuthTokenExchangeAuth {
			client_auth: Some(OAuthClientAuth::new(
				"gateway-client".into(),
				None,
				OAuthClientAuthMethod::ClientSecretBasic,
			)),
			..base_auth(Arc::new(SimpleBackendReference::Invalid))
		},
		"client_secret"
	)]
	#[case::empty_client_id(
		OAuthTokenExchangeAuth {
			client_auth: Some(OAuthClientAuth::new(
				String::new(),
				Some("secret".into()),
				OAuthClientAuthMethod::ClientSecretPost,
			)),
			..base_auth(Arc::new(SimpleBackendReference::Invalid))
		},
		"client_id"
	)]
	#[case::empty_client_secret(
		OAuthTokenExchangeAuth {
			client_auth: Some(OAuthClientAuth::new(
				"gateway-client".into(),
				Some("".into()),
				OAuthClientAuthMethod::ClientSecretPost,
			)),
			..base_auth(Arc::new(SimpleBackendReference::Invalid))
		},
		"client_secret"
	)]
	#[case::reserved_additional_param(
		OAuthTokenExchangeAuth {
			additional_params: BTreeMap::from([(
				"scope".into(),
				Arc::new(cel::Expression::new_strict(r#""read""#).unwrap()),
			)]),
			..base_auth(Arc::new(SimpleBackendReference::Invalid))
		},
		"reserved"
	)]
	#[case::expression_output_location(
		OAuthTokenExchangeAuth {
			authorization_location: Some(AuthorizationLocation::Expression {
				expression: Arc::new(cel::Expression::new_strict(r#""token""#).unwrap()),
			}),
			..base_auth(Arc::new(SimpleBackendReference::Invalid))
		},
		"credential extraction"
	)]
	#[case::query_parameter_output_location(
		OAuthTokenExchangeAuth {
			authorization_location: Some(AuthorizationLocation::QueryParameter {
				name: "access_token".into(),
			}),
			..base_auth(Arc::new(SimpleBackendReference::Invalid))
		},
		"query-parameter"
	)]
	#[test]
	fn validate_load_rejects_invalid_local_config(
		#[case] auth: OAuthTokenExchangeAuth,
		#[case] expected: &str,
	) {
		assert_load_err(auth, expected);
	}

	#[test]
	fn accepts_supported_requested_token_types_from_proto() {
		for token_type in [TOKEN_TYPE_ACCESS, TOKEN_TYPE_JWT, TOKEN_TYPE_ID] {
			let auth = OAuthTokenExchangeAuth::try_from(proto::OAuthTokenExchange {
				requested_token_type: Some(token_type.to_string()),
				..Default::default()
			})
			.unwrap();
			assert_eq!(auth.requested_token_type.as_deref(), Some(token_type));
		}
	}

	#[rstest]
	#[case::unsupported_requested_token_type(
		proto::OAuthTokenExchange {
			requested_token_type: Some("urn:ietf:params:oauth:token-type:saml2".to_string()),
			..Default::default()
		},
		"unsupported requested_token_type"
	)]
	#[case::non_slash_token_endpoint_path(
		proto::OAuthTokenExchange {
			token_endpoint_path: Some("noslash".to_string()),
			..Default::default()
		},
		"must start with /"
	)]
	#[case::empty_client_id(
		proto::OAuthTokenExchange {
			client_auth: Some(proto::o_auth_token_exchange::ClientAuth {
				client_id: String::new(),
				client_secret: Some("s".to_string()),
				method: proto::o_auth_token_exchange::client_auth::Method::ClientSecretPost as i32,
			}),
			..Default::default()
		},
		"client_id"
	)]
	#[case::empty_client_secret(
		proto::OAuthTokenExchange {
			client_auth: Some(proto::o_auth_token_exchange::ClientAuth {
				client_id: "gateway-client".to_string(),
				client_secret: Some(String::new()),
				method: proto::o_auth_token_exchange::client_auth::Method::ClientSecretPost as i32,
			}),
			..Default::default()
		},
		"client_secret"
	)]
	#[case::negative_timeout(
		proto::OAuthTokenExchange {
			token_endpoint_timeout: Some(prost_types::Duration {
				seconds: -1,
				nanos: 0,
			}),
			..Default::default()
		},
		"token_endpoint_timeout"
	)]
	#[case::zero_timeout(
		proto::OAuthTokenExchange {
			token_endpoint_timeout: Some(prost_types::Duration {
				seconds: 0,
				nanos: 0,
			}),
			..Default::default()
		},
		"token_endpoint_timeout"
	)]
	#[case::invalid_timeout_nanos(
		proto::OAuthTokenExchange {
			token_endpoint_timeout: Some(prost_types::Duration {
				seconds: 1,
				nanos: 1_000_000_000,
			}),
			..Default::default()
		},
		"nanos"
	)]
	#[case::jwt_bearer_actor_token(
		proto::OAuthTokenExchange {
			grant_type: proto::o_auth_token_exchange::GrantType::JwtBearer as i32,
			actor_token: Some(proto::o_auth_token_exchange::ActorToken::default()),
			..Default::default()
		},
		"actor_token"
	)]
	#[case::actor_token_without_source(
		proto::OAuthTokenExchange {
			actor_token: Some(proto::o_auth_token_exchange::ActorToken::default()),
			..Default::default()
		},
		"actor_token.source"
	)]
	#[case::expression_output_location(
		proto::OAuthTokenExchange {
			authorization_location: Some(proto::AuthorizationLocation {
				kind: Some(proto::authorization_location::Kind::Expression(
					"foo".to_string(),
				)),
			}),
			..Default::default()
		},
		"credential extraction"
	)]
	#[case::query_parameter_output_location(
		proto::OAuthTokenExchange {
			authorization_location: Some(proto::AuthorizationLocation {
				kind: Some(proto::authorization_location::Kind::QueryParameter(
					proto::authorization_location::QueryParameter {
						name: "access_token".to_string(),
					},
				)),
			}),
			..Default::default()
		},
		"query-parameter"
	)]
	#[test]
	fn rejects_invalid_proto_config(
		#[case] proto: proto::OAuthTokenExchange,
		#[case] expected: &str,
	) {
		assert_proto_err_contains(proto, expected);
	}

	#[test]
	fn disabled_cache_from_proto_disables_storage() {
		let auth = OAuthTokenExchangeAuth::try_from(proto::OAuthTokenExchange {
			cache: Some(proto::o_auth_token_exchange::TokenCache {
				backend: Some(
					proto::o_auth_token_exchange::token_cache::Backend::Disabled(
						proto::o_auth_token_exchange::token_cache::Disabled {},
					),
				),
			}),
			..Default::default()
		})
		.unwrap();

		assert!(auth.cache.entries.is_none());
	}

	#[test]
	fn in_memory_cache_from_proto_uses_default_ttl_and_capacity_defaults() {
		let auth = OAuthTokenExchangeAuth::try_from(proto::OAuthTokenExchange {
			cache: Some(proto::o_auth_token_exchange::TokenCache {
				backend: Some(
					proto::o_auth_token_exchange::token_cache::Backend::InMemory(
						proto::o_auth_token_exchange::token_cache::InMemory {
							max_entries: Some(0),
							default_ttl: Some(prost_types::Duration {
								seconds: 42,
								nanos: 0,
							}),
						},
					),
				),
			}),
			..Default::default()
		})
		.unwrap();

		assert!(auth.cache.entries.is_some());
		assert_eq!(auth.cache.default_ttl, Duration::from_secs(42));
	}

	#[tokio::test]
	async fn sends_actor_token() {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
		let a = auth(endpoint(&mock));
		let request = ExchangeRequest {
			subject_token: "subj".to_string().into(),
			subject_token_type: TOKEN_TYPE_ACCESS.to_string(),
			actor: Some(("actor-tok".to_string().into(), TOKEN_TYPE_JWT.to_string())),
			extra_params: vec![],
		};

		fetch_token(&policy_client(), &a, &request).await.unwrap();

		let pairs = sent_form_params(&mock).await;
		assert_eq!(pairs["actor_token"], "actor-tok");
		assert_eq!(pairs["actor_token_type"], TOKEN_TYPE_JWT);
	}

	#[test]
	fn subject_token_source_and_type_from_proto() {
		let proto = proto::OAuthTokenExchange {
			subject_token: Some(proto::o_auth_token_exchange::TokenSpec {
				source: Some(proto::AuthorizationLocation {
					kind: Some(proto::authorization_location::Kind::Header(
						proto::authorization_location::Header {
							name: "x-subject".to_string(),
							prefix: None,
						},
					)),
				}),
				token_type: String::new(),
			}),
			..Default::default()
		};
		let auth = OAuthTokenExchangeAuth::try_from(proto).unwrap();
		assert!(
			matches!(&auth.subject_token.source, AuthorizationLocation::Header { name, .. } if name.as_str() == "x-subject")
		);
		// Empty proto token_type defaults to access_token.
		assert_eq!(auth.subject_token.token_type, TOKEN_TYPE_ACCESS);
	}

	#[test]
	fn authorization_location_from_proto() {
		let proto = proto::OAuthTokenExchange {
			authorization_location: Some(proto::AuthorizationLocation {
				kind: Some(proto::authorization_location::Kind::Header(
					proto::authorization_location::Header {
						name: "x-upstream-auth".to_string(),
						prefix: None,
					},
				)),
			}),
			..Default::default()
		};
		let auth = OAuthTokenExchangeAuth::try_from(proto).unwrap();
		assert!(matches!(
			auth.authorization_location,
			Some(AuthorizationLocation::Header { ref name, .. }) if name.as_str() == "x-upstream-auth"
		));
	}

	#[tokio::test]
	async fn dispatch_inserts_default_bearer_and_marks_explicit() {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
		let backend_auth =
			crate::http::auth::BackendAuth::OAuthTokenExchange(Box::new(auth(endpoint(&mock))));
		let mut req = incoming_request();

		crate::http::auth::apply_backend_auth(&backend_info(), &backend_auth, &mut req)
			.await
			.unwrap();

		let hv = req
			.headers()
			.get(::http::header::AUTHORIZATION)
			.unwrap()
			.to_str()
			.unwrap();
		assert_eq!(hv, "Bearer upstream-token");
		let applied = req
			.extensions()
			.get::<crate::http::auth::AppliedBackendAuthLocation>()
			.unwrap();
		assert!(applied.explicit, "oauth output must be marked explicit");
	}

	#[tokio::test]
	async fn dispatch_uses_configured_output_location_and_marks_explicit() {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
		let a = OAuthTokenExchangeAuth {
			authorization_location: Some(AuthorizationLocation::Header {
				name: ::http::HeaderName::from_static("x-upstream-auth"),
				prefix: None,
			}),
			..auth(endpoint(&mock))
		};
		let backend_auth = crate::http::auth::BackendAuth::OAuthTokenExchange(Box::new(a));
		let mut req = incoming_request();

		crate::http::auth::apply_backend_auth(&backend_info(), &backend_auth, &mut req)
			.await
			.unwrap();

		let hv = req
			.headers()
			.get("x-upstream-auth")
			.unwrap()
			.to_str()
			.unwrap();
		assert_eq!(hv, "upstream-token");
		let applied = req
			.extensions()
			.get::<crate::http::auth::AppliedBackendAuthLocation>()
			.unwrap();
		assert!(
			applied.explicit,
			"configured location must be marked explicit"
		);
	}

	#[tokio::test]
	async fn dispatch_removes_input_token_locations_before_inserting_output() {
		let mock = mock_token_endpoint(ResponseTemplate::new(200).set_body_json(token_body())).await;
		let a = OAuthTokenExchangeAuth {
			actor_token: Some(ActorTokenSpec {
				source: AuthorizationLocation::Header {
					name: ::http::HeaderName::from_static("x-actor-token"),
					prefix: None,
				},
				token_type: TOKEN_TYPE_JWT.to_string(),
			}),
			authorization_location: Some(AuthorizationLocation::Header {
				name: ::http::HeaderName::from_static("x-upstream-auth"),
				prefix: None,
			}),
			..auth(endpoint(&mock))
		};
		let backend_auth = crate::http::auth::BackendAuth::OAuthTokenExchange(Box::new(a));
		let mut req = ::http::Request::builder()
			.method(::http::Method::GET)
			.uri("http://upstream/")
			.header(::http::header::AUTHORIZATION, "Bearer subj")
			.header("x-actor-token", "actor")
			.body(Body::empty())
			.unwrap();

		crate::http::auth::apply_backend_auth(&backend_info(), &backend_auth, &mut req)
			.await
			.unwrap();

		assert!(req.headers().get(::http::header::AUTHORIZATION).is_none());
		assert!(req.headers().get("x-actor-token").is_none());
		assert_eq!(
			req
				.headers()
				.get("x-upstream-auth")
				.unwrap()
				.to_str()
				.unwrap(),
			"upstream-token"
		);
	}
}
