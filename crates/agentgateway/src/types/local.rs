use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(any(not(test), target_family = "unix"))]
use std::sync::OnceLock;
use std::time::Duration;
#[cfg(not(test))]
use std::time::{SystemTime, UNIX_EPOCH};

use ::http::Uri;
use agent_core::prelude::Strng;
use anyhow::{Context, Error, anyhow, bail};
use indexmap::IndexMap;
use itertools::Itertools;
use secrecy::SecretString;

use crate::http::auth::BackendAuth;
use crate::http::backendtls::{LocalBackendTLS, ResolvedBackendTLS};
use crate::http::transformation_cel::{LocalTransformationConfig, Transformation};
use crate::http::{filters, health, retry, timeout, transformation_cel};
use crate::llm::policy::{PromptCachingConfig, PromptGuard};
use crate::llm::{AIBackend, AIProvider, NamedAIProvider, anthropic, copilot, custom, openai};
use crate::mcp::{FailureMode, McpAuthorization};
use crate::store::{LocalWorkload, RequestPolicy};
use crate::types::agent::{
	A2aPolicy, Authorization, Backend, BackendKey, BackendReference, BackendTrafficPolicy,
	BackendWithPolicies, Bind, BindMode, BindProtocol, FrontendPolicy, HeaderMatch,
	JwtAuthentication, Listener, ListenerKey, ListenerName, ListenerProtocol, ListenerSet,
	ListenerTarget, LocalMcpAuthentication, McpAuthentication, McpBackend, McpPrefixMode, McpTarget,
	McpTargetName, McpTargetSpec, OpenAPITarget, PathMatch, PolicyPhase, PolicyTarget, PolicyType,
	ResourceName, Route, RouteBackendReference, RouteBackendTarget, RouteGroupKey, RouteMatch,
	RouteName, ServerTLSConfig, SimpleBackend, SimpleBackendReference,
	SimpleBackendReferenceWithPolicies, SimpleBackendWithPolicies, SseTargetSpec,
	StreamableHTTPTargetSpec, TCPRoute, TCPRouteBackendReference, Target, TargetedPolicy,
	TracingConfig, TrafficPolicy, TunnelProtocol, TypedResourceName, validate_mcp_target_name,
};
use crate::types::discovery::{NamespacedHostname, Service};
use crate::types::{backend, frontend};
use crate::{agentcore, apply, *};

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
		resources: &crate::resource_manager::ResourceFetcher,
		gateway_name: ListenerTarget,
		s: &str,
	) -> anyhow::Result<NormalizedLocalConfig> {
		// Avoid shell expanding the comment for schema. Probably there are better ways to do this!
		let s = s.replace("# yaml-language-server: $schema", "#");
		let s = shellexpand::full(&s)?;
		let local_config: LocalConfig = serdes::yamlviajson::from_str(&s)?;
		let scope = resources.scope_full_computation();
		let result = Box::pin(convert(resources, gateway_name, config, local_config)).await;
		scope.finish(result.is_ok());
		let t = result?;
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

		let mut policies = if !headers.is_empty() {
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
			let (backend, use_tls) = parse_deprecated_tracing_endpoint(&ep)
				.with_context(|| format!("failed parsing tracing endpoint: {}", ep))?;
			if use_tls {
				policies.push(BackendTrafficPolicy::BackendTLS(
					ResolvedBackendTLS::default().try_into()?,
				));
			}
			frontend_policies.tracing = Some(TracingConfig {
				target: SimpleBackendReferenceWithPolicies {
					target: Arc::new(SimpleBackendReference::InlineBackend(backend)),
					policies,
				},
				attributes: Arc::unwrap_or_clone(fields.add),
				resources: Default::default(), // Not supported in the old config
				filter: None,                  // Not supported in the old config
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

fn parse_deprecated_tracing_endpoint(endpoint: &str) -> anyhow::Result<(Target, bool)> {
	if !endpoint.contains("://") {
		return Ok((Target::try_from(endpoint)?, false));
	}

	let uri = Uri::try_from(endpoint)?;
	let Some(scheme) = uri.scheme_str() else {
		return Ok((Target::try_from(endpoint)?, false));
	};
	if !matches!(scheme, "http" | "https" | "grpc") {
		anyhow::bail!("unsupported tracing endpoint scheme: {scheme}");
	}
	let host = uri
		.host()
		.with_context(|| format!("tracing endpoint {endpoint} must include a host"))?;
	let port = uri.port_u16().or_else(|| match scheme {
		"http" => Some(80),
		"https" => Some(443),
		"grpc" => Some(4317),
		_ => unreachable!("unsupported tracing endpoint scheme checked above"),
	});
	let Some(port) = port else {
		anyhow::bail!("unsupported tracing endpoint scheme: {scheme}");
	};
	Ok((Target::from((host, port)), scheme == "https"))
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
	/// config defines top-level settings for DNS, admin, networking, observability, and session
	/// management. Unlike other sections, these are applied only at startup and are not dynamically reloaded.
	#[serde(default)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<RawConfig>"))]
	#[allow(unused)]
	config: Arc<Option<serde_json::Value>>,
	/// binds defines the low-level API for configuring the proxy.
	/// Each bind represents a single port the proxy listens on, as well as the full set of configuration
	/// (listeners, routes, backends) for that port.
	/// Deprecated; usage of `gateways` and `routes` is recommended instead.
	#[serde(default)]
	binds: Vec<LocalBind>,
	/// frontendPolicies defines top level policies applying to all traffic.
	#[serde(default)]
	frontend_policies: LocalFrontendPolicies,
	/// policies defines additional policies that can be attached to various other configurations.
	/// This is an advanced feature; users should typically use the inline `policies` field under route/gateway.
	#[serde(default)]
	policies: Vec<LocalPolicy>,
	/// workloads defines the set of workloads that the proxy can serve. These are selected by `services`.
	/// This is an advanced feature that is mostly for testing; usage of inline `backends` on routes and
	/// policies is typically preferred.
	#[serde(default)]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "Vec<std::collections::HashMap<String, serde_json::Value>>")
	)]
	workloads: Vec<LocalWorkload>,
	/// services defines the set of services that the proxy can route to. These consist of `workloads`.
	/// This is an advanced feature that is mostly for testing; usage of inline `backends` on routes and
	/// policies is typically preferred.
	#[serde(default)]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "Vec<std::collections::HashMap<String, serde_json::Value>>")
	)]
	services: Vec<Service>,
	/// backends defines explicit backends that can be referenced by routes and policies.
	/// Typically, inline backends are used on the routes/policies, but this allows re-using the same backend
	/// across different configurations.
	#[serde(default)]
	backends: Vec<FullLocalBackend>,
	/// routeGroups provides a set of route groups used for route delegation. This is an advanced feature
	/// primarily used for testing.
	#[serde(default, rename = "routeGroups")]
	route_groups: Vec<LocalRouteGroup>,
	/// gateways defines the entrypoint to the proxy, setting up ports and listeners that features (LLM, MCP, and UI) and routes can attach to.
	/// Each gateway defines a port that proxy will listen on, and optionally TLS settings for that port.
	#[serde(default)]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "std::collections::HashMap<String, LocalGateway>")
	)]
	gateways: IndexMap<Strng, LocalGateway>,
	/// routes defines HTTP routes attached to one or more named gateways.
	#[serde(default)]
	routes: Vec<LocalAttachedRoute>,
	/// tcpRoutes defines TCP routes attached to one or more named TCP/TLS gateways.
	#[serde(default)]
	tcp_routes: Vec<LocalAttachedTCPRoute>,
	/// llm defines a set of LLM models to be exposed by the proxy. When configured, LLM models will be
	/// served under the attached `gateways` using the standard serving paths (`/v1/models`, `/v1/chat/completions`, etc).
	#[serde(default)]
	llm: Option<LocalLLMConfig>,
	/// mcp defines a set of MCP servers exposed by the proxy. When configured, the MCP servers will be
	/// served under the attached `gateways` at /mcp and /sse.
	/// All MCP servers listed will be served as a single virtual MCP server.
	#[serde(default)]
	mcp: Option<LocalSimpleMcpConfig>,
	/// ui defines settings for how the UI and UI backend is exposed. By default, the UI is exposed only
	/// on the admin interface (typically localhost:15000). This setting allows attaching to `gateways`
	/// to serve externally, as well as attaching policies to UI traffic.
	/// It is strongly recommended to utilize authentication (typically OIDC) when exposing the UI externally.
	#[serde(default)]
	ui: Option<LocalUIConfig>,
}

#[apply(schema_de!)]
pub struct LocalLLMConfig {
	/// gateways attaches the LLM routes to named gateways. This can take the form of `<gateway-name>` or `<gateway-name>/<listener-name>` to attach to a specific listener within a gateway.
	/// When omitted and a gateway named `default` exists, the LLM API routes attach to it unless `port` is set.
	#[serde(default, deserialize_with = "de_gateway_refs")]
	#[cfg_attr(feature = "schema", schemars(with = "LocalGatewayRefs"))]
	gateways: Vec<Strng>,
	/// port defines the port to serve the LLM routes under. Deprecated; use `gateways` instead.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	port: Option<u16>,
	/// tls defines the TLS settings to serve the LLM routes under when using `port`. Deprecated; use `gateways` instead.
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
	/// Request payload fields to set when not already present in the request.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	defaults: Option<HashMap<String, serde_json::Value>>,
	/// Request payload fields to set, overriding any existing values in the request.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	overrides: Option<HashMap<String, serde_json::Value>>,
	/// CEL expressions that compute request payload fields, overriding existing values.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	transformation: Option<HashMap<String, Arc<cel::Expression>>>,
	/// Headers to add, set, or remove on requests to the LLM provider.
	#[serde(default)]
	request_headers: Option<filters::HeaderModifier>,
	/// Headers to add, set, or remove on responses from the LLM provider.
	#[serde(default)]
	response_headers: Option<filters::HeaderModifier>,
	/// TLS configuration for connecting to the LLM provider.
	#[serde(rename = "tls", alias = "backendTLS", default)]
	backend_tls: Option<http::backendtls::LocalBackendTLS>,
	/// Authentication configuration for connecting to the LLM provider.
	#[serde(default, deserialize_with = "de_backend_auth")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<BackendAuthCompat>"))]
	auth: Option<BackendAuth>,
	/// Outlier detection and health checking for this provider backend.
	#[serde(default)]
	health: Option<health::LocalHealthPolicy>,
	/// Tunneling configuration for connecting to the LLM provider.
	#[serde(default)]
	backend_tunnel: Option<backend::Tunnel>,
	/// Cache-point insertion for LLM providers that support prompt caching.
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
	/// Relative proportion of traffic sent to this target model. Defaults to 1.
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
	/// gateways attaches the MCP routes to named gateways. This can take the form of `<gateway-name>` or `<gateway-name>/<listener-name>` to attach to a specific listener within a gateway.
	/// When omitted and a gateway named `default` exists, the MCP routes attach to it unless port is set.
	#[serde(default, deserialize_with = "de_gateway_refs")]
	#[cfg_attr(feature = "schema", schemars(with = "LocalGatewayRefs"))]
	gateways: Vec<Strng>,
	/// port defines the port to serve the LLM routes under. Deprecated; use `gateways` instead.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	port: Option<u16>,
	#[serde(flatten)]
	backend: LocalMcpBackend,
	/// Policies applied to MCP requests.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	policies: Option<FilterOrPolicy>,
}

