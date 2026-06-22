use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
#[cfg(not(test))]
use std::time::{SystemTime, UNIX_EPOCH};

use ::http::Uri;
use agent_core::prelude::Strng;
use anyhow::{Context, Error, anyhow, bail};
use itertools::Itertools;
use macro_rules_attribute::apply;
use secrecy::SecretString;

use crate::client::Client;
use crate::http::auth::BackendAuth;
use crate::http::backendtls::LocalBackendTLS;
use crate::http::transformation_cel::{LocalTransformationConfig, Transformation};
use crate::http::{filters, health, retry, timeout, transformation_cel};
use crate::llm::policy::{PromptCachingConfig, PromptGuard};
use crate::llm::{AIBackend, AIProvider, NamedAIProvider, anthropic, copilot, custom, openai};
use crate::mcp::{FailureMode, McpAuthorization};
use crate::store::{LocalWorkload, RequestPolicy};
use crate::types::agent::{
	A2aPolicy, Authorization, Backend, BackendKey, BackendReference, BackendTrafficPolicy,
	BackendWithPolicies, Bind, BindProtocol, FrontendPolicy, HeaderMatch, JwtAuthentication,
	Listener, ListenerKey, ListenerName, ListenerProtocol, ListenerSet, ListenerTarget,
	LocalMcpAuthentication, McpAuthentication, McpBackend, McpTarget, McpTargetName, McpTargetSpec,
	OpenAPITarget, PathMatch, PolicyPhase, PolicyTarget, PolicyType, ResourceName, Route,
	RouteBackendReference, RouteBackendTarget, RouteGroupKey, RouteMatch, RouteName, ServerTLSConfig,
	SimpleBackend, SimpleBackendReference, SimpleBackendWithPolicies, SseTargetSpec,
	StreamableHTTPTargetSpec, TCPRoute, TCPRouteBackendReference, Target, TargetedPolicy,
	TracingConfig, TrafficPolicy, TunnelProtocol, TypedResourceName, validate_mcp_target_name,
};
use crate::types::discovery::{NamespacedHostname, Service};
use crate::types::{backend, frontend};
use crate::{agentcore, *};

type LocalExtAuthzPolicy = LocalExplicitOrConditional<crate::http::ext_authz::ExtAuthz>;
type LocalDirectResponsePolicy = LocalExplicitOrConditional<filters::DirectResponse>;
type LocalExtProcPolicy = LocalExplicitOrConditional<crate::http::ext_proc::ExtProc>;
type LocalRemoteRateLimitPolicy =
	LocalExplicitOrConditional<crate::http::remoteratelimit::RemoteRateLimit>;
type LocalTransformationPolicy = LocalExplicitOrConditional<LocalTransformationConfig>;
type LocalMcpGuardrails = crate::mcp::guardrails::McpGuardrails;
const DEFAULT_LLM_PORT: u16 = 4000;
const DEFAULT_MCP_PORT: u16 = 3000;

// Windows has different output, for now easier to just not deal with it
#[cfg(all(test, target_family = "unix"))]
#[path = "local_tests.rs"]
mod tests;

impl NormalizedLocalConfig {
	pub async fn from(
		config: &crate::Config,
		client: client::Client,
		gateway_name: ListenerTarget,
		s: &str,
	) -> anyhow::Result<NormalizedLocalConfig> {
		// Avoid shell expanding the comment for schema. Probably there are better ways to do this!
		let s = s.replace("# yaml-language-server: $schema", "#");
		let s = shellexpand::full(&s)?;
		let local_config: LocalConfig = serdes::yamlviajson::from_str(&s)?;
		let t = convert(client, gateway_name, config, local_config).await?;
		Ok(t)
	}
}

pub fn migrate_deprecated_local_config(s: &str) -> anyhow::Result<String> {
	let cfg: serde_json::Value = serdes::yamlviajson::from_str(s)?;
	let cfg = migrate_deprecated_frontend_policies(cfg)?;
	serdes::yamlviajson::to_string(&cfg)
}

fn migrate_deprecated_frontend_policies(
	mut cfg: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
	let Some(root) = cfg.as_object_mut() else {
		return Ok(cfg);
	};

	let Some(config) = root.get("config").and_then(serde_json::Value::as_object) else {
		return Ok(cfg);
	};

	let deprecated_logging = config.get("logging").cloned();
	let deprecated_tracing = config.get("tracing").cloned();
	if deprecated_logging.is_none() && deprecated_tracing.is_none() {
		return Ok(cfg);
	}

	let mut deprecated_config = serde_json::Map::new();
	if let Some(logging) = deprecated_logging {
		deprecated_config.insert("logging".to_string(), logging);
	}
	if let Some(tracing) = deprecated_tracing {
		deprecated_config.insert("tracing".to_string(), tracing);
	}
	let mut deprecated_root = serde_json::Map::new();
	deprecated_root.insert(
		"config".to_string(),
		serde_json::Value::Object(deprecated_config),
	);
	let deprecated_cfg_yaml =
		serdes::yamlviajson::to_string(&serde_json::Value::Object(deprecated_root))?;
	let deprecated_cfg = crate::config::parse_config(deprecated_cfg_yaml, None)?;

	let mut frontend_policies: LocalFrontendPolicies = serde_json::from_value(
		root
			.get("frontendPolicies")
			.cloned()
			.unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new())),
	)?;
	merge_deprecated_frontend_policies(&deprecated_cfg, &mut frontend_policies)?;

	let has_deprecated_log = has_deprecated_frontend_log_fields(&deprecated_cfg.logging);
	let has_deprecated_tracing = deprecated_cfg.tracing.is_some();

	let frontend_policies_map = root
		.entry("frontendPolicies")
		.or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
	let Some(frontend_policies_map) = frontend_policies_map.as_object_mut() else {
		anyhow::bail!("frontendPolicies must be an object");
	};
	if has_deprecated_log {
		let Some(access_log) = frontend_policies.access_log else {
			anyhow::bail!("internal error: migrated accessLog was not generated");
		};
		frontend_policies_map.insert("accessLog".to_string(), serde_json::to_value(access_log)?);
		frontend_policies_map.remove("logging");
	}
	if has_deprecated_tracing && let Some(tracing) = frontend_policies.tracing {
		frontend_policies_map.insert("tracing".to_string(), serde_json::to_value(tracing)?);
	}

	let Some(config) = root
		.get_mut("config")
		.and_then(serde_json::Value::as_object_mut)
	else {
		return Ok(cfg);
	};
	if has_deprecated_log {
		match config.get_mut("logging") {
			Some(serde_json::Value::Object(logging)) => {
				logging.remove("filter");
				logging.remove("fields");
				if logging.is_empty() {
					config.remove("logging");
				}
			},
			Some(_) => {
				config.remove("logging");
			},
			None => {},
		}
	}
	if has_deprecated_tracing {
		config.remove("tracing");
	}
	Ok(cfg)
}

fn has_deprecated_frontend_log_fields(log: &crate::telemetry::log::Config) -> bool {
	!log.fields.add.is_empty() || !log.fields.remove.is_empty() || log.filter.is_some()
}

fn merge_deprecated_frontend_policies(
	deprecated: &crate::Config,
	frontend_policies: &mut LocalFrontendPolicies,
) -> anyhow::Result<()> {
	let log = &deprecated.logging;
	let has_deprecated_log = has_deprecated_frontend_log_fields(log);
	if has_deprecated_log {
		if frontend_policies.access_log.is_some() {
			anyhow::bail!(
				"cannot use deprecated config.logging together with frontendPolicies.accessLog"
			);
		}
		frontend_policies.access_log = Some(frontend::LoggingPolicy {
			filter: log.filter.clone(),
			add: log.fields.add.clone(),
			remove: log.fields.remove.clone(),
			otlp: None,
			database: None,
			access_log_policy: None,
		});
	}
	if let Some(tracing) = deprecated.tracing.clone() {
		if frontend_policies.tracing.is_some() {
			anyhow::bail!("cannot use deprecated config.tracing together with frontendPolicies.tracing");
		}
		let trc::DeprecatedConfig {
			endpoint,
			headers,
			protocol,
			fields,
			random_sampling,
			client_sampling,
			path,
		} = tracing;

		let policies = if !headers.is_empty() {
			let backend_xfm = transformation_cel::LocalTransformationConfig {
				request: Some(transformation_cel::LocalTransform {
					set: headers
						.into_iter()
						.map(|(k, v)| (strng::new(k), strng::new(v)))
						.collect(),
					..Default::default()
				}),
				response: None,
			};
			let backend_xfm = Transformation::try_from_local_config(backend_xfm, true)?;
			vec![BackendTrafficPolicy::Transformation(Arc::new(backend_xfm))]
		} else {
			Vec::new()
		};
		if let Some(ep) = endpoint {
			// Strip the scheme (http://, https://), or grpc://) from the endpoint URL to get host:port
			let host_port = ep
				.strip_prefix("http://")
				.or_else(|| ep.strip_prefix("https://"))
				.or_else(|| ep.strip_prefix("grpc://"))
				.unwrap_or(&ep);
			frontend_policies.tracing = Some(TracingConfig {
				provider_backend: SimpleBackendReference::InlineBackend(
					Target::try_from(host_port)
						.with_context(|| format!("failed parsing tracing endpoint: {}", ep))?,
				),
				policies,
				attributes: Arc::unwrap_or_clone(fields.add),
				resources: Default::default(), // Not supported in the old config
				remove: Arc::unwrap_or_clone(fields.remove).into_iter().collect(),
				random_sampling,
				client_sampling,
				path,
				protocol: match protocol {
					Protocol::Grpc => crate::types::agent::TracingProtocol::Grpc,
					Protocol::Http => crate::types::agent::TracingProtocol::Http,
				},
			});
		}
	}
	Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct NormalizedLocalConfig {
	pub binds: Vec<Bind>,
	pub listener_routes: Vec<(ListenerKey, Vec<Route>)>,
	pub listener_tcp_routes: Vec<(ListenerKey, Vec<TCPRoute>)>,
	pub policies: Vec<TargetedPolicy>,
	pub backends: Vec<BackendWithPolicies>,
	pub route_groups: Vec<(RouteGroupKey, Vec<Route>)>,
	// Note: here we use LocalWorkload since it conveys useful info, we could maybe change but not a problem
	// for now
	pub workloads: Vec<LocalWorkload>,
	pub services: Vec<Service>,
}

#[apply(schema_de!)]
pub struct LocalConfig {
	#[serde(default)]
	#[cfg_attr(feature = "schema", schemars(with = "RawConfig"))]
	#[allow(unused)]
	config: Arc<Option<serde_json::Value>>,
	#[serde(default)]
	binds: Vec<LocalBind>,
	#[serde(default)]
	frontend_policies: LocalFrontendPolicies,
	/// policies defines additional policies that can be attached to various other configurations.
	/// This is an advanced feature; users should typically use the inline `policies` field under route/gateway.
	#[serde(default)]
	policies: Vec<LocalPolicy>,
	#[serde(default)]
	#[cfg_attr(feature = "schema", schemars(with = "serde_json::value::RawValue"))]
	workloads: Vec<LocalWorkload>,
	#[serde(default)]
	#[cfg_attr(feature = "schema", schemars(with = "serde_json::value::RawValue"))]
	services: Vec<Service>,
	#[serde(default)]
	backends: Vec<FullLocalBackend>,
	#[serde(default, rename = "routeGroups")]
	route_groups: Vec<LocalRouteGroup>,
	#[serde(default)]
	llm: Option<LocalLLMConfig>,
	#[serde(default)]
	mcp: Option<LocalSimpleMcpConfig>,
}

#[apply(schema_de!)]
pub struct LocalLLMConfig {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	port: Option<u16>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	tls: Option<LocalTLSServerConfig>,
	/// providers defines reusable LLM provider defaults that models may reference.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	providers: Vec<LocalLLMProvider>,
	/// models defines the set of models that can be served by this gateway. The model name refers to the
	/// model in the users request that is matched; the model sent to the actual LLM can be overridden
	/// on a per-model basis.
	models: Vec<LocalLLMModels>,
	/// virtualModels defines a set of models that can be served from the gateway. The model name refers to the
	/// model in the users request that is matched. However, unlike the `models` field, virtual models will
	/// dynamically route to a specific model (configured in `models`) based on the configured logic.
	#[serde(
		default,
		rename = "virtualModels",
		skip_serializing_if = "Vec::is_empty"
	)]
	virtual_models: Vec<LocalLLMVirtualModel>,
	/// policies defines policies for handling incoming requests, before a model is selected
	#[serde(default, skip_serializing_if = "Option::is_none")]
	policies: Option<LocalLLMPolicy>,
}

#[apply(schema_de!)]
pub struct LocalLLMProvider {
	/// name is referenced from llm.models[].provider.reference.
	name: Strng,
	/// params customizes parameters for outgoing requests that use this provider.
	#[serde(default)]
	params: LocalLLMParams,
	/// provider of the LLM we are connecting to.
	provider: LocalModelAIProvider,
	/// defaults defines provider-level policy defaults. Model-level policy fields override these.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	defaults: Option<LocalLLMProviderDefaults>,
}

#[apply(schema_de!)]
#[derive(Default)]
pub struct LocalLLMProviderDefaults {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	defaults: Option<HashMap<String, serde_json::Value>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	overrides: Option<HashMap<String, serde_json::Value>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	transformation: Option<HashMap<String, Arc<cel::Expression>>>,
	#[serde(default)]
	request_headers: Option<filters::HeaderModifier>,
	#[serde(default)]
	response_headers: Option<filters::HeaderModifier>,
	#[serde(rename = "tls", alias = "backendTLS", default)]
	backend_tls: Option<http::backendtls::LocalBackendTLS>,
	#[serde(default, deserialize_with = "de_backend_auth")]
	auth: Option<BackendAuth>,
	#[serde(default)]
	health: Option<health::LocalHealthPolicy>,
	#[serde(default)]
	backend_tunnel: Option<backend::Tunnel>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	prompt_caching: Option<PromptCachingConfig>,
}

#[apply(schema_de!)]
pub struct LocalLLMVirtualModel {
	/// name is the public model name clients request.
	name: String,
	/// routing selects an existing LLM model backend for each request.
	routing: LocalLLMVirtualModelRouting,
}

#[apply(schema_de!)]
pub struct LocalLLMVirtualModelRouting {
	/// weighted enables weight-based selection of the target model.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	weighted: Option<LocalLLMWeightedRouting>,
	/// failover enables priority-based selection of the target model.
	/// Within a priority level, the best provider is selected by a composite score factoring in health
	/// and latency.
	/// If all models within a priority level are degraded, requests will move onto the next priority group.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	failover: Option<LocalLLMFailoverRouting>,
	/// Conditional enables condition-based selection of the target model. Each condition is evaluated
	/// in order until the best match is found.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	conditional: Option<LocalLLMConditionalRouting>,
}

#[apply(schema_de!)]
pub struct LocalLLMWeightedRouting {
	/// targets are existing model names or names matched by wildcard model entries.
	targets: Vec<LocalLLMWeightedTarget>,
}

#[apply(schema_de!)]
pub struct LocalLLMWeightedTarget {
	/// model is resolved against llm.models using the same wildcard matching as client requests.
	model: String,
	#[serde(default = "default_weight")]
	weight: usize,
}

#[apply(schema_de!)]
pub struct LocalLLMFailoverRouting {
	/// targets are grouped by priority. Lower priority values are tried first.
	targets: Vec<LocalLLMFailoverTarget>,
}

#[apply(schema_de!)]
pub struct LocalLLMFailoverTarget {
	/// model is resolved against llm.models using the same wildcard matching as client requests.
	model: String,
	/// priority groups targets for failover. Lower values are preferred.
	priority: usize,
}

#[apply(schema_de!)]
pub struct LocalLLMConditionalRouting {
	/// targets are evaluated in order. The first matching condition selects the model.
	targets: Vec<LocalLLMConditionalTarget>,
}

#[apply(schema_de!)]
pub struct LocalLLMConditionalTarget {
	/// when must evaluate to true for this target to be selected. Omit only on the final fallback target.
	#[serde(default)]
	when: Option<Arc<cel::Expression>>,
	/// model is resolved against llm.models using the same wildcard matching as client requests.
	model: String,
}

#[apply(schema_de!)]
pub struct LocalSimpleMcpConfig {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	port: Option<u16>,
	#[serde(flatten)]
	backend: LocalMcpBackend,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	policies: Option<FilterOrPolicy>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(rename = "LocalConditionalPolicy_{T}"))]
