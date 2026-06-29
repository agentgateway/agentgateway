use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use tracing::{debug, trace};

use super::AuthorizationLocation;
use crate::http::Request;
use crate::http::jwt::Claims;
use crate::http::oauth::{TOKEN_TYPE_ACCESS, supported_oauth_token_type};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::serdes::schema;
use crate::types::agent::{BackendTrafficPolicy, SimpleBackendReference};
use crate::types::agent_xds::{
	Diagnostics, authorization_location, optional_authorization_location,
	permissive_cel_expression_arc,
};
use crate::types::discovery::NamespacedHostname;
use crate::types::proto::{ProtoError, agent as proto};
use crate::{apply, cel, schema_enum, ser_redact};

mod cache;
mod transport;

use cache::{TokenCacheConfig, TokenCacheResult, TokenExchangeCache};
pub(super) use transport::FetchError;

#[apply(schema!)]
pub struct OAuthTokenExchangeAuth {
	// ----- Token endpoint -----
	/// Backend serving the RFC 8693 token endpoint.
	#[serde(flatten)]
	target: Arc<SimpleBackendReference>,
	/// Backend policies (TLS, request timeout, ...) used when connecting to the token endpoint.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	#[serde(deserialize_with = "crate::types::local::de_from_local_backend_policy")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "Option<crate::types::local::SimpleLocalBackendPolicies>")
	)]
	policies: Vec<BackendTrafficPolicy>,
	/// Token endpoint path on the backend; defaults to "/".
	#[serde(default, skip_serializing_if = "String::is_empty")]
	token_endpoint_path: String,

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
	/// Extra form parameters appended to the token request.
	/// Values are CEL expressions evaluated against the incoming request.
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	additional_params: BTreeMap<String, Arc<cel::Expression>>,

	// ----- Authorization server client authentication -----
	/// Client authentication used when calling the token endpoint.
	/// When unset, no client authentication fields are sent.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	client_auth: Option<OAuthClientAuth>,

	// ----- Output and runtime behavior -----
	/// Where to place the exchanged token in the backend request. Defaults to the
	/// Authorization header with a "Bearer " prefix. The CEL `expression` source is
	/// not valid here (it cannot insert).
	#[serde(default)]
	authorization_location: AuthorizationLocation,

	// Runtime cache configuration
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	cache: TokenExchangeCache,
}

impl OAuthTokenExchangeAuth {
	pub(crate) fn default_backend_tls_for_https_port(&mut self) -> anyhow::Result<()> {
		if self
			.policies
			.iter()
			.any(|p| matches!(p, BackendTrafficPolicy::BackendTLS(_)))
		{
			return Ok(());
		}
		if matches!(
			self.target.as_ref(),
			SimpleBackendReference::InlineBackend(crate::types::agent::Target::Hostname(_, 443))
		) {
			self.policies.push(BackendTrafficPolicy::BackendTLS(
				crate::http::backendtls::LocalBackendTLS::default().try_into()?,
			));
		}
		Ok(())
	}

