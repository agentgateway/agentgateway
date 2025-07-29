use std::collections::HashMap;
use std::convert::Infallible;
use std::io::Cursor;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;

use agent_core::prelude::Strng;
use anyhow::{Error, anyhow, bail};
use itertools::Itertools;
use jsonwebtoken::jwk::{AlgorithmParameters, JwkSet, KeyAlgorithm};
use jsonwebtoken::{DecodingKey, Validation};
use macro_rules_attribute::{apply, attribute_alias};
use openapiv3::OpenAPI;
use rmcp::handler::server::router::tool::CallToolHandlerExt;
use rustls::{ClientConfig, ServerConfig};
use serde::de::DeserializeOwned;
use serde_with::{TryFromInto, serde_as};

use crate::http::auth::BackendAuth;
use crate::http::backendtls::{BackendTLS, LocalBackendTLS};
use crate::http::jwt::{JwkError, Jwt};
use crate::http::remoteratelimit::Descriptor;
use crate::http::{filters, retry, timeout};
use crate::llm::AIProvider;
use crate::store::LocalWorkload;
use crate::transport::tls;
use crate::types::agent::PolicyTarget::RouteRule;
use crate::types::agent::{
	A2aPolicy, Backend, BackendName, BackendReference, Bind, BindName, GatewayName, Listener,
	ListenerKey, ListenerProtocol, ListenerSet, McpAuthentication, McpAuthorization, McpBackend,
	McpTarget, McpTargetName, McpTargetSpec, OpenAPITarget, PathMatch, Policy, PolicyTarget, Route,
	RouteBackend, RouteBackendReference, RouteFilter, RouteMatch, RouteName, RouteRuleName, RouteSet,
	SimpleBackend, SimpleBackendReference, SseTargetSpec, StreamableHTTPTargetSpec, TCPRoute,
	TCPRouteBackendReference, TCPRouteSet, TLSConfig, Target, TargetedPolicy, TrafficPolicy,
	parse_cert, parse_key,
};
use crate::types::discovery::{NamespacedHostname, Service};
use crate::*;

attribute_alias! {
		#[apply(schema!)] = #[serde_as] #[derive(Debug, Clone, serde::Deserialize)] #[serde(rename_all = "camelCase", deny_unknown_fields)] #[cfg_attr(feature = "schema", derive(JsonSchema))];
}

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
	pub backends: Vec<Backend>,
	// Note: here we use LocalWorkload since it conveys useful info, we could maybe change but not a problem
	// for now
	pub workloads: Vec<LocalWorkload>,
	pub services: Vec<Service>,
}

#[apply(schema!)]
pub struct LocalConfig {
	#[serde(default)]
	#[cfg_attr(feature = "schema", schemars(with = "RawConfig"))]
	config: Arc<Option<serde_json::value::Value>>,
	#[serde(default)]
	binds: Vec<LocalBind>,
	#[serde(default)]
	#[cfg_attr(feature = "schema", schemars(with = "serde_json::value::RawValue"))]
	workloads: Vec<LocalWorkload>,
	#[serde(default)]
	#[cfg_attr(feature = "schema", schemars(with = "serde_json::value::RawValue"))]
	services: Vec<Service>,
}

#[apply(schema!)]
struct LocalBind {
	port: u16,
	listeners: Vec<LocalListener>,
}

#[apply(schema!)]
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

#[apply(schema!)]
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

#[apply(schema!)]
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

#[apply(schema!)]
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
	AI(crate::llm::AIBackend),
	Invalid,
}