struct LocalConditionalPolicy<T> {
	/// condition must evaluate to true for this policy to execute. If unset, the policy is the fallback.
	#[serde(default)]
	condition: Option<Arc<crate::cel::Expression>>,
	/// Policy settings for this conditional entry.
	#[serde(flatten)]
	policy: T,
}

#[apply(schema_de!)]
#[cfg_attr(feature = "schema", schemars(rename = "LocalConditionalPolicies_{T}"))]
struct LocalConditionalPolicies<T> {
	/// conditional policy entries. An entry without a condition must be the final fallback.
	conditional: Vec<LocalConditionalPolicy<T>>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
	feature = "schema",
	schemars(
		untagged,
		deny_unknown_fields,
		rename = "LocalExplicitOrConditional_{T}"
	)
)]
enum LocalExplicitOrConditional<T> {
	Conditional(LocalConditionalPolicies<T>),
	Explicit(T),
}

// Custom impl to avoid terrible 'not match any variant of untagged' errors.
impl<'de, T> Deserialize<'de> for LocalExplicitOrConditional<T>
where
	T: serde::de::DeserializeOwned,
{
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		serde_untagged::UntaggedEnumVisitor::new()
			.map(|map| {
				let v: serde_json::Value = map.deserialize()?;

				if let serde_json::Value::Object(m) = &v
					&& m.contains_key("conditional")
				{
					Ok(LocalExplicitOrConditional::Conditional(
						serde_json::from_value(v).map_err(serde::de::Error::custom)?,
					))
				} else {
					Ok(LocalExplicitOrConditional::Explicit(
						serde_json::from_value(v).map_err(serde::de::Error::custom)?,
					))
				}
			})
			.deserialize(deserializer)
	}
}

impl<T> LocalExplicitOrConditional<T> {
	fn into_policy(self) -> anyhow::Result<RequestPolicy<T>> {
		match self {
			LocalExplicitOrConditional::Explicit(policy) => Ok(RequestPolicy::single(policy)),
			LocalExplicitOrConditional::Conditional(policies) => {
				validate_local_conditional_policies(&policies)?;
				Ok(RequestPolicy::from_policies(
					policies
						.conditional
						.into_iter()
						.map(|entry| (entry.policy, entry.condition)),
				))
			},
		}
	}
}

fn configure_ext_authz_cache_store(policy: LocalExtAuthzPolicy) -> LocalExtAuthzPolicy {
	match policy {
		LocalExplicitOrConditional::Explicit(policy) => {
			LocalExplicitOrConditional::Explicit(policy.with_configured_cache_store())
		},
		LocalExplicitOrConditional::Conditional(mut policies) => {
			policies.conditional = policies
				.conditional
				.into_iter()
				.map(|entry| LocalConditionalPolicy {
					condition: entry.condition,
					policy: entry.policy.with_configured_cache_store(),
				})
				.collect();
			LocalExplicitOrConditional::Conditional(policies)
		},
	}
}

fn validate_local_conditional_policies<T>(
	policies: &LocalConditionalPolicies<T>,
) -> anyhow::Result<()> {
	if policies.conditional.is_empty() {
		bail!("conditional policies must have at least one entry");
	}
	if policies.conditional.len() > 64 {
		bail!("conditional policies may have at most 64 entries");
	}
	if let Some(unconditional_idx) = policies
		.conditional
		.iter()
		.position(|entry| entry.condition.is_none())
		&& unconditional_idx + 1 != policies.conditional.len()
	{
		bail!("conditional policy entries without condition must be last");
	}
	Ok(())
}

impl LocalExplicitOrConditional<LocalTransformationConfig> {
	fn into_transformation_policy(self) -> anyhow::Result<RequestPolicy<Transformation>> {
		match self {
			LocalExplicitOrConditional::Explicit(policy) => Ok(RequestPolicy::single(
				Transformation::try_from_local_config(policy, true)?,
			)),
			LocalExplicitOrConditional::Conditional(policies) => {
				validate_local_conditional_policies(&policies)?;
				Ok(RequestPolicy::from_policies(
					policies
						.conditional
						.into_iter()
						.map(|entry| {
							Transformation::try_from_local_config(entry.policy, true)
								.map(|policy| (policy, entry.condition))
						})
						.collect::<anyhow::Result<Vec<_>>>()?,
				))
			},
		}
	}
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(untagged)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(untagged, deny_unknown_fields))]
enum LocalRateLimitPolicy {
	Conditional(LocalConditionalPolicies<crate::http::localratelimit::RateLimit>),
	Explicit(Vec<crate::http::localratelimit::RateLimit>),
}

impl LocalRateLimitPolicy {
	fn is_empty(&self) -> bool {
		match self {
			LocalRateLimitPolicy::Conditional(policies) => policies.conditional.is_empty(),
			LocalRateLimitPolicy::Explicit(policies) => policies.is_empty(),
		}
	}

	fn into_request_policy(
		self,
	) -> anyhow::Result<RequestPolicy<Vec<crate::http::localratelimit::RateLimit>>> {
		match self {
			LocalRateLimitPolicy::Explicit(policies) => Ok(RequestPolicy::single(policies)),
			LocalRateLimitPolicy::Conditional(policies) => {
				validate_local_conditional_policies(&policies)?;
				Ok(RequestPolicy::from_policies(
					policies
						.conditional
						.into_iter()
						.map(|entry| (vec![entry.policy], entry.condition)),
				))
			},
		}
	}
}

#[apply(schema_de!)]
pub struct LocalLLMModels {
	/// name is the name of the model we are matching from a users request. If params.model is set, that
	/// will be used in the request to the LLM provider. If not, the incoming model is used.
	name: String,
	/// visibility controls whether clients can request this model directly (rather than only via a `virtualModel`).
	#[serde(
		default,
		skip_serializing_if = "llm::model_router::ModelVisibility::is_public"
	)]
	visibility: llm::model_router::ModelVisibility,
	/// params customizes parameters for the outgoing request
	#[serde(default)]
	params: LocalLLMParams,
	/// provider of the LLM we are connecting too
	provider: LocalModelAIProvider,
	/// passthrough controls how requests are handled.
	/// By default, requests will be parsed and translated as needed.
	/// With passthrough, they will be unmodified and optionally inspected (with `detect`).
	/// In this mode, requests must be sent in the native format of the provider.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	passthrough: Option<LocalLLMPassthrough>,
	/// authorization configures HTTP authorization rules for requests to this model.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	authorization: Option<Authorization>,

	// Policies
	/// defaults allows setting default values for the request. If these are not present in the request body, they will be set.
	/// To override even when set, use `overrides`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	defaults: Option<HashMap<String, serde_json::Value>>,
	/// overrides allows setting values for the request, overriding any existing values
	#[serde(default, skip_serializing_if = "Option::is_none")]
	overrides: Option<HashMap<String, serde_json::Value>>,
	/// transformation allows setting values from CEL expressions for the request, overriding any existing values.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	transformation: Option<HashMap<String, Arc<cel::Expression>>>,
	/// requestHeaders modifies headers in requests to the LLM provider.
	#[serde(default)]
	request_headers: Option<filters::HeaderModifier>,
	/// responseHeaders modifies headers in responses from the LLM provider.
	#[serde(default)]
	response_headers: Option<filters::HeaderModifier>,
	/// tls configures TLS when connecting to the LLM provider.
	#[serde(rename = "tls", alias = "backendTLS", default)]
	backend_tls: Option<http::backendtls::LocalBackendTLS>,
	/// auth configures authentication when connecting to the LLM provider.
	#[serde(default, deserialize_with = "de_backend_auth")]
	auth: Option<BackendAuth>,
	/// health configures outlier detection for this model backend.
	#[serde(default)]
	health: Option<health::LocalHealthPolicy>,
	/// backendTunnel configures tunneling when connecting to the LLM provider.
	#[serde(default)]
	backend_tunnel: Option<backend::Tunnel>,
	/// guardrails to apply to the request or response
	#[serde(default, skip_serializing_if = "Option::is_none")]
	guardrails: Option<PromptGuard>,
	/// promptCaching configures cache point insertion for supported LLM providers.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	prompt_caching: Option<PromptCachingConfig>,

	/// matches specifies the conditions under which this model should be used in addition to matching the model name.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	matches: Vec<LLMRouteMatch>,
}

#[apply(schema_de!)]
pub enum LocalLLMPassthrough {
	/// Pass through the request while extracting LLM telemetry and rate-limit inputs when possible.
	Detect,
	/// Pass through the request without interpreting it as LLM traffic.
	Opaque,
}

impl LocalLLMPassthrough {
	fn route_type(&self) -> crate::llm::RouteType {
		match self {
			LocalLLMPassthrough::Detect => crate::llm::RouteType::Detect,
			LocalLLMPassthrough::Opaque => crate::llm::RouteType::Passthrough,
		}
	}
}

#[apply(schema_de!)]
pub struct LLMRouteMatch {
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub headers: Vec<HeaderMatch>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum LocalModelAIProvider {
	#[serde(rename = "reference")]
	Reference(Strng),
	#[serde(rename = "openai", alias = "openAI")]
	OpenAI,
	Gemini,
	Vertex,
	Anthropic,
	Bedrock,
	Azure,
	Copilot,
	Custom(custom::Provider),
	// Providers below are synthetic conversions to custom with preconfigured defaults.
	Cohere,
	Ollama,
	Baseten,
	Cerebras,
	Deepinfra,
	Deepseek,
	Groq,
	Huggingface,
	Mistral,
	Openrouter,
	Togetherai,
	XAI,
	Fireworks,
}

#[apply(schema_de!)]
pub struct SecretFromFile(
	#[cfg_attr(feature = "schema", schemars(with = "FileOrInline"))]
	#[serde(
		serialize_with = "ser_redact",
		deserialize_with = "deser_key_from_file"
	)]
	SecretString,
);

#[apply(schema_de!)]
#[derive(Default)]
pub struct LocalLLMParams {
	/// The model to send to the provider.
	/// If unset, the same model will be used from the request.
	#[serde(default)]
	model: Option<Strng>,
	/// An API key to attach to the request.
	/// If unset this will be automatically detected from the environment.
	#[serde(default)]
	api_key: Option<SecretFromFile>,
	// For Bedorkc: The AWS region to use
	aws_region: Option<Strng>,
	// For Vertex: The Google region to use
	vertex_region: Option<Strng>,
	// For Vertex: The Google project ID to use
	vertex_project: Option<Strng>,
	/// For Azure: the resource name of the deployment
	azure_resource_name: Option<Strng>,
	/// For Azure: the type of Azure endpoint (openAI or foundry)
	azure_resource_type: Option<crate::llm::azure::AzureResourceType>,
	/// For Azure: the API version to use
	azure_api_version: Option<Strng>,
	/// For Azure: the Foundry project name (required for foundry resource type)
	azure_project_name: Option<Strng>,
	/// Base URL for the upstream provider. Expands to hostOverride, pathPrefix, and tls for https URLs.
	#[serde(default)]
	base_url: Option<Strng>,
	/// Override the upstream host for this provider.
	#[serde(default)]
	#[deprecated(note = "use baseUrl instead")]
	host_override: Option<Target>,
	/// Override the upstream path for this provider.
	#[serde(default)]
	#[deprecated(note = "use baseUrl instead")]
	path_override: Option<Strng>,
	/// Override the default base path prefix for this provider.
	#[serde(default)]
	#[deprecated(note = "use baseUrl instead")]
	path_prefix: Option<Strng>,
	/// Whether to tokenize the request before forwarding it upstream.
	#[serde(default)]
	tokenize: bool,
}

impl LocalLLMModels {
	#[allow(deprecated)]
	fn apply_provider_reference(&mut self, provider: &LocalLLMProvider) -> anyhow::Result<()> {
		let LocalLLMParams {
			model: model_override,
			api_key: None,
			aws_region: None,
			vertex_region: None,
			vertex_project: None,
			azure_resource_name: None,
			azure_resource_type: None,
			azure_api_version: None,
			azure_project_name: None,
			base_url: None,
			host_override: None,
			path_override: None,
			path_prefix: None,
			tokenize: false,
		} = std::mem::take(&mut self.params)
		else {
			bail!(
				"model {} references provider {} and can only set params.model",
				self.name,
				provider.name
			);
		};
		if matches!(&provider.provider, LocalModelAIProvider::Reference(_)) {
			bail!(
				"llm.providers.{} cannot reference another provider",
				provider.name
			);
		}
		self.params = provider.params.clone();
		if let Some(model_override) = model_override {
			self.params.model = Some(model_override);
		}
		self.provider = provider.provider.clone();
		if let Some(defaults) = provider.defaults.clone() {
			self.defaults = merge_optional_maps(defaults.defaults, self.defaults.take());
			self.overrides = merge_optional_maps(defaults.overrides, self.overrides.take());
			self.transformation =
				merge_optional_maps(defaults.transformation, self.transformation.take());
			self.request_headers = self.request_headers.take().or(defaults.request_headers);
			self.response_headers = self.response_headers.take().or(defaults.response_headers);
			self.backend_tls = self.backend_tls.take().or(defaults.backend_tls);
			self.auth = self.auth.take().or(defaults.auth);
			self.health = self.health.take().or(defaults.health);
			self.backend_tunnel = self.backend_tunnel.take().or(defaults.backend_tunnel);
			self.prompt_caching = self.prompt_caching.take().or(defaults.prompt_caching);
		}
		Ok(())
	}

	#[allow(deprecated)]
	fn apply_provider_defaults(&mut self) {
		if self.params.base_url.is_some()
			|| self.params.host_override.is_some()
			|| self.params.path_override.is_some()
		{
			return;
		}
		match &self.provider {
			LocalModelAIProvider::Cohere => {
				self
					.params
					.base_url
					.get_or_insert_with(|| strng::new("https://api.cohere.ai"));
			},
			LocalModelAIProvider::Ollama => {
				self
					.params
					.base_url
					.get_or_insert_with(|| strng::new("http://localhost:11434/v1"));
			},
			LocalModelAIProvider::Baseten => {
				self
					.params
					.base_url
					.get_or_insert_with(|| strng::new("https://inference.baseten.co/v1"));
			},
			LocalModelAIProvider::Cerebras => {
				self
					.params
					.base_url
					.get_or_insert_with(|| strng::new("https://api.cerebras.ai/v1"));
			},
			LocalModelAIProvider::Deepinfra => {
				self
					.params
					.base_url
					.get_or_insert_with(|| strng::new("https://api.deepinfra.com/v1/openai"));
			},
			LocalModelAIProvider::Deepseek => {
				self
					.params
					.base_url
					.get_or_insert_with(|| strng::new("https://api.deepseek.com/v1"));
			},
			LocalModelAIProvider::Groq => {
				self
					.params
					.base_url
					.get_or_insert_with(|| strng::new("https://api.groq.com/openai/v1"));
			},
			LocalModelAIProvider::Huggingface => {
				self
					.params
					.base_url
					.get_or_insert_with(|| strng::new("https://router.huggingface.co/v1"));
			},
			LocalModelAIProvider::Mistral => {
				self
					.params
					.base_url
					.get_or_insert_with(|| strng::new("https://api.mistral.ai/v1"));
			},
			LocalModelAIProvider::Openrouter => {
				self
					.params
					.base_url
					.get_or_insert_with(|| strng::new("https://openrouter.ai/api/v1"));
			},
			LocalModelAIProvider::Togetherai => {
				self
					.params
					.base_url
					.get_or_insert_with(|| strng::new("https://api.together.xyz/v1"));
			},
			LocalModelAIProvider::XAI => {
				self
					.params
					.base_url
					.get_or_insert_with(|| strng::new("https://api.x.ai/v1"));
			},
			LocalModelAIProvider::Fireworks => {
				self
					.params
					.base_url
					.get_or_insert_with(|| strng::new("https://api.fireworks.ai/inference/v1"));
			},
			_ => {},
		}
	}

	#[allow(deprecated)]
	fn apply_base_url(&mut self) -> anyhow::Result<()> {
		let Some(base_url) = self.params.base_url.as_deref() else {
			return Ok(());
		};
		let url = url::Url::parse(base_url)
			.with_context(|| format!("invalid params.baseUrl for model {}", self.name))?;
		let port = url.port_or_known_default().with_context(|| {
			format!(
				"params.baseUrl for model {} must use http or https, or include an explicit port",
				self.name
			)
		})?;
		let host = url
			.host_str()
			.with_context(|| format!("params.baseUrl for model {} must include a host", self.name))?;
		self
			.params
			.host_override
			.get_or_insert_with(|| (host, port).into());
		let path = url.path().trim_end_matches('/');
		if !path.is_empty() && self.params.path_override.is_none() {
			self
				.params
				.path_prefix
				.get_or_insert_with(|| strng::new(path));
		}
		if url.scheme() == "https" && self.backend_tls.is_none() {
			self.backend_tls = Some(http::backendtls::LocalBackendTLS::default());
		}
		Ok(())
	}
}

