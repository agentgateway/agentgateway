use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::TcpListener as StdTcpListener;
use std::sync::Arc;

use crate::cel::ContextBuilder;
use crate::http::auth::BackendAuth;
use crate::http::authorization::{HTTPAuthorizationSet, NetworkAuthorizationSet};
use crate::http::backendtls::BackendTLS;
use crate::http::ext_proc::InferenceRouting;
use crate::http::oidc;
use crate::http::{ext_authz, ext_proc, filters, health, remoteratelimit, retry, timeout};
use crate::llm::policy::ResponseGuard;
use crate::mcp::McpAuthorizationSet;
use crate::proxy::httpproxy::PolicyClient;
use crate::types::agent::{
	A2aPolicy, Backend, BackendKey, BackendPolicy, BackendTargetRef, BackendWithPolicies, Bind,
	BindKey, FrontendPolicy, JwtAuthentication, Listener, ListenerKey, ListenerName,
	McpAuthentication, PolicyKey, PolicyTarget, Route, RouteGroupKey, RouteKey, RouteName, RouteSet,
	TCPRoute, TCPRouteSet, TargetedPolicy, TrafficPolicy,
};
use crate::types::discovery::NamespacedHostname;
use crate::types::proto::agent::resource::Kind as XdsKind;
use crate::types::proto::agent::{
	Backend as XdsBackend, Bind as XdsBind, Listener as XdsListener, Policy as XdsPolicy,
	Resource as ADPResource, Route as XdsRoute, TcpRoute as XdsTcpRoute,
};
use crate::types::{agent, frontend};
use crate::*;
use agent_xds::{RejectedConfig, XdsUpdate};
use anyhow::Context;
use futures_core::Stream;
use hashbrown::{Equivalent, HashMap as HbHashMap};
use itertools::Itertools;
use tracing::{Level, instrument, warn};

#[derive(Debug)]
enum ResourceKind {
	Policy(PolicyKey),
	Bind(BindKey),
	Route(RouteKey),
	TcpRoute(RouteKey),
	Listener(ListenerKey),
	Backend(ListenerKey),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum RouteTarget {
	Listener(ListenerKey),
	Service(NamespacedHostname),
	RouteGroup(RouteGroupKey),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum RouteTargetRef<'a> {
	Listener(&'a str),
	Service {
		namespace: &'a str,
		hostname: &'a str,
	},
	RouteGroup(&'a str),
}

impl Equivalent<RouteTarget> for RouteTargetRef<'_> {
	fn equivalent(&self, key: &RouteTarget) -> bool {
		self == &RouteTargetRef::from(key)
	}
}

impl<'a> From<&'a RouteTarget> for RouteTargetRef<'a> {
	fn from(value: &'a RouteTarget) -> Self {
		match value {
			RouteTarget::Listener(listener) => RouteTargetRef::Listener(listener.as_str()),
			RouteTarget::Service(service) => RouteTargetRef::Service {
				namespace: service.namespace.as_str(),
				hostname: service.hostname.as_str(),
			},
			RouteTarget::RouteGroup(route_group) => RouteTargetRef::RouteGroup(route_group.as_str()),
		}
	}
}

#[derive(Debug)]
pub struct Store {
	ipv6_enabled: bool,
	core_ids: Option<Vec<core_affinity::CoreId>>,
	binds: HashMap<BindKey, Arc<Bind>>,
	resources: HashMap<Strng, ResourceKind>,

	policies_by_key: HashMap<PolicyKey, Arc<TargetedPolicy>>,
	policies_by_target: hashbrown::HashMap<PolicyTarget, HashSet<PolicyKey>>,

	backends: HashMap<BackendKey, Arc<BackendWithPolicies>>,

	// Listeners we got before a Bind arrived
	pending_listeners: HashMap<BindKey, HashMap<ListenerKey, Listener>>,
	http_routes: HbHashMap<RouteTarget, Arc<RouteSet>>,
	tcp_routes: HbHashMap<RouteTarget, Arc<TCPRouteSet>>,

	tx: tokio::sync::mpsc::UnboundedSender<BindEvent>,
	rx: Option<tokio::sync::mpsc::UnboundedReceiver<BindEvent>>,
}

#[derive(Debug)]
pub enum BindEvent {
	Add(Bind, BindListeners),
	Remove(BindKey),
}

#[derive(Debug)]
pub enum BindListeners {
	Single(StdTcpListener),
	PerCore(HashMap<core_affinity::CoreId, StdTcpListener>),
}

#[derive(Default, Debug, Clone)]
pub struct FrontendPolices {
	pub http: Option<frontend::HTTP>,
	pub tls: Option<frontend::TLS>,
	pub tcp: Option<frontend::TCP>,
	pub network_authorization: Option<NetworkAuthorizationSet>,
	pub access_log: Option<frontend::LoggingPolicy>,
	pub tracing: Option<Arc<crate::types::agent::TracingPolicy>>,
	pub access_log_otlp: Option<Arc<crate::types::agent::AccessLogPolicy>>,
}

impl FrontendPolices {
	pub fn set_if_empty(&mut self, rule: &FrontendPolicy) {
		match rule {
			FrontendPolicy::HTTP(p) => {
				self.http.get_or_insert_with(|| p.clone());
			},
			FrontendPolicy::TLS(p) => {
				self.tls.get_or_insert_with(|| p.clone());
			},
			FrontendPolicy::TCP(p) => {
				self.tcp.get_or_insert_with(|| p.clone());
			},
			FrontendPolicy::NetworkAuthorization(p) => {
				if let Some(existing) = self.network_authorization.as_mut() {
					existing.merge_rule_set(p.0.clone());
				} else {
					self.network_authorization = Some(NetworkAuthorizationSet::new(vec![p.0.clone()].into()));
				}
			},
			FrontendPolicy::AccessLog(p) => {
				self.access_log.get_or_insert_with(|| p.clone());
				if let Some(alp) = &p.access_log_policy {
					self.access_log_otlp.get_or_insert_with(|| alp.clone());
				}
			},
			FrontendPolicy::Tracing(p) => {
				self.tracing.get_or_insert_with(|| p.clone());
			},
		}
	}
	pub fn register_cel_expressions(&self, ctx: &mut ContextBuilder) {
		let Some(frontend::LoggingPolicy {
			filter,
			add: fields_add,
			remove: _,
			otlp: _,
			access_log_policy: _,
		}) = &self.access_log
		else {
			return;
		};
		if let Some(f) = filter {
			ctx.register_expression(f)
		}
		for (_, v) in fields_add.iter() {
			ctx.register_expression(v)
		}
	}
}

#[derive(Default, Debug, Clone)]
pub struct BackendPolicies {
	pub backend_tls: Option<BackendTLS>,
	pub backend_auth: Option<BackendAuth>,
	pub a2a: Option<A2aPolicy>,
	pub llm_provider: Option<Arc<llm::NamedAIProvider>>,
	pub llm: Option<Arc<llm::Policy>>,
	pub inference_routing: Option<InferenceRouting>,

	pub mcp_authorization: Option<McpAuthorizationSet>,
	pub mcp_authentication: Option<McpAuthentication>,

	pub http: Option<types::backend::HTTP>,
	pub tcp: Option<types::backend::TCP>,
	pub tunnel: Option<types::backend::Tunnel>,

	pub request_header_modifier: Option<filters::HeaderModifier>,
	pub response_header_modifier: Option<filters::HeaderModifier>,
	pub request_redirect: Option<filters::RequestRedirect>,
	pub request_mirror: Vec<filters::RequestMirror>,
	pub transformation: Option<http::transformation_cel::Transformation>,

	pub session_persistence: Option<http::sessionpersistence::Policy>,

	pub health: Option<health::Policy>,

	/// Internal-only override for destination endpoint selection.
	/// Used for stateful MCP routing (session affinity).
	/// Not exposed through config - set programmatically only.
	pub override_dest: Option<std::net::SocketAddr>,
}

impl BackendPolicies {
	// Merges self and other. Other has precedence
	pub fn merge(self, other: BackendPolicies) -> BackendPolicies {
		Self {
			backend_tls: other.backend_tls.or(self.backend_tls),
			backend_auth: other.backend_auth.or(self.backend_auth),
			a2a: other.a2a.or(self.a2a),
			llm_provider: other.llm_provider.or(self.llm_provider),
			llm: other.llm.or(self.llm),
			// TODO: is this right??
			mcp_authorization: other.mcp_authorization.or(self.mcp_authorization),
			mcp_authentication: other.mcp_authentication.or(self.mcp_authentication),
			inference_routing: other.inference_routing.or(self.inference_routing),
			http: other.http.or(self.http),
			tcp: other.tcp.or(self.tcp),
			tunnel: other.tunnel.or(self.tunnel),
			request_header_modifier: other
				.request_header_modifier
				.or(self.request_header_modifier),
			response_header_modifier: other
				.response_header_modifier
				.or(self.response_header_modifier),
			request_redirect: other.request_redirect.or(self.request_redirect),
			request_mirror: if other.request_mirror.is_empty() {
				self.request_mirror
			} else {
				other.request_mirror
			},
			transformation: other.transformation.or(self.transformation),
			session_persistence: other.session_persistence.or(self.session_persistence),
			health: other.health.or(self.health),
			override_dest: other.override_dest.or(self.override_dest),
		}
	}
	/// build the inference routing configuration. This may be a NO-OP config.
	pub fn build_inference(&self, client: PolicyClient) -> ext_proc::InferencePoolRouter {
		if let Some(inference) = &self.inference_routing {
			inference.build(client)
		} else {
			ext_proc::InferencePoolRouter::default()
		}
	}

