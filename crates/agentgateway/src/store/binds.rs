use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use agent_xds::{RejectedConfig, XdsUpdate};
use futures_core::Stream;
use itertools::Itertools;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tracing::{Level, instrument};

use crate::cel::ContextBuilder;
use crate::http::auth::BackendAuth;
use crate::http::authorization::HTTPAuthorizationSet;
use crate::http::backendtls::BackendTLS;
use crate::http::ext_proc::InferenceRouting;
use crate::http::{ext_authz, ext_proc, filters, remoteratelimit, retry, timeout};
use crate::llm::policy::ResponseGuard;
use crate::mcp::McpAuthorizationSet;
use crate::proxy::httpproxy::PolicyClient;
use crate::store::Event;
use crate::types::agent::{
	A2aPolicy, Backend, BackendKey, BackendPolicy, BackendTargetRef, BackendWithPolicies, Bind,
	BindKey, FrontendPolicy, Listener, ListenerKey, ListenerName, McpAuthentication, PolicyKey,
	PolicyTarget, Route, RouteKey, RouteName, TCPRoute, TargetedPolicy, TrafficPolicy,
};
use crate::types::proto::agent::resource::Kind as XdsKind;
use crate::types::proto::agent::{
	Backend as XdsBackend, Bind as XdsBind, Listener as XdsListener, Policy as XdsPolicy,
	Resource as ADPResource, Route as XdsRoute, TcpRoute as XdsTcpRoute,
};
use crate::types::{agent, frontend};
use crate::*;

#[derive(Debug)]
enum ResourceKind {
	Policy(PolicyKey),
	Bind(BindKey),
	Route(RouteKey),
	TcpRoute(RouteKey),
	Listener(ListenerKey),
	Backend(ListenerKey),
}

#[derive(Debug)]
pub struct Store {
	ipv6_enabled: bool,
	binds: HashMap<BindKey, Arc<Bind>>,
	resources: HashMap<Strng, ResourceKind>,

	policies_by_key: HashMap<PolicyKey, Arc<TargetedPolicy>>,
	policies_by_target: hashbrown::HashMap<PolicyTarget, HashSet<PolicyKey>>,

	backends: HashMap<BackendKey, Arc<BackendWithPolicies>>,

	// Listeners we got before a Bind arrived
	staged_listeners: HashMap<BindKey, HashMap<ListenerKey, Listener>>,
	staged_routes: HashMap<ListenerKey, HashMap<RouteKey, Route>>,
	staged_tcp_routes: HashMap<ListenerKey, HashMap<RouteKey, TCPRoute>>,

	tx: tokio::sync::broadcast::Sender<Event<Arc<Bind>>>,
}

#[derive(Default, Debug, Clone)]
pub struct FrontendPolices {
	pub http: Option<frontend::HTTP>,
	pub tls: Option<frontend::TLS>,
	pub tcp: Option<frontend::TCP>,
	pub access_log: Option<frontend::LoggingPolicy>,
	pub tracing: Option<Arc<crate::types::agent::TracingPolicy>>,
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
			FrontendPolicy::AccessLog(p) => {
				self.access_log.get_or_insert_with(|| p.clone());
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

	pub request_header_modifier: Option<filters::HeaderModifier>,
	pub response_header_modifier: Option<filters::HeaderModifier>,
	pub request_redirect: Option<filters::RequestRedirect>,
	pub request_mirror: Vec<filters::RequestMirror>,

	pub session_persistence: Option<http::sessionpersistence::Policy>,

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
			mcp_authorization: other.mcp_authorization.or(self.mcp_authorization),
			mcp_authentication: other.mcp_authentication.or(self.mcp_authentication),
			inference_routing: other.inference_routing.or(self.inference_routing),
			http: other.http.or(self.http),
			tcp: other.tcp.or(self.tcp),
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
			session_persistence: other.session_persistence.or(self.session_persistence),
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
}

#[derive(Debug, Default)]
pub struct RoutePolicies {
	pub local_rate_limit: Vec<http::localratelimit::RateLimit>,
	pub remote_rate_limit: Option<remoteratelimit::RemoteRateLimit>,
	pub authorization: Option<http::authorization::HTTPAuthorizationSet>,
	pub jwt: Option<http::jwt::Jwt>,
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
	pub jwt: Option<http::jwt::Jwt>,
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

// RoutePath describes the objects traversed to reach the given route
#[derive(Debug, Clone)]
pub struct RoutePath<'a> {
	pub listener: &'a ListenerName,
	pub route: &'a RouteName,
}

impl Store {
	pub fn with_ipv6_enabled(ipv6_enabled: bool) -> Self {
		let (tx, _) = tokio::sync::broadcast::channel(1000);
		Self {
			ipv6_enabled,
			binds: Default::default(),
			resources: Default::default(),
			policies_by_key: Default::default(),
			policies_by_target: Default::default(),
			backends: Default::default(),
			staged_routes: Default::default(),
			staged_listeners: Default::default(),
			staged_tcp_routes: Default::default(),
			tx,
		}
	}
	pub fn subscribe(
		&self,
	) -> impl Stream<Item = Result<Event<Arc<Bind>>, BroadcastStreamRecvError>> + use<> {
		let sub = self.tx.subscribe();
		tokio_stream::wrappers::BroadcastStream::new(sub)
	}