impl LocalBackend {
	pub fn as_backends(&self, name: BackendName) -> Vec<Backend> {
		match self {
			LocalBackend::Service { .. } => vec![], // These stay as references
			LocalBackend::Opaque(tgt) => vec![Backend::Opaque(name, tgt.clone())],
			LocalBackend::Dynamic { .. } => vec![Backend::Dynamic {}],
			LocalBackend::MCP(tgt) => {
				let mut targets = vec![];
				let mut backends = vec![];
				for (idx, t) in tgt.targets.iter().enumerate() {
					let spec = match t.spec.clone() {
						LocalMcpTargetSpec::Sse { backend, path } => {
							let name = strng::format!("mcp/{}/{}", name.clone(), idx);
							let (bref, be) = to_simple_backend_and_ref(name, &backend);
							be.into_iter().for_each(|b| backends.push(b));
							McpTargetSpec::Sse(SseTargetSpec {
								backend: bref,
								path: path.clone(),
							})
						},
						LocalMcpTargetSpec::Mcp { backend, path } => {
							let name = strng::format!("mcp/{}/{}", name.clone(), idx);
							let (bref, be) = to_simple_backend_and_ref(name, &backend);
							be.into_iter().for_each(|b| backends.push(b));
							McpTargetSpec::Mcp(StreamableHTTPTargetSpec {
								backend: bref,
								path: path.clone(),
							})
						},
						LocalMcpTargetSpec::Stdio { cmd, args, env } => McpTargetSpec::Stdio { cmd, args, env },
						LocalMcpTargetSpec::OpenAPI { backend, schema } => {
							let name = strng::format!("mcp/{}/{}", name.clone(), idx);
							let (bref, be) = to_simple_backend_and_ref(name.clone(), &backend);
							be.into_iter().for_each(|b| backends.push(b));
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
				let m = McpBackend { targets };
				backends.push(Backend::MCP(name, m));
				backends
			},
			LocalBackend::AI(tgt) => vec![Backend::AI(name, tgt.clone())],
			LocalBackend::Invalid => vec![Backend::Invalid],
		}
	}
}

#[apply(schema!)]
pub struct LocalMcpBackend {
	pub targets: Vec<Arc<LocalMcpTarget>>,
}

#[apply(schema!)]
pub struct LocalMcpTarget {
	pub name: McpTargetName,
	#[serde(flatten)]
	pub spec: LocalMcpTargetSpec,
}

#[apply(schema!)]
pub struct McpBackendHost {
	pub host: String,
	pub port: u16,
}

impl TryFrom<McpBackendHost> for SimpleLocalBackend {
	type Error = anyhow::Error;
	fn try_from(value: McpBackendHost) -> Result<Self, Self::Error> {
		Ok(SimpleLocalBackend::Opaque(Target::try_from((
			value.host.as_str(),
			value.port,
		))?))
	}
}

#[apply(schema!)]
pub enum LocalMcpTargetSpec {
	#[serde(rename = "sse")]
	Sse {
		#[serde(flatten)]
		#[serde_as(deserialize_as = "TryFromInto<McpBackendHost>")]
		backend: SimpleLocalBackend,
		path: String,
	},
	#[serde(rename = "mcp")]
	Mcp {
		#[serde(flatten)]
		#[serde_as(deserialize_as = "TryFromInto<McpBackendHost>")]
		backend: SimpleLocalBackend,
		path: String,
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
		#[serde_as(deserialize_as = "TryFromInto<McpBackendHost>")]
		backend: SimpleLocalBackend,
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

#[apply(schema!)]
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

#[apply(schema!)]
pub struct LocalTCPRouteBackend {
	#[serde(default = "default_weight")]
	pub weight: usize,
	pub backend: SimpleLocalBackend,
}

#[apply(schema!)]
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

#[apply(schema!)]
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
	/// Authentication for MCP clients.
	#[serde(default)]
	mcp_authentication: Option<McpAuthentication>,
	/// Mark this traffic as A2A to enable A2A processing and telemetry.
	#[serde(default)]
	a2a: Option<A2aPolicy>,
	/// Mark this as LLM traffic to enable LLM processing.
	#[serde(default)]
	#[cfg_attr(feature = "schema", schemars(with = "serde_json::value::RawValue"))]
	ai: Option<llm::Policy>,
	/// Send TLS to the backend.
	#[serde(rename = "backendTLS", default)]
	backend_tls: Option<http::backendtls::LocalBackendTLS>,
	/// Authenticate to the backend.
	#[serde(default)]
	backend_auth: Option<BackendAuth>,
	/// Rate limit incoming requests. State is kept local.
	#[serde(default)]
	#[cfg_attr(feature = "schema", schemars(with = "serde_json::value::RawValue"))]
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

	// TrafficPolicy
	/// Timeout requests that exceed the configured duration.
	#[serde(default)]
	timeout: Option<timeout::Policy>,
	/// Retry matching requests.
	#[serde(default)]
	retry: Option<retry::Policy>,
}

#[apply(schema!)]
struct TCPFilterOrPolicy {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	backend_tls: Option<LocalBackendTLS>,
}

async fn convert(client: client::Client, i: LocalConfig) -> anyhow::Result<NormalizedLocalConfig> {
	let LocalConfig {
		config: _,
		binds,
		workloads,
		services,
	} = i;
	let mut all_policies = vec![];
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
		let b = Bind {
			key: bind_name,
			address: SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), b.port),
			listeners: ls,
		};
		all_binds.push(b)
	}
	Ok(NormalizedLocalConfig {
		binds: all_binds,
		policies: all_policies,
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
	let key = strng::format!(
		"{}/{}/{}",
		listener_key,
		route_name,
		rule_name.clone().unwrap_or_else(|| strng::new("default"))
	);
	let mut filters = vec![];
	let mut traffic_policy: Option<TrafficPolicy> = None;
	let mut external_policies = vec![];
	let mut pol = 0;
	let mut tgt = |p: Policy| {
		pol += 1;
		TargetedPolicy {
			name: format!("{key}/{pol}").into(),
			target: RouteRule(key.clone()),
			policy: p,
		}
	};

	let (refs, external_backends): (Vec<_>, Vec<Vec<Backend>>) = backends
		.into_iter()
		.map(|b| {
			let bref = match &b.backend {
				LocalBackend::Service { name, port } => BackendReference::Service {
					name: name.clone(),
					port: *port,
				},
				LocalBackend::Invalid => BackendReference::Invalid,
				_ => BackendReference::Backend(key.clone()),
			};
			let backends = b.backend.as_backends(bref.name());
			let bref = RouteBackendReference {
				weight: b.weight,
				backend: bref,
				filters: vec![],
				// filters: b.filters,
			};
			(bref, backends)
		})
		.unzip();
	let mut external_backends = external_backends.into_iter().flatten().collect_vec();
	let mut be_pol = 0;
	let mut backend_tgt = |p: Policy| {
		if refs.len() != 1 {
			anyhow::bail!("backend policies currently only work with exactly 1 backend")
		}
		let be = refs.first().unwrap();
		be_pol += 1;
		Ok(TargetedPolicy {
			name: format!("{key}/backend-{be_pol}").into(),
			target: PolicyTarget::Backend(be.backend.name()),
			policy: p,
		})
	};

	let mut traffic_policy = TrafficPolicy {
		timeout: timeout::Policy::default(),
		retry: None,
	};
	if let Some(pol) = policies {
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
			local_rate_limit,
			remote_rate_limit,
			jwt_auth,
			transformations,
			ext_authz,
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
		if let Some(p) = direct_response {
			filters.push(RouteFilter::DirectResponse(p));
		}
		if let Some(p) = cors {
			filters.push(RouteFilter::CORS(p));
		}

		if let Some(p) = mcp_authorization {
			external_policies.push(backend_tgt(Policy::McpAuthorization(p))?)
		}
		if let Some(p) = mcp_authentication {
			let jp = p.as_jwt()?;
			external_policies.push(backend_tgt(Policy::McpAuthentication(p))?);
			external_policies.push(tgt(Policy::JwtAuth(jp.try_into(client.clone()).await?)));
		}
		if let Some(p) = a2a {
			external_policies.push(backend_tgt(Policy::A2a(p))?)
		}
		if let Some(p) = ai {
			external_policies.push(backend_tgt(Policy::AI(p))?)
		}
		if let Some(p) = backend_tls {
			external_policies.push(backend_tgt(Policy::BackendTLS(p.try_into()?))?)
		}
		if let Some(p) = backend_auth {
			external_policies.push(backend_tgt(Policy::BackendAuth(p))?)
		}
		if let Some(p) = jwt_auth {
			external_policies.push(tgt(Policy::JwtAuth(p.try_into(client.clone()).await?)))
		}
		if let Some(p) = transformations {
			external_policies.push(tgt(Policy::Transformation(p)))
		}
		if let Some(p) = ext_authz {
			let (bref, backend) =
				to_simple_backend_and_ref(strng::format!("{}/extauthz", key), &p.target);
			let pol = http::ext_authz::ExtAuthz {
				target: Arc::new(bref),
				context: p.context,
			};
			backend
				.into_iter()
				.for_each(|backend| external_backends.push(backend));
			external_policies.push(tgt(Policy::ExtAuthz(pol)))
		}
		if !local_rate_limit.is_empty() {
			external_policies.push(tgt(Policy::LocalRateLimit(local_rate_limit)))
		}
		if let Some(p) = remote_rate_limit {
			let (bref, backend) =
				to_simple_backend_and_ref(strng::format!("{}/ratelimit", key), &p.target);
			let pol = http::remoteratelimit::RemoteRateLimit {
				target: Arc::new(bref),
				descriptors: p.descriptors,
			};
			backend
				.into_iter()
				.for_each(|backend| external_backends.push(backend));
			external_policies.push(tgt(Policy::RemoteRateLimit(pol)))
		}

		if let Some(p) = timeout {
			traffic_policy.timeout = p;
		}
		if let Some(p) = retry {
			traffic_policy.retry = Some(p);
		}
	}
	let route = Route {
		key,
		route_name,
		rule_name,
		hostnames,
		matches,
		filters,
		backends: refs,
		policies: Some(traffic_policy),
	};
	Ok((route, external_policies, external_backends))
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
	let mut traffic_policy: Option<TrafficPolicy> = None;
	let mut external_policies = vec![];
	let mut pol = 0;
	let mut tgt = |p: Policy| {
		pol += 1;
		TargetedPolicy {
			name: format!("{key}/{pol}").into(),
			target: RouteRule(key.clone()),
			policy: p,
		}
	};
	let mut be_pol = 0;
	let mut backend_tgt = |p: Policy| {
		if backends.len() != 1 {
			anyhow::bail!("backend policies currently only work with exactly 1 backend")
		}

		let (refs, to_add): (Vec<_>, Vec<Option<Backend>>) = backends
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

	let mut traffic_policy = TrafficPolicy {
		timeout: timeout::Policy::default(),
		retry: None,
	};
	if let Some(pol) = policies {
		let TCPFilterOrPolicy { backend_tls } = pol;
		if let Some(p) = backend_tls {
			external_policies.push(backend_tgt(Policy::BackendTLS(p.try_into()?))?)
		}
	}
	let route = TCPRoute {
		key,
		route_name,
		rule_name,
		hostnames,
		backends: todo!(),
	};
	Ok((route, external_policies))
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

#[apply(schema!)]
pub struct LocalRequestMirror {
	pub backend: SimpleLocalBackend,
	// 0.0-1.0
	pub percentage: f64,
}

#[apply(schema!)]
pub struct LocalExtAuthz {
	#[serde(flatten)]
	pub target: SimpleLocalBackend,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub context: Option<HashMap<String, String>>, // TODO: gRPC vs HTTP, fail open, include body,
}

#[apply(schema!)]
pub struct LocalRemoteRateLimit {
	#[serde(flatten)]
	pub target: SimpleLocalBackend,
	pub descriptors: HashMap<String, Descriptor>,
}