	pub fn register_cel_expressions(&self, ctx: &mut ContextBuilder) {
		if let Some(xfm) = &self.transformation {
			for expr in xfm.expressions() {
				ctx.register_expression(expr)
			}
		}
	}
}

#[derive(Debug, Default)]
pub struct RoutePolicies {
	pub local_rate_limit: Vec<http::localratelimit::RateLimit>,
	pub remote_rate_limit: Option<remoteratelimit::RemoteRateLimit>,
	pub authorization: Option<http::authorization::HTTPAuthorizationSet>,
	pub jwt: Option<JwtAuthentication>,
	pub oidc: Option<oidc::OidcPolicy>,
	pub basic_auth: Option<http::basicauth::BasicAuthentication>,
	pub api_key: Option<http::apikey::APIKeyAuthentication>,
	pub ext_authz: Option<ext_authz::ExtAuthz>,
	pub ext_proc: Option<ext_proc::ExtProc>,
	pub transformation: Option<http::transformation_cel::Transformation>,
	pub llm: Option<Arc<llm::Policy>>,
	pub csrf: Option<http::csrf::Csrf>,

	pub timeout: Option<timeout::Policy>,
	pub retry: Option<retry::Policy>,
	pub request_header_modifier: Option<filters::HeaderModifier>,
	pub response_header_modifier: Option<filters::HeaderModifier>,
	pub request_redirect: Option<filters::RequestRedirect>,
	pub url_rewrite: Option<filters::UrlRewrite>,
	pub hostname_rewrite: Option<agent::HostRedirectOverride>,
	pub request_mirror: Vec<filters::RequestMirror>,
	pub direct_response: Option<filters::DirectResponse>,
	pub cors: Option<http::cors::Cors>,
}

#[derive(Debug, Default)]
pub struct GatewayPolicies {
	pub ext_proc: Option<ext_proc::ExtProc>,
	pub oidc: Option<oidc::OidcPolicy>,
	pub jwt: Option<JwtAuthentication>,
	pub ext_authz: Option<ext_authz::ExtAuthz>,
	pub transformation: Option<http::transformation_cel::Transformation>,
	pub basic_auth: Option<http::basicauth::BasicAuthentication>,
	pub api_key: Option<http::apikey::APIKeyAuthentication>,
}

impl GatewayPolicies {
	pub fn register_cel_expressions(&self, ctx: &mut ContextBuilder) {
		if let Some(xfm) = &self.transformation {
			for expr in xfm.expressions() {
				ctx.register_expression(expr)
			}
		}

		if let Some(extauthz) = &self.ext_authz {
			for expr in extauthz.expressions() {
				ctx.register_expression(expr)
			}
		}

		if let Some(extproc) = &self.ext_proc {
			for expr in extproc.expressions() {
				ctx.register_expression(expr);
			}
		}
	}
}

impl RoutePolicies {
	pub fn register_cel_expressions(&self, ctx: &mut ContextBuilder) {
		if let Some(xfm) = &self.transformation {
			for expr in xfm.expressions() {
				ctx.register_expression(expr)
			}
		};
		if let Some(rrl) = &self.remote_rate_limit {
			for expr in rrl.expressions() {
				ctx.register_expression(expr)
			}
		};
		if let Some(rrl) = &self.authorization {
			rrl.register(ctx)
		};
		if let Some(extauthz) = &self.ext_authz {
			for expr in extauthz.expressions() {
				ctx.register_expression(expr)
			}
		}
		if let Some(extproc) = &self.ext_proc {
			for expr in extproc.expressions() {
				ctx.register_expression(expr);
			}
		}
	}
}

impl From<RoutePolicies> for LLMRequestPolicies {
	fn from(value: RoutePolicies) -> Self {
		LLMRequestPolicies {
			remote_rate_limit: value.remote_rate_limit.clone(),
			local_rate_limit: value
				.local_rate_limit
				.iter()
				.filter(|r| r.spec.limit_type == http::localratelimit::RateLimitType::Tokens)
				.cloned()
				.collect(),
			llm: value.llm.clone(),
		}
	}
}

#[derive(Debug, Default, Clone)]
pub struct LLMRequestPolicies {
	pub local_rate_limit: Vec<http::localratelimit::RateLimit>,
	pub remote_rate_limit: Option<http::remoteratelimit::RemoteRateLimit>,
	pub llm: Option<Arc<llm::Policy>>,
}

impl LLMRequestPolicies {
	pub fn merge_backend_policies(
		self: Arc<Self>,
		be: Option<Arc<llm::Policy>>,
	) -> Arc<LLMRequestPolicies> {
		let Some(be) = be else { return self };
		let mut route_policies = Arc::unwrap_or_clone(self);
		let Some(re) = route_policies.llm.take() else {
			route_policies.llm = Some(be);
			return Arc::new(route_policies);
		};

		// Backend aliases replace route aliases entirely (consistent with defaults/overrides)
		let (merged_aliases, merged_wildcard_patterns) = if be.model_aliases.is_empty() {
			(re.model_aliases.clone(), Arc::clone(&re.wildcard_patterns))
		} else {
			(be.model_aliases.clone(), Arc::clone(&be.wildcard_patterns))
		};

		route_policies.llm = Some(Arc::new(llm::Policy {
			prompt_guard: be.prompt_guard.clone().or_else(|| re.prompt_guard.clone()),
			defaults: be.defaults.clone().or_else(|| re.defaults.clone()),
			overrides: be.overrides.clone().or_else(|| re.overrides.clone()),
			transformations: be
				.transformations
				.clone()
				.or_else(|| re.transformations.clone()),
			prompts: be.prompts.clone().or_else(|| re.prompts.clone()),
			model_aliases: merged_aliases,
			wildcard_patterns: merged_wildcard_patterns,
			prompt_caching: be
				.prompt_caching
				.clone()
				.or_else(|| re.prompt_caching.clone()),
			routes: if be.routes.is_empty() {
				re.routes.clone()
			} else {
				be.routes.clone()
			},
		}));
		Arc::new(route_policies)
	}
}

#[derive(Debug, Default)]
pub struct LLMResponsePolicies {
	pub local_rate_limit: Vec<http::localratelimit::RateLimit>,
	pub remote_rate_limit: Option<http::remoteratelimit::LLMResponseAmend>,
	pub prompt_guard: Vec<ResponseGuard>,
}

impl Default for Store {
	fn default() -> Self {
		Self::with_ipv6_enabled(true)
	}
}

// RoutePath describes the objects traversed to reach the given route.
#[derive(Debug, Clone)]
pub struct RoutePath<'a> {
	pub listener: &'a ListenerName,
	// the originally intended service, pre-routing
	pub service: Option<&'a NamespacedHostname>,
	pub routes: Vec<&'a RouteName>,
}

impl<'a> RoutePath<'a> {
	pub fn final_route(&self) -> Option<&'a RouteName> {
		self.routes.last().copied()
	}
}

impl Store {
	fn bind_listener_single(address: std::net::SocketAddr) -> anyhow::Result<StdTcpListener> {
		let listener =
			StdTcpListener::bind(address).with_context(|| format!("bind listener for {address}"))?;
		listener
			.set_nonblocking(true)
			.with_context(|| format!("set nonblocking on {address}"))?;
		Ok(listener)
	}

	fn bind_listener_per_core(
		core_ids: &[core_affinity::CoreId],
		address: std::net::SocketAddr,
	) -> anyhow::Result<HashMap<core_affinity::CoreId, StdTcpListener>> {
		let domain = if address.is_ipv4() {
			socket2::Domain::IPV4
		} else {
			socket2::Domain::IPV6
		};
		let mut listeners = HashMap::with_capacity(core_ids.len());
		for &core_id in core_ids {
			let socket = socket2::Socket::new(domain, socket2::Type::STREAM, None)
				.with_context(|| format!("create listener for {address} on core {}", core_id.id))?;
			#[cfg(target_family = "unix")]
			socket.set_reuse_port(true)?;
			socket
				.bind(&address.into())
				.with_context(|| format!("bind listener for {address} on core {}", core_id.id))?;
			socket
				.listen(1024)
				.with_context(|| format!("listen on {address} on core {}", core_id.id))?;
			let listener: StdTcpListener = socket.into();
			listener
				.set_nonblocking(true)
				.with_context(|| format!("set nonblocking on {address} on core {}", core_id.id))?;
			listeners.insert(core_id, listener);
		}
		Ok(listeners)
	}