	pub fn route_policies(&self, path: &RoutePath<'_>, inline: &[TrafficPolicy]) -> RoutePolicies {
		let &RoutePath { listener, route } = path;
		let gateway = self
			.policies_by_target
			.get(&listener.as_gateway_target_ref());
		let listener = self
			.policies_by_target
			.get(&listener.as_listener_target_ref());
		let route_rule = self
			.policies_by_target
			.get(&route.as_route_rule_target_ref());
		let route = self.policies_by_target.get(&route.as_route_target_ref());
		let rules = route_rule
			.iter()
			.copied()
			.flatten()
			.chain(route.iter().copied().flatten())
			.chain(listener.iter().copied().flatten())
			.chain(gateway.iter().copied().flatten())
			.filter_map(|n| self.policies_by_key.get(n))
			.filter_map(|p| p.policy.as_traffic_route_phase());
		let rules = inline.iter().chain(rules);

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
			None,
		)
	}

	// inline_backend_policies flattens out a list of inline policies,
	pub fn inline_backend_policies(&self, inline_policies: &[BackendPolicy]) -> BackendPolicies {
		self.internal_backend_policies(
			None,
			None,
			std::slice::from_ref(&inline_policies),
			None,
			None,
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
			path.as_ref().map(|p| p.route),
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
		route: Option<&RouteName>,
	) -> BackendPolicies {
		let backend_rules =
			backend.and_then(|t| self.policies_by_target.get(&PolicyTargetRef::Backend(t)));
		let sub_backend_rules =
			sub_backend.and_then(|t| self.policies_by_target.get(&PolicyTargetRef::Backend(t)));
		let route_rule_rules =
			route.and_then(|t| self.policies_by_target.get(&t.as_route_rule_target_ref()));
		let route_rules = route.and_then(|t| self.policies_by_target.get(&t.as_route_target_ref()));
		let listener_rules =
			gateway.and_then(|t| self.policies_by_target.get(&t.as_listener_target_ref()));
		let gateway_rules =
			gateway.and_then(|t| self.policies_by_target.get(&t.as_gateway_target_ref()));

		// RouteRule > Route > SubBackend > Backend > Service > Gateway
		// Most specific (route context) to least specific (gateway-wide default)
		let rules = route_rule_rules
			.iter()
			.copied()
			.flatten()
			.chain(sub_backend_rules.iter().copied().flatten())
			.chain(route_rules.iter().copied().flatten())
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
				BackendPolicy::SessionPersistence(p) => {
					pol.session_persistence.get_or_insert_with(|| p.clone());
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

	pub fn listener_frontend_policies(&self, name: &ListenerName) -> FrontendPolices {
		let gateway = self.policies_by_target.get(&name.as_gateway_target_ref());
		let listener = self.policies_by_target.get(&name.as_listener_target_ref());
		let rules = listener
			.iter()
			.copied()
			.flatten()
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

	pub fn all(&self) -> Vec<Arc<Bind>> {
		self.binds.values().cloned().collect()
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
		if let Some(old) = self.binds.remove(&bind) {
			let _ = self.tx.send(Event::Remove(old));
		}
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
		let Some(bind) = self
			.binds
			.values()
			.find(|v| v.listeners.contains(&listener))
		else {
			return;
		};
		let mut bind = Arc::unwrap_or_clone(bind.clone());
		bind.listeners.remove(&listener);
		self.insert_bind(bind);
	}

	#[instrument(
        level = Level::INFO,
        name="remove_route",
        skip_all,
        fields(route),
    )]
	pub fn remove_route(&mut self, route: RouteKey) {
		let Some((_, bind, listener)) = self.binds.iter().find_map(|(k, v)| {
			let l = v.listeners.iter().find(|l| l.routes.contains(&route));
			l.map(|l| (k.clone(), v.clone(), l.clone()))
		}) else {
			return;
		};
		let mut bind = Arc::unwrap_or_clone(bind.clone());
		let mut lis = listener.clone();
		lis.routes.remove(&route);
		bind.listeners.insert(lis);
		self.insert_bind(bind);
	}

	#[instrument(
        level = Level::INFO,
        name="remove_tcp_route",
        skip_all,
        fields(tcp_route),
    )]
	pub fn remove_tcp_route(&mut self, tcp_route: RouteKey) {
		let Some((_, bind, listener)) = self.binds.iter().find_map(|(k, v)| {
			let l = v
				.listeners
				.iter()
				.find(|l| l.tcp_routes.contains(&tcp_route));
			l.map(|l| (k.clone(), v.clone(), l.clone()))
		}) else {
			return;
		};
		let mut bind = Arc::unwrap_or_clone(bind.clone());
		let mut lis = listener.clone();
		lis.tcp_routes.remove(&tcp_route);
		bind.listeners.insert(lis);
		self.insert_bind(bind);
	}