#[apply(schema_de!)]
pub struct LocalUIConfig {
	/// gateways attaches the UI and UI backend routes to named gateways. This can take the form of `<gateway-name>` or `<gateway-name>/<listener-name>` to attach to a specific listener within a gateway.
	/// When omitted and a gateway named `default` exists, the UI routes attach to it.
	#[serde(default, deserialize_with = "de_gateway_refs")]
	#[cfg_attr(feature = "schema", schemars(with = "LocalGatewayRefs"))]
	gateways: Vec<Strng>,
	/// policies defines route-level policies for the UI and required UI API routes.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	policies: Option<LocalUIPolicy>,
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
	#[cfg_attr(feature = "schema", schemars(with = "Option<BackendAuthCompat>"))]
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
	/// Request headers to match for conditional model routing.
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
	/// AWS region to use for the Bedrock provider.
	aws_region: Option<Strng>,
	/// Google Cloud region to use for the Vertex AI provider.
	vertex_region: Option<Strng>,
	/// Google Cloud project ID to use for the Vertex AI provider.
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

#[derive(Debug, Clone, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
enum LocalGatewayRefs {
	One(Strng),
	Many(Vec<Strng>),
}

fn de_gateway_refs<'de, D>(deserializer: D) -> Result<Vec<Strng>, D::Error>
where
	D: Deserializer<'de>,
{
	Option::<LocalGatewayRefs>::deserialize(deserializer).map(|refs| match refs {
		None => Vec::new(),
		Some(LocalGatewayRefs::One(reference)) => vec![reference],
		Some(LocalGatewayRefs::Many(references)) => references,
	})
}

#[apply(schema_de!)]
struct LocalGateway {
	/// port is the port to listen on for this gateway.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	port: Option<u16>,
	/// protocol controls whether this gateway accepts HTTP/HTTPS routes or TCP/TLS routes. When omitted, gateways
	/// default to HTTP, or HTTPS when tls is set.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	protocol: Option<LocalGatewayProtocol>,
	/// listeners defines multiple named listeners under this gateway. When set, only `port` may be configured on the top level gateway.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	listeners: Vec<LocalGatewayListener>,

