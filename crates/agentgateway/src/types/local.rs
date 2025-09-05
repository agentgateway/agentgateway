use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;

use ::http::Uri;
use agent_core::prelude::Strng;
use anyhow::{Error, anyhow, bail};
use itertools::Itertools;
use macro_rules_attribute::apply;
use openapiv3::OpenAPI;
use rustls::ServerConfig;
use serde_with::{TryFromInto, serde_as};

use crate::client::Client;
use crate::http::auth::BackendAuth;
use crate::http::backendtls::LocalBackendTLS;
use crate::http::ext_proc::FailureMode;
use crate::http::{filters, retry, timeout};
use crate::llm::{AIBackend, AIProvider, NamedAIProvider, RouteType};
use crate::mcp::McpAuthorization;
use crate::store::LocalWorkload;
use crate::types::agent::PolicyTarget::RouteRule;
use crate::types::agent::{
	A2aPolicy, Authorization, Backend, BackendName, BackendReference, Bind, BindName, GatewayName,
	GatewayTargetedPolicy, Listener, ListenerKey, ListenerProtocol, ListenerSet, McpAuthentication,
	McpBackend, McpTarget, McpTargetName, McpTargetSpec, OpenAPITarget, PathMatch, Policy,
	PolicyName, PolicyTarget, Route, RouteBackendReference, RouteFilter, RouteMatch, RouteName,
	RouteRuleName, RouteSet, SimpleBackendReference, SseTargetSpec, StreamableHTTPTargetSpec,
	TCPRoute, TCPRouteBackendReference, TCPRouteSet, TLSConfig, Target, TargetedPolicy,
	TrafficPolicy,
};
use crate::types::discovery::{NamespacedHostname, Service};
use crate::*;

impl NormalizedLocalConfig {
	pub async fn from(client: client::Client, s: &str) -> anyhow::Result<NormalizedLocalConfig> {
		// Avoid shell expanding the comment for schema. Probably there are better ways to do this!
		let s = s.replace("# yaml-language-server: $schema", "#");
		let s = shellexpand::full(&s)?;
		let config: LocalConfig = serdes::yamlviajson::from_str(&s)?;
		let t = convert(client, config).await?;
		Ok(t)
	}
}

#[derive(Debug, Clone)]
pub struct NormalizedLocalConfig {
	pub binds: Vec<Bind>,
	pub policies: Vec<TargetedPolicy>,
	pub gateway_policies: Vec<GatewayTargetedPolicy>,
	pub backends: Vec<Backend>,
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
	config: Arc<Option<serde_json::value::Value>>,
	#[serde(default)]
	binds: Vec<LocalBind>,
	/// policies defines additional policies that can be attached to various other configurations.
	/// This is an advanced feature; users should typically use the inline `policies` field under route.
	#[serde(default)]
	policies: Vec<LocalPolicy>,
	/// gatewayPolicies define policies that run at the Gateway level. This includes a subset of possible
	/// policy types.
	/// In general, normal (route level) policies should be used, except you need the policy to influence
	/// routing.
	#[serde(default)]
	gateway_policies: Vec<LocalGatewayTargetedPolicy>,
	#[serde(default)]
	#[cfg_attr(feature = "schema", schemars(with = "serde_json::value::RawValue"))]
	workloads: Vec<LocalWorkload>,
	#[serde(default)]
	#[cfg_attr(feature = "schema", schemars(with = "serde_json::value::RawValue"))]
	services: Vec<Service>,
}

#[apply(schema_de!)]
struct LocalBind {
	port: u16,
	listeners: Vec<LocalListener>,
}