	#[instrument(
        level = Level::INFO,
        name="insert_bind",
        skip_all,
        fields(bind=%bind.key),
    )]
	pub fn insert_bind(&mut self, mut bind: Bind) {
		debug!(bind=%bind.key, "insert bind");

		// Insert any staged listeners
		for (k, mut v) in self
			.staged_listeners
			.remove(&bind.key)
			.into_iter()
			.flatten()
		{
			debug!("adding staged listener {} to {}", k, bind.key);
			for (rk, r) in self.staged_routes.remove(&k).into_iter().flatten() {
				debug!("adding staged route {} to {}", rk, k);
				v.routes.insert(r)
			}
			for (rk, r) in self.staged_tcp_routes.remove(&k).into_iter().flatten() {
				debug!("adding staged tcp route {} to {}", rk, k);
				v.tcp_routes.insert(r)
			}
			bind.listeners.insert(v)
		}
		let arc = Arc::new(bind);
		self.binds.insert(arc.key.clone(), arc.clone());
		// ok to have no subs
		let _ = self.tx.send(Event::Add(arc));
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

	pub fn insert_listener(&mut self, mut lis: Listener, bind_name: BindKey) {
		debug!(listener=%lis.key,bind=%bind_name, "insert listener");
		if let Some(b) = self.binds.get(&bind_name) {
			let mut bind = Arc::unwrap_or_clone(b.clone());
			// If this is a listener update, copy things over
			if let Some(old) = bind.listeners.remove(&lis.key) {
				debug!("listener update, copy old routes over");
				lis.routes = Arc::unwrap_or_clone(old).routes;
			}
			// Insert any staged routes
			for (k, v) in self.staged_routes.remove(&lis.key).into_iter().flatten() {
				debug!("adding staged route {} to {}", k, lis.key);
				lis.routes.insert(v)
			}
			for (k, v) in self
				.staged_tcp_routes
				.remove(&lis.key)
				.into_iter()
				.flatten()
			{
				debug!("adding staged tcp route {} to {}", k, lis.key);
				lis.tcp_routes.insert(v)
			}
			bind.listeners.insert(lis);
			self.insert_bind(bind);
		} else {
			// Insert any staged routes
			for (k, v) in self.staged_routes.remove(&lis.key).into_iter().flatten() {
				debug!("adding staged route {} to {}", k, lis.key);
				lis.routes.insert(v)
			}
			for (k, v) in self
				.staged_tcp_routes
				.remove(&lis.key)
				.into_iter()
				.flatten()
			{
				debug!("adding staged tcp route {} to {}", k, lis.key);
				lis.tcp_routes.insert(v)
			}
			debug!("no bind found, staging");
			self
				.staged_listeners
				.entry(bind_name)
				.or_default()
				.insert(lis.key.clone(), lis);
		}
	}

	pub fn insert_route(&mut self, r: Route, ln: ListenerKey) {
		debug!(listener=%ln, route=%r.key, "insert route");
		let Some((bind, lis)) = self
			.binds
			.values()
			.find_map(|l| l.listeners.get(&ln).map(|ls| (l, ls)))
		else {
			debug!(listener=%ln,route=%r.key, "no listener found, staging");
			self
				.staged_routes
				.entry(ln)
				.or_default()
				.insert(r.key.clone(), r);
			return;
		};
		let mut bind = Arc::unwrap_or_clone(bind.clone());
		let mut lis = lis.clone();
		lis.routes.insert(r);
		bind.listeners.insert(lis);
		self.insert_bind(bind);
	}

	pub fn insert_tcp_route(&mut self, r: TCPRoute, ln: ListenerKey) {
		debug!(listener=%ln,route=%r.key, "insert tcp route");
		let Some((bind, lis)) = self
			.binds
			.values()
			.find_map(|l| l.listeners.get(&ln).map(|ls| (l, ls)))
		else {
			debug!(listener=%ln,route=%r.key, "no listener found, staging");
			self
				.staged_tcp_routes
				.entry(ln)
				.or_default()
				.insert(r.key.clone(), r);
			return;
		};
		let mut bind = Arc::unwrap_or_clone(bind.clone());
		let mut lis = lis.clone();
		lis.tcp_routes.insert(r);
		bind.listeners.insert(lis);
		self.insert_bind(bind);
	}

	fn remove_resource(&mut self, res: &Strng) {
		trace!("removing res {res}...");
		let Some(old) = self.resources.remove(res) else {
			debug!("unknown resource name {res}");
			return;
		};
		match old {
			ResourceKind::Policy(n) => self.remove_policy(n),
			ResourceKind::Bind(n) => self.remove_bind(n),
			ResourceKind::Route(n) => self.remove_route(n),
			ResourceKind::TcpRoute(n) => self.remove_tcp_route(n),
			ResourceKind::Listener(n) => self.remove_listener(n),
			ResourceKind::Backend(n) => self.remove_backend(n),
		}
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
		if let Some(old) = self.binds.remove(&bind.key) {
			debug!("bind update, copy old listeners over");
			bind.listeners = Arc::unwrap_or_clone(old).listeners;
		}
		self.insert_bind(bind);
		Ok(())
	}
	fn insert_xds_listener(&mut self, raw: XdsListener) -> anyhow::Result<()> {
		let (lis, bind_name): (Listener, BindKey) = (&raw).try_into()?;
		self.insert_listener(lis, bind_name);
		Ok(())
	}
	fn insert_xds_route(&mut self, raw: XdsRoute) -> anyhow::Result<()> {
		let (route, listener_name): (Route, ListenerKey) = (&raw).try_into()?;
		self.insert_route(route, listener_name);
		Ok(())
	}
	fn insert_xds_tcp_route(&mut self, raw: XdsTcpRoute) -> anyhow::Result<()> {
		let (route, listener_name): (TCPRoute, ListenerKey) = (&raw).try_into()?;
		self.insert_tcp_route(route, listener_name);
		Ok(())
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
pub struct Dump {
	binds: Vec<Arc<Bind>>,
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
			.map(|k| k.1.clone())
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
		}
	}
	pub fn sync_local(
		&self,
		binds: Vec<Bind>,
		policies: Vec<TargetedPolicy>,
		backends: Vec<BackendWithPolicies>,
		prev: PreviousState,
	) -> PreviousState {
		let mut s = self.state.write().expect("mutex acquired");
		let mut old_binds = prev.binds;
		let mut old_pols = prev.policies;
		let mut old_backends = prev.backends;
		let mut next_state = PreviousState {
			binds: Default::default(),
			policies: Default::default(),
			backends: Default::default(),
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
		for p in policies {
			old_pols.remove(&p.key);
			next_state.policies.insert(p.key.clone());
			s.insert_policy(p);
		}
		for remaining_bind in old_binds {
			s.remove_bind(remaining_bind);
		}
		for remaining_policy in old_pols {
			s.remove_policy(remaining_policy);
		}
		for remaining_backend in old_backends {
			s.remove_backend(remaining_backend);
		}
		next_state
	}
}

#[derive(Clone, Debug, Default)]
pub struct PreviousState {
	pub binds: HashSet<BindKey>,
	pub policies: HashSet<PolicyKey>,
	pub backends: HashSet<BackendKey>,
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
					state.remove_resource(&strng::new(name))
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
		})
	}

	fn insert_policy_at_level(
		store: &mut Store,
		listener: &ListenerName,
		policy_name: &str,
		for_listener: bool,
		remove_item: &str,
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
			policy: agent::PolicyType::Frontend(create_access_log_policy(remove_item)),
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
		insert_policy_at_level(store, listener, "gw_frontend_policy", false, remove_item);
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
			remove_item,
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
				route: &http_route,
			},
			&[],
		);
		assert_eq!(http_pols.timeout, Some(http_timeout));

		let grpc_pols = store.route_policies(
			&RoutePath {
				listener: &listener,
				route: &grpc_route,
			},
			&[],
		);
		assert_eq!(grpc_pols.timeout, Some(grpc_timeout));
	}

	/// Tests that frontend policies at listener level take precedence over gateway level policies
	#[test]
	fn frontend_policy_listener_precedence() {
		let mut store = Store::default();
		let listener = listener();

		// Insert both gateway and listener level frontend policies
		insert_gateway_level_frontend_policy(&mut store, &listener, "gw_remove");
		insert_listener_level_frontend_policy(&mut store, &listener, "listener_remove");

		let merged_pols = store.listener_frontend_policies(&listener);
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
}