fn merge_optional_maps<T>(
	base: Option<HashMap<String, T>>,
	overrides: Option<HashMap<String, T>>,
) -> Option<HashMap<String, T>> {
	match (base, overrides) {
		(None, None) => None,
		(Some(base), None) => Some(base),
		(None, Some(overrides)) => Some(overrides),
		(Some(mut base), Some(overrides)) => {
			base.extend(overrides);
			Some(base)
		},
	}
}

fn custom_provider_format(
	format: custom::ProviderFormat,
	path: Option<&'static str>,
) -> custom::ProviderFormatConfig {
	custom::ProviderFormatConfig {
		format,
		path: path.map(strng::new),
	}
}

#[apply(schema_de!)]
struct LocalBind {
	port: u16,
	listeners: Vec<LocalListener>,
	#[serde(default)]
	tunnel_protocol: TunnelProtocol,
}

#[apply(schema_de!)]
pub struct LocalListenerName {
	// User facing name
	#[serde(default)]
	pub name: Option<Strng>,
	#[serde(default)]
	pub namespace: Option<Strng>,
}

#[apply(schema_de!)]
struct LocalListener {
	#[serde(flatten)]
	name: LocalListenerName,
	/// Can be a wildcard
	hostname: Option<Strng>,
	#[serde(default)]
	protocol: LocalListenerProtocol,
	tls: Option<LocalTLSServerConfig>,
	routes: Option<Vec<LocalRoute>>,
	tcp_routes: Option<Vec<LocalTCPRoute>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	policies: Option<LocalGatewayPolicy>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE", deny_unknown_fields)]
#[allow(clippy::upper_case_acronyms)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
enum LocalListenerProtocol {
	#[default]
	HTTP,
	HTTPS,
	TLS,
	TCP,
	HBONE,
}

#[derive(Default)]
#[apply(schema_de!)]
pub struct LocalTLSServerConfig {
	/// Certificate source mode. Static mode uses cert/key as the leaf certificate; dynamic CA
	/// mode uses cert/key as a CA for on-demand SNI leaf certificate issuance.
	#[serde(default)]
	pub mode: LocalTLSServerMode,
	pub cert: PathBuf,
	pub key: PathBuf,
	pub root: Option<PathBuf>,
	/// Optional cipher suite allowlist (order is preserved).
	#[cfg_attr(feature = "schema", schemars(with = "Option<Vec<String>>"))]
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub cipher_suites: Option<Vec<crate::transport::tls::CipherSuite>>,
	/// Minimum supported TLS version (only TLS 1.2 and 1.3 are supported).
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		rename = "minTLSVersion",
		alias = "minTlsVersion"
	)]
	pub min_tls_version: Option<frontend::TLSVersion>,
	/// Maximum supported TLS version (only TLS 1.2 and 1.3 are supported).
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		rename = "maxTLSVersion",
		alias = "maxTlsVersion"
	)]
	pub max_tls_version: Option<frontend::TLSVersion>,
	/// Key exchange groups allowed for negotiating TLS.
	#[cfg_attr(feature = "schema", schemars(with = "Option<Vec<String>>"))]
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub key_exchange_groups: Option<Vec<crate::transport::tls::KeyExchangeGroup>>,
}

#[apply(schema_enum!)]
#[derive(Default)]
pub enum LocalTLSServerMode {
	#[default]
	Static,
	DynamicCa,
}

#[apply(schema_de!)]
pub struct LocalRouteName {
	#[serde(default)]
	pub name: Option<Strng>,
	#[serde(default)]
	pub namespace: Option<Strng>,
	#[serde(default)]
	pub rule_name: Option<Strng>,
}

#[apply(schema_de!)]
pub struct LocalRouteGroup {
	name: RouteGroupKey,
	routes: Vec<LocalRoute>,
}

#[apply(schema_de!)]
pub struct LocalRoute {
	#[serde(flatten)]
	name: LocalRouteName,
	/// Can be a wildcard
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	hostnames: Vec<Strng>,
	#[serde(default = "default_matches")]
	matches: Vec<RouteMatch>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	policies: Option<FilterOrPolicy>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	backends: Vec<LocalRouteBackend>,
}

#[apply(schema_de!)]
pub struct LocalRouteBackend {
	#[serde(default = "default_weight")]
	pub weight: usize,
	#[serde(flatten)]
	pub backend: LocalBackend,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub policies: Option<LocalBackendPolicies>,
}

fn default_weight() -> usize {
	1
}

#[apply(schema_de!)]
pub struct FullLocalBackend {
	pub name: BackendKey,
	#[serde(flatten)]
	pub spec: FullLocalBackendSpec,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub policies: Option<LocalBackendPolicies>,
}

#[apply(schema_de!)]
#[allow(clippy::large_enum_variant)]
pub enum FullLocalBackendSpec {
	#[serde(rename = "host")]
	Opaque(Target),
	#[serde(rename = "mcp")]
	MCP(LocalMcpBackend),
	#[serde(rename = "ai")]
	AI(LocalAIBackend),
	#[serde(rename = "aws")]
	Aws(LocalAwsBackend),
}

impl From<FullLocalBackendSpec> for LocalBackend {
	fn from(spec: FullLocalBackendSpec) -> Self {
		match spec {
			FullLocalBackendSpec::Opaque(t) => LocalBackend::Opaque(t),
			FullLocalBackendSpec::MCP(m) => LocalBackend::MCP(m),
			FullLocalBackendSpec::AI(a) => LocalBackend::AI(a),
			FullLocalBackendSpec::Aws(a) => LocalBackend::Aws(a),
		}
	}
}

#[apply(schema_de!)]
pub struct LocalAwsBackend {
	#[serde(flatten)]
	pub service: LocalAwsService,
}

#[apply(schema_de!)]
pub enum LocalAwsService {
	#[serde(rename = "agentCore")]
	AgentCore(LocalAgentCoreBackend),
}

#[apply(schema_de!)]
pub struct LocalAgentCoreBackend {
	pub agent_runtime_arn: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub qualifier: Option<String>,
}

#[apply(schema_de!)]
#[allow(clippy::large_enum_variant)] // Size is not sensitive for local config
pub enum LocalBackend {
	// This one is a reference
	Service {
		name: NamespacedHostname,
		port: u16,
	},
	Backend(BackendKey),
	// Rest are inlined
	#[serde(rename = "host")]
	Opaque(Target), // Hostname or IP
	Dynamic {},
	#[serde(rename = "mcp")]
	MCP(LocalMcpBackend),
	#[serde(rename = "ai")]
	AI(LocalAIBackend),
	#[serde(rename = "aws")]
	Aws(LocalAwsBackend),
	#[serde(rename = "routeGroup")]
	RouteGroup(RouteGroupKey),
	Invalid,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[cfg_attr(feature = "schema", schemars(untagged, deny_unknown_fields))]
#[allow(clippy::large_enum_variant)] // Size is not sensitive for local config
pub enum LocalAIBackend {
	Provider(LocalNamedAIProvider),
	Groups { groups: Vec<LocalAIProviders> },
}

// Custom impl to avoid terrible 'not match any variant of untagged' errors.
impl<'de> Deserialize<'de> for LocalAIBackend {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		serde_untagged::UntaggedEnumVisitor::new()
			.map(|map| {
				let v: serde_json::Value = map.deserialize()?;

				if let serde_json::Value::Object(m) = &v
					&& m.len() == 1
					&& let Some(g) = m.get("groups")
				{
					Ok(LocalAIBackend::Groups {
						groups: Vec::<LocalAIProviders>::deserialize(g).map_err(serde::de::Error::custom)?,
					})
				} else {
					Ok(LocalAIBackend::Provider(
						LocalNamedAIProvider::deserialize(&v).map_err(serde::de::Error::custom)?,
					))
				}
			})
			.deserialize(deserializer)
	}
}

#[apply(schema_de!)]
pub struct LocalAIProviders {
	providers: Vec<LocalNamedAIProvider>,
}

#[apply(schema_de!)]
pub struct LocalNamedAIProvider {
	pub name: Strng,
	pub provider: AIProvider,
	/// Override the upstream host for this provider.
	pub host_override: Option<Target>,
	/// Override the upstream path for this provider.
	pub path_override: Option<Strng>,
	/// Override the default base path prefix for this provider.
	pub path_prefix: Option<Strng>,
	/// Whether to tokenize on the request flow. This enables us to do more accurate rate limits,
	/// since we know (part of) the cost of the request upfront.
	/// This comes with the cost of an expensive operation.
	#[serde(default)]
	pub tokenize: bool,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub policies: Option<LocalBackendPolicies>,
}

impl LocalAIBackend {
	pub fn translate(self) -> anyhow::Result<AIBackend> {
		let providers = match self {
			LocalAIBackend::Provider(p) => {
				vec![vec![p]]
			},
			LocalAIBackend::Groups { groups } => groups.into_iter().map(|g| g.providers).collect_vec(),
		};
		let mut ep_groups = vec![];
		for g in providers {
			let mut group = vec![];
			for p in g {
				validate_inference_routing_scope(
					p.policies.as_ref(),
					InferenceRoutingScope::AIProviderPolicies,
				)?;
				let policies = p
					.policies
					.map(|p| p.translate())
					.transpose()?
					.unwrap_or_default();
				group.push((
					p.name.clone(),
					NamedAIProvider {
						name: p.name,
						provider: p.provider,
						provider_backend: None,
						host_override: p.host_override,
						path_override: p.path_override,
						path_prefix: p.path_prefix,
						tokenize: p.tokenize,
						inline_policies: policies,
					},
				));
			}
			ep_groups.push(group);
		}
		let es = types::loadbalancer::EndpointSet::new(ep_groups);
		Ok(AIBackend { providers: es })
	}
}

impl LocalBackend {
	fn make_mcp_backend(
		b: Backend,
		policies: Option<MCPLocalBackendPolicies>,
		tls: bool,
	) -> Result<BackendWithPolicies, anyhow::Error> {
		let mut inline_policies = policies
			.map(|p| LocalBackendPolicies {
				simple: p.simple,
				mcp_authorization: p.mcp_authorization,
				mcp_guardrails: p.mcp_guardrails,
				a2a: None,
				inference_routing: None,
				ai: None,
				response_header_modifier: None,
				request_redirect: None,
				health: None,
				ext_authz: None,
			})
			.map(LocalBackendPolicies::translate)
			.transpose()?
			.unwrap_or_default();
		if tls {
			inline_policies.push(BackendTrafficPolicy::BackendTLS(
				LocalBackendTLS::default().try_into()?,
			));
		}
		Ok(BackendWithPolicies {
			backend: b,
			inline_policies,
		})
	}

	pub async fn as_backends(
		&self,
		name: ResourceName,
		client: client::Client,
		mcp_session_ttl: Duration,
	) -> anyhow::Result<Vec<BackendWithPolicies>> {
		Ok(match self {
			LocalBackend::Service { .. } => vec![], // These stay as references
			LocalBackend::Backend(_) => vec![],     // These stay as references
			LocalBackend::Opaque(tgt) => vec![Backend::Opaque(name, tgt.clone()).into()],
			LocalBackend::Dynamic { .. } => vec![Backend::Dynamic(name, ()).into()],
			LocalBackend::MCP(tgt) => {
				let mut targets = vec![];
				let mut backends = vec![];
				for (idx, t) in tgt.targets.iter().enumerate() {
					validate_mcp_target_name(t.name.as_str()).map_err(Error::msg)?;
					let name = strng::format!("mcp/{}/{}", name.clone(), idx);
					let mut process_backend = |backend: McpBackendHost| {
						Ok(match backend.process()? {
							ProcessedMcpBackendHost::Inline { backend, path, tls } => {
								let (bref, be) = mcp_to_simple_backend_and_ref(local_name(name.clone()), backend);
								if let Some(b) = be {
									backends.push(Self::make_mcp_backend(b, t.policies.clone(), tls)?);
								}
								(bref, Some(path))
							},
							ProcessedMcpBackendHost::Reference { .. } if t.policies.is_some() => {
								anyhow::bail!(
									"cannot use backend reference when policies are defined for an MCP target"
								);
							},
							ProcessedMcpBackendHost::Reference { backend, path } => (backend, path),
						})
					};

					let spec = match t.spec.clone() {
						LocalMcpTargetSpec::Sse { backend } => {
							let (bref, path) = process_backend(backend)?;
							McpTargetSpec::Sse(SseTargetSpec {
								backend: bref,
								path: path.ok_or_else(|| anyhow!("path is required when backend is set"))?,
							})
						},
						LocalMcpTargetSpec::Mcp { backend } => {
							let (bref, path) = process_backend(backend)?;
							McpTargetSpec::Mcp(StreamableHTTPTargetSpec {
								backend: bref,
								path: path.ok_or_else(|| anyhow!("path is required when backend is set"))?,
							})
						},
						LocalMcpTargetSpec::Stdio {
							cmd,
							args,
							env,
							clear_env,
						} => McpTargetSpec::Stdio {
							cmd,
							args,
							env,
							clear_env,
						},
						LocalMcpTargetSpec::OpenAPI { backend, schema } => {
							let (bref, _) = process_backend(backend)?;

							let openapi_schema = schema.load_openapi_schema(client.clone()).await?;
							McpTargetSpec::OpenAPI(OpenAPITarget {
								backend: bref,
								schema: openapi_schema.into(),
							})
						},
					};
					let t = McpTarget {
						name: t.name.clone(),
						spec,
					};
					targets.push(Arc::new(t));
				}
				let stateful = match &tgt.stateful_mode {
					McpStatefulMode::Stateless => false,
					McpStatefulMode::Stateful => true,
				};
				let m = McpBackend {
					targets,
					stateful,
					always_use_prefix: tgt.prefix_mode.as_ref().is_some_and(|pm| match pm {
						McpPrefixMode::Always => true,
						McpPrefixMode::Conditional => false,
					}),
					failure_mode: tgt.failure_mode.unwrap_or_default(),
					session_idle_ttl: mcp_session_ttl,
				};
				backends.push(Backend::MCP(name, m).into());
				backends
			},
			LocalBackend::AI(tgt) => {
				let be = tgt.clone().translate()?;
				vec![Backend::AI(name, be).into()]
			},
			LocalBackend::Aws(aws_backend) => {
				let config = match &aws_backend.service {
					LocalAwsService::AgentCore(ac) => {
						let agentcore_config =
							agentcore::AgentCoreConfig::new(ac.agent_runtime_arn.clone(), ac.qualifier.clone())?;
						crate::aws::AwsBackendConfig {
							service: crate::aws::AwsService::AgentCore(agentcore_config),
						}
					},
				};
				vec![Backend::Aws(name, config).into()]
			},
			LocalBackend::RouteGroup(_) => vec![], // Route groups stay as references
			LocalBackend::Invalid => vec![Backend::Invalid.into()],
		})
	}
}

impl SimpleLocalBackend {
	pub fn as_backends(
		&self,
		name: ResourceName,
		policies: Vec<BackendTrafficPolicy>,
	) -> Option<SimpleBackendWithPolicies> {
		match self {
			SimpleLocalBackend::Service { .. } => None, // These stay as references
			SimpleLocalBackend::Opaque(tgt) => Some(SimpleBackendWithPolicies {
				backend: SimpleBackend::Opaque(name, tgt.clone()),
				inline_policies: policies,
			}),
			SimpleLocalBackend::Backend(_) => None,
			SimpleLocalBackend::Invalid => Some(SimpleBackend::Invalid.into()),
		}
	}
}

#[apply(schema_de!)]
#[derive(Default)]
pub enum McpStatefulMode {
	Stateless,
	#[default]
	Stateful,
}

#[apply(schema_de!)]
#[derive(Default)]
pub enum McpPrefixMode {
	Always,
	#[default]
	Conditional,
}

#[apply(schema_de!)]
pub struct LocalMcpBackend {
	pub targets: Vec<Arc<LocalMcpTarget>>,
	#[serde(default)]
	pub stateful_mode: McpStatefulMode,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub prefix_mode: Option<McpPrefixMode>,
	/// Behavior when one or more MCP targets fail to initialize or fail during fanout.
	/// Defaults to `failClosed`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub failure_mode: Option<FailureMode>,
}