	fn bind_listeners(&self, address: std::net::SocketAddr) -> anyhow::Result<BindListeners> {
		match self.core_ids.as_deref() {
			Some(core_ids) => Ok(BindListeners::PerCore(Self::bind_listener_per_core(
				core_ids, address,
			)?)),
			None => Ok(BindListeners::Single(Self::bind_listener_single(address)?)),
		}
	}

	pub fn with_ipv6_enabled(ipv6_enabled: bool) -> Self {
		Self::new(ipv6_enabled, crate::ThreadingMode::Multithreaded)
	}

	pub fn new(ipv6_enabled: bool, threading_mode: crate::ThreadingMode) -> Self {
		let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
		Self {
			ipv6_enabled,
			core_ids: match threading_mode {
				crate::ThreadingMode::Multithreaded => None,
				crate::ThreadingMode::ThreadPerCore => {
					Some(core_affinity::get_core_ids().unwrap_or_default())
				},
			},
			binds: Default::default(),
			resources: Default::default(),
			policies_by_key: Default::default(),
			policies_by_target: Default::default(),
			backends: Default::default(),
			pending_listeners: Default::default(),
			http_routes: Default::default(),
			tcp_routes: Default::default(),
			tx,
			rx: Some(rx),
		}
	}

	fn listener_target_ref(listener: &ListenerKey) -> RouteTargetRef<'_> {
		RouteTargetRef::Listener(listener.as_str())
	}

	fn service_target_ref(service: &NamespacedHostname) -> RouteTargetRef<'_> {
		RouteTargetRef::Service {
			namespace: service.namespace.as_str(),
			hostname: service.hostname.as_str(),
		}
	}

	fn route_group_target_ref(route_group: &RouteGroupKey) -> RouteTargetRef<'_> {
		RouteTargetRef::RouteGroup(route_group.as_str())
	}

	pub fn get_listener_routes(&self, listener: &ListenerKey) -> Option<Arc<RouteSet>> {
		self
			.http_routes
			.get(&Self::listener_target_ref(listener))
			.cloned()
	}

	pub fn get_listener_tcp_routes(&self, listener: &ListenerKey) -> Option<Arc<TCPRouteSet>> {
		self
			.tcp_routes
			.get(&Self::listener_target_ref(listener))
			.cloned()
	}

	fn insert_http_route_target(&mut self, target: RouteTarget, route: Route) {
		let routes = self
			.http_routes
			.entry(target)
			.or_insert_with(|| Arc::new(RouteSet::default()));
		Arc::make_mut(routes).insert(route);
	}

	fn insert_tcp_route_target(&mut self, target: RouteTarget, route: TCPRoute) {
		let routes = self
			.tcp_routes
			.entry(target)
			.or_insert_with(|| Arc::new(TCPRouteSet::default()));
		Arc::make_mut(routes).insert(route);
	}

	fn upsert_bind(&mut self, key: BindKey, mut bind: Bind) {
		debug!(bind=%bind.key, "insert bind");

		for (_, listener) in self
			.pending_listeners
			.remove(&bind.key)
			.into_iter()
			.flatten()
		{
			debug!("adding pending listener {} to {}", listener.key, bind.key);
			bind.listeners.insert(listener);
		}

		let listeners = if self.binds.contains_key(&key) {
			None
		} else {
			match self.bind_listeners(bind.address) {
				Ok(listeners) => Some(listeners),
				Err(err) => {
					warn!(bind=%key, address=%bind.address, error=%err, "failed to start bind listener");
					None
				},
			}
		};
		self.binds.insert(key.clone(), Arc::new(bind.clone()));
		if let Some(listeners) = listeners {
			let _ = self.tx.send(BindEvent::Add(bind, listeners));
		}
	}

	pub fn subscribe(&mut self) -> impl Stream<Item = BindEvent> + use<> {
		let sub = self.rx.take().expect("bind subscriber already taken");
		tokio_stream::wrappers::UnboundedReceiverStream::new(sub)
	}

	pub fn route_policies(&self, path: &RoutePath<'_>, inline: &[&[TrafficPolicy]]) -> RoutePolicies {
		let listener = &path.listener;
		let gateway = self
			.policies_by_target
			.get(&listener.as_gateway_target_ref());
		let listener = self
			.policies_by_target
			.get(&listener.as_listener_target_ref());
		let service = path
			.service
			.and_then(|s| self.policies_by_target.get(&s.as_policy_target_ref()));

		let mut route_rules = Vec::new();
		for (idx, route) in path.routes.iter().enumerate().rev() {
			route_rules.extend(inline.get(idx).copied().unwrap_or_default().iter());
			route_rules.extend(
				self
					.policies_by_target
					.get(&route.as_route_rule_target_ref())
					.into_iter()
					.flatten()
					.filter_map(|n| self.policies_by_key.get(n))
					.filter_map(|p| p.policy.as_traffic_route_phase()),
			);
			route_rules.extend(
				self
					.policies_by_target
					.get(&route.as_route_target_ref())
					.into_iter()
					.flatten()
					.filter_map(|n| self.policies_by_key.get(n))
					.filter_map(|p| p.policy.as_traffic_route_phase()),
			);
		}

		let shared_rules = service
			.iter()
			.copied()
			.flatten()
			.chain(listener.iter().copied().flatten())
			.chain(gateway.iter().copied().flatten())
			.filter_map(|n| self.policies_by_key.get(n))
			.filter_map(|p| p.policy.as_traffic_route_phase());

		let rules = route_rules.into_iter().chain(shared_rules);

		let mut authz = Vec::new();
		let mut pol = RoutePolicies::default();
		for rule in rules {
			match &rule {
				TrafficPolicy::LocalRateLimit(p) => {
					if pol.local_rate_limit.is_empty() {
						pol.local_rate_limit = p.clone();
					}
				},
				TrafficPolicy::ExtAuthz(p) => {
					pol.ext_authz.get_or_insert_with(|| p.clone());
				},
				TrafficPolicy::ExtProc(p) => {
					pol.ext_proc.get_or_insert_with(|| p.clone());
				},
				TrafficPolicy::RemoteRateLimit(p) => {
					pol.remote_rate_limit.get_or_insert_with(|| p.clone());
				},
				TrafficPolicy::JwtAuth(p) => {
					pol.jwt.get_or_insert_with(|| p.clone());
				},
				TrafficPolicy::Oidc(p) => {
					pol.oidc.get_or_insert_with(|| p.clone());
				},
				TrafficPolicy::BasicAuth(p) => {
					pol.basic_auth.get_or_insert_with(|| p.clone());
				},
				TrafficPolicy::APIKey(p) => {
					pol.api_key.get_or_insert_with(|| p.clone());
				},
				TrafficPolicy::Transformation(p) => {
					pol.transformation.get_or_insert_with(|| p.clone());
				},
				TrafficPolicy::Authorization(p) => {
					// Authorization policies merge, unlike others
					authz.push(p.clone().0);
				},
				TrafficPolicy::AI(p) => {
					pol.llm.get_or_insert_with(|| p.clone());
				},
				TrafficPolicy::Csrf(p) => {
					pol.csrf.get_or_insert_with(|| p.clone());
				},

				TrafficPolicy::Timeout(p) => {
					pol.timeout.get_or_insert_with(|| p.clone());
				},
				TrafficPolicy::Retry(p) => {
					pol.retry.get_or_insert_with(|| p.clone());
				},
				TrafficPolicy::RequestHeaderModifier(p) => {
					pol.request_header_modifier.get_or_insert_with(|| p.clone());
				},
				TrafficPolicy::ResponseHeaderModifier(p) => {
					pol
						.response_header_modifier
						.get_or_insert_with(|| p.clone());
				},
				TrafficPolicy::RequestRedirect(p) => {
					pol.request_redirect.get_or_insert_with(|| p.clone());
				},
				TrafficPolicy::UrlRewrite(p) => {
					pol.url_rewrite.get_or_insert_with(|| p.clone());
				},
				TrafficPolicy::HostRewrite(p) => {
					pol.hostname_rewrite.get_or_insert(*p);
				},
				TrafficPolicy::RequestMirror(p) => {
					if pol.request_mirror.is_empty() {
						pol.request_mirror = p.clone();
					}
				},
				TrafficPolicy::DirectResponse(p) => {
					pol.direct_response.get_or_insert_with(|| p.clone());
				},
				TrafficPolicy::CORS(p) => {
					pol.cors.get_or_insert_with(|| p.clone());
				},
			}
		}
		if !authz.is_empty() {
			pol.authorization = Some(HTTPAuthorizationSet::new(authz.into()));
		}

		pol
	}

	pub fn gateway_policies(&self, name: &ListenerName) -> GatewayPolicies {
		let gateway = self.policies_by_target.get(&name.as_gateway_target_ref());
		let listener = self.policies_by_target.get(&name.as_listener_target_ref());
		let rules = listener
			.iter()
			.copied()
			.flatten()
			.chain(gateway.iter().copied().flatten())
			.filter_map(|n| self.policies_by_key.get(n))
			.filter_map(|p| p.policy.as_traffic_gateway_phase());

		let mut pol = GatewayPolicies::default();
		for rule in rules {
			match &rule {
				TrafficPolicy::Oidc(p) => {
					pol.oidc.get_or_insert_with(|| p.clone());
				},
				TrafficPolicy::JwtAuth(p) => {
					pol.jwt.get_or_insert_with(|| p.clone());
				},
				TrafficPolicy::BasicAuth(p) => {
					pol.basic_auth.get_or_insert_with(|| p.clone());
				},
				TrafficPolicy::APIKey(p) => {
					pol.api_key.get_or_insert_with(|| p.clone());
				},
				TrafficPolicy::ExtAuthz(p) => {
					pol.ext_authz.get_or_insert_with(|| p.clone());
				},
				TrafficPolicy::ExtProc(p) => {
					pol.ext_proc.get_or_insert_with(|| p.clone());
				},
				TrafficPolicy::Transformation(p) => {
					pol.transformation.get_or_insert_with(|| p.clone());
				},
				other => {
					warn!("unexpected gateway policy: {:?}", other);
				},
			}
		}

		pol
	}

	// sub_backend_policies looks up the sub-backends policies. Generally, these will be queried separately
	// from the primary backend policies and then merged, just due to the lifecycle of when the sub-backend
	// is selected.
	pub fn sub_backend_policies(
		&self,
		sub_backend: BackendTargetRef,
		inline_policies: Option<&[BackendPolicy]>,
	) -> BackendPolicies {
		self.internal_backend_policies(
			None,
			Some(sub_backend),
			if let Some(s) = &inline_policies {
				std::slice::from_ref(s)
			} else {
				&[]
			},
			None,
			&[],
		)
	}

	// inline_backend_policies flattens out a list of inline policies,
	pub fn inline_backend_policies(&self, inline_policies: &[BackendPolicy]) -> BackendPolicies {
		self.internal_backend_policies(
			None,
			None,
			std::slice::from_ref(&inline_policies),
			None,
			&[],
		)
	}

	pub fn backend_policies(
		&self,
		backend: BackendTargetRef,
		inline_policies: &[&[BackendPolicy]],
		path: Option<RoutePath>,
	) -> BackendPolicies {
		self.internal_backend_policies(
			Some(backend.strip_section()),
			Some(backend.clone()),
			inline_policies,
			path.as_ref().map(|p| p.listener),
			path.as_ref().map(|p| p.routes.as_slice()).unwrap_or(&[]),
		)
	}

	#[allow(clippy::too_many_arguments)]
	fn internal_backend_policies(
		&self,
		// backend with section stripped, always
		backend: Option<BackendTargetRef>,
		// backend with section retained.
		// Note this differs from other types, where just one is passed in and we strip them
		sub_backend: Option<BackendTargetRef>,
		inline_policies: &[&[BackendPolicy]],
		gateway: Option<&ListenerName>,
		routes: &[&RouteName],
	) -> BackendPolicies {
		let backend_rules =
			backend.and_then(|t| self.policies_by_target.get(&PolicyTargetRef::Backend(t)));
		let sub_backend_rules =
			sub_backend.and_then(|t| self.policies_by_target.get(&PolicyTargetRef::Backend(t)));
		let listener_rules =
			gateway.and_then(|t| self.policies_by_target.get(&t.as_listener_target_ref()));
		let gateway_rules =
			gateway.and_then(|t| self.policies_by_target.get(&t.as_gateway_target_ref()));

		// Collect route policies across the full delegation chain, child (most specific) first.
		// For each route: rule-level before route-level, matching route_policies() ordering.
		let mut route_based_keys: Vec<&PolicyKey> = Vec::new();
		for route in routes.iter().rev() {
			if let Some(keys) = self
				.policies_by_target
				.get(&route.as_route_rule_target_ref())
			{
				route_based_keys.extend(keys.iter());
			}
			if let Some(keys) = self.policies_by_target.get(&route.as_route_target_ref()) {
				route_based_keys.extend(keys.iter());
			}
		}

		// Route chain (child→parent) > SubBackend > Backend/Service > Gateway
		let rules = route_based_keys
			.into_iter()
			.chain(sub_backend_rules.iter().copied().flatten())
			.chain(backend_rules.iter().copied().flatten())
			.chain(listener_rules.iter().copied().flatten())
			.chain(gateway_rules.iter().copied().flatten())
			.unique()
			.filter_map(|n| self.policies_by_key.get(n))
			.filter_map(|p| p.policy.as_backend());
		let rules = inline_policies
			.iter()
			.rev()
			.flat_map(|p| p.iter())
			.chain(rules);

		let mut mcp_authz = Vec::new();
		let mut pol = BackendPolicies::default();
		for rule in rules {
			match &rule {
				BackendPolicy::A2a(p) => {
					pol.a2a.get_or_insert_with(|| p.clone());
				},
				BackendPolicy::BackendTLS(p) => {
					pol.backend_tls.get_or_insert_with(|| p.clone());
				},
				BackendPolicy::BackendAuth(p) => {
					pol.backend_auth.get_or_insert_with(|| p.clone());
				},
				BackendPolicy::InferenceRouting(p) => {
					pol.inference_routing.get_or_insert_with(|| p.clone());
				},
				BackendPolicy::AI(p) => {
					pol.llm.get_or_insert_with(|| p.clone());
				},

				BackendPolicy::HTTP(p) => {
					pol.http.get_or_insert_with(|| p.clone());
				},
				BackendPolicy::TCP(p) => {
					pol.tcp.get_or_insert_with(|| p.clone());
				},
				BackendPolicy::Tunnel(p) => {
					pol.tunnel.get_or_insert_with(|| p.clone());
				},

				BackendPolicy::RequestHeaderModifier(p) => {
					pol.request_header_modifier.get_or_insert_with(|| p.clone());
				},
				BackendPolicy::ResponseHeaderModifier(p) => {
					pol
						.response_header_modifier
						.get_or_insert_with(|| p.clone());
				},
				BackendPolicy::RequestRedirect(p) => {
					pol.request_redirect.get_or_insert_with(|| p.clone());
				},
				BackendPolicy::Transformation(p) => {
					pol.transformation.get_or_insert_with(|| p.clone());
				},
				BackendPolicy::SessionPersistence(p) => {
					pol.session_persistence.get_or_insert_with(|| p.clone());
				},
				BackendPolicy::Health(p) => {
					pol.health.get_or_insert_with(|| p.clone());
				},
				BackendPolicy::RequestMirror(p) => {
					if pol.request_mirror.is_empty() {
						pol.request_mirror = p.clone();
					}
				},
				BackendPolicy::McpAuthorization(p) => {
					// Authorization policies merge, unlike others
					mcp_authz.push(p.clone().into_inner());
				},
				BackendPolicy::McpAuthentication(p) => {
					pol.mcp_authentication.get_or_insert_with(|| p.clone());
				},
			}
		}
		if !mcp_authz.is_empty() {
			pol.mcp_authorization = Some(McpAuthorizationSet::new(mcp_authz.into()));
		}
		pol
	}

	pub fn all_shutdown_policies(&self) -> Vec<Box<dyn FnOnce() + Send + Sync + 'static>> {
		type ShutdownPolicy = Box<dyn FnOnce() + Send + Sync + 'static>;

		self
			.policies_by_key
			.values()
			.filter_map(|v| v.policy.as_frontend())
			.filter_map(|v| match v {
				FrontendPolicy::Tracing(t) => {
					let tracer_policy = Arc::clone(t);
					Some(Box::new(move || {
						if let Some(t) = tracer_policy.tracer.get() {
							t.shutdown()
						}
					}) as ShutdownPolicy)
				},
				FrontendPolicy::AccessLog(t) => {
					let access_log_policy = t.access_log_policy.clone();
					Some(Box::new(move || {
						if let Some(t) = access_log_policy.as_ref().and_then(|l| l.logger.get()) {
							t.shutdown()
						}
					}) as ShutdownPolicy)
				},
				_ => None,
			})
			.collect_vec()
	}

	pub fn frontend_policies(&self, gateway: PolicyTargetRef) -> FrontendPolices {
		let gw_rules = self.policies_by_target.get(&gateway);
		let rules = gw_rules
			.iter()
			.copied()
			.flatten()
			.filter_map(|n| self.policies_by_key.get(n))
			.filter_map(|p| p.policy.as_frontend());

		let mut pol = FrontendPolices::default();
		rules.for_each(|r| pol.set_if_empty(r));
		pol
	}
	pub fn listener_frontend_policies(
		&self,
		name: &ListenerName,
		service: Option<PolicyTargetRef>,
	) -> FrontendPolices {
		let gateway = self.policies_by_target.get(&name.as_gateway_target_ref());
		let listener = self.policies_by_target.get(&name.as_listener_target_ref());
		let svc = service.and_then(|s| self.policies_by_target.get(&s));
		let rules = svc
			.iter()
			.copied()
			.flatten()
			.chain(listener.iter().copied().flatten())
			.chain(gateway.iter().copied().flatten())
			.filter_map(|n| self.policies_by_key.get(n))
			.filter_map(|p| p.policy.as_frontend());
		let mut pol = FrontendPolices::default();
		rules.for_each(|r| pol.set_if_empty(r));
		pol
	}

	pub fn bind(&self, bind: &BindKey) -> Option<Arc<Bind>> {
		self.binds.get(bind).cloned()
	}

	/// find_bind looks up a bind by address. Typically, this is done by the kernel for us, but in some cases
	/// we do userspace routing to a bind.
	pub fn find_bind(&self, want: SocketAddr) -> Option<Arc<Bind>> {
		self
			.binds
			.values()
			.find(|b| {
				let have = b.address;
				if have.ip().is_unspecified() {
					have.port() == want.port()
				} else {
					have == want
				}
			})
			.cloned()
	}

	pub fn all_policies(&self) -> Vec<Arc<TargetedPolicy>> {
		self.policies_by_key.values().cloned().collect()
	}

	pub fn backend(&self, r: &BackendKey) -> Option<Arc<BackendWithPolicies>> {
		self.backends.get(r).cloned()
	}

	#[instrument(
        level = Level::INFO,
        name="remove_bind",
        skip_all,
        fields(bind),
    )]
	pub fn remove_bind(&mut self, bind: BindKey) {
		self.binds.remove(&bind);
		let _ = self.tx.send(BindEvent::Remove(bind));
	}
	#[instrument(
        level = Level::INFO,
        name="remove_policy",
        skip_all,
        fields(bind),
    )]
	pub fn remove_policy(&mut self, pol: PolicyKey) {
		if let Some(old) = self.policies_by_key.remove(&pol)
			&& let Some(o) = self.policies_by_target.get_mut(&old.target)
		{
			o.remove(&pol);
		}
	}
	#[instrument(
        level = Level::INFO,
        name="remove_backend",
        skip_all,
        fields(bind),
    )]
	pub fn remove_backend(&mut self, backend: BackendKey) {
		self.backends.remove(&backend);
	}

	#[instrument(
        level = Level::INFO,
        name="remove_listener",
        skip_all,
        fields(listener),
    )]
	pub fn remove_listener(&mut self, listener: ListenerKey) {
		let Some((bind_key, bind)) = self.binds.iter().find_map(|(bind_key, bind)| {
			bind
				.listeners
				.contains(&listener)
				.then(|| (bind_key.clone(), bind.clone()))
		}) else {
			return;
		};
		let mut bind = Arc::unwrap_or_clone(bind);
		bind.listeners.remove(&listener);
		self.upsert_bind(bind_key, bind);
	}

	pub fn remove_route_group(&mut self, rg: RouteGroupKey) {
		self.http_routes.remove(&Self::route_group_target_ref(&rg));
	}

	pub fn lookup_route_group(&self, route: &RouteGroupKey) -> Option<Arc<RouteSet>> {
		self
			.http_routes
			.get(&Self::route_group_target_ref(route))
			.cloned()
	}

	fn remove_http_route(&mut self, route_key: &RouteKey) -> bool {
		let mut found = false;
		self.http_routes.retain(|_target, route_set| {
			if route_set.contains(route_key) {
				Arc::make_mut(route_set).remove(route_key);
				found = true;
			}
			!route_set.is_empty()
		});
		found
	}

	fn remove_tcp_route_from_targets(&mut self, route_key: &RouteKey) -> bool {
		let mut found = false;
		self.tcp_routes.retain(|_target, route_set| {
			if route_set.contains(route_key) {
				Arc::make_mut(route_set).remove(route_key);
				found = true;
			}
			!route_set.is_empty()
		});
		found
	}

	#[instrument(
        level = Level::INFO,
        name="remove_route",
        skip_all,
        fields(route),
    )]
	pub fn remove_route(&mut self, route: RouteKey) {
		self.remove_http_route(&route);
	}

	#[instrument(
        level = Level::INFO,
        name="remove_tcp_route",
        skip_all,
        fields(tcp_route),
    )]
	pub fn remove_tcp_route(&mut self, tcp_route: RouteKey) {
		self.remove_tcp_route_from_targets(&tcp_route);
	}

	#[instrument(
        level = Level::INFO,
        name="insert_bind",
        skip_all,
        fields(bind=%bind.key),
    )]
	pub fn insert_bind(&mut self, bind: Bind) {
		let key = bind.key.clone();
		self.upsert_bind(key, bind);
	}

	pub fn insert_backend(&mut self, key: BackendKey, b: BackendWithPolicies) {
		if let Backend::AI(_, t) = &b.backend
			&& t.providers.any(|p| p.tokenize)
		{
			preload_tokenizers()
		}
		let arc = Arc::new(b);
		self.backends.insert(key, arc);
	}

	pub fn insert_policy(&mut self, pol: TargetedPolicy) {
		let pol = Arc::new(pol);
		if let Some(old) = self.policies_by_key.insert(pol.key.clone(), pol.clone()) {
			// Remove the old target. We may add it back, though.
			if let Some(o) = self.policies_by_target.get_mut(&old.target) {
				o.remove(&pol.key);
			}
		}
		self
			.policies_by_target
			.entry(pol.target.clone())
			.or_default()
			.insert(pol.key.clone());
	}

	pub fn insert_listener(&mut self, lis: Listener, bind_name: BindKey) {
		debug!(listener=%lis.key,bind=%bind_name, "insert listener");
		if let Some(b) = self.binds.get(&bind_name) {
			let mut bind = Arc::unwrap_or_clone(b.clone());
			bind.listeners.remove(&lis.key);
			bind.listeners.insert(lis);
			self.upsert_bind(bind_name, bind);
		} else {
			debug!("no bind found, keeping listener pending");
			self
				.pending_listeners
				.entry(bind_name)
				.or_default()
				.insert(lis.key.clone(), lis);
		}
	}

	pub fn insert_route_into_group(&mut self, r: Route, ln: RouteGroupKey) {
		debug!(group=%ln, route=%r.key, "insert route");
		self.insert_http_route_target(RouteTarget::RouteGroup(ln), r);
	}

	pub fn insert_route(&mut self, r: Route, ln: ListenerKey) {
		debug!(listener=%ln, route=%r.key, "insert route");
		self.insert_http_route_target(RouteTarget::Listener(ln), r);
	}

	pub fn insert_tcp_route(&mut self, r: TCPRoute, ln: ListenerKey) {
		debug!(listener=%ln,route=%r.key, "insert tcp route");
		self.insert_tcp_route_target(RouteTarget::Listener(ln), r);
	}

	pub fn insert_service_route(&mut self, r: Route, service_key: NamespacedHostname) {
		debug!(service=%service_key, route=%r.key, "insert service route");
		self.insert_http_route_target(RouteTarget::Service(service_key), r);
	}

	pub fn insert_service_tcp_route(&mut self, r: TCPRoute, service_key: NamespacedHostname) {
		debug!(service=%service_key, route=%r.key, "insert service tcp route");
		self.insert_tcp_route_target(RouteTarget::Service(service_key), r);
	}

	pub fn get_service_routes(&self, key: &NamespacedHostname) -> Option<Arc<RouteSet>> {
		self
			.http_routes
			.get(&Self::service_target_ref(key))
			.cloned()
	}

	pub fn get_service_tcp_routes(&self, key: &NamespacedHostname) -> Option<Arc<TCPRouteSet>> {
		self.tcp_routes.get(&Self::service_target_ref(key)).cloned()
	}

	fn remove_resource(&mut self, res: &Strng) -> anyhow::Result<()> {
		trace!("removing res {res}...");
		let Some(old) = self.resources.remove(res) else {
			debug!("unknown resource name {res}");
			return Ok(());
		};
		match old {
			ResourceKind::Policy(n) => self.remove_policy(n),
			ResourceKind::Bind(n) => self.remove_bind(n),
			ResourceKind::Route(n) => self.remove_route(n),
			ResourceKind::TcpRoute(n) => self.remove_tcp_route(n),
			ResourceKind::Listener(n) => self.remove_listener(n),
			ResourceKind::Backend(n) => self.remove_backend(n),
		}
		Ok(())
	}

	fn insert_xds(&mut self, name: Strng, res: ADPResource) -> anyhow::Result<()> {
		trace!(%name, "insert resource {res:?}");
		match res.kind {
			Some(XdsKind::Bind(w)) => {
				self
					.resources
					.insert(name, ResourceKind::Bind(strng::new(&w.key)));
				self.insert_xds_bind(w)
			},
			Some(XdsKind::Listener(w)) => {
				self
					.resources
					.insert(name, ResourceKind::Listener(strng::new(&w.key)));
				self.insert_xds_listener(w)
			},
			Some(XdsKind::Route(w)) => {
				self
					.resources
					.insert(name, ResourceKind::Route(strng::new(&w.key)));
				self.insert_xds_route(w)
			},
			Some(XdsKind::TcpRoute(w)) => {
				self
					.resources
					.insert(name, ResourceKind::TcpRoute(strng::new(&w.key)));
				self.insert_xds_tcp_route(w)
			},
			Some(XdsKind::Backend(w)) => {
				self
					.resources
					.insert(name, ResourceKind::Backend(strng::new(&w.key)));
				self.insert_xds_backend(w)
			},
			Some(XdsKind::Policy(w)) => {
				self
					.resources
					.insert(name, ResourceKind::Policy(strng::new(&w.key)));
				self.insert_xds_policy(w)
			},
			_ => Err(anyhow::anyhow!("unknown resource type")),
		}
	}

	fn insert_xds_bind(&mut self, raw: XdsBind) -> anyhow::Result<()> {
		let mut bind = Bind::try_from_xds(&raw, self.ipv6_enabled)?;
		// If XDS server pushes the same bind twice (which it shouldn't really do, but oh well),
		// we need to copy the listeners over.
		if let Some(old) = self.binds.get(&bind.key) {
			debug!("bind update, copy old listeners over");
			bind.listeners = Arc::unwrap_or_clone(old.clone()).listeners;
		}
		self.insert_bind(bind);
		Ok(())
	}
	fn insert_xds_listener(&mut self, raw: XdsListener) -> anyhow::Result<()> {
		let (lis, bind_name) = Listener::try_from_xds(&raw)?;
		self.insert_listener(lis, bind_name);
		Ok(())
	}
	fn insert_xds_route(&mut self, raw: XdsRoute) -> anyhow::Result<()> {
		let (route, listener_name, rgk) = Route::try_from_xds(&raw)?;
		if let Some(sk) = route.service_key.clone() {
			self.insert_service_route(route, sk);
		} else if let Some(rgk) = rgk {
			self.insert_route_into_group(route, rgk);
		} else {
			self.insert_route(route, listener_name);
		}
		Ok(())
	}
	fn insert_xds_tcp_route(&mut self, raw: XdsTcpRoute) -> anyhow::Result<()> {
		let (route, listener_name) = TCPRoute::try_from_xds(&raw)?;
		if let Some(sk) = route.service_key.clone() {
			self.insert_service_tcp_route(route, sk);
			Ok(())
		} else {
			self.insert_tcp_route(route, listener_name);
			Ok(())
		}
	}
	fn insert_xds_backend(&mut self, raw: XdsBackend) -> anyhow::Result<()> {
		let key = strng::new(&raw.key);
		let backend: BackendWithPolicies = (&raw).try_into()?;
		self.insert_backend(key, backend);
		Ok(())
	}
	fn insert_xds_policy(&mut self, raw: XdsPolicy) -> anyhow::Result<()> {
		let policy: TargetedPolicy = (&raw).try_into()?;
		self.insert_policy(policy);
		Ok(())
	}
}