	pub(crate) fn validate_load(&self) -> Result<(), String> {
		if !self.token_endpoint_path.is_empty() && !self.token_endpoint_path.starts_with('/') {
			return Err(format!(
				"token_endpoint_path {:?} must start with /",
				self.token_endpoint_path
			));
		}
		if self.grant_type == OAuthGrantType::JwtBearer {
			if self.requested_token_type.is_some() {
				return Err("requested_token_type is only valid with the token-exchange grant".into());
			}
			if self.actor_token.is_some() {
				return Err("actor_token is only valid with the token-exchange grant".into());
			}
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
					"additional parameter {key:?} overrides a reserved OAuth parameter"
				));
			}
		}
		if matches!(
			self.authorization_location,
			AuthorizationLocation::Expression { .. }
		) {
			return Err("expression auth location is only supported for credential extraction".into());
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
		let subject_token = extract_token(&self.subject_token.source, req).ok_or_else(|| {
			debug!("oauth token exchange subject token missing");
			ProxyError::InvalidRequest
		})?;
		let actor = self
			.actor_token
			.as_ref()
			.map(|spec| -> Result<_, ProxyError> {
				let token = extract_token(&spec.source, req).ok_or_else(|| {
					debug!("oauth token exchange actor token missing");
					ProxyError::InvalidRequest
				})?;
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

		// TODO: `AppliedBackendAuthLocation::explicit` currently means both
		// "user configured this" and "downstream must not rewrite Authorization".
		// Keep OAuth's old `true` behavior until those meanings are separated.
		self.authorization_location.insert(req, access_token)?;
		Ok(true)
	}
}

#[apply(schema!)]
pub struct OAuthClientAuth {
	/// `client_id` parameter identifying the gateway at the authorization server.
	client_id: String,
	/// Client secret. When absent the client is public, which is only valid with `ClientSecretPost`.
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
	let defaults = TokenCacheConfig::default();
	let Some(in_memory) = cache.and_then(|c| c.in_memory) else {
		return Ok(defaults);
	};

	Ok(TokenCacheConfig {
		// max_entries == 0 disables the cache; unset falls back to the default capacity.
		max_entries: in_memory
			.max_entries
			.map_or(defaults.max_entries, |n| n as usize),
		default_ttl: in_memory
			.default_ttl
			.map(|d| positive_duration_from_proto("cache.in_memory.default_ttl", d))
			.transpose()?
			.unwrap_or(defaults.default_ttl),
	})
}

fn token_spec_from_proto(
	diagnostics: &mut Diagnostics,
	spec: proto::o_auth_token_exchange::TokenSpec,
) -> Result<TokenSpec, ProtoError> {
	let token_type = if spec.token_type.is_empty() {
		default_token_type()
	} else {
		spec.token_type
	};
	Ok(TokenSpec {
		source: authorization_location(
			diagnostics,
			"backendAuth.oauth.subjectToken.source",
			spec.source.as_ref(),
			AuthorizationLocation::default(),
		)?,
		token_type,
	})
}

fn actor_token_from_proto(
	diagnostics: &mut Diagnostics,
	spec: proto::o_auth_token_exchange::ActorToken,
) -> Result<ActorTokenSpec, ProtoError> {
	// Unlike the subject token, the actor token has no default source: it must be
	// explicit so actor and subject can't accidentally be the same credential.
	if spec.source.is_none() {
		return Err(ProtoError::Generic(
			"oauth token exchange actor_token.source must be set".into(),
		));
	}
	let token_type = if spec.token_type.is_empty() {
		default_token_type()
	} else {
		spec.token_type
	};
	Ok(ActorTokenSpec {
		source: authorization_location(
			diagnostics,
			"backendAuth.oauth.actorToken.source",
			spec.source.as_ref(),
			AuthorizationLocation::default(),
		)?,
		token_type,
	})
}

fn backend_ref_from_proto(target: Option<proto::BackendReference>) -> SimpleBackendReference {
	use proto::backend_reference::Kind;
	let Some(target) = target else {
		return SimpleBackendReference::Invalid;
	};

	match target.kind {
		None => SimpleBackendReference::Invalid,
		Some(Kind::Backend(name)) => SimpleBackendReference::Backend((&name).into()),
		Some(Kind::Service(svc)) => SimpleBackendReference::Service {
			name: NamespacedHostname {
				namespace: svc.namespace.into(),
				hostname: svc.hostname.into(),
			},
			port: target.port as u16,
		},
	}
}

pub(crate) fn from_proto_with_diagnostics(
	t: proto::OAuthTokenExchange,
	diagnostics: &mut Diagnostics,
) -> Result<OAuthTokenExchangeAuth, ProtoError> {
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
		.map(|s| token_spec_from_proto(diagnostics, s))
		.transpose()?
		.unwrap_or_default();
	let actor_token = t
		.actor_token
		.map(|s| actor_token_from_proto(diagnostics, s))
		.transpose()?;

	let token_endpoint_path = t.token_endpoint_path.unwrap_or_default();

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

	let cache_config = token_cache_config_from_proto(t.cache)?;

	let auth = OAuthTokenExchangeAuth {
		target: Arc::new(backend_ref_from_proto(t.token_endpoint)),
		// Inline connection policies are not supported from xDS;
		// the backend resource carries its own policies there.
		policies: Vec::new(),
		token_endpoint_path,
		grant_type,
		subject_token,
		actor_token,
		audiences: t.audiences,
		scopes: t.scopes,
		resources: t.resources,
		requested_token_type,
		client_auth: t.client_auth.map(OAuthClientAuth::try_from).transpose()?,
		additional_params,
		authorization_location: optional_authorization_location(t.authorization_location.as_ref())?
			.unwrap_or_default(),
		cache: TokenExchangeCache::new(&cache_config),
	};
	auth.validate_load().map_err(ProtoError::Generic)?;
	Ok(auth)
}

impl TryFrom<proto::OAuthTokenExchange> for OAuthTokenExchangeAuth {
	type Error = ProtoError;

	fn try_from(t: proto::OAuthTokenExchange) -> Result<Self, Self::Error> {
		from_proto_with_diagnostics(t, &mut Diagnostics::default())
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
#[derive(Default)]
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

	let access_token = fetch_token(&client, auth, auth.build_exchange_request(req)?)
		.await
		.map_err(FetchError::into_proxy_error)?;

	auth.insert_exchanged_token(req, access_token.expose_secret())
}

/// Read a token for exchange from its configured source. For an Authorization
/// Bearer source, a JWT auth policy may have already stripped the header after
/// validation, so fall back to the populated Claims extension.
pub(super) fn extract_token(source: &AuthorizationLocation, req: &Request) -> Option<String> {
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

fn validate_requested_token_type(requested: &str) -> Result<(), String> {
	if supported_oauth_token_type(requested) {
		return Ok(());
	}
	Err(format!(
		"unsupported requested_token_type {requested:?}; supported values are {TOKEN_TYPE_ACCESS}, {}, and {}",
		crate::http::oauth::TOKEN_TYPE_JWT,
		crate::http::oauth::TOKEN_TYPE_ID,
	))
}

async fn fetch_token(
	client: &PolicyClient,
	auth: &OAuthTokenExchangeAuth,
	req: ExchangeRequest,
) -> Result<SecretString, FetchError> {
	let result = auth
		.cache
		.get_or_insert_with(&req, async |req| {
			transport::request_token(client, auth, req).await
		})
		.await?;

	// TODO: export metrics
	match &result {
		TokenCacheResult::Hit(_) => trace!("token exchange cache hit"),
		TokenCacheResult::Miss(_) => trace!("token exchange succeeded"),
	}
	Ok(result.into_token())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