	/// tls enables HTTPS for this gateway. Maybe not be set with `listeners`
	#[serde(default, skip_serializing_if = "Option::is_none")]
	tls: Option<LocalTLSServerConfig>,
	#[serde(flatten)]
	policies: LocalGatewayPolicy,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct LocalAttachedRoute {
	/// gateways attaches this route to named gateways or gateway listeners.
	/// This can take the form of `<gateway-name>` or `<gateway-name>/<listener-name>` to attach to a specific listener within a gateway.
	/// If unset, the 'default' gateway will be used.
	#[serde(default, deserialize_with = "de_gateway_refs")]
	#[cfg_attr(feature = "schema", schemars(with = "LocalGatewayRefs"))]
	gateways: Vec<Strng>,
	#[serde(flatten)]
	route: LocalRoute,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct LocalAttachedTCPRoute {
	/// gateways attaches this route to named TCP/TLS gateways or gateway listeners.
	/// This can take the form of `<gateway-name>` or `<gateway-name>/<listener-name>` to attach to a specific listener within a gateway.
	/// If unset, the 'default' gateway will be used.
	#[serde(default, deserialize_with = "de_gateway_refs")]
	#[cfg_attr(feature = "schema", schemars(with = "LocalGatewayRefs"))]
	gateways: Vec<Strng>,
	#[serde(flatten)]
	route: LocalTCPRoute,
}

#[apply(schema_de!)]
struct LocalGatewayListener {
	/// name identifies this listener for gateway references like `gateways: gateway-name/listener-name`.
	#[serde(default)]
	name: Option<Strng>,
	/// Hostname defines what hostnames are served under this listener. Can be a wildcard.
	/// This allows serving multiple domains with different TLS configurations.
	/// If unset, all domains will be served (implicit wildcard).
	#[serde(default, skip_serializing_if = "Option::is_none")]
	hostname: Option<Strng>,
	/// protocol controls whether this listener accepts HTTP/HTTPS routes or TCP/TLS routes. When omitted, listeners
	/// default to HTTP, or HTTPS when tls is set.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	protocol: Option<LocalGatewayProtocol>,
	/// tls enables HTTPS for this listener.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	tls: Option<LocalTLSServerConfig>,
	#[serde(flatten)]
	policies: LocalGatewayPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[allow(clippy::upper_case_acronyms)]
enum LocalGatewayProtocol {
	HTTP,
	HTTPS,
	TCP,
	TLS,
}

#[apply(schema_de!)]
struct LocalBind {
	/// Port to bind on. Omit it for an internal wildcard bind (which serves any destination port
	/// via in-process routing). A numeric port is required unless `mode` is `internal`.
	#[serde(default)]
	port: Option<u16>,
	/// Named listeners bound on this port, which may use different protocols and TLS.
	listeners: Vec<LocalListener>,
	/// Protocol used to tunnel backend connections, such as Direct or HBONE.
	#[serde(default)]
	tunnel_protocol: TunnelProtocol,
	/// Whether the bind opens an OS listener socket. Defaults to `standard` (binds the port).
	/// Set to `internal` to create a routing-only bind that does not bind a socket.
	#[serde(default)]
	mode: BindMode,
}

#[apply(schema_de!)]
pub struct LocalListenerName {
	// User facing name
	/// Name identifying this listener, referenced by `gateways: gateway-name/listener-name`.
	#[serde(default)]
	pub name: Option<Strng>,
	/// Namespace scoping this listener.
	#[serde(default)]
	pub namespace: Option<Strng>,
}

#[apply(schema_de!)]
struct LocalListener {
	#[serde(flatten)]
	name: LocalListenerName,
	/// Can be a wildcard
	hostname: Option<Strng>,
	/// Protocol this listener accepts: HTTP, HTTPS, TCP, TLS, or HBONE.
	#[serde(default)]
	protocol: LocalListenerProtocol,
	/// TLS configuration, used with the HTTPS and TLS protocols.
	tls: Option<LocalTLSServerConfig>,
	/// HTTP routes attached directly to this listener.
	routes: Option<Vec<LocalRoute>>,
	/// TCP routes attached directly to this listener.
	tcp_routes: Option<Vec<LocalTCPRoute>>,
	/// Gateway-level policies applied to all traffic on this listener.
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
	/// Path to the TLS certificate file (leaf certificate, or CA certificate in dynamic CA mode).
	pub cert: PathBuf,
	/// Path to the TLS private key file.
	pub key: PathBuf,
	/// Path to a root CA certificate file used to validate client certificates.
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
	/// Name identifying this route.
	#[serde(default)]
	pub name: Option<Strng>,
	/// Namespace scoping this route.
	#[serde(default)]
	pub namespace: Option<Strng>,
	/// Specific rule within this route.
	#[serde(default)]
	pub rule_name: Option<Strng>,
}

#[apply(schema_de!)]
pub struct LocalRouteGroup {
	/// Identifier for this route group, referenced by delegating routes.
	name: RouteGroupKey,
	/// HTTP routes grouped together for delegation and reuse.
	routes: Vec<LocalRoute>,
}

#[apply(schema_de!)]
pub struct LocalRoute {
	#[serde(flatten)]
	name: LocalRouteName,
	/// Can be a wildcard
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	hostnames: Vec<Strng>,
	/// Conditions (path, method, headers, query) that select this route.
	#[serde(default = "default_matches")]
	matches: Vec<RouteMatch>,
	/// Route-level policies applied before backend selection.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	policies: Option<FilterOrPolicy>,
	/// Weighted backends this route forwards traffic to.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	backends: Vec<LocalRouteBackend>,
}

#[apply(schema_de!)]
pub struct LocalRouteBackend {
	/// Relative weight for load balancing across backends. Defaults to 1.
	#[serde(default = "default_weight")]
	pub weight: usize,
	#[serde(flatten)]
	pub backend: LocalBackend,
	/// Backend-level policies such as TLS, authentication, and transformations.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub policies: Option<LocalBackendPolicies>,
}

fn default_weight() -> usize {
	1
}

#[apply(schema_de!)]
pub struct FullLocalBackend {
	/// Identifier for this backend, referenced by routes.
	pub name: BackendKey,
	#[serde(flatten)]
	pub spec: FullLocalBackendSpec,
	/// Backend-level policies such as TLS, authentication, transformations, and health checks.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub policies: Option<LocalBackendPolicies>,
}

#[apply(schema_de!)]
#[allow(clippy::large_enum_variant)]
pub enum FullLocalBackendSpec {
	/// Hostname or IP address of the upstream to route to.
	#[serde(rename = "host")]
	Opaque(Target),
	/// Route to the in-process admin service instead of a network upstream.
	#[serde(rename = "internal")]
	Internal(InternalBackend),
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
			FullLocalBackendSpec::Internal(t) => LocalBackend::Internal(t),
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
	/// ARN of the Bedrock AgentCore runtime (arn:aws:bedrock-agentcore:REGION:ACCOUNT:runtime/ID).
	pub agent_runtime_arn: String,
	/// Endpoint qualifier (version or alias) for the AgentCore runtime invocation.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub qualifier: Option<String>,
}

#[apply(schema!)]
/// Selects how an internal backend maps proxy requests to the admin API.
pub enum InternalBackend {
	/// Forward the request to the admin API using the request's current path and query.
	#[serde(rename = "forward")]
	Forward,
	/// Rewrite all requests to this admin API path, preserving the original query string.
	#[serde(untagged)]
	Path(Strng),
}

#[apply(schema_de!)]
#[allow(clippy::large_enum_variant)] // Size is not sensitive for local config
pub enum LocalBackend {
	// This one is a reference
	/// Route to a Service defined in the top-level `services` list.
	Service {
		/// Name of the target Service, as defined in the top-level `services` list.
		name: NamespacedHostname,
		/// Port on the target Service to route to.
		port: u16,
	},
	Backend(BackendKey),
	// Rest are inlined
	/// Hostname or IP address of the upstream to route to.
	#[serde(rename = "host")]
	Opaque(Target),
	/// Route to the in-process admin service instead of a network upstream.
	Internal(InternalBackend),
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
	/// LLM providers in this group, load balanced together.
	providers: Vec<LocalNamedAIProvider>,
}

#[apply(schema_de!)]
pub struct LocalNamedAIProvider {
	/// Name identifying this provider, referenced by `llm.models[].provider`.
	pub name: Strng,
	/// The upstream LLM provider type and its configuration.
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
	/// Backend policies applied to traffic to this provider.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub policies: Option<LocalBackendPolicies>,
}

impl LocalAIBackend {
	pub async fn translate(
		self,
		resources: &crate::resource_manager::ResourceFetcher,
	) -> anyhow::Result<AIBackend> {
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
				let policies = match p.policies {
					Some(p) => p.translate(resources).await?,
					None => Vec::new(),
				};
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
	async fn make_mcp_backend(
		b: Backend,
		policies: Option<SimpleLocalBackendPolicies>,
		tls: bool,
		resources: &crate::resource_manager::ResourceFetcher,
	) -> Result<BackendWithPolicies, anyhow::Error> {
		let mut inline_policies = match policies {
			Some(p) => {
				LocalBackendPolicies {
					simple: p,
					mcp_authorization: None,
					mcp_guardrails: None,
					a2a: None,
					inference_routing: None,
					ai: None,
					response_header_modifier: None,
					request_redirect: None,
					health: None,
					ext_authz: None,
					authorization: None,
				}
				.translate(resources)
				.await?
			},
			None => Vec::new(),
		};
		if tls {
			inline_policies.push(BackendTrafficPolicy::BackendTLS(
				LocalBackendTLS::default().try_into(resources).await?,
			));
		}
		Ok(BackendWithPolicies {
			backend: b,
			inline_policies,
		})
	}

	async fn process_mcp_backend(
		name: ResourceName,
		backend: McpBackendHost,
		policies: Option<SimpleLocalBackendPolicies>,
		resources: &crate::resource_manager::ResourceFetcher,
	) -> anyhow::Result<(
		SimpleBackendReference,
		Option<String>,
		Option<BackendWithPolicies>,
	)> {
		Ok(match backend.process()? {
			ProcessedMcpBackendHost::Inline { backend, path, tls } => {
				let (bref, be) = mcp_to_simple_backend_and_ref(name, backend);
				let be = match be {
					Some(b) => Some(Self::make_mcp_backend(b, policies, tls, resources).await?),
					None => None,
				};
				(bref, Some(path), be)
			},
			ProcessedMcpBackendHost::Reference { .. } if policies.is_some() => {
				anyhow::bail!("cannot use backend reference when policies are defined for an MCP target");
			},
			ProcessedMcpBackendHost::Reference { backend, path } => (backend, path, None),
		})
	}

	pub async fn as_backends(
		&self,
		name: ResourceName,
		resources: &crate::resource_manager::ResourceFetcher,
		mcp_session_ttl: Duration,
	) -> anyhow::Result<Vec<BackendWithPolicies>> {
		Ok(match self {
			LocalBackend::Service { .. } => vec![], // These stay as references
			LocalBackend::Backend(_) => vec![],     // These stay as references
			LocalBackend::Opaque(tgt) => vec![Backend::Opaque(name, tgt.clone()).into()],
			LocalBackend::Internal(tgt) => vec![Backend::Internal(name, tgt.clone()).into()],
			LocalBackend::Dynamic { .. } => vec![Backend::Dynamic(name, ()).into()],
			LocalBackend::MCP(tgt) => {
				let mut targets = vec![];
				let mut backends = vec![];
				for (idx, t) in tgt.targets.iter().enumerate() {
					validate_mcp_target_name(t.name.as_str()).map_err(Error::msg)?;
					let name = strng::format!("mcp/{}/{}", name.clone(), idx);
					let spec = match t.spec.clone() {
						LocalMcpTargetSpec::Sse { backend } => {
							let (bref, path, be) = Self::process_mcp_backend(
								local_name(name.clone()),
								backend,
								t.policies.clone(),
								resources,
							)
							.await?;
							if let Some(be) = be {
								backends.push(be);
							}
							McpTargetSpec::Sse(SseTargetSpec {
								backend: bref,
								path: path.ok_or_else(|| anyhow!("path is required when backend is set"))?,
							})
						},
						LocalMcpTargetSpec::Mcp { backend } => {
							let (bref, path, be) = Self::process_mcp_backend(
								local_name(name.clone()),
								backend,
								t.policies.clone(),
								resources,
							)
							.await?;
							if let Some(be) = be {
								backends.push(be);
							}
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
						} => {
							if t.policies.is_some() {
								anyhow::bail!(
									"stdio MCP target '{}': policies are not supported (stdio has no backend connection)",
									t.name,
								);
							}
							McpTargetSpec::Stdio {
								cmd,
								args,
								env,
								clear_env,
							}
						},
						LocalMcpTargetSpec::OpenAPI { backend, schema } => {
							let (bref, _, be) = Self::process_mcp_backend(
								local_name(name.clone()),
								backend,
								t.policies.clone(),
								resources,
							)
							.await?;
							if let Some(be) = be {
								backends.push(be);
							}

							let openapi_schema = schema.load_openapi_schema(resources).await?;
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
					prefix_mode: tgt.prefix_mode.unwrap_or_default(),
					failure_mode: tgt.failure_mode.unwrap_or_default(),
					session_idle_ttl: mcp_session_ttl,
				};
				backends.push(Backend::MCP(name, m).into());
				backends
			},
			LocalBackend::AI(tgt) => {
				let be = tgt.clone().translate(resources).await?;
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

/// Whether to keep a persistent session across requests (Stateful) or create one per request (Stateless).
#[apply(schema_de!)]
#[derive(Default)]
pub enum McpStatefulMode {
	Stateless,
	#[default]
	Stateful,
}

#[apply(schema_de!)]
pub struct LocalMcpBackend {
	/// MCP server targets to multiplex together.
	pub targets: Vec<Arc<LocalMcpTarget>>,
	#[serde(default)]
	pub stateful_mode: McpStatefulMode,
	/// How to namespace tool names when multiplexing: `always` prefix with the target name, or only prefix when needed (`conditional`).
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub prefix_mode: Option<McpPrefixMode>,
	/// Behavior when one or more MCP targets fail to initialize or fail during fanout.
	/// Defaults to `failClosed`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub failure_mode: Option<FailureMode>,
}

#[apply(schema_de!)]
pub struct LocalMcpTarget {
	/// Name identifying this MCP target, used to prefix tool and resource names when multiplexing.
	pub name: McpTargetName,
	#[serde(flatten)]
	pub spec: LocalMcpTargetSpec,
	/// Transport policies for connecting to this target's backend. Not supported
	/// on stdio targets. MCP policies (mcpAuthorization, mcpGuardrails) apply to
	/// the full target set and belong on the route or `mcp.policies`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub policies: Option<SimpleLocalBackendPolicies>,
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
#[serde(untagged)]
#[allow(dead_code)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
enum McpBackendHostSerde {
	HostUri {
		/// Hostname or URI of the MCP server, for example `https://example.com` or `example.com:443`.
		host: String,
	},
	HostParts {
		/// Hostname or IP address of the MCP server.
		host: String,
		/// Port on the MCP server to connect to.
		port: u16,
		/// Request path on the MCP server.
		path: String,
	},
	Backend {
		backend: BackendKey,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		path: Option<String>,
	},
}

#[apply(schema_de!)]
struct McpBackendHostInput {
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
		let raw = McpBackendHostInput::deserialize(deserializer)?;
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
	/// Connect to a remote MCP server over HTTP with Server-Sent Events (SSE) streaming.
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

fn mcp_matches() -> Vec<RouteMatch> {
	vec![
		RouteMatch {
			headers: vec![],
			path: PathMatch::PathPrefix("/mcp".into()),
			method: None,
			query: vec![],
		},
		RouteMatch {
			headers: vec![],
			path: PathMatch::PathPrefix("/sse".into()),
			method: None,
			query: vec![],
		},
		RouteMatch {
			headers: vec![],
			path: PathMatch::PathPrefix("/.well-known".into()),
			method: None,
			query: vec![],
		},
	]
}

fn ui_matches(oidc_redirect_path: Option<Strng>) -> Vec<RouteMatch> {
	let mut paths = vec![
		PathMatch::Exact("/".into()),
		PathMatch::PathPrefix("/ui".into()),
		PathMatch::PathPrefix("/api/runtime".into()),
		PathMatch::PathPrefix("/api/config".into()),
		PathMatch::PathPrefix("/api/cel".into()),
		PathMatch::PathPrefix("/api/logs".into()),
		PathMatch::PathPrefix("/api/costs".into()),
	];
	if let Some(path) = oidc_redirect_path
		&& !paths.iter().any(|existing| match existing {
			PathMatch::Exact(existing) | PathMatch::PathPrefix(existing) => existing == &path,
			_ => unreachable!(),
		}) {
		paths.push(PathMatch::Exact(path));
	}
	paths
		.into_iter()
		.map(|path| RouteMatch {
			headers: vec![],
			path,
			method: None,
			query: vec![],
		})
		.collect()
}

#[apply(schema_de!)]
struct LocalTCPRoute {
	#[serde(flatten)]
	name: LocalRouteName,
	/// Can be a wildcard
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	hostnames: Vec<Strng>,
	/// TCP-level policies applied to traffic on this route.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	policies: Option<TCPFilterOrPolicy>,
	/// Weighted backends this TCP route forwards traffic to.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	backends: Vec<LocalTCPRouteBackend>,
}

#[apply(schema_de!)]
pub struct LocalTCPRouteBackend {
	/// Relative weight for load balancing across TCP backends. Defaults to 1.
	#[serde(default = "default_weight")]
	pub weight: usize,
	#[serde(flatten)]
	pub backend: SimpleLocalBackend,
	/// Backend-level policies for TCP backends, such as TLS, authentication, and tunneling.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub policies: Option<LocalTCPBackendPolicies>,
}

#[apply(schema_de!)]
#[cfg_attr(feature = "schema", schemars(with = "SimpleLocalBackendSerde"))]
pub enum SimpleLocalBackendWithSchema {
	/// Service reference. Service must be defined in the top level services list.
	Service { name: NamespacedHostname, port: u16 },
	/// Hostname and port, with an optional scheme. Examples: `https://example.com`, `example.com:80`.
	#[serde(rename = "host")]
	Opaque(
		/// Hostname and port, with an optional scheme. Examples: `https://example.com`, `example.com:80`.
		TargetOrUri,
	),
	Backend(
		/// Explicit backend reference. Backend must be defined in the top level backends list
		BackendKey,
	),
	#[cfg_attr(feature = "schema", schemars(skip))]
	Invalid,
}

#[apply(schema_de!)]
#[cfg_attr(feature = "schema", schemars(with = "String"))]
#[serde(untagged)]
pub enum TargetOrUri {
	Target(Target),
	Uri(#[serde(with = "http_serde::uri")] Uri),
}

#[apply(schema_de!)]
#[cfg_attr(feature = "schema", schemars(with = "SimpleLocalBackendSerde"))]
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

#[allow(dead_code)]
#[apply(schema_de!)]
enum SimpleLocalBackendSerde {
	/// Service reference. Service must be defined in the top level services list.
	Service {
		/// Name of the target Service, as defined in the top-level `services` list.
		name: NamespacedHostname,
		/// Port on the target Service to route to.
		port: u16,
	},
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
	Option::<BackendAuthCompat>::deserialize(deserializer)?
		.map(|auth| match auth {
			BackendAuthCompat::Full(BackendAuth::OAuthTokenExchange(auth)) => {
				// OAuth has a few cross-field checks serde won't catch on its own.
				// Keep them here so untagged compat parsing still returns the real error.
				auth.validate_load().map_err(serde::de::Error::custom)?;
				Ok(BackendAuth::OAuthTokenExchange(auth))
			},
			BackendAuthCompat::Full(BackendAuth::CrossAppAccess(auth)) => {
				// Cross App Access is backed by the OAuth exchange implementation but has its own
				// focused config shape and cross-field checks.
				let mut auth = auth;
				auth
					.apply_local_defaults()
					.map_err(serde::de::Error::custom)?;
				auth.validate_load().map_err(serde::de::Error::custom)?;
				Ok(BackendAuth::CrossAppAccess(auth))
			},
			BackendAuthCompat::Full(auth) => Ok(auth),
			BackendAuthCompat::PlainKey { key } => Ok(BackendAuth::Key {
				value: key,
				location: None,
			}),
		})
		.transpose()
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(untagged)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
enum BackendAuthCompat {
	PlainKey {
		#[cfg_attr(feature = "schema", schemars(with = "FileOrInline"))]
		#[serde(deserialize_with = "deser_key_from_file")]
		key: SecretString,
	},
	Full(BackendAuth),
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

impl LocalGatewayPolicy {
	fn is_empty(&self) -> bool {
		self.oidc.is_none()
			&& self.jwt_auth.is_none()
			&& self.authorization.is_none()
			&& self.ext_authz.is_none()
			&& self.ext_proc.is_none()
			&& self.cors.is_none()
			&& self.transformations.is_none()
			&& self.basic_auth.is_none()
			&& self.api_key.is_none()
	}
}

#[apply(schema_de!)]
#[derive(Default)]
struct LocalUIPolicy {
	/// Handle CORS preflight requests and append configured CORS headers to applicable requests.
	#[serde(default)]
	cors: Option<http::cors::Cors>,
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
	/// Authenticate incoming requests with Basic Auth credentials from an htpasswd user database.
	#[serde(default)]
	basic_auth: Option<crate::http::basicauth::LocalBasicAuth>,
	/// Authenticate incoming requests with API keys.
	#[serde(default)]
	api_key: Option<crate::http::apikey::LocalAPIKeys>,
	/// Handle CSRF protection by validating request origins against configured allowed origins.
	#[serde(default)]
	csrf: Option<http::csrf::Csrf>,
}

impl From<LocalUIPolicy> for FilterOrPolicy {
	fn from(val: LocalUIPolicy) -> Self {
		let LocalUIPolicy {
			cors,
			oidc,
			jwt_auth,
			authorization,
			ext_authz,
			basic_auth,
			api_key,
			csrf,
		} = val;
		FilterOrPolicy {
			cors,
			oidc,
			jwt_auth,
			authorization,
			ext_authz,
			basic_auth,
			api_key,
			csrf,
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
	#[cfg_attr(feature = "schema", schemars(with = "Option<BackendAuthCompat>"))]
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
	/// Authorize incoming requests after this backend is selected.
	#[serde(default)]
	pub authorization: Option<Authorization>,

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
	pub async fn translate(
		self,
		resources: &crate::resource_manager::ResourceFetcher,
	) -> anyhow::Result<Vec<BackendTrafficPolicy>> {
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
			authorization,
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
			pols.push(BackendTrafficPolicy::BackendTLS(
				p.try_into(resources).await?,
			))
		}
		if let Some(p) = backend_auth {
			pols.push(BackendTrafficPolicy::BackendAuth(p))
		}
		if let Some(p) = ext_authz {
			pols.push(BackendTrafficPolicy::ExtAuthz(Arc::new(
				p.with_configured_cache_store(),
			)))
		}
		if let Some(p) = authorization {
			pols.push(BackendTrafficPolicy::Authorization(p))
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
	pub async fn translate(
		self,
		resources: &crate::resource_manager::ResourceFetcher,
	) -> anyhow::Result<Vec<BackendTrafficPolicy>> {
		let LocalTCPBackendPolicies {
			backend_tls,
			backend_tunnel,
		} = self;
		let mut pols = vec![];
		if let Some(p) = backend_tls {
			pols.push(BackendTrafficPolicy::BackendTLS(
				p.try_into(resources).await?,
			))
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
	#[cfg_attr(feature = "schema", schemars(with = "Option<BackendAuthCompat>"))]
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
	/// TLS configuration for connections to the TCP route's backend.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	#[serde(rename = "backendTLS")]
	backend_tls: Option<LocalBackendTLS>,
}

async fn convert(
	resources: &crate::resource_manager::ResourceFetcher,
	gateway: ListenerTarget,
	config: &crate::Config,
	mut i: LocalConfig,
) -> anyhow::Result<NormalizedLocalConfig> {
	apply_implicit_default_gateway(&mut i);
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
		gateways,
		routes,
		tcp_routes,
		llm,
		mcp,
		ui,
	} = i;
	merge_deprecated_frontend_policies(config, &mut frontend_policies)?;
	let mut all_policies = vec![];
	let mut all_backends = vec![];
	let mut all_binds = vec![];
	let mut all_listener_routes = vec![];
	let mut all_listener_tcp_routes = vec![];
	let gateway_refs = Box::pin(convert_gateways(
		resources,
		config,
		gateway.clone(),
		gateways,
		&mut all_binds,
		&mut all_listener_routes,
		&mut all_listener_tcp_routes,
		&mut all_policies,
	))
	.await?;
	for b in binds {
		// A standard bind requires a numeric port; an internal bind may omit the port to act as
		// the wildcard fallback that serves any destination port via in-process routing.
		if b.port.is_none() && b.mode != BindMode::Internal {
			bail!("a bind without a port must set mode: internal");
		}
		// The runtime treats an internal bind with port 0 as the wildcard, whether the port was
		// omitted or explicitly set to 0. Keep both representations on the single `bind/wildcard`
		// key (and address port 0) so they are indistinguishable downstream.
		let bind_port = b.port.unwrap_or(0);
		let is_wildcard = b.mode == BindMode::Internal && bind_port == 0;
		let bind_name = if is_wildcard {
			strng::literal!("bind/wildcard")
		} else {
			strng::format!("bind/{bind_port}")
		};
		let mut ls = ListenerSet::default();
		for (idx, l) in b.listeners.into_iter().enumerate() {
			let (l, routes, tcp_routes, pol, backends) = Box::pin(convert_listener(
				resources,
				config,
				idx,
				l,
				bind_name.clone(),
				gateway.clone(),
			))
			.await?;
			all_listener_routes.push((l.key.clone(), routes));
			all_listener_tcp_routes.push((l.key.clone(), tcp_routes));
			all_policies.extend_from_slice(&pol);
			all_backends.extend_from_slice(&backends);
			ls.insert(l)
		}
		let sockaddr = if cfg!(target_family = "unix") && config.ipv6_enabled {
			SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), bind_port)
		} else {
			// Windows and IPv6 don't mix well apparently?
			SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), bind_port)
		};
		let b = Bind {
			key: bind_name,
			address: sockaddr,
			protocol: detect_bind_protocol(&ls),
			listeners: ls,
			tunnel_protocol: b.tunnel_protocol,
			mode: b.mode,
		};
		all_binds.push(b)
	}

	for p in policies {
		p.target.validate()?;
		let policy_key = p.name.to_string();
		let backend_target = matches!(p.target, PolicyTarget::Backend(_));
		let res = split_policies_for_target(
			resources,
			p.policy,
			config.as_policy_context(&policy_key),
			backend_target,
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
		let policies = b.policies.map(|p| async { p.translate(resources).await });
		let policies = match policies {
			Some(policies) => policies.await?,
			None => Vec::new(),
		};
		let name = local_name(b.name);
		let lb: LocalBackend = b.spec.into();
		let mut bws = lb
			.as_backends(name.clone(), resources, config.mcp.session_ttl)
			.await?;

		// as_backends may expand a single LocalBackend into multiple Backends (e.g. MCP)
		// attach the policies to the "main" one
		// do not use `Backend::name` as it may create a computed name, not based
		if let Some(primary_bw) = bws.iter_mut().find(|bw| match &bw.backend {
			Backend::Opaque(n, _)
			| Backend::MCP(n, _)
			| Backend::AI(n, _)
			| Backend::LLMRouter(n, _)
			| Backend::Aws(n, _)
			| Backend::Dynamic(n, _)
			| Backend::Internal(n, _) => n == &name,
			Backend::Service(_, _) | Backend::Invalid => false,
		}) {
			primary_bw.inline_policies.extend_from_slice(&policies);
		} else {
			anyhow::bail!("as_backends did not return a backend with the expected name: {name}");
		}

		all_backends.extend(bws);
	}

	for route in routes {
		if route.gateways.is_empty() {
			bail!(
				"routes.{} must set gateways",
				route.route.name.name.as_deref().unwrap_or("<unnamed>")
			);
		}
		let listeners =
			resolve_gateway_references(&gateway_refs, &route.gateways, GatewayRouteKind::Http)?;
		for listener_key in listeners {
			let idx = listener_route_count(&all_listener_routes, &listener_key);
			let (route, backends) = Box::pin(convert_route(
				resources,
				config,
				route.route.clone(),
				idx,
				listener_key.clone(),
			))
			.await?;
			push_listener_routes(&mut all_listener_routes, listener_key, vec![route]);
			all_backends.extend_from_slice(&backends);
		}
	}

	for route in tcp_routes {
		if route.gateways.is_empty() {
			bail!(
				"tcpRoutes.{} must set gateways",
				route.route.name.name.as_deref().unwrap_or("<unnamed>")
			);
		}
		let listeners =
			resolve_gateway_references(&gateway_refs, &route.gateways, GatewayRouteKind::Tcp)?;
		for listener_key in listeners {
			let idx = listener_tcp_route_count(&all_listener_tcp_routes, &listener_key);
			let (route, policies, backends) = Box::pin(convert_tcp_route(
				route.route.clone(),
				idx,
				listener_key.clone(),
				resources,
			))
			.await?;
			push_listener_tcp_routes(&mut all_listener_tcp_routes, listener_key, vec![route]);
			all_policies.extend_from_slice(&policies);
			all_backends.extend_from_slice(&backends);
		}
	}

	match (llm, mcp) {
		(Some(llm_config), Some(mcp_config))
			if llm_config.gateways.is_empty()
				&& mcp_config.gateways.is_empty()
				&& llm_config.port.unwrap_or(DEFAULT_LLM_PORT)
					== mcp_config.port.unwrap_or(DEFAULT_MCP_PORT) =>
		{
			if llm_config.tls.is_some() {
				bail!("top-level llm and mcp cannot share a port when llm.tls is configured");
			}
			let (llm_bind, mut llm_routes, llm_policies, llm_backends) = Box::pin(convert_llm_config(
				resources,
				config,
				gateway.clone(),
				llm_config,
				false,
			))
			.await?;
			let (_mcp_bind, mcp_routes, mcp_policies, mcp_backends) = Box::pin(convert_mcp_config(
				resources,
				config,
				gateway.clone(),
				mcp_config,
				true,
			))
			.await?;
			llm_routes.extend(mcp_routes);
			all_listener_routes.push((strng::new(llm::LOCAL_LISTENER_NAME), llm_routes));
			all_listener_tcp_routes.push((strng::new(llm::LOCAL_LISTENER_NAME), Vec::new()));
			all_binds.push(llm_bind);
			all_policies.extend_from_slice(&llm_policies);
			all_policies.extend_from_slice(&mcp_policies);
			all_backends.extend_from_slice(&llm_backends);
			all_backends.extend_from_slice(&mcp_backends);
		},
		(llm, mcp) => {
			if let Some(llm_config) = llm {
				if llm_config.gateways.is_empty() {
					let (llm_bind, llm_routes, llm_policies, llm_backends) = Box::pin(convert_llm_config(
						resources,
						config,
						gateway.clone(),
						llm_config,
						false,
					))
					.await?;
					all_listener_routes.push((strng::new(llm::LOCAL_LISTENER_NAME), llm_routes));
					all_listener_tcp_routes.push((strng::new(llm::LOCAL_LISTENER_NAME), Vec::new()));
					all_binds.push(llm_bind);
					all_policies.extend_from_slice(&llm_policies);
					all_backends.extend_from_slice(&llm_backends);
				} else {
					Box::pin(convert_attached_llm(
						resources,
						config,
						gateway.clone(),
						llm_config,
						&gateway_refs,
						&mut all_listener_routes,
						&mut all_policies,
						&mut all_backends,
					))
					.await?;
				}
			}
			if let Some(mcp_config) = mcp {
				if mcp_config.gateways.is_empty() {
					let (mcp_bind, mcp_routes, mcp_policies, mcp_backends) = Box::pin(convert_mcp_config(
						resources,
						config,
						gateway.clone(),
						mcp_config,
						false,
					))
					.await?;
					all_listener_routes.push((strng::new("mcp"), mcp_routes));
					all_listener_tcp_routes.push((strng::new("mcp"), Vec::new()));
					all_binds.push(mcp_bind);
					all_policies.extend_from_slice(&mcp_policies);
					all_backends.extend_from_slice(&mcp_backends);
				} else {
					Box::pin(convert_attached_mcp(
						resources,
						config,
						gateway.clone(),
						mcp_config,
						&gateway_refs,
						&mut all_listener_routes,
						&mut all_backends,
					))
					.await?;
				}
			}
		},
	}
	if let Some(ui_config) = ui {
		Box::pin(convert_attached_ui(
			resources,
			config,
			ui_config,
			&gateway_refs,
			&mut all_listener_routes,
			&mut all_backends,
		))
		.await?;
	}

	// Convert route groups
	let mut all_route_groups = vec![];
	for rg in route_groups {
		let rg_key = rg.name.clone();
		let mut routes = vec![];
		for (idx, lr) in rg.routes.into_iter().enumerate() {
			let route_group_listener_key: ListenerKey = strng::format!("routegroup/{rg_key}");
			let (route, backends) = Box::pin(convert_route(
				resources,
				config,
				lr,
				idx,
				route_group_listener_key,
			))
			.await?;
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

fn apply_implicit_default_gateway(config: &mut LocalConfig) {
	let default_gateway = strng::literal!("default");
	let Some(gateway) = config.gateways.get(&default_gateway) else {
		return;
	};
	let default_has_http = gateway_config_has_route_kind(gateway, GatewayRouteKind::Http);
	let default_has_tcp = gateway_config_has_route_kind(gateway, GatewayRouteKind::Tcp);

	for route in &mut config.routes {
		if default_has_http && route.gateways.is_empty() {
			route.gateways.push(default_gateway.clone());
		}
	}
	for route in &mut config.tcp_routes {
		if default_has_tcp && route.gateways.is_empty() {
			route.gateways.push(default_gateway.clone());
		}
	}

	if let Some(ui) = &mut config.ui
		&& default_has_http
		&& ui.gateways.is_empty()
	{
		ui.gateways.push(default_gateway.clone());
	}

	if let Some(llm) = &mut config.llm
		&& default_has_http
		&& llm.gateways.is_empty()
		&& llm.port.is_none()
		&& llm.tls.is_none()
	{
		llm.gateways.push(default_gateway.clone());
	}

	if let Some(mcp) = &mut config.mcp
		&& default_has_http
		&& mcp.gateways.is_empty()
		&& mcp.port.is_none()
	{
		mcp.gateways.push(default_gateway);
	}
}

fn gateway_config_has_route_kind(gateway: &LocalGateway, kind: GatewayRouteKind) -> bool {
	if gateway.listeners.is_empty() {
		return effective_gateway_protocol(gateway.protocol, gateway.tls.is_some())
			.map(|protocol| gateway_route_kind(protocol) == kind)
			.unwrap_or(false);
	}
	gateway.listeners.iter().any(|listener| {
		effective_gateway_protocol(listener.protocol, listener.tls.is_some())
			.map(|protocol| gateway_route_kind(protocol) == kind)
			.unwrap_or(false)
	})
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
	let mut wildcard_binds = Vec::new();
	for (idx, bind) in config.binds.iter().enumerate() {
		// An internal bind whose effective port is 0 (port omitted or explicitly `0`) is the
		// wildcard fallback, which opens no socket (so there is no port to deconflict). There can be
		// at most one, since lookups would otherwise resolve the wildcard ambiguously. Everything
		// else with a numeric port participates in uniqueness checks.
		if bind.mode == BindMode::Internal && bind.port.unwrap_or(0) == 0 {
			wildcard_binds.push(idx);
		} else if let Some(port) = bind.port {
			insert_local_listener_port(port, format!("binds[{idx}]"))?;
		}
	}
	let mut gateway_ports = HashMap::<u16, String>::new();
	for (gateway_name, gateway_config) in &config.gateways {
		let Some(port) = gateway_config.port else {
			bail!("gateways.{gateway_name}.port is required");
		};
		let label = format!("gateways.{gateway_name}");
		if let Some(existing) = gateway_ports.insert(port, label.clone()) {
			bail!(
				"port {port} is configured by both {existing} and {label}; binds, llm, and mcp must use unique ports"
			);
		}
		if !gateway_config.listeners.is_empty()
			&& (gateway_config.protocol.is_some()
				|| gateway_config.tls.is_some()
				|| !gateway_config.policies.is_empty())
		{
			bail!(
				"gateways.{gateway_name} cannot set protocol, tls, or policy fields when listeners are configured"
			);
		}
		if !gateway_config.listeners.is_empty() {
			let mut listener_tls = None;
			let mut listener_kind = None;
			for (idx, listener) in gateway_config.listeners.iter().enumerate() {
				let protocol = effective_gateway_protocol(listener.protocol, listener.tls.is_some())
					.with_context(|| format!("gateways.{gateway_name}.listeners[{idx}]"))?;
				let kind = gateway_route_kind(protocol);
				if let Some(existing_kind) = listener_kind
					&& existing_kind != kind
				{
					bail!("gateway listeners on port {port} cannot mix HTTP and TCP protocols");
				}
				listener_kind = Some(kind);
				let tls = matches!(
					protocol,
					LocalGatewayProtocol::HTTPS | LocalGatewayProtocol::TLS
				);
				if let Some(existing_tls) = listener_tls
					&& existing_tls != tls
				{
					bail!("gateway listeners on port {port} cannot mix TLS and plaintext");
				}
				listener_tls = Some(tls);
			}
		} else {
			effective_gateway_protocol(gateway_config.protocol, gateway_config.tls.is_some())
				.with_context(|| format!("gateways.{gateway_name}"))?;
		}
	}
	for (port, label) in gateway_ports {
		insert_local_listener_port(port, label)?;
	}
	if let Some(llm) = &config.llm
		&& !llm.gateways.is_empty()
		&& (llm.port.is_some() || llm.tls.is_some())
	{
		bail!("llm.gateways cannot be used with llm.port or llm.tls");
	}
	if let Some(mcp) = &config.mcp
		&& !mcp.gateways.is_empty()
		&& mcp.port.is_some()
	{
		bail!("mcp.gateways cannot be used with mcp.port");
	}
	if let Some(ui) = &config.ui
		&& ui.gateways.is_empty()
	{
		bail!("ui.gateways must be set");
	}
	if wildcard_binds.len() > 1 {
		bail!(
			"at most one wildcard bind (an internal bind without a numeric port) is allowed, but found {}: binds{:?}",
			wildcard_binds.len(),
			wildcard_binds
		);
	}
	match (&config.llm, &config.mcp) {
		(Some(llm), Some(mcp))
			if llm.gateways.is_empty()
				&& mcp.gateways.is_empty()
				&& llm.port.unwrap_or(DEFAULT_LLM_PORT) == mcp.port.unwrap_or(DEFAULT_MCP_PORT) =>
		{
			insert_local_listener_port(
				llm.port.unwrap_or(DEFAULT_LLM_PORT),
				"llm and mcp".to_string(),
			)?;
		},
		(llm, mcp) => {
			if let Some(llm) = llm
				&& llm.gateways.is_empty()
			{
				insert_local_listener_port(
					llm.port.unwrap_or(DEFAULT_LLM_PORT),
					if llm.port.is_some() {
						"llm".to_string()
					} else {
						"llm (default)".to_string()
					},
				)?;
			}
			if let Some(mcp) = mcp
				&& mcp.gateways.is_empty()
			{
				insert_local_listener_port(
					mcp.port.unwrap_or(DEFAULT_MCP_PORT),
					if mcp.port.is_some() {
						"mcp".to_string()
					} else {
						"mcp (default)".to_string()
					},
				)?;
			}
		},
	}
	Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GatewayRouteKind {
	Http,
	Tcp,
}

impl GatewayRouteKind {
	fn label(self) -> &'static str {
		match self {
			Self::Http => "HTTP",
			Self::Tcp => "TCP",
		}
	}
}

#[derive(Clone)]
struct LocalGatewayReference {
	listener: ListenerKey,
	kind: GatewayRouteKind,
}

type LocalGatewayReferences = HashMap<Strng, Vec<LocalGatewayReference>>;

#[allow(clippy::too_many_arguments)]
async fn convert_gateways(
	resources: &crate::resource_manager::ResourceFetcher,
	config: &crate::Config,
	gateway: ListenerTarget,
	gateways: IndexMap<Strng, LocalGateway>,
	all_binds: &mut Vec<Bind>,
	all_listener_routes: &mut Vec<(ListenerKey, Vec<Route>)>,
	all_listener_tcp_routes: &mut Vec<(ListenerKey, Vec<TCPRoute>)>,
	all_policies: &mut Vec<TargetedPolicy>,
) -> anyhow::Result<LocalGatewayReferences> {
	let mut refs = LocalGatewayReferences::new();
	for (gateway_name, gateway_config) in gateways {
		let port = gateway_config
			.port
			.with_context(|| format!("gateways.{gateway_name}.port is required"))?;
		let mut listeners = ListenerSet::default();
		if gateway_config.listeners.is_empty() {
			let listener = LocalGatewayListener {
				name: Some(gateway_name.clone()),
				hostname: None,
				protocol: gateway_config.protocol,
				tls: gateway_config.tls,
				policies: gateway_config.policies,
			};
			let (listener, policies) = Box::pin(convert_gateway_listener(
				resources,
				config,
				gateway.clone(),
				gateway_name.clone(),
				listener,
			))
			.await?;
			refs.insert(
				gateway_name,
				vec![LocalGatewayReference {
					listener: listener.key.clone(),
					kind: gateway_listener_route_kind(&listener.protocol),
				}],
			);
			all_listener_routes.push((listener.key.clone(), Vec::new()));
			all_listener_tcp_routes.push((listener.key.clone(), Vec::new()));
			all_policies.extend(policies);
			listeners.insert(listener);
		} else {
			let mut listener_keys = Vec::new();
			for (idx, listener_config) in gateway_config.listeners.into_iter().enumerate() {
				let listener_name = listener_config
					.name
					.clone()
					.unwrap_or_else(|| strng::format!("listener{}", idx));
				let reference = strng::format!("{gateway_name}/{listener_name}");
				let (listener, policies) = Box::pin(convert_gateway_listener(
					resources,
					config,
					gateway.clone(),
					reference.clone(),
					listener_config,
				))
				.await?;
				let gateway_ref = LocalGatewayReference {
					listener: listener.key.clone(),
					kind: gateway_listener_route_kind(&listener.protocol),
				};
				refs.insert(reference, vec![gateway_ref.clone()]);
				listener_keys.push(gateway_ref);
				all_listener_routes.push((listener.key.clone(), Vec::new()));
				all_listener_tcp_routes.push((listener.key.clone(), Vec::new()));
				all_policies.extend(policies);
				listeners.insert(listener);
			}
			refs.insert(gateway_name, listener_keys);
		}
		let sockaddr = if cfg!(target_family = "unix") && config.ipv6_enabled {
			SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port)
		} else {
			SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port)
		};
		all_binds.push(Bind {
			key: strng::format!("bind/{port}"),
			address: sockaddr,
			protocol: detect_bind_protocol(&listeners),
			listeners,
			tunnel_protocol: TunnelProtocol::Direct,
			mode: BindMode::Standard,
		});
	}
	Ok(refs)
}

async fn convert_gateway_listener(
	resources: &crate::resource_manager::ResourceFetcher,
	config: &crate::Config,
	gateway: ListenerTarget,
	reference: Strng,
	listener: LocalGatewayListener,
) -> anyhow::Result<(Listener, Vec<TargetedPolicy>)> {
	let LocalGatewayListener {
		name: _,
		hostname,
		protocol,
		tls,
		policies,
	} = listener;
	let protocol = match effective_gateway_protocol(protocol, tls.is_some())? {
		LocalGatewayProtocol::HTTP => ListenerProtocol::HTTP,
		LocalGatewayProtocol::HTTPS => ListenerProtocol::HTTPS(
			tls
				.ok_or(anyhow!("HTTPS gateway listener requires 'tls'"))?
				.into_server_tls_config_with_resources(config.dynamic_ca_cert_cache.clone(), resources)
				.await?,
		),
		LocalGatewayProtocol::TCP => ListenerProtocol::TCP,
		LocalGatewayProtocol::TLS => ListenerProtocol::TLS(match tls {
			Some(tls) => Some(
				tls
					.into_server_tls_config_with_resources(config.dynamic_ca_cert_cache.clone(), resources)
					.await?,
			),
			None => None,
		}),
	};
	let name = ListenerName {
		gateway_name: gateway.gateway_name.clone(),
		gateway_namespace: gateway.gateway_namespace.clone(),
		listener_name: reference.clone(),
		listener_set: None,
	};
	let key: ListenerKey = strng::format!("gateway/{reference}");
	let listener = Listener {
		key: key.clone(),
		name: name.clone(),
		hostname: hostname.unwrap_or_else(|| strng::new("*")),
		protocol,
	};
	let mut all_policies = Vec::new();
	if !policies.is_empty() {
		let policy_context = strng::format!("gateway/{reference}");
		let pols = split_policies(
			resources,
			policies.into(),
			config.as_policy_context(policy_context),
		)
		.await?;
		let target = PolicyTarget::Gateway(name.into());
		for (idx, pol) in pols.route_policies.into_iter().enumerate() {
			all_policies.push(TargetedPolicy {
				key: strng::format!("gateway/{reference}/{idx}"),
				name: None,
				target: target.clone(),
				inheritance: Default::default(),
				policy: (pol, PolicyPhase::Gateway).into(),
			});
		}
	}
	Ok((listener, all_policies))
}

fn effective_gateway_protocol(
	protocol: Option<LocalGatewayProtocol>,
	tls: bool,
) -> anyhow::Result<LocalGatewayProtocol> {
	let protocol = protocol.unwrap_or(if tls {
		LocalGatewayProtocol::HTTPS
	} else {
		LocalGatewayProtocol::HTTP
	});
	match protocol {
		LocalGatewayProtocol::HTTP if tls => bail!("protocol HTTP cannot set tls"),
		LocalGatewayProtocol::HTTPS if !tls => bail!("protocol HTTPS requires tls"),
		LocalGatewayProtocol::TCP if tls => bail!("protocol TCP cannot set tls"),
		_ => Ok(protocol),
	}
}

fn gateway_route_kind(protocol: LocalGatewayProtocol) -> GatewayRouteKind {
	match protocol {
		LocalGatewayProtocol::HTTP | LocalGatewayProtocol::HTTPS => GatewayRouteKind::Http,
		LocalGatewayProtocol::TCP | LocalGatewayProtocol::TLS => GatewayRouteKind::Tcp,
	}
}

fn gateway_listener_route_kind(protocol: &ListenerProtocol) -> GatewayRouteKind {
	match protocol {
		ListenerProtocol::HTTP | ListenerProtocol::HTTPS(_) => GatewayRouteKind::Http,
		ListenerProtocol::TCP | ListenerProtocol::TLS(_) => GatewayRouteKind::Tcp,
		ListenerProtocol::HBONE => GatewayRouteKind::Tcp,
	}
}

fn resolve_gateway_references(
	gateway_refs: &LocalGatewayReferences,
	references: &[Strng],
	route_kind: GatewayRouteKind,
) -> anyhow::Result<Vec<ListenerKey>> {
	let mut listeners = Vec::new();
	for reference in references {
		let Some(resolved) = gateway_refs.get(reference) else {
			bail!("unknown gateway reference {reference}");
		};
		let mut matched = false;
		for resolved in resolved {
			if resolved.kind != route_kind {
				continue;
			}
			matched = true;
			let listener = &resolved.listener;
			if !listeners.contains(listener) {
				listeners.push(listener.clone());
			}
		}
		if !matched {
			bail!(
				"gateway reference {reference} has no {} listeners",
				route_kind.label()
			);
		}
	}
	Ok(listeners)
}

fn listener_route_count(
	all_listener_routes: &[(ListenerKey, Vec<Route>)],
	listener_key: &ListenerKey,
) -> usize {
	all_listener_routes
		.iter()
		.find(|(key, _)| key == listener_key)
		.map(|(_, routes)| routes.len())
		.unwrap_or_default()
}

fn listener_tcp_route_count(
	all_listener_tcp_routes: &[(ListenerKey, Vec<TCPRoute>)],
	listener_key: &ListenerKey,
) -> usize {
	all_listener_tcp_routes
		.iter()
		.find(|(key, _)| key == listener_key)
		.map(|(_, routes)| routes.len())
		.unwrap_or_default()
}

fn push_listener_routes(
	all_listener_routes: &mut Vec<(ListenerKey, Vec<Route>)>,
	listener_key: ListenerKey,
	routes: Vec<Route>,
) {
	if let Some((_, existing)) = all_listener_routes
		.iter_mut()
		.find(|(key, _)| key == &listener_key)
	{
		existing.extend(routes);
	} else {
		all_listener_routes.push((listener_key, routes));
	}
}

fn push_listener_tcp_routes(
	all_listener_tcp_routes: &mut Vec<(ListenerKey, Vec<TCPRoute>)>,
	listener_key: ListenerKey,
	routes: Vec<TCPRoute>,
) {
	if let Some((_, existing)) = all_listener_tcp_routes
		.iter_mut()
		.find(|(key, _)| key == &listener_key)
	{
		existing.extend(routes);
	} else {
		all_listener_tcp_routes.push((listener_key, routes));
	}
}

fn scoped_routes(listener_key: &ListenerKey, routes: Vec<Route>) -> Vec<Route> {
	routes
		.into_iter()
		.map(|mut route| {
			route.key = strng::format!("{listener_key}/{}", route.key);
			route
		})
		.collect()
}

fn ui_oidc_redirect_path(policies: Option<&LocalUIPolicy>) -> anyhow::Result<Option<Strng>> {
	let Some(oidc) = policies.and_then(|pol| pol.oidc.as_ref()) else {
		return Ok(None);
	};
	let uri: Uri = oidc.redirect_uri.parse().with_context(|| {
		format!(
			"invalid ui.policies.oidc.redirectURI: {}",
			oidc.redirect_uri
		)
	})?;
	Ok(Some(strng::new(uri.path())))
}

#[cfg(any(not(test), target_family = "unix"))]
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

#[allow(clippy::too_many_arguments)]
async fn convert_attached_llm(
	resources: &crate::resource_manager::ResourceFetcher,
	config: &crate::Config,
	gateway: ListenerTarget,
	llm_config: LocalLLMConfig,
	gateway_refs: &LocalGatewayReferences,
	all_listener_routes: &mut Vec<(ListenerKey, Vec<Route>)>,
	all_policies: &mut Vec<TargetedPolicy>,
	all_backends: &mut Vec<BackendWithPolicies>,
) -> anyhow::Result<()> {
	let listeners =
		resolve_gateway_references(gateway_refs, &llm_config.gateways, GatewayRouteKind::Http)?;
	let (_bind, routes, policies, backends) = Box::pin(convert_llm_config(
		resources, config, gateway, llm_config, true,
	))
	.await?;
	for listener_key in listeners {
		push_listener_routes(
			all_listener_routes,
			listener_key.clone(),
			scoped_routes(&listener_key, routes.clone()),
		);
	}
	all_policies.extend_from_slice(&policies);
	all_backends.extend_from_slice(&backends);
	Ok(())
}

async fn convert_attached_mcp(
	resources: &crate::resource_manager::ResourceFetcher,
	config: &crate::Config,
	gateway: ListenerTarget,
	mcp_config: LocalSimpleMcpConfig,
	gateway_refs: &LocalGatewayReferences,
	all_listener_routes: &mut Vec<(ListenerKey, Vec<Route>)>,
	all_backends: &mut Vec<BackendWithPolicies>,
) -> anyhow::Result<()> {
	let listeners =
		resolve_gateway_references(gateway_refs, &mcp_config.gateways, GatewayRouteKind::Http)?;
	let (_bind, routes, policies, backends) = Box::pin(convert_mcp_config(
		resources, config, gateway, mcp_config, true,
	))
	.await?;
	if !policies.is_empty() {
		bail!("internal error: attached mcp generated targeted policies");
	}
	for listener_key in listeners {
		push_listener_routes(
			all_listener_routes,
			listener_key.clone(),
			scoped_routes(&listener_key, routes.clone()),
		);
	}
	all_backends.extend_from_slice(&backends);
	Ok(())
}

async fn convert_attached_ui(
	resources: &crate::resource_manager::ResourceFetcher,
	config: &crate::Config,
	ui_config: LocalUIConfig,
	gateway_refs: &LocalGatewayReferences,
	all_listener_routes: &mut Vec<(ListenerKey, Vec<Route>)>,
	all_backends: &mut Vec<BackendWithPolicies>,
) -> anyhow::Result<()> {
	let listeners =
		resolve_gateway_references(gateway_refs, &ui_config.gateways, GatewayRouteKind::Http)?;
	let route_key = strng::new("ui");
	let route_matches = ui_matches(ui_oidc_redirect_path(ui_config.policies.as_ref())?);
	let resolved_policies = if let Some(pol) = ui_config.policies {
		split_policies(resources, pol.into(), config.as_policy_context(&route_key)).await?
	} else {
		ResolvedPolicies::default()
	};
	if !resolved_policies.backend_policies.is_empty() {
		bail!("ui.policies cannot contain backend policies");
	}
	let backend_key = strng::new("ui");
	let backends = LocalBackend::Internal(InternalBackend::Forward)
		.as_backends(
			local_name(backend_key.clone()),
			resources,
			config.mcp.session_ttl,
		)
		.await?;
	let routes = vec![Route {
		key: route_key,
		service_key: None,
		service_port: 0,
		name: RouteName {
			name: strng::new("ui"),
			namespace: strng::new("internal"),
			rule_name: None,
			kind: None,
		},
		hostnames: vec![],
		matches: route_matches,
		backends: vec![RouteBackendReference {
			weight: 1,
			target: BackendReference::Backend(strng::format!("/{backend_key}")).into(),
			inline_policies: vec![],
		}],
		llm_router: None,
		inline_policies: resolved_policies.route_policies,
	}];
	for listener_key in listeners {
		push_listener_routes(
			all_listener_routes,
			listener_key.clone(),
			scoped_routes(&listener_key, routes.clone()),
		);
	}
	all_backends.extend_from_slice(&backends);
	Ok(())
}

#[derive(Clone)]
struct ResolvedLLMModelTarget {
	name: String,
	provider: NamedAIProvider,
	inline_policies: Vec<BackendTrafficPolicy>,
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
	resources: &crate::resource_manager::ResourceFetcher,
	config: &crate::Config,
	gateway: ListenerTarget,
	llm_config: LocalLLMConfig,
	attach_policies_to_route: bool,
) -> anyhow::Result<(
	Bind,
	Vec<Route>,
	Vec<TargetedPolicy>,
	Vec<BackendWithPolicies>,
)> {
	let LocalLLMConfig {
		gateways: _,
		port,
		tls,
		providers,
		models,
		virtual_models,
		policies,
	} = llm_config;
	let port = port.unwrap_or(DEFAULT_LLM_PORT);
	let tls = match tls {
		Some(tls) => Some(
			tls
				.into_server_tls_config_with_resources(config.dynamic_ca_cert_cache.clone(), resources)
				.await?,
		),
		None => None,
	};
	let llm_registry = LocalLLMModelRegistry::new(models, virtual_models)?;

	let mut all_policies = vec![];
	let mut all_backends = vec![];
	let mut routes = Vec::new();
	let mut llm_request_policies = Vec::new();
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
		let feature_route_policies = split_policies(
			resources,
			FilterOrPolicy {
				local_rate_limit: (!local_rate_limit.is_empty())
					.then_some(LocalRateLimitPolicy::Explicit(local_rate_limit)),
				remote_rate_limit: remote_rate_limit.map(LocalExplicitOrConditional::Explicit),
				..Default::default()
			},
			None,
		)
		.await?;

		let gateway_policies: FilterOrPolicy = gateway.into();
		let gateway_policies = split_policies(
			resources,
			gateway_policies,
			config.as_policy_context("listener/llm"),
		)
		.await?;
		if attach_policies_to_route {
			llm_request_policies.extend(gateway_policies.route_policies);
			llm_request_policies.extend(feature_route_policies.route_policies);
			(vec![], vec![])
		} else {
			// Legacy top-level LLM owns its listener, so gateway-style policy fields target that listener.
			(
				gateway_policies.route_policies,
				feature_route_policies.route_policies,
			)
		}
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
			LocalModelAIProvider::Bedrock => AIProvider::bedrock(crate::llm::bedrock::Provider {
				model,
				region: p.aws_region.context("bedrock requires aws_region")?,
				guardrail_identifier: None,
				guardrail_version: None,
			}),
			LocalModelAIProvider::Azure => AIProvider::azure(crate::llm::azure::Provider {
				model,
				resource_name: p
					.azure_resource_name
					.context("azure requires azureResourceName")?,
				resource_type: p
					.azure_resource_type
					.context("azure requires azureResourceType")?,
				api_version: p.azure_api_version,
				project_name: p.azure_project_name,
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
			pols.push(BackendTrafficPolicy::BackendTLS(
				p.try_into(resources).await?,
			));
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
		if let Some(authorization) = model_config.authorization.clone() {
			pols.push(BackendTrafficPolicy::Authorization(authorization));
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
		});

		router_models.push(llm::model_router::ModelRoute {
			name: model_config.name.clone(),
			visibility: model_config.visibility,
			header_matches: model_config
				.matches
				.iter()
				.map(|m| m.headers.clone())
				.collect(),
			backend_key,
			policies: llm::model_router::ModelRoutePolicies {
				llm: Arc::new(crate::llm::Policy {
					routes: llm_routes.into_iter().collect(),
					..Default::default()
				}),
				authorization: model_config.authorization.clone(),
			},
			backend_policies: vec![],
		});
	}

	let virtual_models = llm_registry.into_virtual_models();
	let mut router_virtual_models = Vec::new();
	for (idx, virtual_model) in virtual_models.into_iter().enumerate() {
		let llm_policy = Arc::new(crate::llm::Policy {
			routes: llm_route_types(None).into_iter().collect(),
			..Default::default()
		});
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
								let resolved = resolved_models.resolve(&target.model)?;
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
			llm_policy,
			routing,
		});
	}

	let router_backend_key = strng::new("llm:router");
	all_backends.push(BackendWithPolicies {
		backend: Backend::LLMRouter(
			local_name(router_backend_key.clone()),
			Arc::new(llm::model_router::ModelRouter::new(
				router_models,
				router_virtual_models,
				startup_timestamp,
			)),
		),
		inline_policies: vec![],
	});

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
		backends: vec![RouteBackendReference {
			weight: 1,
			target: BackendReference::Backend(strng::format!("/{router_backend_key}")).into(),
			inline_policies: vec![],
		}],
		llm_router: None,
		inline_policies: llm_request_policies,
	});

	// Create listener
	let listener_key: ListenerKey = strng::new(llm::LOCAL_LISTENER_NAME);
	let listener_name = ListenerName {
		gateway_name: gateway.gateway_name.clone(),
		gateway_namespace: gateway.gateway_namespace.clone(),
		listener_name: strng::new(llm::LOCAL_LISTENER_NAME),
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
		mode: BindMode::Standard,
	};

	Ok((bind, routes, all_policies, all_backends))
}

async fn convert_mcp_config(
	resources: &crate::resource_manager::ResourceFetcher,
	config: &crate::Config,
	gateway: ListenerTarget,
	mcp_config: LocalSimpleMcpConfig,
	shared_port: bool,
) -> anyhow::Result<(
	Bind,
	Vec<Route>,
	Vec<TargetedPolicy>,
	Vec<BackendWithPolicies>,
)> {
	let LocalSimpleMcpConfig {
		gateways: _,
		port,
		backend,
		policies,
	} = mcp_config;
	let port = port.unwrap_or(DEFAULT_MCP_PORT);
	let route_key = strng::new("mcp:default");

	let resolved_policies = if let Some(pol) = policies {
		split_policies(resources, pol, config.as_policy_context(&route_key)).await?
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
		matches: if shared_port {
			mcp_matches()
		} else {
			default_matches()
		},
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
		mode: BindMode::Standard,
	};

	let backends = LocalBackend::MCP(backend)
		.as_backends(
			local_name(strng::new("mcp")),
			resources,
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
	resources: &crate::resource_manager::ResourceFetcher,
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
					.into_server_tls_config_with_resources(config.dynamic_ca_cert_cache.clone(), resources)
					.await?,
			)
		},
		LocalListenerProtocol::TLS => {
			if tcp_routes.is_none() {
				bail!("protocol TLS requires 'tcpRoutes'")
			}
			ListenerProtocol::TLS(match tls {
				Some(tls) => Some(
					tls
						.into_server_tls_config_with_resources(config.dynamic_ca_cert_cache.clone(), resources)
						.await?,
				),
				None => None,
			})
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
		let (route, backends) = Box::pin(convert_route(resources, config, l, idx, key.clone())).await?;
		all_backends.extend_from_slice(&backends);
		rs.push(route)
	}

	let mut trs = Vec::new();
	for (idx, l) in tcp_routes.into_iter().flatten().enumerate() {
		let (route, policies, backends) =
			Box::pin(convert_tcp_route(l, idx, key.clone(), resources)).await?;
		all_policies.extend_from_slice(&policies);
		all_backends.extend_from_slice(&backends);
		trs.push(route)
	}

	if let Some(pol) = policies {
		let listener_policy_id = strng::format!("listener/{key}");
		let pols = split_policies(
			resources,
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
	resources: &crate::resource_manager::ResourceFetcher,
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
			.map(|p| async { p.translate(resources).await });
		let policies = match policies {
			Some(policies) => policies.await?,
			None => Vec::new(),
		};
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
			.as_backends(be_name.clone(), resources, config.mcp.session_ttl)
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
			resources,
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
	resources: &crate::resource_manager::ResourceFetcher,
	pol: FilterOrPolicy,
	attached: Option<AttachedPolicyContext<'_>>,
) -> Result<ResolvedPolicies, Error> {
	split_policies_for_target(resources, pol, attached, false).await
}

/// Like [`split_policies`], but when `backend_target` is true dual-role policy
/// types (transformations, header modifiers, etc.) become [`BackendTrafficPolicy`]
/// so top-level `policies` with `target.backend` apply via `as_backend()`.
pub(crate) async fn split_policies_for_target(
	resources: &crate::resource_manager::ResourceFetcher,
	pol: FilterOrPolicy,
	attached: Option<AttachedPolicyContext<'_>>,
	backend_target: bool,
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
		if backend_target {
			backend_policies.push(BackendTrafficPolicy::RequestHeaderModifier(p));
		} else {
			route_policies.push(TrafficPolicy::RequestHeaderModifier(RequestPolicy::single(
				p,
			)));
		}
	}
	if let Some(p) = response_header_modifier {
		if backend_target {
			backend_policies.push(BackendTrafficPolicy::ResponseHeaderModifier(Arc::new(p)));
		} else {
			route_policies.push(TrafficPolicy::ResponseHeaderModifier(
				RequestPolicy::single(p),
			));
		}
	}
	if let Some(p) = request_redirect {
		if backend_target {
			backend_policies.push(BackendTrafficPolicy::RequestRedirect(p));
		} else {
			route_policies.push(TrafficPolicy::RequestRedirect(RequestPolicy::single(p)));
		}
	}
	if let Some(p) = url_rewrite {
		route_policies.push(TrafficPolicy::UrlRewrite(RequestPolicy::single(p)));
	}
	if let Some(p) = request_mirror {
		if backend_target {
			backend_policies.push(BackendTrafficPolicy::RequestMirror(vec![p]));
		} else {
			route_policies.push(TrafficPolicy::RequestMirror(vec![p]));
		}
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
		let authn: McpAuthentication = p.translate(resources).await?;
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
		backend_policies.push(BackendTrafficPolicy::BackendTLS(
			p.try_into(resources).await?,
		))
	}
	if let Some(p) = backend_tunnel {
		backend_policies.push(BackendTrafficPolicy::Tunnel(p))
	}
	if let Some(p) = backend_auth {
		backend_policies.push(BackendTrafficPolicy::BackendAuth(p))
	}

	// Route policies (AI is dual-role when targeting a backend)
	if let Some(mut p) = ai {
		p.compile_model_alias_patterns();
		if backend_target {
			backend_policies.push(BackendTrafficPolicy::AI(Arc::new(p)));
		} else {
			route_policies.push(TrafficPolicy::AI(Arc::new(p)));
		}
	}
	if let Some(p) = jwt_auth {
		route_policies.push(TrafficPolicy::JwtAuth(RequestPolicy::single(
			JwtAuthentication {
				jwt: p.try_into(resources).await?,
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
				.compile(resources, oidc_policy_id, oidc_cookie_encoder)
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
		if backend_target {
			let LocalExplicitOrConditional::Explicit(cfg) = p else {
				bail!("conditional transformations are not supported on backend-targeted policies");
			};
			backend_policies.push(BackendTrafficPolicy::Transformation(Arc::new(
				Transformation::try_from_local_config(cfg, true)?,
			)));
		} else {
			route_policies.push(TrafficPolicy::Transformation(
				p.into_transformation_policy()?,
			));
		}
	}
	if let Some(p) = csrf {
		route_policies.push(TrafficPolicy::Csrf(RequestPolicy::single(p)))
	}
	if let Some(p) = authorization {
		if backend_target {
			backend_policies.push(BackendTrafficPolicy::Authorization(p));
		} else {
			route_policies.push(TrafficPolicy::Authorization(p));
		}
	}
	if let Some(p) = ext_authz {
		if backend_target {
			let LocalExplicitOrConditional::Explicit(cfg) = configure_ext_authz_cache_store(p) else {
				bail!("conditional extAuthz is not supported on backend-targeted policies");
			};
			backend_policies.push(BackendTrafficPolicy::ExtAuthz(Arc::new(cfg)));
		} else {
			route_policies.push(TrafficPolicy::ExtAuthz(
				configure_ext_authz_cache_store(p).into_policy()?,
			));
		}
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
	resources: &crate::resource_manager::ResourceFetcher,
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
			.map(|p| async { p.translate(resources).await });
		let policies = match policies {
			Some(policies) => policies.await?,
			None => Vec::new(),
		};
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
			let backend_tls = BackendTrafficPolicy::BackendTLS(p.try_into(resources).await?);
			for br in backend_refs.iter_mut() {
				br.inline_policies.push(backend_tls.clone());
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

impl LocalTLSServerConfig {
	async fn into_server_tls_config_with_resources(
		self,
		dynamic_ca_cert_cache: crate::DynamicCaCertCacheConfig,
		resources: &crate::resource_manager::ResourceFetcher,
	) -> anyhow::Result<ServerTLSConfig> {
		let cert_pem = resources
			.fetch(crate::resource_manager::ResourceRef::File(self.cert))
			.await?
			.to_vec();
		let key_pem = resources
			.fetch(crate::resource_manager::ResourceRef::File(self.key))
			.await?
			.to_vec();
		let root_pem = match self.root {
			Some(root) => Some(
				resources
					.fetch(crate::resource_manager::ResourceRef::File(root))
					.await?
					.to_vec(),
			),
			None => None,
		};
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
	let resources = crate::resource_manager::ResourceFetcher::files_only();
	// This serde hook has no runtime resource manager, but backend TLS policy
	// compatibility can still resolve local file references.
	// Not ideal but greatly simplifies the code
	futures::executor::block_on(
		LocalBackendPolicies {
			simple: s,
			..Default::default()
		}
		.translate(&resources),
	)
	.map_err(serde::de::Error::custom)
}