#[apply(schema_de!)]
pub struct LocalMcpTarget {
	pub name: McpTargetName,
	#[serde(flatten)]
	pub spec: LocalMcpTargetSpec,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub policies: Option<MCPLocalBackendPolicies>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[cfg_attr(feature = "schema", schemars(with = "McpBackendHostSerde"))]
pub enum McpBackendHost {
	Host {
		host: String,
		port: Option<u16>,
		path: Option<String>,
	},
	Backend {
		backend: BackendKey,
		path: Option<String>,
	},
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
struct McpBackendHostSerde {
	host: Option<String>,
	port: Option<u16>,
	path: Option<String>,
	backend: Option<BackendKey>,
}

pub enum ProcessedMcpBackendHost {
	Inline {
		backend: Target,
		path: String,
		tls: bool,
	},
	Reference {
		backend: SimpleBackendReference,
		path: Option<String>,
	},
}

impl<'de> serde::Deserialize<'de> for McpBackendHost {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		let raw = McpBackendHostSerde::deserialize(deserializer)?;
		match (raw.host, raw.port, raw.path, raw.backend) {
			(Some(host), port, path, None) => Ok(Self::Host { host, port, path }),
			(None, None, path, Some(backend)) => Ok(Self::Backend { backend, path }),
			(None, Some(_), _, Some(_)) | (Some(_), _, _, Some(_)) => Err(serde::de::Error::custom(
				"cannot mix host/port with backend for MCP target backend configuration",
			)),
			(None, Some(_), _, None) => Err(serde::de::Error::custom(
				"host is required when port is set for MCP target backend configuration",
			)),
			(None, None, Some(_), None) => Err(serde::de::Error::custom(
				"host or backend is required when path is set for MCP target backend configuration",
			)),
			(None, None, None, None) => Err(serde::de::Error::custom(
				"host or backend is required for MCP target backend configuration",
			)),
		}
	}
}

impl McpBackendHost {
	pub fn process(&self) -> anyhow::Result<ProcessedMcpBackendHost> {
		Ok(match self {
			McpBackendHost::Backend { backend, path } => ProcessedMcpBackendHost::Reference {
				backend: SimpleBackendReference::Backend(backend.clone()),
				path: path.clone(),
			},
			McpBackendHost::Host { host, port, path } => match (host, port, path) {
				(host, Some(port), Some(path)) => {
					let b = Target::from((host.as_str(), *port));
					ProcessedMcpBackendHost::Inline {
						backend: b,
						path: path.clone(),
						tls: false,
					}
				},
				(host, None, None) => {
					let uri = Uri::try_from(host.as_str())?;
					let Some(host) = uri.host() else {
						anyhow::bail!("no host")
					};
					let scheme = uri.scheme().unwrap_or(&http::Scheme::HTTP);
					let port = uri.port_u16();
					let path = uri.path();
					let port = match (scheme, port) {
						(s, p) if s == &http::Scheme::HTTP => p.unwrap_or(80),
						(s, p) if s == &http::Scheme::HTTPS => p.unwrap_or(443),
						(_, _) => {
							anyhow::bail!("invalid scheme: {:?}", scheme);
						},
					};

					let b = Target::from((host, port));
					ProcessedMcpBackendHost::Inline {
						backend: b,
						path: path.to_string(),
						tls: scheme == &http::Scheme::HTTPS,
					}
				},
				_ => {
					anyhow::bail!("if port or path is set, both must be set; otherwise, use only host")
				},
			},
		})
	}
}

#[apply(schema_de!)]
pub enum LocalMcpTargetSpec {
	#[serde(rename = "sse")]
	Sse {
		#[serde(flatten)]
		backend: McpBackendHost,
	},
	#[serde(rename = "mcp")]
	Mcp {
		#[serde(flatten)]
		backend: McpBackendHost,
	},
	#[serde(rename = "stdio")]
	Stdio {
		cmd: String,
		#[serde(default, skip_serializing_if = "Vec::is_empty")]
		args: Vec<String>,
		#[serde(default, skip_serializing_if = "HashMap::is_empty")]
		env: HashMap<String, String>,
		#[serde(default, skip_serializing_if = "std::ops::Not::not")]
		clear_env: bool,
	},
	#[serde(rename = "openapi")]
	OpenAPI {
		#[serde(flatten)]
		backend: McpBackendHost,
		schema: serdes::FileInlineOrRemote,
	},
}

fn default_matches() -> Vec<RouteMatch> {
	vec![RouteMatch {
		headers: vec![],
		path: PathMatch::PathPrefix("/".into()),
		method: None,
		query: vec![],
	}]
}

#[apply(schema_de!)]
struct LocalTCPRoute {
	#[serde(flatten)]
	name: LocalRouteName,
	/// Can be a wildcard
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	hostnames: Vec<Strng>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	policies: Option<TCPFilterOrPolicy>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	backends: Vec<LocalTCPRouteBackend>,
}

#[apply(schema_de!)]
pub struct LocalTCPRouteBackend {
	#[serde(default = "default_weight")]
	pub weight: usize,
	#[serde(flatten)]
	pub backend: SimpleLocalBackend,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub policies: Option<LocalTCPBackendPolicies>,
}

#[apply(schema_de!)]
pub enum SimpleLocalBackend {
	/// Service reference. Service must be defined in the top level services list.
	Service { name: NamespacedHostname, port: u16 },
	/// Hostname or IP address
	#[serde(rename = "host")]
	Opaque(
		/// Hostname or IP address
		Target,
	),
	Backend(
		/// Explicit backend reference. Backend must be defined in the top level backends list
		BackendKey,
	),
	#[serde(skip_deserializing)] // No need to deserialize an intentionally invalid entry
	#[cfg_attr(feature = "schema", schemars(skip))]
	Invalid,
}

impl SimpleLocalBackend {
	pub fn as_backend(&self, name: ResourceName) -> Option<Backend> {
		match self {
			SimpleLocalBackend::Service { .. } => None, // These stay as references
			SimpleLocalBackend::Backend(_) => None,     // These stay as references
			SimpleLocalBackend::Opaque(tgt) => Some(Backend::Opaque(name, tgt.clone())),
			SimpleLocalBackend::Invalid => Some(Backend::Invalid),
		}
	}
}

#[apply(schema_de!)]
struct LocalPolicy {
	/// Policy name used when attaching this policy to a target.
	pub name: ResourceName,
	/// Gateway, listener, route, or backend that this policy attaches to.
	pub target: PolicyTarget,

	/// When the policy runs. Gateway policies run before route selection, while route policies run after route selection.
	/// Use route policies by default unless the policy needs to affect route selection.
	#[serde(default)]
	pub phase: PolicyPhase,
	/// Policy settings to apply to the selected target.
	pub policy: FilterOrPolicy,
}

pub fn de_transform<'de, D>(
	deserializer: D,
) -> Result<Option<crate::http::transformation_cel::Transformation>, D::Error>
where
	D: Deserializer<'de>,
{
	<Option<LocalTransformationConfig>>::deserialize(deserializer)?
		.map(|c| http::transformation_cel::Transformation::try_from_local_config(c, true))
		.transpose()
		.map_err(serde::de::Error::custom)
}

pub fn de_backend_auth<'de, D>(deserializer: D) -> Result<Option<BackendAuth>, D::Error>
where
	D: Deserializer<'de>,
{
	#[derive(serde::Deserialize)]
	#[serde(untagged)]
	enum BackendAuthCompat {
		PlainKey {
			#[serde(deserialize_with = "deser_key_from_file")]
			key: SecretString,
		},
		Full(BackendAuth),
	}

	Option::<BackendAuthCompat>::deserialize(deserializer).map(|auth| {
		auth.map(|auth| match auth {
			BackendAuthCompat::Full(auth) => auth,
			BackendAuthCompat::PlainKey { key } => BackendAuth::Key {
				value: key,
				location: None,
			},
		})
	})
}

#[apply(schema_de!)]
#[derive(Default)]
struct LocalLLMPolicy {
	#[serde(flatten)]
	gateway: LocalGatewayPolicy,
	/// Guardrails to apply to every configured model.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	guardrails: Option<PromptGuard>,
	/// Local rate limits for incoming requests.
	#[serde(default)]
	local_rate_limit: Vec<crate::http::localratelimit::RateLimit>,
	/// Remote rate limit checks for incoming requests.
	#[serde(default)]
	remote_rate_limit: Option<crate::http::remoteratelimit::RemoteRateLimit>,
}

#[apply(schema_de!)]
#[derive(Default)]
struct LocalGatewayPolicy {
	/// Authenticate browser requests with OIDC authorization code flow.
	#[serde(default)]
	oidc: Option<crate::http::oidc::LocalOidcConfig>,
	/// Authenticate incoming requests with JWT bearer tokens.
	#[serde(default)]
	jwt_auth: Option<crate::http::jwt::LocalJwtConfig>,
	/// Authorization rules for incoming HTTP requests.
	#[serde(default)]
	authorization: Option<Authorization>,
	/// Authorize incoming requests by calling an external authorization service.
	#[serde(default)]
	ext_authz: Option<LocalExtAuthzPolicy>,
	/// Send request and response data to an external processing service.
	#[serde(default)]
	ext_proc: Option<LocalExtProcPolicy>,
	/// Handle CORS preflight requests and append configured CORS headers to applicable requests.
	#[serde(default)]
	cors: Option<http::cors::Cors>,
	/// Modify request and response headers, bodies, or metadata.
	#[serde(default)]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "Option<LocalTransformationPolicy>")
	)]
	transformations: Option<LocalTransformationPolicy>,
	/// Authenticate incoming requests with Basic Auth credentials from an htpasswd user database.
	#[serde(default)]
	basic_auth: Option<crate::http::basicauth::LocalBasicAuth>,
	/// Authenticate incoming requests with API keys.
	#[serde(default)]
	api_key: Option<crate::http::apikey::LocalAPIKeys>,
}

impl From<LocalGatewayPolicy> for FilterOrPolicy {
	fn from(val: LocalGatewayPolicy) -> Self {
		let LocalGatewayPolicy {
			oidc,
			jwt_auth,
			authorization,
			ext_authz,
			ext_proc,
			cors,
			transformations,
			basic_auth,
			api_key,
		} = val;
		FilterOrPolicy {
			oidc,
			jwt_auth,
			authorization,
			ext_authz,
			ext_proc,
			cors,
			transformations,
			basic_auth,
			api_key,
			..Default::default()
		}
	}
}

#[apply(schema_de!)]
#[derive(Default)]
pub struct SimpleLocalBackendPolicies {
	// Filters. Keep in sync with RouteFilter
	/// Modify request headers before forwarding to this backend.
	#[serde(default)]
	pub request_header_modifier: Option<filters::HeaderModifier>,

	/// Modify request and response data for this backend.
	#[serde(default)]
	#[serde(deserialize_with = "de_transform")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "Option<http::transformation_cel::LocalTransformationConfig>")
	)]
	pub transformations: Option<crate::http::transformation_cel::Transformation>,

	/// TLS settings used when connecting to this backend.
	#[serde(rename = "backendTLS", default)]
	pub backend_tls: Option<http::backendtls::LocalBackendTLS>,
	/// Authentication credentials sent to this backend.
	#[serde(default, deserialize_with = "de_backend_auth")]
	pub backend_auth: Option<BackendAuth>,

	/// HTTP protocol settings for this backend.
	#[serde(default)]
	pub http: Option<backend::HTTP>,
	/// TCP protocol settings for this backend.
	#[serde(default)]
	pub tcp: Option<backend::TCP>,

	/// Tunnel settings used when connecting to this backend.
	#[serde(default)]
	pub backend_tunnel: Option<backend::Tunnel>,
}

#[apply(schema_de!)]
#[derive(Default)]
pub struct MCPLocalBackendPolicies {
	#[serde(flatten)]
	simple: SimpleLocalBackendPolicies,
	/// Authorization rules for MCP requests.
	#[serde(default)]
	pub mcp_authorization: Option<McpAuthorization>,
	/// External MCP policy processors.
	#[serde(default)]
	pub mcp_guardrails: Option<LocalMcpGuardrails>,
}

#[apply(schema_de!)]
#[derive(Default)]
pub struct LocalBackendPolicies {
	#[serde(flatten)]
	simple: SimpleLocalBackendPolicies,

	/// Modify response headers returned from this backend.
	#[serde(default)]
	pub response_header_modifier: Option<filters::HeaderModifier>,

	/// Return a redirect response instead of forwarding to this backend.
	#[serde(default)]
	pub request_redirect: Option<filters::RequestRedirect>,

	/// Detect unhealthy backend responses and temporarily remove unhealthy endpoints.
	#[serde(default)]
	pub health: Option<health::LocalHealthPolicy>,

	/// Authorize incoming requests by calling an external authorization service after this backend is selected.
	#[serde(default)]
	pub ext_authz: Option<crate::http::ext_authz::ExtAuthz>,

	/// Authorization rules for MCP requests.
	#[serde(default)]
	pub mcp_authorization: Option<McpAuthorization>,
	/// External MCP policy processors.
	#[serde(default)]
	pub mcp_guardrails: Option<LocalMcpGuardrails>,
	/// Mark this traffic as A2A to enable A2A processing and telemetry.
	#[serde(default)]
	pub a2a: Option<A2aPolicy>,
	/// Route requests through an endpoint picker before forwarding to this backend.
	#[serde(default)]
	pub inference_routing: Option<crate::http::ext_proc::InferenceRouting>,
	/// Mark this as LLM traffic to enable LLM processing.
	#[serde(default)]
	pub ai: Option<llm::Policy>,
}

enum InferenceRoutingScope {
	ServiceRouteBackend,
	NonServiceRouteBackend,
	NamedBackend,
	AIProviderPolicies,
}

fn validate_inference_routing_scope(
	policies: Option<&LocalBackendPolicies>,
	scope: InferenceRoutingScope,
) -> anyhow::Result<()> {
	if policies.is_none_or(|p| p.inference_routing.is_none()) {
		return Ok(());
	}
	match scope {
		InferenceRoutingScope::ServiceRouteBackend => Ok(()),
		InferenceRoutingScope::NonServiceRouteBackend => {
			bail!("inferenceRouting is only supported on service route backends")
		},
		InferenceRoutingScope::NamedBackend => {
			bail!("inferenceRouting is only supported on service route backends, not named backends")
		},
		InferenceRoutingScope::AIProviderPolicies => {
			bail!(
				"inferenceRouting is only supported on service route backends, not AI provider policies"
			)
		},
	}
}

impl LocalBackendPolicies {
	pub fn translate(self) -> anyhow::Result<Vec<BackendTrafficPolicy>> {
		let LocalBackendPolicies {
			simple:
				SimpleLocalBackendPolicies {
					request_header_modifier,
					transformations,
					backend_tls,
					backend_auth,
					http,
					tcp,
					backend_tunnel,
				},
			mcp_authorization,
			mcp_guardrails,
			a2a,
			inference_routing,
			ai,
			response_header_modifier,
			request_redirect,
			health,
			ext_authz,
		} = self;
		let mut pols = vec![];
		if let Some(p) = tcp {
			pols.push(BackendTrafficPolicy::TCP(p));
		}
		if let Some(p) = backend_tunnel {
			pols.push(BackendTrafficPolicy::Tunnel(p));
		}
		if let Some(p) = http {
			pols.push(BackendTrafficPolicy::HTTP(p));
		}
		if let Some(p) = request_header_modifier {
			pols.push(BackendTrafficPolicy::RequestHeaderModifier(p));
		}
		if let Some(p) = response_header_modifier {
			pols.push(BackendTrafficPolicy::ResponseHeaderModifier(Arc::new(p)));
		}
		if let Some(p) = request_redirect {
			pols.push(BackendTrafficPolicy::RequestRedirect(p));
		}
		if let Some(p) = transformations {
			pols.push(BackendTrafficPolicy::Transformation(Arc::new(p)));
		}
		if let Some(p) = mcp_authorization {
			pols.push(BackendTrafficPolicy::McpAuthorization(p))
		}
		if let Some(p) = mcp_guardrails {
			for w in p.load_warnings() {
				tracing::warn!("{w}");
			}
			pols.push(BackendTrafficPolicy::McpGuardrails(Arc::new(p)))
		}
		if let Some(p) = a2a {
			pols.push(BackendTrafficPolicy::A2a(p))
		}
		if let Some(p) = inference_routing {
			pols.push(BackendTrafficPolicy::InferenceRouting(p))
		}
		if let Some(p) = backend_tls {
			pols.push(BackendTrafficPolicy::BackendTLS(p.try_into()?))
		}
		if let Some(p) = backend_auth {
			pols.push(BackendTrafficPolicy::BackendAuth(p))
		}
		if let Some(p) = ext_authz {
			pols.push(BackendTrafficPolicy::ExtAuthz(Arc::new(
				p.with_configured_cache_store(),
			)))
		}
		if let Some(mut p) = ai {
			p.compile_model_alias_patterns();
			pols.push(BackendTrafficPolicy::AI(Arc::new(p)))
		}
		if let Some(p) = health {
			pols.push(BackendTrafficPolicy::Health(p.try_into().map_err(
				|e: crate::cel::Error| anyhow::anyhow!("health.unhealthyExpression: {}", e),
			)?));
		}
		Ok(pols)
	}
}