#[derive(Clone, Debug)]
pub struct StoreUpdater {
	state: Arc<RwLock<Store>>,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutesDump {
	http_mesh: HashMap<NamespacedHostname, RouteSet>,
	tcp_mesh: HashMap<NamespacedHostname, TCPRouteSet>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DumpListener {
	#[serde(flatten)]
	listener: Listener,
	#[serde(skip_serializing_if = "Option::is_none")]
	routes: Option<Arc<RouteSet>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	tcp_routes: Option<Arc<TCPRouteSet>>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DumpBind {
	#[serde(flatten)]
	bind: Arc<Bind>,
	listeners: BTreeMap<ListenerKey, DumpListener>,
}

#[derive(serde::Serialize)]
pub struct Dump {
	binds: Vec<DumpBind>,
	routes: RoutesDump,
	policies: Vec<Arc<TargetedPolicy>>,
	backends: Vec<Arc<BackendWithPolicies>>,
}

impl StoreUpdater {
	pub fn new(state: Arc<RwLock<Store>>) -> StoreUpdater {
		Self { state }
	}
	pub fn read(&self) -> std::sync::RwLockReadGuard<'_, Store> {
		self.state.read().expect("mutex acquired")
	}
	pub fn write(&self) -> std::sync::RwLockWriteGuard<'_, Store> {
		self.state.write().expect("mutex acquired")
	}
	pub fn dump(&self) -> Dump {
		let store = self.state.read().expect("mutex");

		// Services all have hostname, so use that as the key
		let binds: Vec<_> = store
			.binds
			.iter()
			.sorted_by_key(|k| k.0)
			.map(|(_, bind)| DumpBind {
				bind: bind.clone(),
				listeners: bind
					.listeners
					.iter()
					.map(|listener| {
						(
							listener.key.clone(),
							DumpListener {
								listener: listener.clone(),
								routes: store.get_listener_routes(&listener.key),
								tcp_routes: store.get_listener_tcp_routes(&listener.key),
							},
						)
					})
					.collect(),
			})
			.collect();
		let policies: Vec<_> = store
			.policies_by_key
			.iter()
			.sorted_by_key(|k| k.0)
			.map(|k| k.1.clone())
			.collect();
		let backends: Vec<_> = store
			.backends
			.iter()
			.sorted_by_key(|k| k.0)
			.map(|k| k.1.clone())
			.collect();
		Dump {
			binds,
			policies,
			backends,
			routes: RoutesDump {
				http_mesh: store
					.http_routes
					.iter()
					.filter_map(|(target, routes)| match target {
						RouteTarget::Service(service) => Some((service.clone(), routes.as_ref().clone())),
						_ => None,
					})
					.collect(),
				tcp_mesh: store
					.tcp_routes
					.iter()
					.filter_map(|(target, routes)| match target {
						RouteTarget::Service(service) => Some((service.clone(), routes.as_ref().clone())),
						_ => None,
					})
					.collect(),
			},
		}
	}
	#[allow(clippy::too_many_arguments)]
	pub fn sync_local(
		&self,
		binds: Vec<Bind>,
		listener_routes: Vec<(ListenerKey, Vec<Route>)>,
		listener_tcp_routes: Vec<(ListenerKey, Vec<TCPRoute>)>,
		policies: Vec<TargetedPolicy>,
		backends: Vec<BackendWithPolicies>,
		route_groups: Vec<(RouteGroupKey, Vec<Route>)>,
		prev: PreviousState,
	) -> PreviousState {
		let mut s = self.state.write().expect("mutex acquired");
		let mut old_binds = prev.binds;
		let mut old_routes = prev.routes;
		let mut old_tcp_routes = prev.tcp_routes;
		let mut old_pols = prev.policies;
		let mut old_backends = prev.backends;
		let mut old_route_groups = prev.route_groups;
		let mut next_state = PreviousState {
			binds: Default::default(),
			routes: Default::default(),
			tcp_routes: Default::default(),
			policies: Default::default(),
			backends: Default::default(),
			route_groups: Default::default(),
		};
		for b in binds {
			old_binds.remove(&b.key);
			next_state.binds.insert(b.key.clone());
			s.insert_bind(b);
		}
		for b in backends {
			// Here we use the 'name' as the key. This is appropriate for local case only
			old_backends.remove(&b.backend.name());
			next_state.backends.insert(b.backend.name());
			s.insert_backend(b.backend.name(), b);
		}
		for (listener_key, routes) in listener_routes {
			for route in routes {
				old_routes.remove(&route.key);
				next_state.routes.insert(route.key.clone());
				s.insert_route(route, listener_key.clone());
			}
		}
		for (listener_key, routes) in listener_tcp_routes {
			for route in routes {
				old_tcp_routes.remove(&route.key);
				next_state.tcp_routes.insert(route.key.clone());
				s.insert_tcp_route(route, listener_key.clone());
			}
		}
		for p in policies {
			old_pols.remove(&p.key);
			next_state.policies.insert(p.key.clone());
			s.insert_policy(p);
		}
		for (rg_key, routes) in route_groups {
			old_route_groups.remove(&rg_key);
			next_state.route_groups.insert(rg_key.clone());
			for r in routes {
				s.insert_route_into_group(r, rg_key.clone());
			}
		}
		for remaining_bind in old_binds {
			s.remove_bind(remaining_bind);
		}
		for remaining_route in old_routes {
			s.remove_route(remaining_route);
		}
		for remaining_route in old_tcp_routes {
			s.remove_tcp_route(remaining_route);
		}
		for remaining_policy in old_pols {
			s.remove_policy(remaining_policy);
		}
		for remaining_backend in old_backends {
			s.remove_backend(remaining_backend);
		}
		for remaining_rg in old_route_groups {
			s.remove_route_group(remaining_rg);
		}
		next_state
	}
}

#[derive(Clone, Debug, Default)]
pub struct PreviousState {
	pub binds: HashSet<BindKey>,
	pub routes: HashSet<RouteKey>,
	pub tcp_routes: HashSet<RouteKey>,
	pub policies: HashSet<PolicyKey>,
	pub backends: HashSet<BackendKey>,
	pub route_groups: HashSet<RouteGroupKey>,
}

impl agent_xds::Handler<ADPResource> for StoreUpdater {
	fn handle(
		&self,
		updates: Box<&mut dyn Iterator<Item = XdsUpdate<ADPResource>>>,
	) -> Result<(), Vec<RejectedConfig>> {
		let mut state = self.state.write().unwrap();
		let handle = |res: XdsUpdate<ADPResource>| {
			match res {
				XdsUpdate::Update(w) => state.insert_xds(w.name, w.resource)?,
				XdsUpdate::Remove(name) => {
					debug!("handling delete {}", name);
					state.remove_resource(&strng::new(name))?
				},
			}
			Ok(())
		};
		agent_xds::handle_single_resource(updates, handle)
	}
}

fn preload_tokenizers() {
	static INIT_TOKENIZERS: std::sync::Once = std::sync::Once::new();

	tokio::task::spawn_blocking(|| {
		INIT_TOKENIZERS.call_once(|| {
			let t0 = std::time::Instant::now();
			crate::llm::preload_tokenizers();
			info!("tokenizers loaded in {}ms", t0.elapsed().as_millis());
		});
	});
}

#[cfg(test)]
mod tests {
	use std::time::Duration;

