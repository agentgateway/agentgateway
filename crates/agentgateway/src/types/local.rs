use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::agentcore;
use crate::client::Client;
use crate::http::auth::BackendAuth;
use crate::http::backendtls::LocalBackendTLS;
use crate::http::filters::HeaderModifier;
use crate::http::transformation_cel::{LocalTransformationConfig, Transformation};
use crate::http::{
	HeaderName, HeaderOrPseudo, filters, health, retry, timeout, transformation_cel,
};
use crate::llm::policy::PromptGuard;
use crate::llm::{AIBackend, AIProvider, LocalModelAIProvider, NamedAIProvider};
use crate::llm::{anthropic, openai};
use crate::mcp::FailureMode;
use crate::mcp::McpAuthorization;
use crate::store::LocalWorkload;
use crate::types::agent::{
	A2aPolicy, Authorization, Backend, BackendKey, BackendPolicy, BackendReference,
	BackendWithPolicies, Bind, BindProtocol, FrontendPolicy, HeaderMatch, HeaderValueMatch, Listener,
	ListenerKey, ListenerName, ListenerProtocol, ListenerSet, ListenerTarget, LocalMcpAuthentication,
	McpAuthentication, McpBackend, McpTarget, McpTargetName, McpTargetSpec, OpenAPITarget, PathMatch,
	PolicyKey, PolicyPhase, PolicyTarget, PolicyType, ResourceName, Route, RouteBackendReference,
	RouteKey, RouteMatch, RouteName, RouteSet, ServerTLSConfig, SimpleBackend,
	SimpleBackendReference, SimpleBackendWithPolicies, SseTargetSpec, StreamableHTTPTargetSpec,
	TCPRoute, TCPRouteBackendReference, TCPRouteSet, Target, TargetedPolicy, TracingConfig,
	TrafficPolicy, TunnelProtocol, TypedResourceName,
};
use crate::types::discovery::{NamespacedHostname, Service};
use crate::types::{backend, frontend};
use crate::*;
use ::http::Uri;
use agent_core::prelude::Strng;
use anyhow::{Error, anyhow, bail};
use bytes::Bytes;
use frozen_collections::Len;
use itertools::Itertools;
use macro_rules_attribute::apply;
use secrecy::SecretString;

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
			vec![BackendPolicy::Transformation(backend_xfm)]
		} else {
			Vec::new()
		};
		if let Some(ep) = endpoint {
			// Strip the scheme (http:// or https://) from the endpoint URL to get host:port
			let host_port = ep
				.strip_prefix("http://")
				.or_else(|| ep.strip_prefix("https://"))
				.unwrap_or(&ep);
			frontend_policies.tracing = Some(TracingConfig {
				provider_backend: SimpleBackendReference::InlineBackend(Target::try_from(host_port)?),
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
	pub policies: Vec<TargetedPolicy>,
	pub backends: Vec<BackendWithPolicies>,
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
	#[serde(default)]
	llm: Option<LocalLLMConfig>,
	#[serde(default)]
	mcp: Option<LocalSimpleMcpConfig>,
}

#[apply(schema_de!)]
pub struct LocalLLMConfig {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	port: Option<u16>,
	/// models defines the set of models that can be served by this gateway. The model name refers to the
	/// model in the users request that is matched; the model sent to the actual LLM can be overridden
	/// on a per-model basis.
	models: Vec<LocalLLMModels>,
	/// policies defines policies for handling incoming requests, before a model is selected
	#[serde(default, skip_serializing_if = "Option::is_none")]
	policies: Option<LocalLLMPolicy>,
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

#[apply(schema_de!)]
pub struct LocalLLMModels {
	/// name is the name of the model we are matching from a users request. If params.model is set, that
	/// will be used in the request to the LLM provider. If not, the incoming model is used.
	name: String,
	/// params customizes parameters for the outgoing request
	#[serde(default)]
	params: LocalLLMParams,
	/// provider of the LLM we are connecting too
	provider: LocalModelAIProvider,

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
	/// backendTLS configures TLS when connecting to the LLM provider.
	#[serde(rename = "backendTLS", default)]
	backend_tls: Option<http::backendtls::LocalBackendTLS>,
	/// health configures outlier detection for this model backend.
	#[serde(default)]
	health: Option<health::LocalHealthPolicy>,
	/// backendTunnel configures tunneling when connecting to the LLM provider.
	#[serde(default)]
	backend_tunnel: Option<backend::Tunnel>,
	/// guardrails to apply to the request or response
	#[serde(default, skip_serializing_if = "Option::is_none")]
	guardrails: Option<PromptGuard>,

	/// matches specifies the conditions under which this model should be used in addition to matching the model name.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	matches: Vec<LLMRouteMatch>,
}

#[apply(schema!)]
pub struct LLMRouteMatch {
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub headers: Vec<HeaderMatch>,
}

#[apply(schema!)]
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
	/// For Azure: the host of the deployment
	azure_host: Option<Strng>,
	/// For Azure: the API version to use
	azure_api_version: Option<Strng>,
	/// Override the upstream host for this provider.
	#[serde(default)]
	host_override: Option<Target>,
	/// Override the upstream path for this provider.
	#[serde(default)]
	path_override: Option<Strng>,
	/// Override the default base path prefix for this provider.
	#[serde(default)]
	path_prefix: Option<Strng>,
	/// Whether to tokenize the request before forwarding it upstream.
	#[serde(default)]
	tokenize: bool,
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

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct LocalTLSServerConfig {
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
	pub host: Target,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub policies: Option<LocalBackendPolicies>,
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
				a2a: None,
				ai: None,
			})
			.map(LocalBackendPolicies::translate)
			.transpose()?
			.unwrap_or_default();
		if tls {
			inline_policies.push(BackendPolicy::BackendTLS(
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
					let name = strng::format!("mcp/{}/{}", name.clone(), idx);
					let spec = match t.spec.clone() {
						LocalMcpTargetSpec::Sse { backend } => {
							let (backend, path, tls) = backend.process()?;
							let (bref, be) = mcp_to_simple_backend_and_ref(local_name(name.clone()), backend);
							if let Some(b) = be {
								backends.push(Self::make_mcp_backend(b, t.policies.clone(), tls)?);
							}
							McpTargetSpec::Sse(SseTargetSpec {
								backend: bref,
								path: path.clone(),
							})
						},
						LocalMcpTargetSpec::Mcp { backend } => {
							let (backend, path, tls) = backend.process()?;
							let (bref, be) = mcp_to_simple_backend_and_ref(local_name(name.clone()), backend);
							if let Some(b) = be {
								backends.push(Self::make_mcp_backend(b, t.policies.clone(), tls)?);
							}
							McpTargetSpec::Mcp(StreamableHTTPTargetSpec {
								backend: bref,
								path: path.clone(),
							})
						},
						LocalMcpTargetSpec::Stdio { cmd, args, env } => McpTargetSpec::Stdio { cmd, args, env },
						LocalMcpTargetSpec::OpenAPI { backend, schema } => {
							let (backend, _, tls) = backend.process()?;
							let (bref, be) = mcp_to_simple_backend_and_ref(local_name(name.clone()), backend);
							if let Some(b) = be {
								backends.push(Self::make_mcp_backend(b, t.policies.clone(), tls)?);
							}
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
			LocalBackend::Invalid => vec![Backend::Invalid.into()],
		})
	}
}

impl SimpleLocalBackend {
	pub fn as_backends(
		&self,
		name: ResourceName,
		policies: Vec<BackendPolicy>,
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

#[apply(schema_de!)]
// Ideally this would be an enum of Simple|Explicit, but serde bug prevents it:
// https://github.com/serde-rs/serde/issues/1600
pub struct McpBackendHost {
	host: String,
	port: Option<u16>,
	path: Option<String>,
}

impl McpBackendHost {
	pub fn process(&self) -> anyhow::Result<(Target, String, bool)> {
		let McpBackendHost { host, port, path } = self;
		Ok(match (host, port, path) {
			(host, Some(port), Some(path)) => {
				let b = Target::try_from((host.as_str(), *port))?;
				(b, path.clone(), false)
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

				let b = Target::try_from((host, port))?;
				(b, path.to_string(), scheme == &http::Scheme::HTTPS)
			},
			_ => {
				anyhow::bail!("if port or path is set, both must be set; otherwise, use only host")
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
	Service {
		name: NamespacedHostname,
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
	pub name: ResourceName,
	pub target: PolicyTarget,

	/// phase defines at what level the policy runs at. Gateway policies run pre-routing, while
	/// Route policies apply post-routing.
	/// Only a subset of policies are eligible as Gateway policies.
	/// In general, normal (route level) policies should be used, except you need the policy to influence
	/// routing.
	#[serde(default)]
	pub phase: PolicyPhase,
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

#[apply(schema_de!)]
#[derive(Default)]
struct LocalLLMPolicy {
	#[serde(flatten)]
	gateway: LocalGatewayPolicy,
	/// Authorization policies for HTTP access.
	#[serde(default)]
	authorization: Option<Authorization>,
}

#[apply(schema_de!)]
#[derive(Default)]
struct LocalGatewayPolicy {
	/// Authenticate incoming browser requests with OIDC authorization code flow.
	#[serde(default)]
	oidc: Option<crate::http::oidc::LocalOidcConfig>,
	/// Authenticate incoming JWT requests.
	#[serde(default)]
	jwt_auth: Option<crate::http::jwt::LocalJwtConfig>,
	/// Authenticate incoming requests by calling an external authorization server.
	#[serde(default)]
	ext_authz: Option<crate::http::ext_authz::ExtAuthz>,
	/// Extend agentgateway with an external processor
	#[serde(default)]
	ext_proc: Option<crate::http::ext_proc::ExtProc>,
	/// Modify requests and responses
	#[serde(default)]
	#[serde(deserialize_with = "de_transform")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "Option<http::transformation_cel::LocalTransformationConfig>")
	)]
	transformations: Option<crate::http::transformation_cel::Transformation>,
	/// Authenticate incoming requests using Basic Authentication with htpasswd.
	#[serde(default)]
	basic_auth: Option<crate::http::basicauth::LocalBasicAuth>,
	/// Authenticate incoming requests using API Keys
	#[serde(default)]
	api_key: Option<crate::http::apikey::LocalAPIKeys>,
}

impl From<LocalGatewayPolicy> for FilterOrPolicy {
	fn from(val: LocalGatewayPolicy) -> Self {
		let LocalGatewayPolicy {
			oidc,
			jwt_auth,
			ext_authz,
			ext_proc,
			transformations,
			basic_auth,
			api_key,
		} = val;
		FilterOrPolicy {
			oidc,
			jwt_auth,
			ext_authz,
			ext_proc,
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
	/// Headers to be modified in the request.
	#[serde(default)]
	pub request_header_modifier: Option<filters::HeaderModifier>,

	/// Headers to be modified in the response.
	#[serde(default)]
	pub response_header_modifier: Option<filters::HeaderModifier>,

	/// Directly respond to the request with a redirect.
	#[serde(default)]
	pub request_redirect: Option<filters::RequestRedirect>,

	/// Modify requests and responses sent to and from the backend.
	#[serde(default)]
	#[serde(deserialize_with = "de_transform")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "Option<http::transformation_cel::LocalTransformationConfig>")
	)]
	pub transformations: Option<crate::http::transformation_cel::Transformation>,

	/// Send TLS to the backend.
	#[serde(rename = "backendTLS", default)]
	pub backend_tls: Option<http::backendtls::LocalBackendTLS>,
	/// Authenticate to the backend.
	#[serde(default)]
	pub backend_auth: Option<BackendAuth>,

	/// Specify HTTP settings for the backend
	#[serde(default)]
	pub http: Option<backend::HTTP>,
	/// Specify TCP settings for the backend
	#[serde(default)]
	pub tcp: Option<backend::TCP>,

	/// Health policy for backend outlier detection; evicts on unhealthy responses based on CEL condition and configurable duration.
	#[serde(default)]
	pub health: Option<health::LocalHealthPolicy>,

	/// Specify a tunnel to use when connecting to the backend
	#[serde(default)]
	pub backend_tunnel: Option<backend::Tunnel>,
}

#[apply(schema_de!)]
#[derive(Default)]
pub struct MCPLocalBackendPolicies {
	#[serde(flatten)]
	simple: SimpleLocalBackendPolicies,
	/// Authorization policies for MCP access.
	#[serde(default)]
	pub mcp_authorization: Option<McpAuthorization>,
}

#[apply(schema_de!)]
#[derive(Default)]
pub struct LocalBackendPolicies {
	#[serde(flatten)]
	simple: SimpleLocalBackendPolicies,

	/// Authorization policies for MCP access.
	#[serde(default)]
	pub mcp_authorization: Option<McpAuthorization>,
	/// Mark this traffic as A2A to enable A2A processing and telemetry.
	#[serde(default)]
	pub a2a: Option<A2aPolicy>,
	/// Mark this as LLM traffic to enable LLM processing.
	#[serde(default)]
	pub ai: Option<llm::Policy>,
}

impl LocalBackendPolicies {
	pub fn translate(self) -> anyhow::Result<Vec<BackendPolicy>> {
		let LocalBackendPolicies {
			simple:
				SimpleLocalBackendPolicies {
					request_header_modifier,
					response_header_modifier,
					request_redirect,
					transformations,
					backend_tls,
					backend_auth,
					http,
					tcp,
					health,
					backend_tunnel,
				},
			mcp_authorization,
			a2a,
			ai,
		} = self;
		let mut pols = vec![];
		if let Some(p) = tcp {
			pols.push(BackendPolicy::TCP(p));
		}
		if let Some(p) = backend_tunnel {
			pols.push(BackendPolicy::Tunnel(p));
		}
		if let Some(p) = http {
			pols.push(BackendPolicy::HTTP(p));
		}
		if let Some(p) = request_header_modifier {
			pols.push(BackendPolicy::RequestHeaderModifier(p));
		}
		if let Some(p) = response_header_modifier {
			pols.push(BackendPolicy::ResponseHeaderModifier(p));
		}
		if let Some(p) = request_redirect {
			pols.push(BackendPolicy::RequestRedirect(p));
		}
		if let Some(p) = transformations {
			pols.push(BackendPolicy::Transformation(p));
		}
		if let Some(p) = mcp_authorization {
			pols.push(BackendPolicy::McpAuthorization(p))
		}
		if let Some(p) = a2a {
			pols.push(BackendPolicy::A2a(p))
		}
		if let Some(p) = backend_tls {
			pols.push(BackendPolicy::BackendTLS(p.try_into()?))
		}
		if let Some(p) = backend_auth {
			pols.push(BackendPolicy::BackendAuth(p))
		}
		if let Some(mut p) = ai {
			p.compile_model_alias_patterns();
			pols.push(BackendPolicy::AI(Arc::new(p)))
		}
		if let Some(p) = health {
			pols.push(BackendPolicy::Health(p.try_into().map_err(
				|e: crate::cel::Error| anyhow::anyhow!("health.unhealthyExpression: {}", e),
			)?));
		}
		Ok(pols)
	}
}

#[apply(schema_de!)]
#[derive(Default)]
pub struct LocalTCPBackendPolicies {
	/// Send TLS to the backend.
	#[serde(rename = "backendTLS", default)]
	pub backend_tls: Option<http::backendtls::LocalBackendTLS>,
	/// Tunnel to the backend.
	#[serde(default)]
	pub backend_tunnel: Option<backend::Tunnel>,
}

impl LocalTCPBackendPolicies {
	pub fn translate(self) -> anyhow::Result<Vec<BackendPolicy>> {
		let LocalTCPBackendPolicies {
			backend_tls,
			backend_tunnel,
		} = self;
		let mut pols = vec![];
		if let Some(p) = backend_tls {
			pols.push(BackendPolicy::BackendTLS(p.try_into()?))
		}
		if let Some(p) = backend_tunnel {
			pols.push(BackendPolicy::Tunnel(p))
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
	/// Settings for request access logs.
	#[serde(default, alias = "logging")]
	pub access_log: Option<frontend::LoggingPolicy>,
	#[serde(default)]
	pub tracing: Option<TracingConfig>,
}

#[apply(schema_de!)]
#[derive(Default)]
pub struct FilterOrPolicy {
	// Filters. Keep in sync with RouteFilter
	/// Headers to be modified in the request.
	#[serde(default)]
	request_header_modifier: Option<filters::HeaderModifier>,

	/// Headers to be modified in the response.
	#[serde(default)]
	response_header_modifier: Option<filters::HeaderModifier>,

	/// Directly respond to the request with a redirect.
	#[serde(default)]
	request_redirect: Option<filters::RequestRedirect>,

	/// Modify the URL path or authority.
	#[serde(default)]
	url_rewrite: Option<filters::UrlRewrite>,

	/// Mirror incoming requests to another destination.
	#[serde(default)]
	request_mirror: Option<filters::RequestMirror>,

	/// Directly respond to the request with a static response.
	#[serde(default)]
	direct_response: Option<filters::DirectResponse>,

	/// Handle CORS preflight requests and append configured CORS headers to applicable requests.
	#[serde(default)]
	cors: Option<http::cors::Cors>,

	// Policy
	/// Authorization policies for MCP access.
	#[serde(default)]
	mcp_authorization: Option<McpAuthorization>,
	/// Authorization policies for HTTP access.
	#[serde(default)]
	authorization: Option<Authorization>,
	/// Authentication for MCP clients.
	#[serde(default)]
	mcp_authentication: Option<LocalMcpAuthentication>,
	/// Mark this traffic as A2A to enable A2A processing and telemetry.
	#[serde(default)]
	a2a: Option<A2aPolicy>,
	/// Mark this as LLM traffic to enable LLM processing.
	#[serde(default)]
	ai: Option<llm::Policy>,
	/// Send TLS to the backend.
	#[serde(rename = "backendTLS", default)]
	backend_tls: Option<http::backendtls::LocalBackendTLS>,
	/// Tunnel to the backend.
	#[serde(rename = "backendTunnel", default)]
	backend_tunnel: Option<backend::Tunnel>,
	/// Authenticate to the backend.
	#[serde(default)]
	backend_auth: Option<BackendAuth>,
	/// Rate limit incoming requests. State is kept local.
	#[serde(default)]
	local_rate_limit: Vec<crate::http::localratelimit::RateLimit>,
	/// Rate limit incoming requests. State is managed by a remote server.
	#[serde(default)]
	remote_rate_limit: Option<crate::http::remoteratelimit::RemoteRateLimit>,
	/// Authenticate incoming JWT requests.
	#[serde(default)]
	jwt_auth: Option<crate::http::jwt::LocalJwtConfig>,
	/// Authenticate incoming browser requests with OIDC authorization code flow.
	#[serde(default)]
	oidc: Option<crate::http::oidc::LocalOidcConfig>,
	/// Authenticate incoming requests using Basic Authentication with htpasswd.
	#[serde(default)]
	basic_auth: Option<crate::http::basicauth::LocalBasicAuth>,
	/// Authenticate incoming requests using API Keys
	#[serde(default)]
	api_key: Option<crate::http::apikey::LocalAPIKeys>,
	/// Authenticate incoming requests by calling an external authorization server.
	#[serde(default)]
	ext_authz: Option<crate::http::ext_authz::ExtAuthz>,
	/// Extend agentgateway with an external processor
	#[serde(default)]
	ext_proc: Option<crate::http::ext_proc::ExtProc>,
	/// Modify requests and responses
	#[serde(default)]
	#[serde(deserialize_with = "de_transform")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "Option<http::transformation_cel::LocalTransformationConfig>")
	)]
	transformations: Option<crate::http::transformation_cel::Transformation>,

	/// Handle CSRF protection by validating request origins against configured allowed origins.
	#[serde(default)]
	csrf: Option<http::csrf::Csrf>,

	// TrafficPolicy
	/// Timeout requests that exceed the configured duration.
	#[serde(default)]
	timeout: Option<timeout::Policy>,
	/// Retry matching requests.
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
	let LocalConfig {
		config: _,
		mut frontend_policies,
		binds,
		policies,
		workloads,
		services,
		backends,
		llm,
		mcp,
	} = i;
	merge_deprecated_frontend_policies(config, &mut frontend_policies)?;
	let mut all_policies = vec![];
	let mut all_backends = vec![];
	let mut all_binds = vec![];
	for b in binds {
		let bind_name = strng::format!("bind/{}", b.port);
		let mut ls = ListenerSet::default();
		for (idx, l) in b.listeners.into_iter().enumerate() {
			let (l, pol, backends) = convert_listener(
				client.clone(),
				config,
				idx,
				l,
				bind_name.clone(),
				gateway.clone(),
			)
			.await?;
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
		let res = split_policies(
			client.clone(),
			p.policy,
			config.oidc_cookie_encoder.as_ref(),
		)
		.await?;
		if (res.route_policies.len() + res.backend_policies.len()) != 1 {
			anyhow::bail!("'policies' must contain exactly 1 policy")
		}
		if let Some(TrafficPolicy::Oidc(_)) = res.route_policies.first()
			&& p.phase == PolicyPhase::Gateway
		{
			anyhow::bail!("oidc policies must run at route phase");
		}
		let tp = res
			.route_policies
			.first()
			.map(|r| PolicyType::from((r.clone(), p.phase)))
			.unwrap_or_else(|| res.backend_policies.first().unwrap().clone().into());
		let tgt_policy = TargetedPolicy {
			name: Some(TypedResourceName {
				kind: strng::literal!("Local"),
				name: p.name.name.clone(),
				namespace: p.name.namespace.clone(),
			}),
			key: p.name.to_string().into(),
			target: p.target,
			policy: tp,
		};
		all_policies.push(tgt_policy);
	}

	for b in backends {
		let policies = b
			.policies
			.map(|p| p.translate())
			.transpose()?
			.unwrap_or_default();
		all_backends.push(BackendWithPolicies {
			backend: Backend::Opaque(local_name(b.name), b.host),
			inline_policies: policies,
		})
	}

	// Convert llm config if present
	if let Some(llm_config) = llm {
		let (llm_bind, llm_policies, llm_backends) =
			convert_llm_config(client.clone(), config, gateway.clone(), llm_config).await?;
		all_binds.push(llm_bind);
		all_policies.extend_from_slice(&llm_policies);
		all_backends.extend_from_slice(&llm_backends);
	}
	if let Some(mcp_config) = mcp {
		let (mcp_bind, mcp_policies, mcp_backends) =
			convert_mcp_config(client.clone(), config, gateway.clone(), mcp_config).await?;
		all_binds.push(mcp_bind);
		all_policies.extend_from_slice(&mcp_policies);
		all_backends.extend_from_slice(&mcp_backends);
	}

	// Add frontend policies targeted to this listener
	all_policies.extend_from_slice(&split_frontend_policies(gateway, frontend_policies).await?);

	let mut normalized = NormalizedLocalConfig {
		binds: all_binds,
		policies: all_policies,
		backends: all_backends.into_iter().collect(),
		workloads,
		services,
	};
	finalize_oidc_normalized_config(config, &mut normalized)?;
	Ok(normalized)
}

static STARTUP_TIMESTAMP: OnceLock<u64> = OnceLock::new();

fn llm_model_name_header_match(model_name: &str) -> anyhow::Result<HeaderValueMatch> {
	let wildcard_count = model_name.matches('*').count();
	if wildcard_count > 1 {
		bail!("model name '{model_name}' may only include a single '*' wildcard");
	}

	if wildcard_count == 0 {
		return Ok(HeaderValueMatch::Exact(::http::HeaderValue::from_str(
			model_name,
		)?));
	}

	if model_name == "*" {
		return Ok(HeaderValueMatch::Regex(regex::Regex::new(r".*")?));
	}

	if let Some(suffix) = model_name.strip_prefix('*') {
		let pattern = format!(".*{}", regex::escape(suffix));
		return Ok(HeaderValueMatch::Regex(regex::Regex::new(&pattern)?));
	}

	if let Some(prefix) = model_name.strip_suffix('*') {
		let pattern = format!("{}.*", regex::escape(prefix));
		return Ok(HeaderValueMatch::Regex(regex::Regex::new(&pattern)?));
	}

	bail!("model name wildcard must be either at the beginning or the end: '{model_name}'")
}

async fn convert_llm_config(
	client: client::Client,
	config: &crate::Config,
	gateway: ListenerTarget,
	llm_config: LocalLLMConfig,
) -> anyhow::Result<(Bind, Vec<TargetedPolicy>, Vec<BackendWithPolicies>)> {
	const DEFAULT_LLM_PORT: u16 = 4000;
	let LocalLLMConfig {
		port,
		models,
		policies,
	} = llm_config;
	let port = port.unwrap_or(DEFAULT_LLM_PORT);

	let mut all_policies = vec![];
	let mut all_backends = vec![];
	let mut routes = RouteSet::default();
	let (listener_gateway_policies, listener_route_policies) = if let Some(pol) = policies {
		let LocalLLMPolicy {
			gateway,
			authorization,
		} = pol;
		let authorization_policies = split_policies(
			client.clone(),
			FilterOrPolicy {
				authorization,
				..Default::default()
			},
			config.oidc_cookie_encoder.as_ref(),
		)
		.await?;
		let gateway_policies = split_policies(
			client.clone(),
			gateway.into(),
			config.oidc_cookie_encoder.as_ref(),
		)
		.await?;
		(
			gateway_policies.route_policies,
			authorization_policies.route_policies,
		)
	} else {
		(vec![], vec![])
	};

	// Create transformation policy to set x-gateway-model-name header from request body
	let transformation = http::transformation_cel::Transformation::try_from_local_config(
		LocalTransformationConfig {
			request: Some(http::transformation_cel::LocalTransform {
				metadata: Vec::new(),
				set: vec![
					(
						strng::new("x-gateway-model-name"),
						strng::new(
							r#"
request.path.endsWith(":streamRawPredict") || request.path.endsWith(":rawPredict") ?
request.path.regexReplace("^.*/publishers/anthropic/models/(.+?):.*", "${1}") :
json(request.body).model
"#,
						),
					),
					(
						strng::new("anthropic-beta"),
						strng::new("request.headers['anthropic-beta'].split(',').filter(v, v.trim() in [])"),
					),
				],
				add: vec![],
				remove: vec![],
				body: None,
			}),
			response: None,
		},
		false,
	)?;

	// Get static startup unix timestamp
	let startup_timestamp = *STARTUP_TIMESTAMP.get_or_init(|| {
		SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs()
	});

	// Create model list route
	let model_list_body = serde_json::json!({
		"data": models
			.iter()
			.map(|m| serde_json::json!({
				"id": m.name,
				"object": "model",
				"created": startup_timestamp,
				"owned_by": "openai"
			}))
			.collect::<Vec<_>>(),
		"object": "list"
	})
	.to_string();

	let model_list_route = Route {
		key: strng::new("llm:admin:model-list"),
		service_key: None,
		name: RouteName {
			name: strng::new("admin:model-list"),
			namespace: strng::new("internal"),
			kind: None,
			rule_name: None,
		},
		hostnames: vec![],
		matches: vec![
			RouteMatch {
				path: PathMatch::PathPrefix(strng::new("/v1/models")),
				headers: vec![],
				method: None,
				query: vec![],
			},
			RouteMatch {
				path: PathMatch::PathPrefix(strng::new("/models")),
				headers: vec![],
				method: None,
				query: vec![],
			},
		],
		backends: vec![],
		inline_policies: vec![
			TrafficPolicy::ResponseHeaderModifier(crate::http::filters::HeaderModifier {
				set: vec![(strng::new("Content-Type"), strng::new("application/json"))],
				add: vec![],
				remove: vec![],
			}),
			TrafficPolicy::DirectResponse(filters::DirectResponse {
				body: Bytes::copy_from_slice(model_list_body.as_bytes()),
				status: ::http::StatusCode::OK,
			}),
		],
	};
	routes.insert(model_list_route);

	// Create routes and backends for each model
	for (idx, model_config) in models.iter().enumerate() {
		let model_name = strng::new(&model_config.name);
		// Index is needed because the same name can be used with different match criteria
		let backend_key = strng::format!("llm:model:{}:{idx}", model_config.name);
		let p = model_config.params.clone();
		let model = p.model;

		// Use provider from config and set the model name
		let provider = match &model_config.provider {
			LocalModelAIProvider::Anthropic => AIProvider::Anthropic(anthropic::Provider { model }),
			LocalModelAIProvider::OpenAI => AIProvider::OpenAI(openai::Provider { model }),
			LocalModelAIProvider::Gemini => AIProvider::Gemini(crate::llm::gemini::Provider { model }),
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
			}),
			LocalModelAIProvider::AzureOpenAI => {
				AIProvider::AzureOpenAI(crate::llm::azureopenai::Provider {
					model,
					host: p.azure_host.context("azure requires azure_host")?,
					api_version: p.azure_api_version,
					cached_cred: Default::default(),
				})
			},
		};

		// Create backend auth policy
		let mut pols = vec![];
		if let Some(key) = p.api_key.as_ref() {
			let backend_auth = BackendAuth::Key(key.0.clone());
			pols.push(BackendPolicy::BackendAuth(backend_auth));
		}

		// Create AI backend
		let named_provider = NamedAIProvider {
			name: model_name.clone(),
			provider,
			host_override: p.host_override,
			path_override: p.path_override,
			path_prefix: p.path_prefix,
			tokenize: p.tokenize,
			inline_policies: pols,
		};

		let ai_backend = AIBackend {
			providers: crate::types::loadbalancer::EndpointSet::new(vec![vec![(
				model_name.clone(),
				named_provider,
			)]]),
		};

		let mut pols = vec![];
		if let Some(p) = model_config.backend_tls.clone() {
			pols.push(BackendPolicy::BackendTLS(p.try_into()?));
		}
		if let Some(p) = model_config.backend_tunnel.clone() {
			pols.push(BackendPolicy::Tunnel(p));
		}
		if let Some(mut rh) = model_config.request_headers.clone() {
			rh.remove.push(strng::literal!("x-gateway-model-name"));
			pols.push(BackendPolicy::RequestHeaderModifier(rh));
		} else {
			pols.push(BackendPolicy::RequestHeaderModifier(HeaderModifier {
				remove: vec![strng::literal!("x-gateway-model-name")],
				add: vec![],
				set: vec![],
			}));
		}
		if let Some(rh) = model_config.response_headers.clone() {
			pols.push(BackendPolicy::ResponseHeaderModifier(rh));
		}
		if let Some(p) = model_config.health.clone() {
			pols.push(BackendPolicy::Health(p.try_into().map_err(
				|e: crate::cel::Error| anyhow::anyhow!("health.unhealthyExpression: {}", e),
			)?));
		}
		pols.push(BackendPolicy::AI(Arc::new(llm::Policy {
			defaults: model_config.defaults.clone(),
			overrides: model_config.overrides.clone(),
			transformations: model_config.transformation.clone(),
			prompt_guard: model_config.guardrails.clone(),
			prompts: None,
			model_aliases: Default::default(),
			wildcard_patterns: Arc::new(vec![]),
			prompt_caching: None,
			routes: Default::default(),
		})));
		let backend_with_policies = BackendWithPolicies {
			backend: Backend::AI(local_name(backend_key.clone()), ai_backend),
			inline_policies: pols,
		};
		all_backends.push(backend_with_policies);

		// Create route for this model
		// Index is needed because the same name can be used with different match criteria
		// Important: index is before model, to ensure we rank ties by order of first-in-list
		// This ensures if I have '*' and 'foo/*', I can prefer `foo/*` by making it first.
		// TODO: should we automatically make more explicit prefixes higher ranked?
		// 999999 routes ought to be enough for anyone.
		let route_key = strng::format!("llm:model:{idx:06}:{}", model_config.name);
		let user_matches = if model_config.matches.is_empty() {
			vec![RouteMatch {
				path: PathMatch::PathPrefix(strng::new("/")),
				method: None,
				headers: vec![],
				query: vec![],
			}]
		} else {
			model_config
				.matches
				.iter()
				.map(|m| RouteMatch {
					headers: m.headers.clone(),
					path: PathMatch::PathPrefix(strng::new("/")),
					method: None,
					query: vec![],
				})
				.collect_vec()
		};
		let matches = user_matches
			.into_iter()
			.map(|mut m| {
				let header_match = HeaderMatch {
					name: HeaderOrPseudo::Header(HeaderName::from_static("x-gateway-model-name")),
					value: llm_model_name_header_match(&model_config.name)?,
				};
				m.headers.push(header_match);
				Ok(m)
			})
			.collect::<anyhow::Result<Vec<_>>>()?;

		let model_route = Route {
			key: route_key.clone(),
			service_key: None,
			name: RouteName {
				name: strng::format!("model:{}", model_config.name),
				namespace: strng::new("internal"),
				rule_name: None,
				kind: None,
			},
			hostnames: vec![],
			matches,
			backends: vec![RouteBackendReference {
				weight: 1,
				backend: BackendReference::Backend(strng::format!("/{}", backend_key)),
				inline_policies: vec![],
			}],
			inline_policies: vec![TrafficPolicy::AI(Arc::new(crate::llm::Policy {
				routes: [
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
						strng::new("/v1/responses/compact"),
						crate::llm::RouteType::Detect,
					),
					(
						strng::new("/v1/embeddings"),
						crate::llm::RouteType::Embeddings,
					),
					(strng::new("*"), crate::llm::RouteType::Passthrough),
				]
				.into_iter()
				.collect(),
				..Default::default()
			}))],
		};
		routes.insert(model_route);
	}

	// Create listener
	let listener_key: ListenerKey = strng::new("llm");
	let listener_name = ListenerName {
		gateway_name: gateway.gateway_name.clone(),
		gateway_namespace: gateway.gateway_namespace.clone(),
		listener_name: strng::new("llm"),
		listener_set: None,
	};
	let listener = Listener {
		key: listener_key.clone(),
		name: listener_name.clone(),
		hostname: strng::new("*"),
		protocol: ListenerProtocol::HTTP,
		routes,
		tcp_routes: Default::default(),
	};

	if !listener_gateway_policies.is_empty() || !listener_route_policies.is_empty() {
		let pc = listener_gateway_policies.len();
		for (idx, pol) in listener_gateway_policies.into_iter().enumerate() {
			let key = strng::format!("listener/{idx}");
			all_policies.push(TargetedPolicy {
				key: key.clone(),
				name: None,
				target: PolicyTarget::Gateway(listener_name.clone().into()),
				policy: (pol, PolicyPhase::Gateway).into(),
			})
		}
		for (idx, pol) in listener_route_policies.into_iter().enumerate() {
			let key = strng::format!("listener/{}", pc + idx);
			all_policies.push(TargetedPolicy {
				key: key.clone(),
				name: None,
				target: PolicyTarget::Gateway(listener_name.clone().into()),
				policy: (pol, PolicyPhase::Route).into(),
			})
		}
	}

	// Create transformation policy for the listener
	let listener_target: ListenerTarget = listener_name.clone().into();
	let transformation_policy = TargetedPolicy {
		name: Some(TypedResourceName {
			kind: strng::literal!("Local"),
			name: strng::new("llm:transformation"),
			namespace: strng::new("internal"),
		}),
		key: strng::new("llm:transformation"),
		target: PolicyTarget::Gateway(listener_target),
		policy: PolicyType::from((
			TrafficPolicy::Transformation(transformation),
			PolicyPhase::Gateway,
		)),
	};
	all_policies.push(transformation_policy);

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
		protocol: BindProtocol::http,
		listeners: listener_set,
		tunnel_protocol: TunnelProtocol::Direct,
	};

	Ok((bind, all_policies, all_backends))
}

async fn convert_mcp_config(
	client: client::Client,
	config: &crate::Config,
	gateway: ListenerTarget,
	mcp_config: LocalSimpleMcpConfig,
) -> anyhow::Result<(Bind, Vec<TargetedPolicy>, Vec<BackendWithPolicies>)> {
	const DEFAULT_MCP_PORT: u16 = 3000;
	let LocalSimpleMcpConfig {
		port,
		backend,
		policies,
	} = mcp_config;
	let port = port.unwrap_or(DEFAULT_MCP_PORT);

	let resolved_policies = if let Some(pol) = policies {
		split_policies(client.clone(), pol, config.oidc_cookie_encoder.as_ref()).await?
	} else {
		ResolvedPolicies::default()
	};

	let mut routes = RouteSet::default();
	let route = Route {
		key: strng::new("mcp:default"),
		service_key: None,
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
			backend: BackendReference::Backend(strng::new("/mcp")),
			inline_policies: resolved_policies.backend_policies,
		}],
		inline_policies: resolved_policies.route_policies,
	};
	routes.insert(route);

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
		routes,
		tcp_routes: Default::default(),
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
		.as_backends(local_name(strng::new("mcp")), client)
		.await?;

	Ok((bind, vec![], backends))
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
) -> anyhow::Result<(Listener, Vec<TargetedPolicy>, Vec<BackendWithPolicies>)> {
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
					.try_into()?,
			)
		},
		LocalListenerProtocol::TLS => {
			if tcp_routes.is_none() {
				bail!("protocol TLS requires 'tcpRoutes'")
			}
			ListenerProtocol::TLS(tls.map(TryInto::try_into).transpose()?)
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

	let mut rs = RouteSet::default();
	for (idx, l) in routes.into_iter().flatten().enumerate() {
		let (route, backends) = convert_route(client.clone(), config, l, idx, key.clone()).await?;
		all_backends.extend_from_slice(&backends);
		rs.insert(route)
	}

	let mut trs = TCPRouteSet::default();
	for (idx, l) in tcp_routes.into_iter().flatten().enumerate() {
		let (route, policies, backends) = convert_tcp_route(l, idx, key.clone()).await?;
		all_policies.extend_from_slice(&policies);
		all_backends.extend_from_slice(&backends);
		trs.insert(route)
	}

	if let Some(pol) = policies {
		let pols = split_policies(
			client.clone(),
			pol.into(),
			config.oidc_cookie_encoder.as_ref(),
		)
		.await?;
		for (idx, pol) in pols.route_policies.into_iter().enumerate() {
			let key = strng::format!("listener/{key}/{idx}");
			let phase = if matches!(pol, TrafficPolicy::Oidc(_)) {
				PolicyPhase::Route
			} else {
				PolicyPhase::Gateway
			};
			all_policies.push(TargetedPolicy {
				key: key.clone(),
				name: None,
				target: PolicyTarget::Gateway(name.clone().into()),
				policy: (pol, phase).into(),
			})
		}
	}

	let l = Listener {
		key,
		name,
		hostname,
		protocol,
		routes: rs,
		tcp_routes: trs,
	};
	Ok((l, all_policies, all_backends))
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
		let backend_key = strng::format!("{key}/backend{idx}");
		let policies = b
			.policies
			.clone()
			.map(|p| p.translate())
			.transpose()?
			.unwrap_or_default();
		let be_name = local_name(backend_key.clone());
		let bref = match &b.backend {
			LocalBackend::Service { name, port } => BackendReference::Service {
				name: name.clone(),
				port: *port,
			},
			LocalBackend::Backend(n) => BackendReference::Backend(n.clone()),
			LocalBackend::Invalid => BackendReference::Invalid,
			LocalBackend::Dynamic {} => BackendReference::Backend("dynamic".into()),
			_ => BackendReference::Backend(strng::format!("/{}", backend_key)),
		};
		let backends = b
			.backend
			.as_backends(be_name.clone(), client.clone())
			.await?;
		let bref = RouteBackendReference {
			weight: b.weight,
			backend: bref,
			inline_policies: policies,
		};
		backend_refs.push(bref);
		external_backends.extend_from_slice(&backends);
	}
	let resolved = if let Some(pol) = policies {
		split_policies(client, pol, config.oidc_cookie_encoder.as_ref()).await?
	} else {
		ResolvedPolicies::default()
	};
	for br in backend_refs.iter_mut() {
		br.inline_policies
			.extend_from_slice(&resolved.backend_policies);
	}
	let route = Route {
		key,
		service_key: None,
		name: RouteName {
			name: route_name,
			namespace,
			rule_name: None,
			kind: None,
		},
		hostnames,
		matches,
		backends: backend_refs,
		inline_policies: resolved.route_policies,
	};
	Ok((route, external_backends))
}

#[derive(Default)]
pub struct ResolvedPolicies {
	pub backend_policies: Vec<BackendPolicy>,
	pub route_policies: Vec<TrafficPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum OidcPolicySource {
	Inline { route_key: RouteKey },
	Targeted { policy_key: PolicyKey },
}

fn finalize_oidc_normalized_config(
	config: &crate::Config,
	normalized: &mut NormalizedLocalConfig,
) -> anyhow::Result<()> {
	let stores = build_oidc_validation_stores(config, normalized);
	let mut assignments = HashMap::new();
	for bind in normalized.binds.iter() {
		for listener in bind.listeners.iter() {
			for route in listener.routes.iter() {
				let path = crate::store::RoutePath {
					listener: &listener.name,
					route: &route.name,
				};
				let policies = stores
					.read_binds()
					.route_policies(&path, &route.inline_policies);
				if let Some(policy) = &policies.oidc {
					let route_name = route.name.as_route_name();
					let source = stores
						.read_binds()
						.route_oidc_source(&path, &route.inline_policies)
						.ok_or_else(|| {
							anyhow!(
								"internal error: route '{}' resolved to oidc without an identifiable source",
								route_name
							)
						})?;
					let callback_path = policy.redirect_uri.callback_path.path();
					let req = oidc_callback_validation_request(policy)?;
					let Some((selected_route, path_match)) = crate::http::route::select_best_route(
						stores.clone(),
						config.network.clone(),
						config.self_addr.as_ref(),
						bind.address,
						listener,
						&req,
					) else {
						anyhow::bail!(
							"listener '{}' route '{}' resolves to an oidc policy with callback path '{}', but no route can own that callback",
							listener.name.listener_name,
							route_name,
							callback_path
						);
					};

					let selected_path_is_exact =
						matches!(&path_match, PathMatch::Exact(path) if path.as_str() == callback_path);
					let selected_route_is_explicit_owner = selected_route
						.matches
						.iter()
						.any(|m| route_match_is_explicit_oidc_callback_owner(m, callback_path));
					anyhow::ensure!(
						selected_path_is_exact && selected_route_is_explicit_owner,
						"listener '{}' route '{}' resolves to an oidc policy with callback path '{}', but callback ownership requires an exact callback route without header or query constraints; prefix and regex routes do not count",
						listener.name.listener_name,
						route_name,
						callback_path
					);

					let selected_route_policies = stores.read_binds().route_policies(
						&crate::store::RoutePath {
							listener: &listener.name,
							route: &selected_route.name,
						},
						&selected_route.inline_policies,
					);
					let callback_has_competing_authn = selected_route_policies.oidc.is_some()
						&& [
							selected_route_policies.jwt.is_some(),
							selected_route_policies.basic_auth.is_some(),
							selected_route_policies.api_key.is_some(),
						]
						.into_iter()
						.any(|present| present);
					anyhow::ensure!(
						!callback_has_competing_authn,
						"listener '{}' route '{}' resolves to an oidc policy with callback path '{}', but route '{}' wins callback selection with multiple authentication mechanisms; oidc callback ownership must be unambiguous",
						listener.name.listener_name,
						route_name,
						callback_path,
						selected_route.name.as_route_name()
					);
					match &selected_route_policies.oidc {
						Some(selected_policy) if selected_policy.policy_id == policy.policy_id => {},
						Some(_) => {
							anyhow::bail!(
								"listener '{}' route '{}' resolves to an oidc policy with callback path '{}', but route '{}' wins callback selection with a different effective oidc policy",
								listener.name.listener_name,
								route_name,
								callback_path,
								selected_route.name.as_route_name()
							);
						},
						None => {
							anyhow::bail!(
								"listener '{}' route '{}' resolves to an oidc policy with callback path '{}', but route '{}' wins callback selection without oidc",
								listener.name.listener_name,
								route_name,
								callback_path,
								selected_route.name.as_route_name()
							);
						},
					}

					let source = oidc_policy_source_for_assignment(route, source);
					let policy_id = stable_oidc_policy_id(&source, &selected_route);
					match assignments.entry(source) {
						std::collections::hash_map::Entry::Occupied(existing) => {
							anyhow::ensure!(
								existing.get() == &policy_id,
								"internal error: conflicting normalized oidc identity assignment"
							);
						},
						std::collections::hash_map::Entry::Vacant(entry) => {
							entry.insert(policy_id);
						},
					}
				}
			}
		}
	}

	apply_oidc_policy_assignments(normalized, assignments)?;
	Ok(())
}

fn oidc_policy_source_for_assignment(
	route: &Arc<Route>,
	source: crate::store::RouteOidcSource,
) -> OidcPolicySource {
	match source {
		crate::store::RouteOidcSource::Inline => OidcPolicySource::Inline {
			route_key: route.key.clone(),
		},
		crate::store::RouteOidcSource::Targeted(policy_key) => {
			OidcPolicySource::Targeted { policy_key }
		},
	}
}

fn stable_oidc_policy_id(
	source: &OidcPolicySource,
	callback_owner: &Route,
) -> crate::http::oidc::PolicyId {
	match source {
		OidcPolicySource::Inline { .. } => {
			crate::http::oidc::PolicyId::from(format!("route/{}", callback_owner.key))
		},
		OidcPolicySource::Targeted { policy_key } => {
			crate::http::oidc::PolicyId::from(format!("policy/{policy_key}"))
		},
	}
}

fn apply_oidc_policy_assignments(
	normalized: &mut NormalizedLocalConfig,
	assignments: HashMap<OidcPolicySource, crate::http::oidc::PolicyId>,
) -> anyhow::Result<()> {
	for (source, policy_id) in assignments {
		match source {
			OidcPolicySource::Inline { route_key } => {
				assign_inline_oidc_policy_id(normalized, &route_key, policy_id)?;
			},
			OidcPolicySource::Targeted { policy_key } => {
				assign_targeted_oidc_policy_id(normalized, &policy_key, policy_id)?;
			},
		}
	}
	Ok(())
}

fn assign_inline_oidc_policy_id(
	normalized: &mut NormalizedLocalConfig,
	route_key: &RouteKey,
	policy_id: crate::http::oidc::PolicyId,
) -> anyhow::Result<()> {
	for bind in normalized.binds.iter_mut() {
		let listener_keys = bind.listeners.inner.keys().cloned().collect_vec();
		for listener_key in listener_keys {
			let Some(listener) = bind.listeners.remove(&listener_key) else {
				continue;
			};
			let mut listener = Arc::unwrap_or_clone(listener);
			let Some(route) = listener.routes.get(route_key) else {
				bind.listeners.insert(listener);
				continue;
			};
			let mut route = Arc::unwrap_or_clone(route);
			assign_oidc_policy_id_in_route_policies(&mut route.inline_policies, policy_id)?;
			listener.routes.insert(route);
			bind.listeners.insert(listener);
			return Ok(());
		}
	}

	anyhow::bail!(
		"internal error: failed to locate inline oidc policy source '{}'",
		route_key
	);
}

fn assign_targeted_oidc_policy_id(
	normalized: &mut NormalizedLocalConfig,
	policy_key: &PolicyKey,
	policy_id: crate::http::oidc::PolicyId,
) -> anyhow::Result<()> {
	for policy in normalized.policies.iter_mut() {
		if &policy.key != policy_key {
			continue;
		}
		let PolicyType::Traffic(phased) = &mut policy.policy else {
			anyhow::bail!(
				"internal error: targeted oidc policy source '{}' is not a traffic policy",
				policy_key
			);
		};
		anyhow::ensure!(
			phased.phase == PolicyPhase::Route,
			"internal error: targeted oidc policy source '{}' is not route phase",
			policy_key
		);
		let TrafficPolicy::Oidc(oidc) = &mut phased.policy else {
			anyhow::bail!(
				"internal error: targeted oidc policy source '{}' is not oidc",
				policy_key
			);
		};
		oidc.assign_policy_id(policy_id);
		return Ok(());
	}

	anyhow::bail!(
		"internal error: failed to locate targeted oidc policy source '{}'",
		policy_key
	);
}

fn assign_oidc_policy_id_in_route_policies(
	policies: &mut [TrafficPolicy],
	policy_id: crate::http::oidc::PolicyId,
) -> anyhow::Result<()> {
	for policy in policies.iter_mut() {
		if let TrafficPolicy::Oidc(oidc) = policy {
			oidc.assign_policy_id(policy_id);
			return Ok(());
		}
	}

	anyhow::bail!("internal error: route oidc source no longer contains oidc");
}

fn route_match_is_explicit_oidc_callback_owner(
	route_match: &RouteMatch,
	callback_path: &str,
) -> bool {
	if route_match
		.method
		.as_ref()
		.is_some_and(|m| m.method.as_str() != "GET")
	{
		return false;
	}
	if !route_match.headers.is_empty() || !route_match.query.is_empty() {
		return false;
	}

	matches!(&route_match.path, PathMatch::Exact(path) if path.as_str() == callback_path)
}

fn oidc_callback_validation_request(
	policy: &crate::http::oidc::OidcPolicy,
) -> anyhow::Result<crate::http::Request> {
	let uri = format!(
		"{}?code=validation-code&state=validation-state",
		policy.redirect_uri.canonical_uri()
	);
	::http::Request::builder()
		.method(::http::Method::GET)
		.uri(uri)
		.body(crate::http::Body::empty())
		.map_err(Into::into)
}

fn build_oidc_validation_stores(
	config: &crate::Config,
	normalized: &NormalizedLocalConfig,
) -> crate::store::Stores {
	let stores = crate::store::Stores::with_ipv6_enabled(config.ipv6_enabled);
	{
		let mut binds = stores.binds.write();
		for bind in normalized.binds.iter().cloned() {
			binds.insert_bind(bind);
		}
		for backend in normalized.backends.iter().cloned() {
			binds.insert_backend(backend.backend.name(), backend);
		}
		for policy in normalized.policies.iter().cloned() {
			binds.insert_policy(policy);
		}
	}
	stores
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
			policy: p.into(),
		});
	};
	let LocalFrontendPolicies {
		http,
		tls,
		tcp,
		network_authorization,
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
pub async fn split_policies(
	client: Client,
	pol: FilterOrPolicy,
	oidc_cookie_encoder: Option<&crate::http::sessionpersistence::Encoder>,
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
		oidc,
		basic_auth,
		api_key,
		transformations,
		csrf,
		ext_authz,
		ext_proc,
		timeout,
		retry,
	} = pol;
	if let Some(p) = request_header_modifier {
		route_policies.push(TrafficPolicy::RequestHeaderModifier(p));
	}
	if let Some(p) = response_header_modifier {
		route_policies.push(TrafficPolicy::ResponseHeaderModifier(p));
	}
	if let Some(p) = request_redirect {
		route_policies.push(TrafficPolicy::RequestRedirect(p));
	}
	if let Some(p) = url_rewrite {
		route_policies.push(TrafficPolicy::UrlRewrite(p));
	}
	if let Some(p) = request_mirror {
		route_policies.push(TrafficPolicy::RequestMirror(vec![p]));
	}

	// Filters
	if let Some(p) = direct_response {
		route_policies.push(TrafficPolicy::DirectResponse(p));
	}
	if let Some(p) = cors {
		route_policies.push(TrafficPolicy::CORS(p));
	}

	// Backend policies
	if let Some(p) = mcp_authorization {
		backend_policies.push(BackendPolicy::McpAuthorization(p))
	}
	if let Some(p) = mcp_authentication {
		// Translate local MCP authn into runtime authn with a ready JWT validator.
		let authn: McpAuthentication = p.translate(client.clone()).await?;
		backend_policies.push(BackendPolicy::McpAuthentication(authn));
		// Do NOT inject a separate route-level JwtAuth; MCP router handles validation using jwt_validator.
	}
	if let Some(p) = a2a {
		backend_policies.push(BackendPolicy::A2a(p))
	}
	if let Some(p) = backend_tls {
		backend_policies.push(BackendPolicy::BackendTLS(p.try_into()?))
	}
	if let Some(p) = backend_tunnel {
		backend_policies.push(BackendPolicy::Tunnel(p))
	}
	if let Some(p) = backend_auth {
		backend_policies.push(BackendPolicy::BackendAuth(p))
	}

	// Route policies
	if let Some(mut p) = ai {
		p.compile_model_alias_patterns();
		route_policies.push(TrafficPolicy::AI(Arc::new(p)))
	}
	if let Some(p) = jwt_auth {
		route_policies.push(TrafficPolicy::JwtAuth(p.try_into(client.clone()).await?));
	}
	if let Some(p) = oidc {
		let Some(oidc_cookie_encoder) = oidc_cookie_encoder else {
			return Err(Error::msg(
				"OIDC_COOKIE_SECRET is required when oidc is configured",
			));
		};
		route_policies.push(TrafficPolicy::Oidc(
			p.translate(client.clone(), oidc_cookie_encoder).await?,
		));
	}
	if let Some(p) = basic_auth {
		route_policies.push(TrafficPolicy::BasicAuth(p.try_into()?));
	}
	if let Some(p) = api_key {
		route_policies.push(TrafficPolicy::APIKey(p.into()));
	}
	if let Some(p) = transformations {
		route_policies.push(TrafficPolicy::Transformation(p));
	}
	if let Some(p) = csrf {
		route_policies.push(TrafficPolicy::Csrf(p))
	}
	if let Some(p) = authorization {
		route_policies.push(TrafficPolicy::Authorization(p))
	}
	if let Some(p) = ext_authz {
		route_policies.push(TrafficPolicy::ExtAuthz(p))
	}
	if let Some(p) = ext_proc {
		route_policies.push(TrafficPolicy::ExtProc(p))
	}
	if !local_rate_limit.is_empty() {
		route_policies.push(TrafficPolicy::LocalRateLimit(local_rate_limit))
	}
	if let Some(p) = remote_rate_limit {
		route_policies.push(TrafficPolicy::RemoteRateLimit(p))
	}

	// Traffic policies
	if let Some(p) = timeout {
		route_policies.push(TrafficPolicy::Timeout(p));
	}
	if let Some(p) = retry {
		route_policies.push(TrafficPolicy::Retry(p));
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
					.push(BackendPolicy::BackendTLS(p.clone().try_into()?));
			}
		}
	}
	let route = TCPRoute {
		key,
		service_key: None,
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
		let cert_pem = fs_err::read(self.cert)?;
		let key_pem = fs_err::read(self.key)?;
		let root_pem = self.root.map(fs_err::read).transpose()?;
		ServerTLSConfig::from_pem_with_profile(
			cert_pem,
			key_pem,
			root_pem,
			vec![b"h2".to_vec(), b"http/1.1".to_vec()],
			self.min_tls_version.map(Into::into),
			self.max_tls_version.map(Into::into),
			self.cipher_suites,
			false,
		)
	}
}

pub fn local_name(name: Strng) -> ResourceName {
	ResourceName::new(name, "".into())
}

pub fn de_from_local_backend_policy<'de: 'a, 'a, D>(
	deserializer: D,
) -> Result<Vec<BackendPolicy>, D::Error>
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