#[apply(schema_de!)]
#[derive(Default)]
pub struct LocalTCPBackendPolicies {
	/// TLS settings used when connecting to this backend.
	#[serde(rename = "backendTLS", default)]
	pub backend_tls: Option<http::backendtls::LocalBackendTLS>,
	/// Tunnel settings used when connecting to this backend.
	#[serde(default)]
	pub backend_tunnel: Option<backend::Tunnel>,
}

impl LocalTCPBackendPolicies {
	pub fn translate(self) -> anyhow::Result<Vec<BackendTrafficPolicy>> {
		let LocalTCPBackendPolicies {
			backend_tls,
			backend_tunnel,
		} = self;
		let mut pols = vec![];
		if let Some(p) = backend_tls {
			pols.push(BackendTrafficPolicy::BackendTLS(p.try_into()?))
		}
		if let Some(p) = backend_tunnel {
			pols.push(BackendTrafficPolicy::Tunnel(p))
		}
		Ok(pols)
	}
}

#[apply(schema_de!)]
#[derive(Default)]
struct LocalFrontendPolicies {
	/// Settings for handling incoming HTTP requests.
	#[serde(default)]
	pub http: Option<frontend::HTTP>,
	/// Settings for handling incoming TLS connections.
	#[serde(default)]
	pub tls: Option<frontend::TLS>,
	/// Settings for handling incoming TCP connections.
	#[serde(default)]
	pub tcp: Option<frontend::TCP>,
	/// CEL authorization for downstream network connections.
	#[serde(default)]
	pub network_authorization: Option<frontend::NetworkAuthorization>,
	/// Enable downstream PROXY protocol handling on this gateway or port, including
	/// version matching and whether PROXY headers are required or optional.
	#[serde(default, rename = "proxyProtocol", alias = "proxy")]
	pub proxy_protocol: Option<frontend::Proxy>,
	/// Enable or disable downstream HTTP CONNECT handling.
	#[serde(default)]
	pub connect: Option<frontend::Connect>,
	/// Settings for request access logs.
	#[serde(default, alias = "logging")]
	pub access_log: Option<frontend::LoggingPolicy>,
	/// Settings for exporting request traces.
	#[serde(default)]
	pub tracing: Option<TracingConfig>,
}

#[apply(schema_de!)]
#[derive(Default)]
pub struct FilterOrPolicy {
	// Filters. Keep in sync with RouteFilter
	/// Modify request headers before forwarding.
	#[serde(default)]
	request_header_modifier: Option<filters::HeaderModifier>,

	/// Modify response headers before returning to the client.
	#[serde(default)]
	response_header_modifier: Option<filters::HeaderModifier>,

	/// Return a redirect response instead of forwarding the request.
	#[serde(default)]
	request_redirect: Option<filters::RequestRedirect>,

	/// Rewrite the request path or authority before forwarding.
	#[serde(default)]
	url_rewrite: Option<filters::UrlRewrite>,

	/// Send a copy of matching requests to another backend.
	#[serde(default)]
	request_mirror: Option<filters::RequestMirror>,

	/// Return a configured response instead of forwarding the request.
	#[serde(default)]
	direct_response: Option<LocalDirectResponsePolicy>,

	/// Handle CORS preflight requests and append configured CORS headers to applicable requests.
	#[serde(default)]
	cors: Option<http::cors::Cors>,

	// Policy
	/// Authorization rules for MCP requests.
	#[serde(default)]
	mcp_authorization: Option<McpAuthorization>,
	/// External MCP policy processors.
	#[serde(default)]
	mcp_guardrails: Option<LocalMcpGuardrails>,
	/// Authorization rules for incoming HTTP requests.
	#[serde(default)]
	authorization: Option<Authorization>,
	/// Authenticate MCP clients.
	#[serde(default)]
	mcp_authentication: Option<LocalMcpAuthentication>,
	/// Mark this traffic as A2A to enable A2A processing and telemetry.
	#[serde(default)]
	a2a: Option<A2aPolicy>,
	/// Mark this as LLM traffic to enable LLM processing.
	#[serde(default)]
	ai: Option<llm::Policy>,
	/// TLS settings used when connecting to the backend.
	#[serde(rename = "backendTLS", default)]
	backend_tls: Option<http::backendtls::LocalBackendTLS>,
	/// Tunnel settings used when connecting to the backend.
	#[serde(rename = "backendTunnel", default)]
	backend_tunnel: Option<backend::Tunnel>,
	/// Authentication credentials sent to the backend.
	#[serde(default, deserialize_with = "de_backend_auth")]
	backend_auth: Option<BackendAuth>,
	/// Local rate limits for incoming requests.
	#[serde(default)]
	local_rate_limit: Option<LocalRateLimitPolicy>,
	/// Remote rate limit checks for incoming requests.
	#[serde(default)]
	remote_rate_limit: Option<LocalRemoteRateLimitPolicy>,
	/// Authenticate incoming requests with JWT bearer tokens.
	#[serde(default)]
	jwt_auth: Option<crate::http::jwt::LocalJwtConfig>,
	/// Authenticate browser requests with OIDC authorization code flow.
	#[serde(default)]
	oidc: Option<crate::http::oidc::LocalOidcConfig>,
	/// Authenticate incoming requests with Basic Auth credentials from an htpasswd user database.
	#[serde(default)]
	basic_auth: Option<crate::http::basicauth::LocalBasicAuth>,
	/// Authenticate incoming requests with API keys.
	#[serde(default)]
	api_key: Option<crate::http::apikey::LocalAPIKeys>,
	/// Authorize incoming requests by calling an external authorization service.
	#[serde(default)]
	ext_authz: Option<LocalExtAuthzPolicy>,
	/// Send request and response data to an external processing service.
	#[serde(default)]
	ext_proc: Option<LocalExtProcPolicy>,
	/// Modify request and response headers, bodies, or metadata.
	#[serde(default)]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "Option<LocalTransformationPolicy>")
	)]
	transformations: Option<LocalTransformationPolicy>,

	/// Handle CSRF protection by validating request origins against configured allowed origins.
	#[serde(default)]
	csrf: Option<http::csrf::Csrf>,

	// TrafficPolicy
	/// Buffer request and response bodies.
	#[serde(default)]
	buffer: Option<http::buffer::Buffer>,
	/// Set request timeout limits.
	#[serde(default)]
	timeout: Option<timeout::Policy>,
	/// Retry matching failed upstream requests.
	#[serde(default)]
	retry: Option<retry::Policy>,
}

#[apply(schema_de!)]
struct TCPFilterOrPolicy {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	#[serde(rename = "backendTLS")]
	backend_tls: Option<LocalBackendTLS>,
}

async fn convert(
	client: client::Client,
	gateway: ListenerTarget,
	config: &crate::Config,
	i: LocalConfig,
) -> anyhow::Result<NormalizedLocalConfig> {
	validate_local_listener_ports(&i)?;
	let LocalConfig {
		config: _,
		mut frontend_policies,
		binds,
		policies,
		workloads,
		services,
		backends,
		route_groups,
		llm,
		mcp,
	} = i;
	merge_deprecated_frontend_policies(config, &mut frontend_policies)?;
	let mut all_policies = vec![];
	let mut all_backends = vec![];
	let mut all_binds = vec![];
	let mut all_listener_routes = vec![];
	let mut all_listener_tcp_routes = vec![];
	for b in binds {
		let bind_name = strng::format!("bind/{}", b.port);
		let mut ls = ListenerSet::default();
		for (idx, l) in b.listeners.into_iter().enumerate() {
			let (l, routes, tcp_routes, pol, backends) = convert_listener(
				client.clone(),
				config,
				idx,
				l,
				bind_name.clone(),
				gateway.clone(),
			)
			.await?;
			all_listener_routes.push((l.key.clone(), routes));
			all_listener_tcp_routes.push((l.key.clone(), tcp_routes));
			all_policies.extend_from_slice(&pol);
			all_backends.extend_from_slice(&backends);
			ls.insert(l)
		}
		let sockaddr = if cfg!(target_family = "unix") && config.ipv6_enabled {
			SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), b.port)
		} else {
			// Windows and IPv6 don't mix well apparently?
			SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), b.port)
		};
		let b = Bind {
			key: bind_name,
			address: sockaddr,
			protocol: detect_bind_protocol(&ls),
			listeners: ls,
			tunnel_protocol: b.tunnel_protocol,
		};
		all_binds.push(b)
	}

	for p in policies {
		p.target.validate()?;
		let policy_key = p.name.to_string();
		let res = split_policies(
			client.clone(),
			p.policy,
			config.as_policy_context(&policy_key),
		)
		.await?;
		if (res.route_policies.len() + res.backend_policies.len()) != 1 {
			bail!("'policies' must contain exactly 1 policy");
		}
		let tp = if let Some(route_policy) = res.route_policies.into_iter().next() {
			PolicyType::from((route_policy, p.phase))
		} else {
			res.backend_policies.into_iter().next().unwrap().into()
		};
		let tgt_policy = TargetedPolicy {
			name: Some(TypedResourceName {
				kind: strng::literal!("Local"),
				name: p.name.name.clone(),
				namespace: p.name.namespace.clone(),
			}),
			key: p.name.to_string().into(),
			target: p.target,
			inheritance: Default::default(),
			policy: tp,
		};
		all_policies.push(tgt_policy);
	}

	for b in backends {
		validate_inference_routing_scope(b.policies.as_ref(), InferenceRoutingScope::NamedBackend)?;
		let policies = b
			.policies
			.map(|p| p.translate())
			.transpose()?
			.unwrap_or_default();
		let name = local_name(b.name);
		let lb: LocalBackend = b.spec.into();
		let mut bws = lb
			.as_backends(name.clone(), client.clone(), config.mcp.session_ttl)
			.await?;

		// as_backends may expand a single LocalBackend into multiple Backends (e.g. MCP)
		// attach the policies to the "main" one
		// do not use `Backend::name` as it may create a computed name, not based
		if let Some(primary_bw) = bws.iter_mut().find(|bw| match &bw.backend {
			Backend::Opaque(n, _)
			| Backend::MCP(n, _)
			| Backend::AI(n, _)
			| Backend::Aws(n, _)
			| Backend::Dynamic(n, _) => n == &name,
			Backend::Service(_, _) | Backend::Invalid => false,
		}) {
			primary_bw.inline_policies.extend_from_slice(&policies);
		} else {
			anyhow::bail!("as_backends did not return a backend with the expected name: {name}");
		}

		all_backends.extend(bws);
	}

	// Convert llm config if present
	if let Some(llm_config) = llm {
		let (llm_bind, llm_routes, llm_policies, llm_backends) =
			convert_llm_config(client.clone(), config, gateway.clone(), llm_config).await?;
		all_listener_routes.push((strng::new("llm"), llm_routes));
		all_listener_tcp_routes.push((strng::new("llm"), Vec::new()));
		all_binds.push(llm_bind);
		all_policies.extend_from_slice(&llm_policies);
		all_backends.extend_from_slice(&llm_backends);
	}
	if let Some(mcp_config) = mcp {
		let (mcp_bind, mcp_routes, mcp_policies, mcp_backends) =
			convert_mcp_config(client.clone(), config, gateway.clone(), mcp_config).await?;
		all_listener_routes.push((strng::new("mcp"), mcp_routes));
		all_listener_tcp_routes.push((strng::new("mcp"), Vec::new()));
		all_binds.push(mcp_bind);
		all_policies.extend_from_slice(&mcp_policies);
		all_backends.extend_from_slice(&mcp_backends);
	}

	// Convert route groups
	let mut all_route_groups = vec![];
	for rg in route_groups {
		let rg_key = rg.name.clone();
		let mut routes = vec![];
		for (idx, lr) in rg.routes.into_iter().enumerate() {
			let route_group_listener_key: ListenerKey = strng::format!("routegroup/{rg_key}");
			let (route, backends) =
				convert_route(client.clone(), config, lr, idx, route_group_listener_key).await?;
			all_backends.extend_from_slice(&backends);
			routes.push(route);
		}
		all_route_groups.push((rg_key, routes));
	}

	// Add frontend policies targeted to this listener
	all_policies.extend_from_slice(&split_frontend_policies(gateway, frontend_policies).await?);

	let normalized = NormalizedLocalConfig {
		binds: all_binds,
		listener_routes: all_listener_routes,
		listener_tcp_routes: all_listener_tcp_routes,
		policies: all_policies,
		backends: all_backends.into_iter().collect(),
		route_groups: all_route_groups,
		workloads,
		services,
	};
	Ok(normalized)
}

fn validate_local_listener_ports(config: &LocalConfig) -> anyhow::Result<()> {
	let mut ports = HashMap::new();

	let mut insert_local_listener_port = |port: u16, label: String| {
		if let Some(existing) = ports.insert(port, label.clone()) {
			bail!(
				"port {port} is configured by both {existing} and {label}; binds, llm, and mcp must use unique ports"
			);
		}
		Ok(())
	};
	for (idx, bind) in config.binds.iter().enumerate() {
		insert_local_listener_port(bind.port, format!("binds[{idx}]"))?;
	}
	if let Some(llm) = &config.llm {
		insert_local_listener_port(
			llm.port.unwrap_or(DEFAULT_LLM_PORT),
			if llm.port.is_some() {
				"llm".to_string()
			} else {
				"llm (default)".to_string()
			},
		)?;
	}
	if let Some(mcp) = &config.mcp {
		insert_local_listener_port(
			mcp.port.unwrap_or(DEFAULT_MCP_PORT),
			if mcp.port.is_some() {
				"mcp".to_string()
			} else {
				"mcp (default)".to_string()
			},
		)?;
	}
	Ok(())
}

static STARTUP_TIMESTAMP: OnceLock<u64> = OnceLock::new();

fn llm_model_match_specificity(model_name: &str) -> usize {
	model_name.chars().filter(|c| *c != '*').count()
}

fn validate_llm_model_pattern(pattern: &str) -> anyhow::Result<()> {
	let wildcard_count = pattern.chars().filter(|c| *c == '*').count();
	if wildcard_count > 1 {
		bail!("model name wildcard may only appear once: '{pattern}'");
	}
	if wildcard_count == 1 && pattern != "*" && !pattern.starts_with('*') && !pattern.ends_with('*') {
		bail!("model name wildcard must be either at the beginning or the end: '{pattern}'");
	}
	Ok(())
}

fn merge_prompt_guards(
	shared: Option<PromptGuard>,
	model: Option<PromptGuard>,
) -> Option<PromptGuard> {
	match (shared, model) {
		(None, None) => None,
		(Some(guardrails), None) | (None, Some(guardrails)) => Some(guardrails),
		(Some(mut shared), Some(model)) => {
			if model.streaming.is_enabled() {
				shared.streaming = model.streaming;
			}
			shared.request.extend(model.request);
			shared.response.extend(model.response);
			Some(shared)
		},
	}
}

#[derive(Clone)]
struct ResolvedLLMModelTarget {
	name: String,
	provider: NamedAIProvider,
	inline_policies: Vec<BackendTrafficPolicy>,
	authorization: Option<Authorization>,
}

struct LocalLLMModelRegistry {
	models: Vec<LocalLLMModels>,
	virtual_models: Vec<LocalLLMVirtualModel>,
}

struct ResolvedLLMModelRegistry {
	models: Vec<ResolvedLLMModelTarget>,
}

