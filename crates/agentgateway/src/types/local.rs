use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::client::Client;
use crate::http::auth::BackendAuth;
use crate::http::backendtls::LocalBackendTLS;
use crate::http::filters::HeaderModifier;
use crate::http::transformation_cel::LocalTransformationConfig;
use crate::http::{HeaderName, HeaderOrPseudo, filters, retry, timeout};
use crate::llm::policy::PromptGuard;
use crate::llm::{AIBackend, AIProvider, LocalModelAIProvider, NamedAIProvider};
use crate::llm::{anthropic, openai};
use crate::mcp::McpAuthorization;
use crate::store::LocalWorkload;
use crate::types::agent::{
	A2aPolicy, Authorization, Backend, BackendKey, BackendPolicy, BackendReference,
	BackendWithPolicies, Bind, BindProtocol, FrontendPolicy, HeaderMatch, HeaderValueMatch, Listener,
	ListenerKey, ListenerName, ListenerProtocol, ListenerSet, ListenerTarget, LocalMcpAuthentication,
	McpAuthentication, McpBackend, McpTarget, McpTargetName, McpTargetSpec, OpenAPITarget, PathMatch,
	PolicyPhase, PolicyTarget, PolicyType, ResourceName, Route, RouteBackendReference, RouteMatch,
	RouteName, RouteSet, ServerTLSConfig, SimpleBackend, SimpleBackendReference,
	SimpleBackendWithPolicies, SseTargetSpec, StreamableHTTPTargetSpec, TCPRoute,
	TCPRouteBackendReference, TCPRouteSet, Target, TargetedPolicy, TracingConfig, TrafficPolicy,
	TunnelProtocol, TypedResourceName,
};
use crate::types::discovery::{NamespacedHostname, Service};
use crate::types::{backend, frontend};
use crate::*;
use ::http::Uri;
use agent_core::prelude::Strng;
use anyhow::{Error, anyhow, bail};
use bytes::Bytes;
use itertools::Itertools;
use macro_rules_attribute::apply;
use openapiv3::OpenAPI;
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
	api_key: Option<String>,
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
struct LocalRoute {
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
	name: BackendKey,
	host: Target,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	policies: Option<LocalBackendPolicies>,
}