#[apply(schema_de!)]
struct LocalListener {
	// User facing name
	name: Option<Strng>,
	// User facing name of the Gateway. Option, one will be set if not.
	gateway_name: Option<Strng>,
	/// Can be a wildcard
	hostname: Option<Strng>,
	#[serde(default)]
	protocol: LocalListenerProtocol,
	tls: Option<LocalTLSServerConfig>,
	routes: Option<Vec<LocalRoute>>,
	tcp_routes: Option<Vec<LocalTCPRoute>>,
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
struct LocalTLSServerConfig {
	cert: PathBuf,
	key: PathBuf,
}

#[apply(schema_de!)]
struct LocalRoute {
	#[serde(default, skip_serializing_if = "Option::is_none", rename = "name")]
	// User facing name of the route
	route_name: Option<RouteName>,
	// User facing name of the rule
	#[serde(default, skip_serializing_if = "Option::is_none")]
	rule_name: Option<RouteRuleName>,
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
	// TODO: add back per-backend filters
	// #[serde(default, skip_serializing_if = "Vec::is_empty")]
	// pub filters: Vec<RouteFilter>,
}

fn default_weight() -> usize {
	1
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

#[apply(schema_de!)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)] // Size is not sensitive for local config
pub enum LocalAIBackend {
	Provider(LocalNamedAIProvider),
	Groups { groups: Vec<LocalAIProviders> },
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
	/// Routes defines how to identify the type of traffic we should handle
	/// The keys are URL suffix matches, like `/v1/models`. The special `*` can be used to match anything.
	#[serde(default)]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "std::collections::HashMap<String, String>")
	)]
	pub routes: IndexMap<Strng, RouteType>,
	#[serde(rename = "backendTLS", default)]
	pub backend_tls: Option<LocalBackendTLS>,
	#[serde(default)]
	pub backend_auth: Option<BackendAuth>,

	#[serde(flatten, default)]
	pub policies: Option<Arc<llm::Policy>>,
}

impl LocalAIBackend {
	pub fn translate(
		self,
		backend_name: BackendName,
	) -> anyhow::Result<(AIBackend, Vec<TargetedPolicy>)> {
		let providers = match self {
			LocalAIBackend::Provider(p) => {
				vec![vec![p]]
			},
			LocalAIBackend::Groups { groups } => groups.into_iter().map(|g| g.providers).collect_vec(),
		};
		let mut ep_groups = vec![];
		let mut policies = vec![];
		for g in providers {
			let mut group = vec![];
			for p in g {
				if let Some(btls) = p.backend_tls {
					policies.push(TargetedPolicy {
						name: strng::format!("{backend_name}/{}-tls", p.name),
						target: PolicyTarget::SubBackend(strng::format!("{}/{}", backend_name, p.name.clone())),
						policy: Policy::BackendTLS(btls.try_into()?),
					});
				}
				if let Some(bauth) = p.backend_auth {
					policies.push(TargetedPolicy {
						name: strng::format!("{backend_name}/{}-auth", p.name),
						target: PolicyTarget::SubBackend(strng::format!("{}/{}", backend_name, p.name.clone())),
						policy: Policy::BackendAuth(bauth),
					});
				}
				if let Some(llm) = p.policies {
					policies.push(TargetedPolicy {
						name: strng::format!("{backend_name}/{}-ai", p.name),
						target: PolicyTarget::SubBackend(strng::format!("{}/{}", backend_name, p.name.clone())),
						policy: Policy::AI(llm),
					});
				}
				group.push((
					p.name.clone(),
					NamedAIProvider {
						name: p.name,
						provider: p.provider,
						host_override: p.host_override,
						path_override: p.path_override,
						tokenize: p.tokenize,
						routes: p.routes,
					},
				));
			}
			ep_groups.push(group);
		}
		let es = types::loadbalancer::EndpointSet::new(ep_groups);
		Ok((AIBackend { providers: es }, policies))
	}
}