enum LocalLLMVirtualRoutingStrategy<'a> {
	Weighted(&'a LocalLLMWeightedRouting),
	Failover(&'a LocalLLMFailoverRouting),
	Conditional(&'a LocalLLMConditionalRouting),
}

fn llm_model_matches(pattern: &str, model: &str) -> anyhow::Result<bool> {
	validate_llm_model_pattern(pattern)?;
	if pattern == "*" {
		return Ok(true);
	}
	if let Some(prefix) = pattern.strip_suffix('*') {
		return Ok(model.starts_with(prefix));
	}
	if let Some(suffix) = pattern.strip_prefix('*') {
		return Ok(model.ends_with(suffix));
	}
	Ok(pattern == model)
}

impl<'a> LocalLLMVirtualRoutingStrategy<'a> {
	fn targets(&self) -> Box<dyn Iterator<Item = &'a str> + 'a> {
		match self {
			Self::Weighted(weighted) => {
				Box::new(weighted.targets.iter().map(|target| target.model.as_str()))
			},
			Self::Failover(failover) => {
				Box::new(failover.targets.iter().map(|target| target.model.as_str()))
			},
			Self::Conditional(conditional) => Box::new(
				conditional
					.targets
					.iter()
					.map(|target| target.model.as_str()),
			),
		}
	}
}

impl LocalLLMVirtualModel {
	fn routing_strategy(&self) -> anyhow::Result<LocalLLMVirtualRoutingStrategy<'_>> {
		let strategy_count = usize::from(self.routing.weighted.is_some())
			+ usize::from(self.routing.failover.is_some())
			+ usize::from(self.routing.conditional.is_some());
		if strategy_count != 1 {
			bail!(
				"virtual model {} must specify exactly one routing strategy",
				self.name
			);
		}
		if let Some(conditional) = self.routing.conditional.as_ref() {
			if conditional.targets.is_empty() {
				bail!(
					"virtual model {} must specify at least one conditional target",
					self.name
				);
			}
			if let Some(unconditional_idx) = conditional
				.targets
				.iter()
				.position(|target| target.when.is_none())
				&& unconditional_idx + 1 != conditional.targets.len()
			{
				bail!(
					"virtual model {} conditional fallback target must be last",
					self.name
				);
			}
			return Ok(LocalLLMVirtualRoutingStrategy::Conditional(conditional));
		}
		if let Some(weighted) = self.routing.weighted.as_ref() {
			if weighted.targets.is_empty() {
				bail!(
					"virtual model {} must specify at least one weighted target",
					self.name
				);
			}
			return Ok(LocalLLMVirtualRoutingStrategy::Weighted(weighted));
		}
		let failover = self
			.routing
			.failover
			.as_ref()
			.expect("strategy count checked");
		if failover.targets.is_empty() {
			bail!(
				"virtual model {} must specify at least one failover target",
				self.name
			);
		}
		Ok(LocalLLMVirtualRoutingStrategy::Failover(failover))
	}
}

impl LocalLLMModelRegistry {
	fn new(
		models: Vec<LocalLLMModels>,
		virtual_models: Vec<LocalLLMVirtualModel>,
	) -> anyhow::Result<Self> {
		let registry = Self {
			models,
			virtual_models,
		};
		registry.validate_model_patterns()?;
		registry.validate_virtual_models()?;
		Ok(registry)
	}

	fn validate_model_patterns(&self) -> anyhow::Result<()> {
		for model in &self.models {
			validate_llm_model_pattern(&model.name)?;
		}
		Ok(())
	}

	fn model_matches(&self, target: &str) -> anyhow::Result<bool> {
		for model in &self.models {
			if llm_model_matches(&model.name, target)? {
				return Ok(true);
			}
		}
		Ok(false)
	}

	fn validate_virtual_models(&self) -> anyhow::Result<()> {
		for virtual_model in &self.virtual_models {
			for target in virtual_model.routing_strategy()?.targets() {
				if !self.model_matches(target)? {
					bail!("virtual model target {target} does not match any llm.models entry");
				}
			}
		}
		Ok(())
	}

	fn ordered_models(&self) -> Vec<(usize, LocalLLMModels)> {
		self
			.models
			.iter()
			.cloned()
			.enumerate()
			.sorted_by_key(|(original_idx, model)| {
				(
					std::cmp::Reverse(llm_model_match_specificity(&model.name)),
					*original_idx,
				)
			})
			.collect_vec()
	}

	fn into_virtual_models(self) -> Vec<LocalLLMVirtualModel> {
		self.virtual_models
	}
}

impl ResolvedLLMModelRegistry {
	fn new() -> Self {
		Self { models: Vec::new() }
	}

	fn push(&mut self, model: ResolvedLLMModelTarget) {
		self.models.push(model);
	}

	fn resolve(&self, target: &str) -> anyhow::Result<ResolvedLLMModelTarget> {
		for model in &self.models {
			if llm_model_matches(&model.name, target)? {
				return Ok(model.clone());
			}
		}
		bail!("virtual model target {target} does not match any llm.models entry")
	}

	fn resolve_failover_target(&self, target: &str) -> anyhow::Result<ResolvedLLMModelTarget> {
		let resolved = self.resolve(target)?;
		if resolved.authorization.is_some() {
			// Technically this is possible but would require us to move authorization down into post-LB.
			bail!(
				"virtual model target {target} has authorization; failover virtual models cannot target authorized models"
			);
		}
		Ok(resolved)
	}
}

fn llm_route_types(
	passthrough: Option<&LocalLLMPassthrough>,
) -> Vec<(Strng, crate::llm::RouteType)> {
	if let Some(passthrough) = passthrough {
		return vec![(strng::new("*"), passthrough.route_type())];
	}
	vec![
		(
			strng::new("/v1/chat/completions"),
			crate::llm::RouteType::Completions,
		),
		(strng::new("/v1/messages"), crate::llm::RouteType::Messages),
		// TODO: we could do this to support vertex calls. But we would need to extract the model name from the URL
		(strng::new(":rawPredict"), crate::llm::RouteType::Messages),
		(
			strng::new(":streamRawPredict"),
			crate::llm::RouteType::Messages,
		),
		(
			strng::new("/v1/responses"),
			crate::llm::RouteType::Responses,
		),
		(
			strng::new("/v1/images/generations"),
			crate::llm::RouteType::Detect,
		),
		(
			strng::new("/v1/images/edits"),
			crate::llm::RouteType::Detect,
		),
		(
			strng::new("/v1/images/variations"),
			crate::llm::RouteType::Detect,
		),
		(
			strng::new("/v1/responses/compact"),
			crate::llm::RouteType::Detect,
		),
		(
			strng::new("/v1/embeddings"),
			crate::llm::RouteType::Embeddings,
		),
		(strng::new("/v1/rerank"), crate::llm::RouteType::Rerank),
		(strng::new("/v2/rerank"), crate::llm::RouteType::Rerank),
		(strng::new("*"), crate::llm::RouteType::Passthrough),
	]
}

fn ensure_ai_provider_model(provider: &mut AIProvider, model: &str) {
	let model = || Some(strng::new(model));
	match provider {
		AIProvider::Anthropic(p) => p.model = p.model.clone().or_else(model),
		AIProvider::OpenAI(p) => p.model = p.model.clone().or_else(model),
		AIProvider::Copilot(p) => p.model = p.model.clone().or_else(model),
		AIProvider::Gemini(p) => p.model = p.model.clone().or_else(model),
		AIProvider::Custom(p) => p.model = p.model.clone().or_else(model),
		AIProvider::Vertex(p) => p.model = p.model.clone().or_else(model),
		AIProvider::Bedrock(p) => p.model = p.model.clone().or_else(model),
		AIProvider::Azure(p) => p.model = p.model.clone().or_else(model),
	}
}

#[allow(deprecated)]
async fn convert_llm_config(
	client: client::Client,
	config: &crate::Config,
	gateway: ListenerTarget,
	llm_config: LocalLLMConfig,
) -> anyhow::Result<(
	Bind,
	Vec<Route>,
	Vec<TargetedPolicy>,
	Vec<BackendWithPolicies>,
)> {
	let LocalLLMConfig {
		port,
		tls,
		providers,
		models,
		virtual_models,
		policies,
	} = llm_config;
	let port = port.unwrap_or(DEFAULT_LLM_PORT);
	let tls = tls.map(TryInto::try_into).transpose()?;
	let llm_registry = LocalLLMModelRegistry::new(models, virtual_models)?;

	let mut all_policies = vec![];
	let mut all_backends = vec![];
	let mut routes = Vec::new();
	let mut shared_prompt_guard = None;
	let (listener_gateway_policies, listener_route_policies) = if let Some(pol) = policies {
		let LocalLLMPolicy {
			gateway,
			guardrails,
			local_rate_limit,
			remote_rate_limit,
		} = pol;
		// Guardrail is per-model config, but we let users configure it top level. Pull it out here.
		shared_prompt_guard = guardrails;
		// Rate limit is applied per-route as well. Resolve them here.
		let route_policies = split_policies(
			client.clone(),
			FilterOrPolicy {
				local_rate_limit: (!local_rate_limit.is_empty())
					.then_some(LocalRateLimitPolicy::Explicit(local_rate_limit)),
				remote_rate_limit: remote_rate_limit.map(LocalExplicitOrConditional::Explicit),
				..Default::default()
			},
			None,
		)
		.await?;

		// Rest of policies are PreRoute gateway policies; resolve these to our listener.
		let gateway_policies: FilterOrPolicy = gateway.into();
		let gateway_policies = split_policies(
			client.clone(),
			gateway_policies,
			config.as_policy_context("listener/llm"),
		)
		.await?;
		(
			gateway_policies.route_policies,
			route_policies.route_policies,
		)
	} else {
		(vec![], vec![])
	};

	// Get static startup unix timestamp
	#[cfg(test)]
	let startup_timestamp = 0;
	#[cfg(not(test))]
	let startup_timestamp = *STARTUP_TIMESTAMP.get_or_init(|| {
		SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs()
	});

	let ordered_models = llm_registry.ordered_models();
	let mut resolved_models = ResolvedLLMModelRegistry::new();
	let mut router_models = Vec::new();
	let mut providers_by_name = HashMap::new();
	for provider in providers {
		if providers_by_name
			.insert(provider.name.clone(), provider)
			.is_some()
		{
			bail!("llm.providers contains duplicate provider names");
		}
	}

	// Create routes and backends for each model
	for (idx, (_, model_config)) in ordered_models.into_iter().enumerate() {
		let mut model_config = model_config;
		if let LocalModelAIProvider::Reference(reference) = &model_config.provider {
			let provider = providers_by_name.get(reference).with_context(|| {
				format!(
					"model {} references unknown provider {}",
					model_config.name, reference
				)
			})?;
			model_config.apply_provider_reference(provider)?;
		}
		model_config.apply_provider_defaults();
		model_config.apply_base_url()?;
		let model_name = strng::new(&model_config.name);
		// Index is needed because the same name can be used with different match criteria
		let backend_key = strng::format!("llm:model:{}:{idx}", model_config.name);
		let p = model_config.params.clone();
		let model = p.model;
		let llm_routes = llm_route_types(model_config.passthrough.as_ref());

		// Use provider from config and set the model name
		let provider = match &model_config.provider {
			LocalModelAIProvider::Reference(reference) => {
				bail!(
					"model {} has unresolved provider reference {}",
					model_config.name,
					reference
				)
			},
			LocalModelAIProvider::Anthropic => AIProvider::Anthropic(anthropic::Provider { model }),
			LocalModelAIProvider::OpenAI => AIProvider::OpenAI(openai::Provider { model }),
			LocalModelAIProvider::Copilot => AIProvider::Copilot(copilot::Provider { model }),
			LocalModelAIProvider::Gemini => AIProvider::Gemini(crate::llm::gemini::Provider { model }),
			LocalModelAIProvider::Custom(custom_provider) => {
				if custom_provider.formats.is_empty() {
					bail!(
						"custom provider for model {} must specify at least one format",
						model_config.name
					);
				}
				if p.host_override.is_none() {
					bail!(
						"custom provider for model {} requires params.baseUrl",
						model_config.name
					);
				}
				AIProvider::Custom(crate::llm::custom::Provider {
					model: model.or_else(|| custom_provider.model.clone()),
					..custom_provider.clone()
				})
			},
			LocalModelAIProvider::Cohere => AIProvider::Custom(custom::Provider {
				model,
				provider_override: Some(strng::literal!("cohere")),
				formats: vec![
					custom_provider_format(
						custom::ProviderFormat::Completions,
						Some("/compatibility/v1/chat/completions"),
					),
					custom_provider_format(
						custom::ProviderFormat::Embeddings,
						Some("/compatibility/v1/embeddings"),
					),
					custom_provider_format(custom::ProviderFormat::Rerank, Some("/v2/rerank")),
				],
			}),
			LocalModelAIProvider::Ollama => AIProvider::Custom(custom::Provider {
				model,
				provider_override: Some(strng::literal!("ollama")),
				formats: vec![
					custom_provider_format(custom::ProviderFormat::Completions, None),
					custom_provider_format(custom::ProviderFormat::Responses, None),
					custom_provider_format(custom::ProviderFormat::Embeddings, None),
				],
			}),
			LocalModelAIProvider::Baseten => AIProvider::Custom(custom::Provider {
				model,
				provider_override: Some(strng::literal!("baseten")),
				formats: vec![
					custom_provider_format(custom::ProviderFormat::Completions, None),
					custom_provider_format(custom::ProviderFormat::Messages, None),
				],
			}),
			LocalModelAIProvider::Cerebras => AIProvider::Custom(custom::Provider {
				model,
				provider_override: Some(strng::literal!("cerebras")),
				formats: vec![custom_provider_format(
					custom::ProviderFormat::Completions,
					None,
				)],
			}),
			LocalModelAIProvider::Deepinfra => AIProvider::Custom(custom::Provider {
				model,
				provider_override: Some(strng::literal!("deepinfra")),
				formats: vec![
					custom_provider_format(custom::ProviderFormat::Completions, None),
					custom_provider_format(
						custom::ProviderFormat::Messages,
						Some("/anthropic/v1/messages"),
					),
					custom_provider_format(custom::ProviderFormat::Embeddings, None),
				],
			}),
			LocalModelAIProvider::Deepseek => AIProvider::Custom(custom::Provider {
				model,
				provider_override: Some(strng::literal!("deepseek")),
				formats: vec![
					custom_provider_format(custom::ProviderFormat::Completions, None),
					custom_provider_format(
						custom::ProviderFormat::Messages,
						Some("/anthropic/v1/messages"),
					),
				],
			}),
			LocalModelAIProvider::Groq => AIProvider::Custom(custom::Provider {
				model,
				provider_override: Some(strng::literal!("groq")),
				formats: vec![
					custom_provider_format(custom::ProviderFormat::Completions, None),
					custom_provider_format(custom::ProviderFormat::Responses, None),
				],
			}),
			LocalModelAIProvider::Huggingface => AIProvider::Custom(custom::Provider {
				model,
				provider_override: Some(strng::literal!("huggingface")),
				formats: vec![
					custom_provider_format(custom::ProviderFormat::Completions, None),
					custom_provider_format(custom::ProviderFormat::Responses, None),
				],
			}),
			LocalModelAIProvider::Mistral => AIProvider::Custom(custom::Provider {
				model,
				provider_override: Some(strng::literal!("mistral")),
				formats: vec![
					custom_provider_format(custom::ProviderFormat::Completions, None),
					custom_provider_format(custom::ProviderFormat::Embeddings, None),
				],
			}),
			LocalModelAIProvider::Openrouter => AIProvider::Custom(custom::Provider {
				model,
				provider_override: Some(strng::literal!("openrouter")),
				formats: vec![
					custom_provider_format(custom::ProviderFormat::Completions, None),
					custom_provider_format(custom::ProviderFormat::Messages, None),
					custom_provider_format(custom::ProviderFormat::Responses, None),
					custom_provider_format(custom::ProviderFormat::Embeddings, None),
					custom_provider_format(custom::ProviderFormat::Rerank, None),
				],
			}),
			LocalModelAIProvider::Togetherai => AIProvider::Custom(custom::Provider {
				model,
				provider_override: Some(strng::literal!("togetherai")),
				formats: vec![
					custom_provider_format(custom::ProviderFormat::Completions, None),
					custom_provider_format(custom::ProviderFormat::Embeddings, None),
					custom_provider_format(custom::ProviderFormat::Rerank, None),
				],
			}),
			LocalModelAIProvider::XAI => AIProvider::Custom(custom::Provider {
				model,
				provider_override: Some(strng::literal!("xai")),
				formats: vec![
					custom_provider_format(custom::ProviderFormat::Completions, None),
					custom_provider_format(custom::ProviderFormat::Responses, None),
					custom_provider_format(custom::ProviderFormat::Realtime, None),
				],
			}),
			LocalModelAIProvider::Fireworks => AIProvider::Custom(custom::Provider {
				model,
				provider_override: Some(strng::literal!("fireworks")),
				formats: vec![
					custom_provider_format(custom::ProviderFormat::Completions, None),
					custom_provider_format(custom::ProviderFormat::Messages, None),
					custom_provider_format(custom::ProviderFormat::Responses, None),
					custom_provider_format(custom::ProviderFormat::Embeddings, None),
					custom_provider_format(custom::ProviderFormat::Rerank, None),
				],
			}),
			LocalModelAIProvider::Vertex => AIProvider::Vertex(crate::llm::vertex::Provider {
				model,
				region: p.vertex_region,
				project_id: p.vertex_project.context("vertex requires vertex_project")?,
			}),
			LocalModelAIProvider::Bedrock => AIProvider::Bedrock(crate::llm::bedrock::Provider {
				model,
				region: p.aws_region.context("bedrock requires aws_region")?,
				guardrail_identifier: None,
				guardrail_version: None,
				source_credentials_cache: Default::default(),
				assume_role_cache: Default::default(),
			}),
			LocalModelAIProvider::Azure => AIProvider::Azure(crate::llm::azure::Provider {
				model,
				resource_name: p
					.azure_resource_name
					.context("azure requires azureResourceName")?,
				resource_type: p
					.azure_resource_type
					.context("azure requires azureResourceType")?,
				api_version: p.azure_api_version,
				project_name: p.azure_project_name,
				cached_cred: Default::default(),
			}),
		};

		// Create backend auth policy
		let mut pols = vec![];
		if let Some(key) = p.api_key.as_ref() {
			let backend_auth = BackendAuth::Key {
				value: key.0.clone(),
				location: None,
			};
			pols.push(BackendTrafficPolicy::BackendAuth(backend_auth));
		}

		// Create AI backend
		let named_provider = NamedAIProvider {
			name: model_name.clone(),
			provider,
			provider_backend: None,
			host_override: p.host_override,
			path_override: p.path_override,
			path_prefix: p.path_prefix,
			tokenize: p.tokenize,
			inline_policies: pols,
		};
		let resolved_provider = named_provider.clone();

		let ai_backend = AIBackend {
			providers: crate::types::loadbalancer::EndpointSet::new(vec![vec![(
				model_name.clone(),
				named_provider,
			)]]),
		};

		let mut pols = vec![];
		if let Some(p) = model_config.backend_tls.clone() {
			pols.push(BackendTrafficPolicy::BackendTLS(p.try_into()?));
		}
		if let Some(p) = model_config.auth.clone() {
			pols.push(BackendTrafficPolicy::BackendAuth(p));
		}
		if let Some(p) = model_config.backend_tunnel.clone() {
			pols.push(BackendTrafficPolicy::Tunnel(p));
		}
		if let Some(rh) = model_config.request_headers.clone() {
			pols.push(BackendTrafficPolicy::RequestHeaderModifier(rh));
		}
		if let Some(rh) = model_config.response_headers.clone() {
			pols.push(BackendTrafficPolicy::ResponseHeaderModifier(Arc::new(rh)));
		}
		if let Some(p) = model_config.health.clone() {
			pols.push(BackendTrafficPolicy::Health(p.try_into().map_err(
				|e: crate::cel::Error| anyhow::anyhow!("health.unhealthyExpression: {}", e),
			)?));
		}
		let prompt_guard =
			merge_prompt_guards(shared_prompt_guard.clone(), model_config.guardrails.clone());
		pols.push(BackendTrafficPolicy::AI(Arc::new(llm::Policy {
			defaults: model_config.defaults.clone(),
			overrides: model_config.overrides.clone(),
			transformations: model_config.transformation.clone(),
			prompt_guard,
			prompts: None,
			model_aliases: Default::default(),
			wildcard_patterns: Arc::new(vec![]),
			prompt_caching: model_config.prompt_caching.clone(),
			routes: Default::default(),
		})));
		let resolved_inline_policies = pols.clone();
		let backend_with_policies = BackendWithPolicies {
			backend: Backend::AI(local_name(backend_key.clone()), ai_backend),
			inline_policies: pols,
		};
		all_backends.push(backend_with_policies);
		resolved_models.push(ResolvedLLMModelTarget {
			name: model_config.name.clone(),
			provider: resolved_provider,
			inline_policies: resolved_inline_policies,
			authorization: model_config.authorization.clone(),
		});

		let mut model_route_inline_policies = vec![TrafficPolicy::AI(Arc::new(crate::llm::Policy {
			routes: llm_routes.into_iter().collect(),
			..Default::default()
		}))];
		if let Some(p) = model_config.authorization.clone() {
			model_route_inline_policies.push(TrafficPolicy::Authorization(p));
		}
		router_models.push(llm::model_router::ModelRoute {
			name: model_config.name.clone(),
			visibility: model_config.visibility,
			header_matches: model_config
				.matches
				.iter()
				.map(|m| m.headers.clone())
				.collect(),
			backend_key,
			route_policies: model_route_inline_policies,
			backend_policies: vec![],
		});
	}

	let virtual_models = llm_registry.into_virtual_models();
	let mut router_virtual_models = Vec::new();
	for (idx, virtual_model) in virtual_models.into_iter().enumerate() {
		let route_policies = vec![TrafficPolicy::AI(Arc::new(crate::llm::Policy {
			routes: llm_route_types(None).into_iter().collect(),
			..Default::default()
		}))];
		let routing = match virtual_model.routing_strategy()? {
			LocalLLMVirtualRoutingStrategy::Conditional(conditional) => {
				for target in &conditional.targets {
					resolved_models.resolve(&target.model)?;
				}
				llm::model_router::VirtualModelRouting::Conditional(
					conditional
						.targets
						.iter()
						.map(|target| llm::model_router::ConditionalTarget {
							model: target.model.clone(),
							when: target.when.clone(),
						})
						.collect(),
				)
			},
			LocalLLMVirtualRoutingStrategy::Weighted(weighted) => {
				for target in &weighted.targets {
					resolved_models.resolve(&target.model)?;
				}
				llm::model_router::VirtualModelRouting::Weighted(
					weighted
						.targets
						.iter()
						.map(|target| llm::model_router::WeightedTarget {
							model: target.model.clone(),
							weight: target.weight,
						})
						.collect(),
				)
			},
			LocalLLMVirtualRoutingStrategy::Failover(failover) => {
				let provider_groups = failover
					.targets
					.iter()
					.sorted_by_key(|target| target.priority)
					.chunk_by(|target| target.priority)
					.into_iter()
					.map(|(_, targets)| {
						targets
							.map(|target| {
								let resolved = resolved_models.resolve_failover_target(&target.model)?;
								let mut provider = resolved.provider.clone();
								provider.name = strng::new(&target.model);
								ensure_ai_provider_model(&mut provider.provider, &target.model);
								provider
									.inline_policies
									.extend(resolved.inline_policies.clone());
								Ok((strng::new(&target.model), provider))
							})
							.collect::<anyhow::Result<Vec<_>>>()
					})
					.collect::<anyhow::Result<Vec<_>>>()?;
				let backend_key = strng::format!("llm:virtual-model:{}:{idx}", virtual_model.name);
				all_backends.push(BackendWithPolicies {
					backend: Backend::AI(
						local_name(backend_key.clone()),
						AIBackend {
							providers: crate::types::loadbalancer::EndpointSet::new(provider_groups),
						},
					),
					inline_policies: vec![],
				});
				llm::model_router::VirtualModelRouting::Failover { backend_key }
			},
		};
		router_virtual_models.push(llm::model_router::VirtualModelRoute {
			name: virtual_model.name,
			route_policies,
			routing,
		});
	}

	routes.push(Route {
		key: strng::new("llm:request"),
		service_key: None,
		service_port: 0,
		name: RouteName {
			name: strng::new("llm:request"),
			namespace: strng::new("internal"),
			rule_name: None,
			kind: None,
		},
		hostnames: vec![],
		matches: vec![RouteMatch {
			path: PathMatch::PathPrefix(strng::new("/")),
			method: None,
			headers: vec![],
			query: vec![],
		}],
		backends: vec![],
		llm_router: Some(Arc::new(llm::model_router::ModelRouter::new(
			router_models,
			router_virtual_models,
			startup_timestamp,
		))),
		inline_policies: vec![],
	});

	// Create listener
	let listener_key: ListenerKey = strng::new("llm");
	let listener_name = ListenerName {
		gateway_name: gateway.gateway_name.clone(),
		gateway_namespace: gateway.gateway_namespace.clone(),
		listener_name: strng::new("llm"),
		listener_set: None,
	};
	let tls_enabled = tls.is_some();
	let listener = Listener {
		key: listener_key.clone(),
		name: listener_name.clone(),
		hostname: strng::new("*"),
		protocol: match tls {
			Some(tls) => ListenerProtocol::HTTPS(tls),
			None => ListenerProtocol::HTTP,
		},
	};

	if !listener_gateway_policies.is_empty() || !listener_route_policies.is_empty() {
		let pc = listener_gateway_policies.len();
		let target = PolicyTarget::Gateway(listener_name.clone().into());
		for (idx, pol) in listener_gateway_policies.into_iter().enumerate() {
			let key = strng::format!("listener/{idx}");
			all_policies.push(TargetedPolicy {
				key,
				name: None,
				target: target.clone(),
				inheritance: Default::default(),
				policy: (pol, PolicyPhase::Gateway).into(),
			});
		}
		for (idx, pol) in listener_route_policies.into_iter().enumerate() {
			let key = strng::format!("listener/{}", pc + idx);
			all_policies.push(TargetedPolicy {
				key,
				name: None,
				target: target.clone(),
				inheritance: Default::default(),
				policy: (pol, PolicyPhase::Route).into(),
			});
		}
	}

	let mut listener_set = ListenerSet::default();
	listener_set.insert(listener);

	// Create bind
	let sockaddr = if cfg!(target_family = "unix") && config.ipv6_enabled {
		SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port)
	} else {
		SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port)
	};

	let bind = Bind {
		key: strng::format!("bind/{}", port),
		address: sockaddr,
		protocol: if tls_enabled {
			BindProtocol::tls
		} else {
			BindProtocol::http
		},
		listeners: listener_set,
		tunnel_protocol: TunnelProtocol::Direct,
	};

	Ok((bind, routes, all_policies, all_backends))
}