	use frozen_collections::FzHashSet;

	use super::*;
	use crate::telemetry::log::OrderedStringMap;
	use crate::types::agent::{
		BackendTarget, BindProtocol, ListenerProtocol, ListenerSet, PolicyType, TunnelProtocol,
	};
	use crate::types::frontend::LoggingPolicy;

	fn listener() -> ListenerName {
		ListenerName {
			gateway_name: strng::literal!("gw"),
			gateway_namespace: strng::literal!("ns"),
			listener_name: strng::literal!("listener"),
			listener_set: None,
		}
	}

	fn route(name: &'static str, namespace: &'static str, kind: Option<&'static str>) -> RouteName {
		RouteName {
			name: strng::new(name),
			namespace: strng::new(namespace),
			rule_name: None,
			kind: kind.map(strng::new),
		}
	}

	fn insert_route_timeout_policy(
		store: &mut Store,
		key: &str,
		route_target: RouteName,
		request_timeout_secs: u64,
	) -> timeout::Policy {
		let policy_key: PolicyKey = strng::new(key);
		let pol = timeout::Policy {
			request_timeout: Some(Duration::from_secs(request_timeout_secs)),
			backend_request_timeout: None,
		};
		let targeted = TargetedPolicy {
			key: policy_key.clone(),
			name: None,
			target: PolicyTarget::Route(route_target.clone()),
			policy: TrafficPolicy::Timeout(pol.clone()).into(),
		};

		store
			.policies_by_key
			.insert(policy_key.clone(), Arc::new(targeted));
		store
			.policies_by_target
			.entry(PolicyTarget::Route(route_target))
			.or_default()
			.insert(policy_key);

		pol
	}