impl LocalBackend {
	pub fn as_backends(
		&self,
		name: BackendName,
	) -> anyhow::Result<(Vec<Backend>, Vec<TargetedPolicy>)> {
		Ok(match self {
			LocalBackend::Service { .. } => (vec![], vec![]), // These stay as references
			LocalBackend::Opaque(tgt) => (vec![Backend::Opaque(name, tgt.clone())], vec![]),
			LocalBackend::Dynamic { .. } => (vec![Backend::Dynamic {}], vec![]),
			LocalBackend::MCP(tgt) => {
				let mut targets = vec![];
				let mut backends = vec![];
				let mut policies = vec![];
				for (idx, t) in tgt.targets.iter().enumerate() {
					let name = strng::format!("mcp/{}/{}", name.clone(), idx);
					let (spec, tls) = match t.spec.clone() {
						LocalMcpTargetSpec::Sse { backend } => {
							let (backend, path, tls) = backend.process()?;
							let (bref, be) = to_simple_backend_and_ref(name.clone(), &backend);
							be.into_iter().for_each(|b| backends.push(b));
							(
								McpTargetSpec::Sse(SseTargetSpec {
									backend: bref,
									path: path.clone(),
								}),
								tls,
							)
						},
						LocalMcpTargetSpec::Mcp { backend } => {
							let (backend, path, tls) = backend.process()?;
							let (bref, be) = to_simple_backend_and_ref(name.clone(), &backend);
							be.into_iter().for_each(|b| backends.push(b));
							(
								McpTargetSpec::Mcp(StreamableHTTPTargetSpec {
									backend: bref,
									path: path.clone(),
								}),
								tls,
							)
						},
						LocalMcpTargetSpec::Stdio { cmd, args, env } => {
							(McpTargetSpec::Stdio { cmd, args, env }, false)
						},
						LocalMcpTargetSpec::OpenAPI { backend, schema } => {
							let (backend, _, tls) = backend.process()?;
							let (bref, be) = to_simple_backend_and_ref(name.clone(), &backend);
							be.into_iter().for_each(|b| backends.push(b));
							(
								McpTargetSpec::OpenAPI(OpenAPITarget {
									backend: bref,
									schema,
								}),
								tls,
							)
						},
					};
					if tls {
						policies.push(TargetedPolicy {
							name: strng::format!("implicit-tls/{}", name),
							target: PolicyTarget::Backend(name.clone()),
							policy: Policy::BackendTLS(LocalBackendTLS::default().try_into()?),
						});
					}
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
				let m = McpBackend { targets, stateful };
				backends.push(Backend::MCP(name, m));
				(backends, policies)
			},
			LocalBackend::AI(tgt) => {
				let (be, pols) = tgt.clone().translate(name.clone())?;
				(vec![Backend::AI(name, be)], pols)
			},
			LocalBackend::Invalid => (vec![Backend::Invalid], vec![]),
		})
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
pub struct LocalMcpBackend {
	pub targets: Vec<Arc<LocalMcpTarget>>,
	#[serde(default)]
	pub stateful_mode: McpStatefulMode,
}

#[apply(schema_de!)]
pub struct LocalMcpTarget {
	pub name: McpTargetName,
	#[serde(flatten)]
	pub spec: LocalMcpTargetSpec,
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
	pub fn process(&self) -> anyhow::Result<(SimpleLocalBackend, String, bool)> {
		let McpBackendHost { host, port, path } = self;
		Ok(match (host, port, path) {
			(host, Some(port), Some(path)) => {
				let b = SimpleLocalBackend::Opaque(Target::try_from((host.as_str(), *port))?);
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

				let b = SimpleLocalBackend::Opaque(Target::try_from((host, port))?);
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
	#[serde(default, skip_serializing_if = "Option::is_none", rename = "name")]
	// User facing name of the route
	route_name: Option<RouteName>,
	// User facing name of the rule
	#[serde(default, skip_serializing_if = "Option::is_none")]
	rule_name: Option<RouteRuleName>,
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
	pub backend: SimpleLocalBackend,
}

#[apply(schema_de!)]
pub enum SimpleLocalBackend {
	Service {
		name: NamespacedHostname,
		port: u16,
	},
	#[serde(rename = "host")]
	Opaque(Target), // Hostname or IP
	Invalid,
}

impl SimpleLocalBackend {
	pub fn as_backend(&self, name: BackendName) -> Option<Backend> {
		match self {
			SimpleLocalBackend::Service { .. } => None, // These stay as references
			SimpleLocalBackend::Opaque(tgt) => Some(Backend::Opaque(name, tgt.clone())),
			SimpleLocalBackend::Invalid => Some(Backend::Invalid),
		}
	}
}

#[apply(schema_de!)]
struct LocalPolicy {
	pub name: PolicyName,
	pub target: PolicyTarget,
	pub policy: FilterOrPolicy,
}

#[apply(schema_de!)]
struct LocalGatewayTargetedPolicy {
	pub name: PolicyName,
	pub target: PolicyTarget,
	pub policy: LocalGatewayPolicy,
}

impl From<LocalGatewayPolicy> for FilterOrPolicy {
	fn from(value: LocalGatewayPolicy) -> Self {
		Self {
			ext_proc: value.ext_proc,
			transformations: value.transformations,
			jwt_auth: value.jwt_auth,
			..Default::default()
		}
	}
}

#[apply(schema_de!)]
struct LocalGatewayPolicy {
	/// Extend agentgateway with an external processor
	#[serde(default)]
	ext_proc: Option<LocalExtProc>,

	/// Authenticate incoming JWT requests.
	#[serde(default)]
	jwt_auth: Option<crate::http::jwt::LocalJwtConfig>,

	/// Modify requests and responses
	#[serde(default)]
	#[serde_as(
		deserialize_as = "Option<TryFromInto<http::transformation_cel::LocalTransformationConfig>>"
	)]
	// serde_as is supposed to generate this automatically; not sure why its failing...
	#[cfg_attr(
		feature = "schema",
		schemars(
			with = "serde_with::Schema::<Option<crate::http::transformation_cel::Transformation>, Option<TryFromInto<http::transformation_cel::LocalTransformationConfig>>>"
		)
	)]
	transformations: Option<crate::http::transformation_cel::Transformation>,
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
	request_mirror: Option<LocalRequestMirror>,

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
	mcp_authentication: Option<McpAuthentication>,
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
	remote_rate_limit: Option<LocalRemoteRateLimit>,
	/// Authenticate incoming JWT requests.
	#[serde(default)]
	jwt_auth: Option<crate::http::jwt::LocalJwtConfig>,
	/// Authenticate incoming requests by calling an external authorization server.
	#[serde(default)]
	ext_authz: Option<LocalExtAuthz>,
	/// Extend agentgateway with an external processor
	#[serde(default)]
	ext_proc: Option<LocalExtProc>,
	/// Modify requests and responses
	#[serde(default)]
	#[serde_as(
		deserialize_as = "Option<TryFromInto<http::transformation_cel::LocalTransformationConfig>>"
	)]
	// serde_as is supposed to generate this automatically; not sure why its failing...
	#[cfg_attr(
		feature = "schema",
		schemars(
			with = "serde_with::Schema::<Option<crate::http::transformation_cel::Transformation>, Option<TryFromInto<http::transformation_cel::LocalTransformationConfig>>>"
		)
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
	backend_tls: Option<LocalBackendTLS>,
}

async fn convert(client: client::Client, i: LocalConfig) -> anyhow::Result<NormalizedLocalConfig> {
	let LocalConfig {
		config: _,
		binds,
		policies,
		gateway_policies,
		workloads,
		services,
	} = i;
	let mut all_policies = vec![];
	let mut all_gateway_policies = vec![];
	let mut all_backends = vec![];
	let mut all_binds = vec![];
	for b in binds {
		let bind_name = strng::format!("bind/{}", b.port);
		let mut ls = ListenerSet::default();
		for (idx, l) in b.listeners.into_iter().enumerate() {
			let (l, pol, backends) = convert_listener(client.clone(), bind_name.clone(), idx, l).await?;
			all_policies.extend_from_slice(&pol);
			all_backends.extend_from_slice(&backends);
			ls.insert(l)
		}
		let sockaddr = if cfg!(target_family = "unix") {
			SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), b.port)
		} else {
			// Windows and IPv6 don't mix well apparently?
			SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), b.port)
		};
		let b = Bind {
			key: bind_name,
			address: sockaddr,
			listeners: ls,
		};
		all_binds.push(b)
	}

	for p in policies {
		let res = split_policies(client.clone(), &p.name, &mut all_backends, p.policy).await?;
		if !res.filters.is_empty() {
			anyhow::bail!("'policies' may not contain filters")
		}
		if res.traffic_policy != TrafficPolicy::default() {
			anyhow::bail!("'policies' may not contain traffic policies")
		}
		if (res.route_policies.len() + res.backend_policies.len()) != 1 {
			anyhow::bail!("'policies' must contain exactly 1 policy")
		}
		let tp = res
			.route_policies
			.first()
			.unwrap_or_else(|| res.backend_policies.first().unwrap())
			.clone();
		let tgt_policy = TargetedPolicy {
			name: p.name,
			target: p.target,
			policy: tp,
		};
		all_policies.push(tgt_policy);
	}

	for p in gateway_policies {
		let res = split_policies(client.clone(), &p.name, &mut all_backends, p.policy.into()).await?;
		if !res.filters.is_empty() {
			anyhow::bail!("'gateway_policies' may not contain filters")
		}
		if res.traffic_policy != TrafficPolicy::default() {
			anyhow::bail!("'gateway_policies' may not contain traffic policies")
		}
		if !res.backend_policies.is_empty() {
			anyhow::bail!("'gateway_policies' may not contain backend policies")
		}
		if res.route_policies.len() != 1 {
			anyhow::bail!("'gateway_policies' must contain exactly 1 policy")
		}
		let tp = res.route_policies.first().unwrap().clone();
		let tgt_policy = GatewayTargetedPolicy {
			name: p.name,
			target: p.target,
			policy: tp.try_into()?,
		};
		all_gateway_policies.push(tgt_policy);
	}

	Ok(NormalizedLocalConfig {
		binds: all_binds,
		policies: all_policies,
		gateway_policies: all_gateway_policies,
		backends: all_backends,
		workloads,
		services,
	})
}