async fn convert_mcp_config(
	client: client::Client,
	config: &crate::Config,
	gateway: ListenerTarget,
	mcp_config: LocalSimpleMcpConfig,
) -> anyhow::Result<(
	Bind,
	Vec<Route>,
	Vec<TargetedPolicy>,
	Vec<BackendWithPolicies>,
)> {
	let LocalSimpleMcpConfig {
		port,
		backend,
		policies,
	} = mcp_config;
	let port = port.unwrap_or(DEFAULT_MCP_PORT);
	let route_key = strng::new("mcp:default");

	let resolved_policies = if let Some(pol) = policies {
		split_policies(client.clone(), pol, config.as_policy_context(&route_key)).await?
	} else {
		ResolvedPolicies::default()
	};

	let mut routes = Vec::new();
	let route = Route {
		key: route_key.clone(),
		service_key: None,
		service_port: 0,
		name: RouteName {
			name: strng::new("default"),
			namespace: strng::new("internal"),
			rule_name: None,
			kind: None,
		},
		hostnames: vec![],
		matches: default_matches(),
		backends: vec![RouteBackendReference {
			weight: 1,
			target: BackendReference::Backend(strng::new("/mcp")).into(),
			inline_policies: resolved_policies.backend_policies,
		}],
		llm_router: None,
		inline_policies: resolved_policies.route_policies,
	};
	routes.push(route);

	let listener_key: ListenerKey = strng::new("mcp");
	let listener_name = ListenerName {
		gateway_name: gateway.gateway_name.clone(),
		gateway_namespace: gateway.gateway_namespace.clone(),
		listener_name: strng::new("mcp"),
		listener_set: None,
	};
	let listener = Listener {
		key: listener_key,
		name: listener_name,
		hostname: strng::new("*"),
		protocol: ListenerProtocol::HTTP,
	};

	let mut listener_set = ListenerSet::default();
	listener_set.insert(listener);

	let sockaddr = if cfg!(target_family = "unix") && config.ipv6_enabled {
		SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port)
	} else {
		SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port)
	};

	let bind = Bind {
		key: strng::format!("bind/{}", port),
		address: sockaddr,
		protocol: BindProtocol::http,
		listeners: listener_set,
		tunnel_protocol: TunnelProtocol::Direct,
	};

	let backends = LocalBackend::MCP(backend)
		.as_backends(
			local_name(strng::new("mcp")),
			client,
			config.mcp.session_ttl,
		)
		.await?;

	Ok((bind, routes, vec![], backends))
}

fn detect_bind_protocol(listeners: &ListenerSet) -> BindProtocol {
	if listeners
		.iter()
		.any(|l| matches!(l.protocol, ListenerProtocol::HTTPS(_)))
	{
		return BindProtocol::tls;
	}
	if listeners
		.iter()
		.any(|l| matches!(l.protocol, ListenerProtocol::TLS(_)))
	{
		return BindProtocol::tls;
	}
	if listeners
		.iter()
		.any(|l| matches!(l.protocol, ListenerProtocol::TCP))
	{
		return BindProtocol::tcp;
	}
	BindProtocol::http
}

async fn convert_listener(
	client: client::Client,
	config: &crate::Config,
	idx: usize,
	l: LocalListener,
	bind_key: Strng,
	gateway: ListenerTarget,
) -> anyhow::Result<(
	Listener,
	Vec<Route>,
	Vec<TCPRoute>,
	Vec<TargetedPolicy>,
	Vec<BackendWithPolicies>,
)> {
	let LocalListener {
		name,
		policies,
		hostname,
		protocol,
		tls,
		routes,
		tcp_routes,
	} = l;

	let protocol = match protocol {
		LocalListenerProtocol::HTTP => {
			if routes.is_none() {
				bail!("protocol HTTP requires 'routes'")
			}
			ListenerProtocol::HTTP
		},
		LocalListenerProtocol::HTTPS => {
			if routes.is_none() {
				bail!("protocol HTTPS requires 'routes'")
			}
			ListenerProtocol::HTTPS(
				tls
					.ok_or(anyhow!("HTTPS listener requires 'tls'"))?
					.into_server_tls_config(config.dynamic_ca_cert_cache.clone())?,
			)
		},
		LocalListenerProtocol::TLS => {
			if tcp_routes.is_none() {
				bail!("protocol TLS requires 'tcpRoutes'")
			}
			ListenerProtocol::TLS(
				tls
					.map(|tls| tls.into_server_tls_config(config.dynamic_ca_cert_cache.clone()))
					.transpose()?,
			)
		},
		LocalListenerProtocol::TCP => {
			if tcp_routes.is_none() {
				bail!("protocol TCP requires 'tcpRoutes'")
			}
			ListenerProtocol::TCP
		},
		LocalListenerProtocol::HBONE => ListenerProtocol::HBONE,
	};

	if tcp_routes.is_some() && routes.is_some() {
		bail!("only 'routes' or 'tcpRoutes' may be set");
	}

	let listener_name = name
		.name
		.unwrap_or_else(|| strng::format!("listener{}", idx));
	let gateway_name = gateway.gateway_name.clone();
	let gateway_namespace = gateway.gateway_namespace.clone();
	let name = ListenerName {
		gateway_name,
		gateway_namespace,
		listener_name,
		listener_set: None,
	};
	let hostname = hostname.unwrap_or_default();
	let key: ListenerKey = strng::format!(
		"{}/{}/{bind_key}/{}",
		name.gateway_namespace,
		name.gateway_name,
		name.listener_name
	);

	let mut all_policies = vec![];
	let mut all_backends = vec![];

	let mut rs = Vec::new();
	for (idx, l) in routes.into_iter().flatten().enumerate() {
		let (route, backends) = convert_route(client.clone(), config, l, idx, key.clone()).await?;
		all_backends.extend_from_slice(&backends);
		rs.push(route)
	}

	let mut trs = Vec::new();
	for (idx, l) in tcp_routes.into_iter().flatten().enumerate() {
		let (route, policies, backends) = convert_tcp_route(l, idx, key.clone()).await?;
		all_policies.extend_from_slice(&policies);
		all_backends.extend_from_slice(&backends);
		trs.push(route)
	}

	if let Some(pol) = policies {
		let listener_policy_id = strng::format!("listener/{key}");
		let pols = split_policies(
			client.clone(),
			pol.into(),
			config.as_policy_context(listener_policy_id),
		)
		.await?;
		let target = PolicyTarget::Gateway(name.clone().into());
		for (idx, pol) in pols.route_policies.into_iter().enumerate() {
			let key = strng::format!("listener/{key}/{idx}");
			all_policies.push(TargetedPolicy {
				key,
				name: None,
				target: target.clone(),
				inheritance: Default::default(),
				policy: (pol, PolicyPhase::Gateway).into(),
			});
		}
	}

	let l = Listener {
		key,
		name,
		hostname,
		protocol,
	};
	Ok((l, rs, trs, all_policies, all_backends))
}