#[apply(schema_de!)]
#[allow(clippy::large_enum_variant)] // Size is not sensitive for local config
pub enum LocalBackend {
	// This one is a reference
	Service {
		name: NamespacedHostname,
		port: u16,
	},
	// Rest are inlined
	#[serde(rename = "host")]
	Opaque(Target), // Hostname or IP
	Dynamic {},
	#[serde(rename = "mcp")]
	MCP(LocalMcpBackend),
	#[serde(rename = "ai")]
	AI(LocalAIBackend),
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
	pub host_override: Option<Target>,
	pub path_override: Option<Strng>,
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
	fn make_backend(
		b: Backend,
		policies: Option<LocalBackendPolicies>,
		tls: bool,
	) -> Result<BackendWithPolicies, anyhow::Error> {
		let mut inline_policies = policies
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

	pub fn as_backends(&self, name: ResourceName) -> anyhow::Result<Vec<BackendWithPolicies>> {
		Ok(match self {
			LocalBackend::Service { .. } => vec![], // These stay as references
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
								backends.push(Self::make_backend(b, t.policies.clone(), tls)?);
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
								backends.push(Self::make_backend(b, t.policies.clone(), tls)?);
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
								backends.push(Self::make_backend(b, t.policies.clone(), tls)?);
							}
							McpTargetSpec::OpenAPI(OpenAPITarget {
								backend: bref,
								schema,
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
				};
				backends.push(Backend::MCP(name, m).into());
				backends
			},
			LocalBackend::AI(tgt) => {
				let be = tgt.clone().translate()?;
				vec![Backend::AI(name, be).into()]
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
}

#[apply(schema_de!)]
pub struct LocalMcpTarget {
	pub name: McpTargetName,
	#[serde(flatten)]
	pub spec: LocalMcpTargetSpec,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub policies: Option<LocalBackendPolicies>,
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
		#[serde(deserialize_with = "types::agent::de_openapi")]
		#[cfg_attr(feature = "schema", schemars(with = "serde_json::value::RawValue"))]
		schema: Arc<OpenAPI>,
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
			jwt_auth,
			ext_authz,
			ext_proc,
			transformations,
			basic_auth,
			api_key,
		} = val;
		FilterOrPolicy {
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
pub struct LocalBackendPolicies {
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

	/// Authorization policies for MCP access.
	#[serde(default)]
	pub mcp_authorization: Option<McpAuthorization>,
	/// Mark this traffic as A2A to enable A2A processing and telemetry.
	#[serde(default)]
	pub a2a: Option<A2aPolicy>,
	/// Mark this as LLM traffic to enable LLM processing.
	#[serde(default)]
	pub ai: Option<llm::Policy>,
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
}

impl LocalBackendPolicies {
	pub fn translate(self) -> anyhow::Result<Vec<BackendPolicy>> {
		let LocalBackendPolicies {
			request_header_modifier,
			response_header_modifier,
			request_redirect,
			mcp_authorization,
			a2a,
			ai,
			backend_tls,
			backend_auth,
			http,
			tcp,
		} = self;
		let mut pols = vec![];
		if let Some(p) = tcp {
			pols.push(BackendPolicy::TCP(p));
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
		Ok(pols)
	}
}

#[apply(schema_de!)]
#[derive(Default)]
pub struct LocalTCPBackendPolicies {
	/// Send TLS to the backend.
	#[serde(rename = "backendTLS", default)]
	pub backend_tls: Option<http::backendtls::LocalBackendTLS>,
}

impl LocalTCPBackendPolicies {
	pub fn translate(self) -> anyhow::Result<Vec<BackendPolicy>> {
		let LocalTCPBackendPolicies { backend_tls } = self;
		let mut pols = vec![];
		if let Some(p) = backend_tls {
			pols.push(BackendPolicy::BackendTLS(p.try_into()?))
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
	/// Settings for request access logs.
	#[serde(default)]
	pub access_log: Option<frontend::LoggingPolicy>,
	#[serde(default)]
	pub tracing: Option<TracingConfig>,
}

#[apply(schema_de!)]
#[derive(Default)]
struct FilterOrPolicy {
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
		frontend_policies,
		binds,
		policies,
		workloads,
		services,
		backends,
		llm,
		mcp,
	} = i;
	let mut all_policies = vec![];
	let mut all_backends = vec![];
	let mut all_binds = vec![];
	for b in binds {
		let bind_name = strng::format!("bind/{}", b.port);
		let mut ls = ListenerSet::default();
		for (idx, l) in b.listeners.into_iter().enumerate() {
			let (l, pol, backends) =
				convert_listener(client.clone(), idx, l, bind_name.clone(), gateway.clone()).await?;
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
		let res = split_policies(client.clone(), p.policy).await?;
		if (res.route_policies.len() + res.backend_policies.len()) != 1 {
			anyhow::bail!("'policies' must contain exactly 1 policy")
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

	Ok(NormalizedLocalConfig {
		binds: all_binds,
		policies: all_policies,
		backends: all_backends.into_iter().collect(),
		workloads,
		services,
	})
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
	let port = llm_config.port.unwrap_or(DEFAULT_LLM_PORT);

	let mut all_policies = vec![];
	let mut all_backends = vec![];
	let mut routes = RouteSet::default();

	// Create transformation policy to set x-gateway-model-name header from request body
	let transformation = http::transformation_cel::Transformation::try_from_local_config(
		LocalTransformationConfig {
			request: Some(http::transformation_cel::LocalTransform {
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
		"data": llm_config
			.models
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
		inline_policies: vec![TrafficPolicy::DirectResponse(filters::DirectResponse {
			body: Bytes::copy_from_slice(model_list_body.as_bytes()),
			status: ::http::StatusCode::OK,
		})],
	};
	routes.insert(model_list_route);

	// Create routes and backends for each model
	for (idx, model_config) in llm_config.models.iter().enumerate() {
		let model_name = strng::new(&model_config.name);
		let backend_key = strng::format!("llm:{}", model_config.name);
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
				})
			},
		};

		// Create backend auth policy
		let mut pols = vec![];
		if let Some(key) = p.api_key.as_ref() {
			let backend_auth = BackendAuth::Key(SecretString::new(key.clone().into_boxed_str()));
			pols.push(BackendPolicy::BackendAuth(backend_auth));
		}

		// Create AI backend
		let named_provider = NamedAIProvider {
			name: model_name.clone(),
			provider,
			host_override: None,
			path_override: None,
			tokenize: false,
			inline_policies: pols,
		};

		let ai_backend = AIBackend {
			providers: crate::types::loadbalancer::EndpointSet::new(vec![vec![(
				model_name.clone(),
				named_provider,
			)]]),
		};

		let mut pols = vec![];
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
		let route_key = strng::format!("llm:model:{}:{idx}", model_config.name);
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

	if let Some(pol) = llm_config.policies {
		let route_pols = split_policies(
			client.clone(),
			FilterOrPolicy {
				authorization: pol.authorization.clone(),
				..Default::default()
			},
		)
		.await?;
		let pols = split_policies(client.clone(), pol.gateway.into()).await?;

		let pc = pols.route_policies.len();
		for (idx, pol) in pols.route_policies.into_iter().enumerate() {
			let key = strng::format!("listener/{idx}");
			all_policies.push(TargetedPolicy {
				key: key.clone(),
				name: None,
				target: PolicyTarget::Gateway(listener_name.clone().into()),
				policy: (pol, PolicyPhase::Gateway).into(),
			})
		}
		for (idx, pol) in route_pols.route_policies.into_iter().enumerate() {
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
		split_policies(client, pol).await?
	} else {
		ResolvedPolicies::default()
	};

	let mut routes = RouteSet::default();
	let route = Route {
		key: strng::new("mcp:default"),
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

	let backends = LocalBackend::MCP(backend).as_backends(local_name(strng::new("mcp")))?;

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
	let key: ListenerKey =
		strng::format!("{gateway_namespace}/{gateway_name}/{bind_key}/{listener_name}");

	let mut all_policies = vec![];
	let mut all_backends = vec![];

	let mut rs = RouteSet::default();
	for (idx, l) in routes.into_iter().flatten().enumerate() {
		let (route, policies, backends) = convert_route(client.clone(), l, idx, key.clone()).await?;
		all_policies.extend_from_slice(&policies);
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

	let name = ListenerName {
		gateway_name,
		gateway_namespace,
		listener_name,
		listener_set: None,
	};

	if let Some(pol) = policies {
		let pols = split_policies(client.clone(), pol.into()).await?;
		for (idx, pol) in pols.route_policies.into_iter().enumerate() {
			let key = strng::format!("listener/{key}/{idx}");
			all_policies.push(TargetedPolicy {
				key: key.clone(),
				name: None,
				target: PolicyTarget::Gateway(name.clone().into()),
				policy: (pol, PolicyPhase::Gateway).into(),
			})
		}
	}

	let l = Listener {
		key,
		name,
		hostname: hostname.unwrap_or_default(),
		protocol,
		routes: rs,
		tcp_routes: trs,
	};
	Ok((l, all_policies, all_backends))
}

async fn convert_route(
	client: client::Client,
	lr: LocalRoute,
	idx: usize,
	listener_key: ListenerKey,
) -> anyhow::Result<(Route, Vec<TargetedPolicy>, Vec<BackendWithPolicies>)> {
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
			LocalBackend::Invalid => BackendReference::Invalid,
			LocalBackend::Dynamic {} => BackendReference::Backend("dynamic".into()),
			_ => BackendReference::Backend(strng::format!("/{}", backend_key)),
		};
		let backends = b.backend.as_backends(be_name.clone())?;
		let bref = RouteBackendReference {
			weight: b.weight,
			backend: bref,
			inline_policies: policies,
		};
		backend_refs.push(bref);
		external_backends.extend_from_slice(&backends);
	}
	let resolved = if let Some(pol) = policies {
		split_policies(client, pol).await?
	} else {
		ResolvedPolicies::default()
	};
	for br in backend_refs.iter_mut() {
		br.inline_policies
			.extend_from_slice(&resolved.backend_policies);
	}
	let route = Route {
		key,
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
	Ok((route, vec![], external_backends))
}

#[derive(Default)]
struct ResolvedPolicies {
	backend_policies: Vec<BackendPolicy>,
	route_policies: Vec<TrafficPolicy>,
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
	if let Some(p) = access_log {
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
async fn split_policies(client: Client, pol: FilterOrPolicy) -> Result<ResolvedPolicies, Error> {
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
		backend_auth,
		authorization,
		local_rate_limit,
		remote_rate_limit,
		jwt_auth,
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
		)
	}
}

fn local_name(name: Strng) -> ResourceName {
	ResourceName::new(name, "".into())
}