async fn convert_listener(
	client: client::Client,
	bind_name: BindName,
	idx: usize,
	l: LocalListener,
) -> anyhow::Result<(Listener, Vec<TargetedPolicy>, Vec<Backend>)> {
	let LocalListener {
		name,
		gateway_name,
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
			ListenerProtocol::HTTPS(convert_tls_server(
				tls.ok_or(anyhow!("HTTPS listener requires 'tls'"))?,
			)?)
		},
		LocalListenerProtocol::TLS => {
			if tcp_routes.is_none() {
				bail!("protocol TLS requires 'tcpRoutes'")
			}
			ListenerProtocol::TLS(convert_tls_server(
				tls.ok_or(anyhow!("TLS listener requires 'tls'"))?,
			)?)
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
	let name = name.unwrap_or_else(|| strng::format!("listener{}", idx));
	let gateway_name: GatewayName = gateway_name.unwrap_or(bind_name);
	let key: ListenerKey = strng::format!("{}/{}", name, gateway_name);

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
		let (route, policies) = convert_tcp_route(l, idx, key.clone()).await?;
		all_policies.extend_from_slice(&policies);
		trs.insert(route)
	}

	let l = Listener {
		key,
		name,
		gateway_name,
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
) -> anyhow::Result<(Route, Vec<TargetedPolicy>, Vec<Backend>)> {
	let LocalRoute {
		route_name,
		rule_name,
		hostnames,
		matches,
		policies,
		backends,
	} = lr;

	let route_name = route_name.unwrap_or_else(|| strng::format!("route{}", idx));
	let rule_name = rule_name.clone().unwrap_or_else(|| strng::new("default"));
	let key = strng::format!("{}/{}/{}", listener_key, route_name, rule_name,);
	let rule_name = strng::format!("{route_name}/{rule_name}");
	let mut backend_policies = vec![];

	let mut backend_refs = Vec::new();
	let mut external_backends = Vec::new();
	for b in backends {
		let bref = match &b.backend {
			LocalBackend::Service { name, port } => BackendReference::Service {
				name: name.clone(),
				port: *port,
			},
			LocalBackend::Invalid => BackendReference::Invalid,
			_ => BackendReference::Backend(key.clone()),
		};
		let (backends, policies_from_backends) = b.backend.as_backends(bref.name())?;
		let bref = RouteBackendReference {
			weight: b.weight,
			backend: bref,
			filters: vec![],
			// filters: b.filters,
		};
		backend_refs.push(bref);
		external_backends.extend_from_slice(&backends);
		backend_policies.extend_from_slice(&policies_from_backends);
	}
	let mut be_pol = 0;
	let backend_tgt = |p: Policy| {
		if backend_refs.len() != 1 {
			anyhow::bail!("backend policies currently only work with exactly 1 backend")
		}
		let be = backend_refs.first().unwrap();
		be_pol += 1;
		Ok(TargetedPolicy {
			name: format!("{key}/backend-{be_pol}").into(),
			target: PolicyTarget::Backend(be.backend.name()),
			policy: p,
		})
	};

	let resolved = if let Some(pol) = policies {
		split_policies(client, &key, &mut external_backends, pol).await?
	} else {
		ResolvedPolicies::default()
	};
	let external_policies = resolved
		.backend_policies
		.into_iter()
		.map(backend_tgt)
		.chain(backend_policies.into_iter().map(Ok))
		.collect::<Result<Vec<_>, _>>()?;
	let route = Route {
		key,
		route_name,
		rule_name: Some(rule_name),
		hostnames,
		matches,
		filters: resolved.filters,
		backends: backend_refs,
		policies: Some(resolved.traffic_policy),
		inline_policies: resolved.route_policies,
	};
	Ok((route, external_policies, external_backends))
}

#[derive(Default)]
struct ResolvedPolicies {
	filters: Vec<RouteFilter>,
	backend_policies: Vec<Policy>,
	route_policies: Vec<Policy>,
	traffic_policy: TrafficPolicy,
}

async fn split_policies(
	client: Client,
	key: &Strng,
	external_backends: &mut Vec<Backend>,
	pol: FilterOrPolicy,
) -> Result<ResolvedPolicies, Error> {
	let mut resolved = ResolvedPolicies::default();
	let ResolvedPolicies {
		filters,
		backend_policies,
		route_policies,
		traffic_policy,
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
		transformations,
		csrf,
		ext_authz,
		ext_proc,
		timeout,
		retry,
	} = pol;
	if let Some(p) = request_header_modifier {
		filters.push(RouteFilter::RequestHeaderModifier(p));
	}
	if let Some(p) = response_header_modifier {
		filters.push(RouteFilter::ResponseHeaderModifier(p));
	}
	if let Some(p) = request_redirect {
		filters.push(RouteFilter::RequestRedirect(p));
	}
	if let Some(p) = url_rewrite {
		filters.push(RouteFilter::UrlRewrite(p));
	}
	if let Some(p) = request_mirror {
		let (bref, backend) = to_simple_backend_and_ref(strng::format!("{}/mirror", key), &p.backend);
		let pol = filters::RequestMirror {
			backend: bref,
			percentage: p.percentage,
		};
		backend
			.into_iter()
			.for_each(|backend| external_backends.push(backend));
		filters.push(RouteFilter::RequestMirror(pol));
	}

	// Filters
	if let Some(p) = direct_response {
		filters.push(RouteFilter::DirectResponse(p));
	}
	if let Some(p) = cors {
		filters.push(RouteFilter::CORS(p));
	}

	// Backend policies
	if let Some(p) = mcp_authorization {
		backend_policies.push(Policy::McpAuthorization(p))
	}
	if let Some(p) = mcp_authentication {
		let jp = p.as_jwt()?;
		backend_policies.push(Policy::McpAuthentication(p));
		route_policies.push(Policy::JwtAuth(jp.try_into(client.clone()).await?));
	}
	if let Some(p) = a2a {
		backend_policies.push(Policy::A2a(p))
	}
	if let Some(p) = backend_tls {
		backend_policies.push(Policy::BackendTLS(p.try_into()?))
	}
	if let Some(p) = backend_auth {
		backend_policies.push(Policy::BackendAuth(p))
	}

	// Route policies
	if let Some(p) = ai {
		route_policies.push(Policy::AI(Arc::new(p)))
	}
	if let Some(p) = jwt_auth {
		route_policies.push(Policy::JwtAuth(p.try_into(client.clone()).await?));
	}
	if let Some(p) = transformations {
		route_policies.push(Policy::Transformation(p));
	}
	if let Some(p) = csrf {
		route_policies.push(Policy::Csrf(p))
	}
	if let Some(p) = authorization {
		route_policies.push(Policy::Authorization(p))
	}
	if let Some(p) = ext_authz {
		let (bref, backend) = to_simple_backend_and_ref(strng::format!("{}/extauthz", key), &p.target);
		// Convert to FailureMode
		let failure_mode = match (p.fail_open, p.status_on_error) {
			(Some(true), _) => crate::http::ext_authz::FailureMode::Allow,
			(Some(false), Some(code)) => crate::http::ext_authz::FailureMode::DenyWithStatus(code),
			(Some(false), None) | (None, None) => crate::http::ext_authz::FailureMode::Deny,
			(None, Some(code)) => crate::http::ext_authz::FailureMode::DenyWithStatus(code),
		};

		let pol = http::ext_authz::ExtAuthz {
			target: Arc::new(bref),
			context: p.context,
			failure_mode,
			include_request_headers: vec![],
			include_request_body: None,
			timeout: None,
		};
		backend
			.into_iter()
			.for_each(|backend| external_backends.push(backend));
		route_policies.push(Policy::ExtAuthz(pol))
	}
	if let Some(p) = ext_proc {
		let (bref, backend) = to_simple_backend_and_ref(strng::format!("{}/extproc", key), &p.target);

		let pol = http::ext_proc::ExtProc {
			target: Arc::new(bref),
			failure_mode: p.failure_mode,
		};
		backend
			.into_iter()
			.for_each(|backend| external_backends.push(backend));
		route_policies.push(Policy::ExtProc(pol))
	}
	if !local_rate_limit.is_empty() {
		route_policies.push(Policy::LocalRateLimit(local_rate_limit))
	}
	if let Some(p) = remote_rate_limit {
		let (bref, backend) = to_simple_backend_and_ref(strng::format!("{}/ratelimit", key), &p.target);
		let pol = http::remoteratelimit::RemoteRateLimit {
			domain: p.domain,
			target: Arc::new(bref),
			descriptors: Arc::new(p.descriptors),
		};
		backend
			.into_iter()
			.for_each(|backend| external_backends.push(backend));
		route_policies.push(Policy::RemoteRateLimit(pol))
	}

	// Traffic policies
	if let Some(p) = timeout {
		traffic_policy.timeout = p;
	}
	if let Some(p) = retry {
		traffic_policy.retry = Some(p);
	}
	Ok(resolved)
}

async fn convert_tcp_route(
	lr: LocalTCPRoute,
	idx: usize,
	listener_key: ListenerKey,
) -> anyhow::Result<(TCPRoute, Vec<TargetedPolicy>)> {
	let LocalTCPRoute {
		route_name,
		rule_name,
		hostnames,
		policies,
		backends,
	} = lr;

	let route_name = route_name.unwrap_or_else(|| strng::format!("tcproute{}", idx));
	let key = strng::format!(
		"{}/{}/{}",
		listener_key,
		route_name,
		rule_name.clone().unwrap_or_else(|| strng::new("default"))
	);
	let mut external_policies = vec![];
	let mut pol = 0;
	let _tgt = |p: Policy| {
		pol += 1;
		TargetedPolicy {
			name: format!("{key}/{pol}").into(),
			target: RouteRule(key.clone()),
			policy: p,
		}
	};
	let mut be_pol = 0;
	let backend_tgt = |p: Policy| {
		if backends.len() != 1 {
			anyhow::bail!("backend policies currently only work with exactly 1 backend")
		}

		let (refs, _to_add): (Vec<_>, Vec<Option<Backend>>) = backends
			.into_iter()
			.map(|b| {
				let (bref, backend) = to_simple_backend_and_ref(key.clone(), &b.backend);
				let bref = TCPRouteBackendReference {
					weight: b.weight,
					backend: bref,
				};
				(bref, backend)
			})
			.unzip();
		let be = refs.first().unwrap();
		be_pol += 1;
		Ok(TargetedPolicy {
			name: format!("{key}/backend-{be_pol}").into(),
			target: PolicyTarget::Backend(be.backend.name()),
			policy: p,
		})
	};

	if let Some(pol) = policies {
		let TCPFilterOrPolicy { backend_tls } = pol;
		if let Some(p) = backend_tls {
			external_policies.push(backend_tgt(Policy::BackendTLS(p.try_into()?))?)
		}
	}
	#[allow(unreachable_code)]
	let _route = TCPRoute {
		key,
		route_name,
		rule_name,
		hostnames,
		backends: todo!(),
	};
	Ok((_route, external_policies))
}

fn to_simple_backend_and_ref(
	name: BackendName,
	b: &SimpleLocalBackend,
) -> (SimpleBackendReference, Option<Backend>) {
	let bref = match &b {
		SimpleLocalBackend::Service { name, port } => SimpleBackendReference::Service {
			name: name.clone(),
			port: *port,
		},
		SimpleLocalBackend::Invalid => SimpleBackendReference::Invalid,
		_ => SimpleBackendReference::Backend(name.clone()),
	};
	let backend = b.as_backend(name);
	(bref, backend)
}

fn convert_tls_server(tls: LocalTLSServerConfig) -> anyhow::Result<TLSConfig> {
	let cert = fs_err::read(tls.cert)?;
	let cert_chain = crate::types::agent::parse_cert(&cert)?;
	let key = fs_err::read(tls.key)?;
	let private_key = crate::types::agent::parse_key(&key)?;

	let mut ccb = ServerConfig::builder_with_provider(transport::tls::provider())
		.with_protocol_versions(transport::tls::ALL_TLS_VERSIONS)
		.expect("server config must be valid")
		.with_no_client_auth()
		.with_single_cert(cert_chain, private_key)?;
	ccb.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
	Ok(TLSConfig {
		config: Arc::new(ccb),
	})
}

#[apply(schema_de!)]
pub struct LocalRequestMirror {
	pub backend: SimpleLocalBackend,
	// 0.0-1.0
	pub percentage: f64,
}

#[apply(schema_de!)]
pub struct LocalExtAuthz {
	#[serde(flatten)]
	pub target: SimpleLocalBackend,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub context: Option<HashMap<String, String>>,
	// Backwards compatibility: support both old and new failure handling approaches
	#[serde(default)]
	pub fail_open: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub status_on_error: Option<u16>,
}

#[apply(schema_de!)]
pub struct LocalExtProc {
	#[serde(flatten)]
	pub target: SimpleLocalBackend,
	#[serde(default)]
	pub failure_mode: FailureMode,
}

#[apply(schema_de!)]
pub struct LocalRemoteRateLimit {
	pub domain: String,
	#[serde(flatten)]
	pub target: SimpleLocalBackend,
	pub descriptors: crate::http::remoteratelimit::DescriptorSet,
}