	fn create_access_log_policy(remove_item: &str) -> FrontendPolicy {
		FrontendPolicy::AccessLog(LoggingPolicy {
			filter: None,
			add: Arc::new(OrderedStringMap::default()),
			remove: Arc::new(FzHashSet::new(vec![remove_item.into()])),
			otlp: None,
			access_log_policy: None,
		})
	}

	fn create_network_authorization_policy(cidr: &str) -> FrontendPolicy {
		FrontendPolicy::NetworkAuthorization(crate::types::frontend::NetworkAuthorization(
			crate::http::authorization::RuleSet::new(crate::http::authorization::PolicySet::new(
				vec![Arc::new(
					cel::Expression::new_strict(format!(r#"cidr("{cidr}").containsIP(source.address)"#))
						.unwrap(),
				)],
				vec![],
				vec![],
			)),
		))
	}

	#[test]
	fn dump_includes_listener_routes() {
		let updater = StoreUpdater::new(Arc::new(RwLock::new(Store::with_ipv6_enabled(true))));
		let bind = Bind {
			key: strng::literal!("bind"),
			address: "127.0.0.1:0".parse().unwrap(),
			protocol: BindProtocol::http,
			tunnel_protocol: TunnelProtocol::Direct,
			listeners: ListenerSet::from_list([Listener {
				key: strng::literal!("listener"),
				name: ListenerName {
					gateway_name: strng::literal!("gw"),
					gateway_namespace: strng::literal!("ns"),
					listener_name: strng::literal!("listener"),
					listener_set: None,
				},
				hostname: strng::literal!("example.com"),
				protocol: ListenerProtocol::HTTP,
			}]),
		};
		let route = Route {
			key: strng::literal!("route"),
			service_key: None,
			name: RouteName {
				name: strng::literal!("route"),
				namespace: strng::literal!("ns"),
				rule_name: None,
				kind: None,
			},
			hostnames: vec![],
			matches: vec![],
			inline_policies: vec![],
			backends: vec![],
		};

		{
			let mut store = updater.write();
			store.insert_bind(bind);
			store.insert_route(route, strng::literal!("listener"));
		}

		let dump = updater.dump();
		assert_eq!(dump.binds.len(), 1);
		let listener = dump.binds[0]
			.listeners
			.get(&strng::literal!("listener"))
			.expect("listener dump entry");
		assert!(
			listener
				.routes
				.as_ref()
				.is_some_and(|routes| routes.contains(&strng::literal!("route")))
		);
	}

	fn insert_policy_at_level(
		store: &mut Store,
		listener: &ListenerName,
		policy_name: &str,
		for_listener: bool,
		policy: FrontendPolicy,
	) {
		let policy_key = strng::new(policy_name);
		let listener_name = if for_listener {
			Some(listener.listener_name.clone())
		} else {
			None
		};
		let target = PolicyTarget::Gateway(ListenerTarget {
			gateway_name: listener.gateway_name.clone(),
			gateway_namespace: listener.gateway_namespace.clone(),
			listener_name,
		});
		let policy = TargetedPolicy {
			key: policy_key.clone(),
			name: None,
			target: target.clone(),
			policy: agent::PolicyType::Frontend(policy),
		};

		store
			.policies_by_key
			.insert(policy_key.clone(), Arc::new(policy));
		store
			.policies_by_target
			.entry(target.clone())
			.or_default()
			.insert(policy_key);
	}

	fn insert_gateway_level_frontend_policy(
		store: &mut Store,
		listener: &ListenerName,
		remove_item: &str,
	) {
		insert_policy_at_level(
			store,
			listener,
			"gw_frontend_policy",
			false,
			create_access_log_policy(remove_item),
		);
	}

	fn insert_listener_level_frontend_policy(
		store: &mut Store,
		listener: &ListenerName,
		remove_item: &str,
	) {
		insert_policy_at_level(
			store,
			listener,
			"listener_frontend_policy",
			true,
			create_access_log_policy(remove_item),
		);
	}

	fn insert_gateway_level_network_authorization_policy(
		store: &mut Store,
		listener: &ListenerName,
		policy_name: &str,
		cidr: &str,
	) {
		insert_policy_at_level(
			store,
			listener,
			policy_name,
			false,
			create_network_authorization_policy(cidr),
		);
	}

	#[test]
	fn route_policies_are_kind_scoped() {
		let mut store = Store::default();
		let listener = listener();

		let http_route = route("r", "ns", Some("HTTPRoute"));
		let grpc_route = route("r", "ns", Some("GRPCRoute"));

		let http_timeout = insert_route_timeout_policy(&mut store, "p-http", http_route.clone(), 1);
		let grpc_timeout = insert_route_timeout_policy(&mut store, "p-grpc", grpc_route.clone(), 2);

		let http_pols = store.route_policies(
			&RoutePath {
				listener: &listener,
				service: None,
				routes: vec![&http_route],
			},
			&[],
		);
		assert_eq!(http_pols.timeout, Some(http_timeout));

		let grpc_pols = store.route_policies(
			&RoutePath {
				listener: &listener,
				service: None,
				routes: vec![&grpc_route],
			},
			&[],
		);
		assert_eq!(grpc_pols.timeout, Some(grpc_timeout));
	}

	#[test]
	fn route_policies_give_precedence_to_later_routes_in_path() {
		let mut store = Store::default();
		let listener = listener();
		let parent_route = route("parent", "ns", Some("HTTPRoute"));
		let child_route = route("child", "ns", Some("HTTPRoute"));

		let parent_timeout =
			insert_route_timeout_policy(&mut store, "p-parent", parent_route.clone(), 1);
		let child_timeout = insert_route_timeout_policy(&mut store, "p-child", child_route.clone(), 2);

		let pols = store.route_policies(
			&RoutePath {
				listener: &listener,
				service: None,
				routes: vec![&parent_route, &child_route],
			},
			&[],
		);

		assert_ne!(parent_timeout, child_timeout);
		assert_eq!(pols.timeout, Some(child_timeout));
	}

	/// Tests that frontend policies at listener level take precedence over gateway level policies
	#[test]
	fn frontend_policy_listener_precedence() {
		let mut store = Store::default();
		let listener = listener();

		// Insert both gateway and listener level frontend policies
		insert_gateway_level_frontend_policy(&mut store, &listener, "gw_remove");
		insert_listener_level_frontend_policy(&mut store, &listener, "listener_remove");

		let merged_pols = store.listener_frontend_policies(&listener, None);
		// Verify that listener policy takes precedence over gateway policy
		assert!(
			merged_pols.access_log.is_some(),
			"Expected access log policy to be present"
		);

		let access_log = merged_pols.access_log.as_ref().unwrap();
		assert!(
			access_log.remove.contains("listener_remove"),
			"Expected listener policy to take precedence for remove field"
		);
		assert!(
			!access_log.remove.contains("gw_remove"),
			"Gateway policy should not override listener policy"
		);
	}

	#[test]
	fn frontend_network_authorization_policies_merge() {
		let mut store = Store::default();
		let listener = listener();
		insert_gateway_level_network_authorization_policy(
			&mut store,
			&listener,
			"gw-frontend-network-authz-1",
			"10.0.0.0/8",
		);
		insert_gateway_level_network_authorization_policy(
			&mut store,
			&listener,
			"gw-frontend-network-authz-2",
			"192.168.0.0/16",
		);

		let merged_pols = store.frontend_policies(listener.as_gateway_target_ref());
		let network_authz = merged_pols
			.network_authorization
			.as_ref()
			.expect("expected merged network authorization");

		assert!(
			network_authz
				.apply(&crate::cel::SourceContext {
					address: "10.1.2.3".parse().unwrap(),
					port: 12345,
					raw_address: "10.1.2.3".parse().unwrap(),
					raw_port: 12345,
					tls: None,
					unverified_workload: None,
				})
				.is_ok()
		);
		assert!(
			network_authz
				.apply(&crate::cel::SourceContext {
					address: "192.168.1.2".parse().unwrap(),
					port: 12345,
					raw_address: "192.168.1.2".parse().unwrap(),
					raw_port: 12345,
					tls: None,
					unverified_workload: None,
				})
				.is_ok()
		);
		assert!(
			network_authz
				.apply(&crate::cel::SourceContext {
					address: "172.16.0.1".parse().unwrap(),
					port: 12345,
					raw_address: "172.16.0.1".parse().unwrap(),
					raw_port: 12345,
					tls: None,
					unverified_workload: None,
				})
				.is_err()
		);
	}

	#[test]
	fn xds_bind_uses_ipv4_when_ipv6_disabled() {
		use std::net::{IpAddr, Ipv4Addr};

		let xds_bind = XdsBind {
			key: "test-bind".to_string(),
			port: 8080,
			protocol: 0,        // HTTP
			tunnel_protocol: 0, // Direct
		};

		let bind = Bind::try_from_xds(&xds_bind, false).unwrap();
		assert_eq!(bind.address.port(), 8080);
		assert_eq!(bind.address.ip(), IpAddr::V4(Ipv4Addr::UNSPECIFIED));
	}

	#[cfg(target_family = "unix")]
	#[test]
	fn xds_bind_uses_ipv6_when_ipv6_enabled_on_unix() {
		use std::net::{IpAddr, Ipv6Addr};

		let xds_bind = XdsBind {
			key: "test-bind".to_string(),
			port: 9090,
			protocol: 0,        // HTTP
			tunnel_protocol: 0, // Direct
		};

		let bind = Bind::try_from_xds(&xds_bind, true).unwrap();
		assert_eq!(bind.address.port(), 9090);
		assert_eq!(bind.address.ip(), IpAddr::V6(Ipv6Addr::UNSPECIFIED));
	}

	/// Tests backend policy merging precedence:
	/// Inline policies > Attached policies (with SubBackend > Backend among attached)
	#[test]
	fn backend_policy_merging_precedence() {
		use crate::http::filters::HeaderModifier;

		let mut store = Store::default();

		// Create backend-attached policy - sets x-foo=bar
		let backend_attached_policy_key: PolicyKey = strng::new("backend-attached-policy");
		let backend_attached_policy = TargetedPolicy {
			key: backend_attached_policy_key.clone(),
			name: None,
			target: PolicyTarget::Backend(BackendTarget::Backend {
				name: strng::new("test-backend"),
				namespace: strng::new("test-ns"),
				section: None,
			}),
			policy: PolicyType::Backend(BackendPolicy::RequestHeaderModifier(HeaderModifier {
				add: vec![],
				set: vec![(strng::new("x-foo"), strng::new("bar"))],
				remove: vec![],
			})),
		};
		store.insert_policy(backend_attached_policy);

		// Create section-level attached policy - sets x-foo=bar3
		let section_policy_key: PolicyKey = strng::new("section-policy");
		let section_policy = TargetedPolicy {
			key: section_policy_key.clone(),
			name: None,
			target: PolicyTarget::Backend(BackendTarget::Backend {
				name: strng::new("test-backend"),
				namespace: strng::new("test-ns"),
				section: Some(strng::new("target")),
			}),
			policy: PolicyType::Backend(BackendPolicy::RequestHeaderModifier(HeaderModifier {
				add: vec![],
				set: vec![(strng::new("x-foo"), strng::new("bar3"))],
				remove: vec![],
			})),
		};
		store.insert_policy(section_policy);

		// Create inline policies - sets x-foo=bar2
		let backend_inline_policies = vec![BackendPolicy::RequestHeaderModifier(HeaderModifier {
			add: vec![],
			set: vec![(strng::new("x-foo"), strng::new("bar2"))],
			remove: vec![],
		})];

		// Test case 1: Inline policy beats backend attached policy
		let policies_no_section = store.backend_policies(
			BackendTargetRef::Backend {
				name: "test-backend",
				namespace: "test-ns",
				section: None,
			},
			&[&backend_inline_policies],
			None,
		);

		assert!(
			policies_no_section.request_header_modifier.is_some(),
			"Expected request header modifier to be present"
		);
		let modifier = policies_no_section
			.request_header_modifier
			.as_ref()
			.unwrap();
		assert_eq!(
			modifier.set.len(),
			1,
			"Expected exactly one header to be set"
		);
		assert_eq!(
			modifier.set[0],
			(strng::new("x-foo"), strng::new("bar2")),
			"Inline policy (bar2) should win over backend attached policy (bar)"
		);

		// Test case 2: Inline policy beats section attached policy
		let policies_with_section = store.backend_policies(
			BackendTargetRef::Backend {
				name: "test-backend",
				namespace: "test-ns",
				section: Some("target"),
			},
			&[&backend_inline_policies],
			None,
		);

		assert!(
			policies_with_section.request_header_modifier.is_some(),
			"Expected request header modifier to be present"
		);
		let modifier = policies_with_section
			.request_header_modifier
			.as_ref()
			.unwrap();
		assert_eq!(
			modifier.set.len(),
			1,
			"Expected exactly one header to be set"
		);
		assert_eq!(
			modifier.set[0],
			(strng::new("x-foo"), strng::new("bar2")),
			"Inline policy (bar2) should win over section attached policy (bar3)"
		);

		// Test case 3: Without inline policies, backend attached policy is used
		let policies_no_inline = store.backend_policies(
			BackendTargetRef::Backend {
				name: "test-backend",
				namespace: "test-ns",
				section: None,
			},
			&[],
			None,
		);

		assert!(
			policies_no_inline.request_header_modifier.is_some(),
			"Expected request header modifier to be present"
		);
		let modifier = policies_no_inline.request_header_modifier.as_ref().unwrap();
		assert_eq!(
			modifier.set.len(),
			1,
			"Expected exactly one header to be set"
		);
		assert_eq!(
			modifier.set[0],
			(strng::new("x-foo"), strng::new("bar")),
			"Backend attached policy (bar) should be used when no inline policies exist"
		);

		// Test case 4: Without inline policies, section attached policy beats backend attached
		let policies_section_no_inline = store.backend_policies(
			BackendTargetRef::Backend {
				name: "test-backend",
				namespace: "test-ns",
				section: Some("target"),
			},
			&[],
			None,
		);

		assert!(
			policies_section_no_inline.request_header_modifier.is_some(),
			"Expected request header modifier to be present"
		);
		let modifier = policies_section_no_inline
			.request_header_modifier
			.as_ref()
			.unwrap();
		assert_eq!(
			modifier.set.len(),
			1,
			"Expected exactly one header to be set"
		);
		assert_eq!(
			modifier.set[0],
			(strng::new("x-foo"), strng::new("bar3")),
			"Section attached policy (bar3) should win over backend attached policy (bar)"
		);
	}
}