pub async fn convert_route(
	client: client::Client,
	config: &crate::Config,
	lr: LocalRoute,
	idx: usize,
	listener_key: ListenerKey,
) -> anyhow::Result<(Route, Vec<BackendWithPolicies>)> {
	let LocalRoute {
		name,
		hostnames,
		matches,
		policies,
		backends,
	} = lr;

	let route_name = name.name.unwrap_or_else(|| strng::format!("route{}", idx));
	let namespace = name.namespace.unwrap_or_else(|| strng::new("default"));
	let key = strng::format!("{listener_key}/{namespace}/{route_name}");

	let mut backend_refs = Vec::new();
	let mut external_backends = Vec::new();
	for (idx, b) in backends.iter().enumerate() {
		validate_inference_routing_scope(
			b.policies.as_ref(),
			if matches!(b.backend, LocalBackend::Service { .. }) {
				InferenceRoutingScope::ServiceRouteBackend
			} else {
				InferenceRoutingScope::NonServiceRouteBackend
			},
		)?;
		let backend_key = strng::format!("{key}/backend{idx}");
		let policies = b
			.policies
			.clone()
			.map(|p| p.translate())
			.transpose()?
			.unwrap_or_default();
		let be_name = local_name(backend_key.clone());
		let target = match &b.backend {
			LocalBackend::RouteGroup(rg) => RouteBackendTarget::RouteGroup(rg.clone()),
			other => {
				let bref = match other {
					LocalBackend::Service { name, port } => BackendReference::Service {
						name: name.clone(),
						port: *port,
					},
					LocalBackend::Backend(n) => BackendReference::Backend(n.clone()),
					LocalBackend::Invalid => BackendReference::Invalid,
					_ => BackendReference::Backend(strng::format!("/{}", backend_key)),
				};
				bref.into()
			},
		};
		let backends = b
			.backend
			.as_backends(be_name.clone(), client.clone(), config.mcp.session_ttl)
			.await?;
		let bref = RouteBackendReference {
			weight: b.weight,
			target,
			inline_policies: policies,
		};
		backend_refs.push(bref);
		external_backends.extend_from_slice(&backends);
	}
	let resolved = if let Some(pol) = policies {
		split_policies(
			client,
			pol,
			Some(AttachedPolicyContext {
				oidc_policy_id: crate::http::oidc::PolicyId::route(&key),
				oidc_cookie_encoder: config.oidc_cookie_encoder.as_ref(),
			}),
		)
		.await?
	} else {
		ResolvedPolicies::default()
	};
	for br in backend_refs.iter_mut() {
		br.inline_policies
			.extend_from_slice(&resolved.backend_policies);
	}
	let inline_policies = resolved.route_policies;
	let route = Route {
		key,
		service_key: None,
		service_port: 0,
		name: RouteName {
			name: route_name,
			namespace,
			rule_name: None,
			kind: None,
		},
		hostnames,
		matches,
		backends: backend_refs,
		llm_router: None,
		inline_policies,
	};
	Ok((route, external_backends))
}

#[derive(Default)]
pub(crate) struct ResolvedPolicies {
	pub(crate) backend_policies: Vec<BackendTrafficPolicy>,
	pub(crate) route_policies: Vec<TrafficPolicy>,
}

pub struct AttachedPolicyContext<'a> {
	pub oidc_policy_id: crate::http::oidc::PolicyId,
	pub oidc_cookie_encoder: Option<&'a crate::http::sessionpersistence::Encoder>,
}

async fn split_frontend_policies(
	gateway: ListenerTarget,
	pol: LocalFrontendPolicies,
) -> Result<Vec<TargetedPolicy>, Error> {
	let mut pols = Vec::new();

	let mut add = |p: FrontendPolicy, name: &str| {
		let key = strng::format!("frontend/{name}");
		pols.push(TargetedPolicy {
			key: key.clone(),
			name: None,
			target: PolicyTarget::Gateway(gateway.clone()),
			inheritance: Default::default(),
			policy: p.into(),
		});
	};
	let LocalFrontendPolicies {
		http,
		tls,
		tcp,
		network_authorization,
		proxy_protocol,
		connect,
		access_log,
		tracing,
	} = pol;
	if let Some(p) = http {
		add(FrontendPolicy::HTTP(p), "http");
	}
	if let Some(p) = tls {
		add(FrontendPolicy::TLS(p), "tls");
	}
	if let Some(p) = tcp {
		add(FrontendPolicy::TCP(p), "tcp");
	}
	if let Some(p) = network_authorization {
		add(
			FrontendPolicy::NetworkAuthorization(p),
			"networkAuthorization",
		);
	}
	if let Some(p) = proxy_protocol {
		add(FrontendPolicy::Proxy(p), "proxy");
	}
	if let Some(p) = connect {
		add(FrontendPolicy::Connect(p), "connect");
	}
	if let Some(mut p) = access_log {
		p.init_access_log_policy();
		add(FrontendPolicy::AccessLog(p), "accessLog");
	}
	if let Some(tracing_config) = tracing {
		// Build logging fields from attributes for lazy tracer creation
		let logging_fields = Arc::new(crate::telemetry::log::LoggingFields {
			remove: Arc::new(tracing_config.remove.iter().cloned().collect()),
			add: Arc::new(tracing_config.attributes.clone()),
		});

		add(
			FrontendPolicy::Tracing(Arc::new(crate::types::agent::TracingPolicy {
				config: tracing_config,
				fields: logging_fields,
				tracer: once_cell::sync::OnceCell::new(),
			})),
			"tracing",
		);
	}
	Ok(pols)
}
pub(crate) async fn split_policies(
	client: Client,
	pol: FilterOrPolicy,
	attached: Option<AttachedPolicyContext<'_>>,
) -> Result<ResolvedPolicies, Error> {
	let mut resolved = ResolvedPolicies::default();
	let ResolvedPolicies {
		backend_policies,
		route_policies,
	} = &mut resolved;
	let FilterOrPolicy {
		request_header_modifier,
		response_header_modifier,
		request_redirect,
		url_rewrite,
		request_mirror,
		direct_response,
		cors,
		mcp_authorization,
		mcp_guardrails,
		mcp_authentication,
		a2a,
		ai,
		backend_tls,
		backend_tunnel,
		backend_auth,
		authorization,
		local_rate_limit,
		remote_rate_limit,
		jwt_auth,
		oidc: oidc_config,
		basic_auth,
		api_key,
		transformations,
		csrf,
		ext_authz,
		ext_proc,
		buffer,
		timeout,
		retry,
	} = pol;
	if let Some(p) = request_header_modifier {
		route_policies.push(TrafficPolicy::RequestHeaderModifier(RequestPolicy::single(
			p,
		)));
	}
	if let Some(p) = response_header_modifier {
		route_policies.push(TrafficPolicy::ResponseHeaderModifier(
			RequestPolicy::single(p),
		));
	}
	if let Some(p) = request_redirect {
		route_policies.push(TrafficPolicy::RequestRedirect(RequestPolicy::single(p)));
	}
	if let Some(p) = url_rewrite {
		route_policies.push(TrafficPolicy::UrlRewrite(RequestPolicy::single(p)));
	}
	if let Some(p) = request_mirror {
		route_policies.push(TrafficPolicy::RequestMirror(vec![p]));
	}

	// Filters
	if let Some(p) = direct_response {
		route_policies.push(TrafficPolicy::DirectResponse(p.into_policy()?));
	}
	if let Some(p) = cors {
		route_policies.push(TrafficPolicy::CORS(RequestPolicy::single(p)));
	}

	// Backend policies
	if let Some(p) = mcp_authorization {
		backend_policies.push(BackendTrafficPolicy::McpAuthorization(p))
	}
	if let Some(p) = mcp_guardrails {
		for w in p.load_warnings() {
			tracing::warn!("{w}");
		}
		backend_policies.push(BackendTrafficPolicy::McpGuardrails(Arc::new(p)))
	}
	if let Some(p) = mcp_authentication {
		let authn: McpAuthentication = p.translate(client.clone()).await?;
		route_policies.push(TrafficPolicy::JwtAuth(RequestPolicy::single(
			JwtAuthentication {
				jwt: authn.jwt_validator.as_ref().clone(),
				mcp: Some(authn),
			},
		)));
	}
	if let Some(p) = a2a {
		backend_policies.push(BackendTrafficPolicy::A2a(p))
	}
	if let Some(p) = backend_tls {
		backend_policies.push(BackendTrafficPolicy::BackendTLS(p.try_into()?))
	}
	if let Some(p) = backend_tunnel {
		backend_policies.push(BackendTrafficPolicy::Tunnel(p))
	}
	if let Some(p) = backend_auth {
		backend_policies.push(BackendTrafficPolicy::BackendAuth(p))
	}

	// Route policies
	if let Some(mut p) = ai {
		p.compile_model_alias_patterns();
		route_policies.push(TrafficPolicy::AI(Arc::new(p)))
	}
	if let Some(p) = jwt_auth {
		route_policies.push(TrafficPolicy::JwtAuth(RequestPolicy::single(
			JwtAuthentication {
				jwt: p.try_into(client.clone()).await?,
				mcp: None,
			},
		)));
	}
	let compiled_oidc = if let Some(oidc) = oidc_config {
		let Some(AttachedPolicyContext {
			oidc_policy_id,
			oidc_cookie_encoder,
		}) = attached
		else {
			return Err(Error::msg("oidc policies must be attached"));
		};
		let Some(oidc_cookie_encoder) = oidc_cookie_encoder else {
			return Err(Error::msg(
				"OIDC_COOKIE_SECRET is required when oidc is configured",
			));
		};
		Some(TrafficPolicy::Oidc(RequestPolicy::single(
			oidc
				.compile(client.clone(), oidc_policy_id, oidc_cookie_encoder)
				.await?,
		)))
	} else {
		None
	};
	if let Some(p) = basic_auth {
		route_policies.push(TrafficPolicy::BasicAuth(RequestPolicy::single(
			p.try_into()?,
		)));
	}
	if let Some(p) = api_key {
		route_policies.push(TrafficPolicy::APIKey(RequestPolicy::single(p.into())));
	}
	if let Some(p) = transformations {
		route_policies.push(TrafficPolicy::Transformation(
			p.into_transformation_policy()?,
		));
	}
	if let Some(p) = csrf {
		route_policies.push(TrafficPolicy::Csrf(RequestPolicy::single(p)))
	}
	if let Some(p) = authorization {
		route_policies.push(TrafficPolicy::Authorization(p))
	}
	if let Some(p) = ext_authz {
		route_policies.push(TrafficPolicy::ExtAuthz(
			configure_ext_authz_cache_store(p).into_policy()?,
		))
	}
	if let Some(p) = ext_proc {
		route_policies.push(TrafficPolicy::ExtProc(p.into_policy()?))
	}
	if let Some(p) = local_rate_limit
		&& !p.is_empty()
	{
		route_policies.push(TrafficPolicy::LocalRateLimit(p.into_request_policy()?))
	}
	if let Some(p) = remote_rate_limit {
		route_policies.push(TrafficPolicy::RemoteRateLimit(p.into_policy()?))
	}

	// Traffic policies
	if let Some(p) = buffer {
		route_policies.push(TrafficPolicy::Buffer(RequestPolicy::single(p)));
	}
	if let Some(p) = timeout {
		route_policies.push(TrafficPolicy::Timeout(p));
	}
	if let Some(p) = retry {
		route_policies.push(TrafficPolicy::Retry(p));
	}
	if let Some(oidc) = compiled_oidc {
		route_policies.push(oidc);
	}
	Ok(resolved)
}

async fn convert_tcp_route(
	lr: LocalTCPRoute,
	idx: usize,
	listener_key: ListenerKey,
) -> anyhow::Result<(TCPRoute, Vec<TargetedPolicy>, Vec<BackendWithPolicies>)> {
	let LocalTCPRoute {
		name,
		hostnames,
		policies,
		backends,
	} = lr;

	let route_name = name
		.name
		.unwrap_or_else(|| strng::format!("tcproute{}", idx));
	let namespace = name.namespace.unwrap_or_else(|| strng::new("default"));
	let key = strng::format!("{listener_key}/{namespace}/{route_name}");

	let external_policies = vec![];

	let mut backend_refs = Vec::new();
	let mut external_backends = Vec::new();
	for (idx, b) in backends.iter().enumerate() {
		let backend_key = strng::format!("{key}/backend{idx}");
		let be_name = local_name(backend_key.clone());
		let policies = b
			.policies
			.clone()
			.map(|p| p.translate())
			.transpose()?
			.unwrap_or_default();
		let bref = match &b.backend {
			SimpleLocalBackend::Service { name, port } => SimpleBackendReference::Service {
				name: name.clone(),
				port: *port,
			},
			SimpleLocalBackend::Invalid => SimpleBackendReference::Invalid,
			_ => SimpleBackendReference::Backend(strng::format!("/{}", backend_key)),
		};
		let maybe_backend = b.backend.as_backends(be_name.clone(), policies);
		let bref = TCPRouteBackendReference {
			weight: b.weight,
			backend: bref,
			inline_policies: Vec::new(),
		};
		backend_refs.push(bref);
		if let Some(be) = maybe_backend {
			external_backends.push(be.into());
		}
	}

	if let Some(pol) = policies {
		let TCPFilterOrPolicy { backend_tls } = pol;
		if let Some(p) = backend_tls {
			for br in backend_refs.iter_mut() {
				br.inline_policies
					.push(BackendTrafficPolicy::BackendTLS(p.clone().try_into()?));
			}
		}
	}
	let route = TCPRoute {
		key,
		service_key: None,
		service_port: 0,
		name: RouteName {
			name: route_name,
			namespace,
			rule_name: None,
			kind: None,
		},
		hostnames,
		backends: backend_refs,
	};
	Ok((route, external_policies, external_backends))
}

// For most local backends we can just use `InlineBackend`. However, for MCP we allow `https://domain/path`
// which implies adding inline policies + parsing the path. So we need to use references.
fn mcp_to_simple_backend_and_ref(
	name: ResourceName,
	b: Target,
) -> (SimpleBackendReference, Option<Backend>) {
	let bref = SimpleBackendReference::Backend(name.to_string().into());
	let backend = SimpleLocalBackend::Opaque(b).as_backend(name);
	(bref, backend)
}

impl TryInto<ServerTLSConfig> for LocalTLSServerConfig {
	type Error = anyhow::Error;

	fn try_into(self) -> Result<ServerTLSConfig, Self::Error> {
		self.into_server_tls_config(Default::default())
	}
}

impl LocalTLSServerConfig {
	fn into_server_tls_config(
		self,
		dynamic_ca_cert_cache: crate::DynamicCaCertCacheConfig,
	) -> anyhow::Result<ServerTLSConfig> {
		let cert_pem = fs_err::read(self.cert)?;
		let key_pem = fs_err::read(self.key)?;
		let root_pem = self.root.map(fs_err::read).transpose()?;
		match self.mode {
			LocalTLSServerMode::Static => ServerTLSConfig::from_pem_with_profile(
				cert_pem,
				key_pem,
				root_pem,
				vec![b"h2".to_vec(), b"http/1.1".to_vec()],
				self.min_tls_version.map(Into::into),
				self.max_tls_version.map(Into::into),
				self.cipher_suites,
				self.key_exchange_groups,
				false,
			),
			LocalTLSServerMode::DynamicCa => {
				if root_pem.is_some() {
					anyhow::bail!("tls.root is not supported with tls.mode=dynamicCa")
				}
				super::dynamic_ca_cert::build_dynamic_ca_tls_config_with_profile(
					cert_pem,
					key_pem,
					vec![b"h2".to_vec(), b"http/1.1".to_vec()],
					self.min_tls_version.map(Into::into),
					self.max_tls_version.map(Into::into),
					self.cipher_suites,
					self.key_exchange_groups,
					dynamic_ca_cert_cache,
				)
			},
		}
	}
}

pub fn local_name(name: Strng) -> ResourceName {
	ResourceName::new(name, "".into())
}

pub fn de_from_local_backend_policy<'de: 'a, 'a, D>(
	deserializer: D,
) -> Result<Vec<BackendTrafficPolicy>, D::Error>
where
	D: Deserializer<'de>,
{
	let s = SimpleLocalBackendPolicies::deserialize(deserializer)?;
	LocalBackendPolicies {
		simple: s,
		..Default::default()
	}
	.translate()
	.map_err(serde::de::Error::custom)
}
