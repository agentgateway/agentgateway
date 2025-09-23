use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use agent_xds::{RejectedConfig, XdsUpdate};
use futures_core::Stream;
use itertools::Itertools;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tracing::{Level, instrument};

use crate::cel::ContextBuilder;
use crate::http::auth::BackendAuth;
use crate::http::authorization::{HTTPAuthorizationSet, RuleSets};
use crate::http::backendtls::BackendTLS;
use crate::http::ext_proc::InferenceRouting;
use crate::http::{ext_authz, ext_proc, remoteratelimit};
use crate::llm::policy::ResponseGuard;
use crate::mcp::McpAuthorizationSet;
use crate::proxy::httpproxy::PolicyClient;
use crate::store::Event;
use crate::types::agent::{
	A2aPolicy, Backend, BackendName, Bind, BindName, GatewayName, Listener, ListenerKey, ListenerSet,
	McpAuthentication, Policy, PolicyName, PolicyTarget, Route, RouteKey, RouteName, RouteRuleName,
	ServiceName, SubBackendName, TCPRoute, TargetedPolicy,
};
use crate::types::proto::agent::resource::Kind as XdsKind;
use crate::types::proto::agent::{
	Backend as XdsBackend, Bind as XdsBind, Listener as XdsListener, Policy as XdsPolicy,
	Resource as ADPResource, Route as XdsRoute, TcpRoute as XdsTcpRoute,
};
use crate::*;

#[derive(Debug)]
pub struct Store {
	/// Allows for lookup of services by network address, the service's xds secondary key.
	by_name: HashMap<BindName, Arc<Bind>>,

	policies_by_name: HashMap<PolicyName, Arc<TargetedPolicy>>,
	policies_by_target: HashMap<PolicyTarget, HashSet<PolicyName>>,

	backends_by_name: HashMap<BackendName, Arc<Backend>>,

	// Listeners we got before a Bind arrived
	staged_listeners: HashMap<BindName, HashMap<ListenerKey, Listener>>,
	staged_routes: HashMap<ListenerKey, HashMap<RouteKey, Route>>,
	staged_tcp_routes: HashMap<ListenerKey, HashMap<RouteKey, TCPRoute>>,

	tx: tokio::sync::broadcast::Sender<Event<Arc<Bind>>>,
}

#[derive(Default, Debug, Clone)]
pub struct BackendPolicies {
	pub backend_tls: Option<BackendTLS>,
	pub backend_auth: Option<BackendAuth>,
	pub a2a: Option<A2aPolicy>,
	pub llm_provider: Option<Arc<llm::NamedAIProvider>>,
	pub llm: Option<Arc<llm::Policy>>,
	pub inference_routing: Option<InferenceRouting>,
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
			inference_routing: other.inference_routing.or(self.inference_routing),
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

#[derive(Debug)]
pub struct RoutePolicies {
	pub local_rate_limit: Vec<http::localratelimit::RateLimit>,
	pub remote_rate_limit: Option<remoteratelimit::RemoteRateLimit>,
	pub authorization: Option<http::authorization::HTTPAuthorizationSet>,
	pub jwt: Option<http::jwt::Jwt>,
	pub ext_authz: Option<ext_authz::ExtAuthz>,
	pub transformation: Option<http::transformation_cel::Transformation>,
	pub llm: Option<Arc<llm::Policy>>,
	pub csrf: Option<http::csrf::Csrf>,
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

		route_policies.llm = Some(Arc::new(llm::Policy {
			prompt_guard: be.prompt_guard.clone().or_else(|| re.prompt_guard.clone()),
			defaults: be.defaults.clone().or_else(|| re.defaults.clone()),
			overrides: be.overrides.clone().or_else(|| re.overrides.clone()),
			prompts: be.prompts.clone().or_else(|| re.prompts.clone()),
			model_aliases: if be.model_aliases.is_empty() {
				re.model_aliases.clone()
			} else {
				be.model_aliases.clone()
			},
		}));
		Arc::new(route_policies)
	}
}

#[derive(Debug, Default)]
pub struct LLMResponsePolicies {
	pub local_rate_limit: Vec<http::localratelimit::RateLimit>,
	pub remote_rate_limit: Option<http::remoteratelimit::LLMResponseAmend>,
	pub prompt_guard: Option<ResponseGuard>,
}

impl Default for Store {
	fn default() -> Self {
		Self::new()
	}
}
impl Store {
	pub fn new() -> Self {
		let (tx, _) = tokio::sync::broadcast::channel(1000);
		Self {
			by_name: Default::default(),
			policies_by_name: Default::default(),
			policies_by_target: Default::default(),
			backends_by_name: Default::default(),
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

	pub fn route_policies(
		&self,
		route_rule: Option<RouteRuleName>,
		route: RouteName,
		listener: ListenerKey,
		gateway: GatewayName,
		inline: &[Policy],
	) -> RoutePolicies {
		// Changes we must do:
		// * Index the store by the target
		// * Avoid the N lookups, or at least the boilerplate, for each type
		// Changes we may want to consider:
		// * We do this lookup under one lock, but we will lookup backend rules and listener rules under a different
		//   lock. This can lead to inconsistent state..
		let gateway = self.policies_by_target.get(&PolicyTarget::Gateway(gateway));
		let listener = self
			.policies_by_target
			.get(&PolicyTarget::Listener(listener));
		let route = self.policies_by_target.get(&PolicyTarget::Route(route));
		let route_rule =
			route_rule.and_then(|rr| self.policies_by_target.get(&PolicyTarget::RouteRule(rr)));
		let rules = route_rule
			.iter()
			.copied()
			.flatten()
			.chain(route.iter().copied().flatten())
			.chain(listener.iter().copied().flatten())
			.chain(gateway.iter().copied().flatten())
			.filter_map(|n| self.policies_by_name.get(n))
			.map(|p| &p.policy);
		let rules = inline.iter().chain(rules);

		let mut authz = Vec::new();
		let mut pol = RoutePolicies {
			local_rate_limit: vec![],
			remote_rate_limit: None,
			jwt: None,
			ext_authz: None,
			transformation: None,
			authorization: None,
			llm: None,
			csrf: None,
		};
		for rule in rules {
			match &rule {
				Policy::LocalRateLimit(p) => {
					if pol.local_rate_limit.is_empty() {
						pol.local_rate_limit = p.clone();
					}
				},
				Policy::ExtAuthz(p) => {
					pol.ext_authz.get_or_insert_with(|| p.clone());
				},
				Policy::RemoteRateLimit(p) => {
					pol.remote_rate_limit.get_or_insert_with(|| p.clone());
				},
				Policy::JwtAuth(p) => {
					pol.jwt.get_or_insert_with(|| p.clone());
				},
				Policy::Transformation(p) => {
					pol.transformation.get_or_insert_with(|| p.clone());
				},
				Policy::Authorization(p) => {
					// Authorization policies merge, unlike others
					authz.push(p.clone().0);
				},
				Policy::AI(p) => {
					pol.llm.get_or_insert_with(|| p.clone());
				},
				Policy::Csrf(p) => {
					pol.csrf.get_or_insert_with(|| p.clone());
				},
				_ => {}, // others are not route policies
			}
		}
		if !authz.is_empty() {
			pol.authorization = Some(HTTPAuthorizationSet::new(authz.into()));
		}

		pol
	}

	pub fn backend_policies(
		&self,
		backend: BackendName,
		service: Option<ServiceName>,
		sub_backend: Option<SubBackendName>,
	) -> BackendPolicies {
		let backend_rules = self.policies_by_target.get(&PolicyTarget::Backend(backend));
		let service_rules =
			service.and_then(|t| self.policies_by_target.get(&PolicyTarget::Service(t)));
		let sub_backend_rules =
			sub_backend.and_then(|t| self.policies_by_target.get(&PolicyTarget::SubBackend(t)));

		// Subbackend > Backend > Service
		let rules = sub_backend_rules
			.iter()
			.copied()
			.flatten()
			.chain(backend_rules.iter().copied().flatten())
			.chain(service_rules.iter().copied().flatten())
			.filter_map(|n| self.policies_by_name.get(n));

		let mut pol = BackendPolicies {
			backend_tls: None,
			backend_auth: None,
			a2a: None,
			inference_routing: None,
			// These are not attached policies but are represented in this struct for code organization
			llm_provider: None,
			llm: None,
		};
		for rule in rules {
			match &rule.policy {
				Policy::A2a(p) => {
					pol.a2a.get_or_insert_with(|| p.clone());
				},
				Policy::BackendTLS(p) => {
					pol.backend_tls.get_or_insert_with(|| p.clone());
				},
				Policy::BackendAuth(p) => {
					pol.backend_auth.get_or_insert_with(|| p.clone());
				},
				Policy::InferenceRouting(p) => {
					pol.inference_routing.get_or_insert_with(|| p.clone());
				},
				Policy::AI(p) => {
					pol.llm.get_or_insert_with(|| p.clone());
				},
				_ => {},
			}
		}
		pol
	}

	pub fn mcp_policies(
		&self,
		backend: BackendName,
	) -> (McpAuthorizationSet, Option<McpAuthentication>) {
		let t = PolicyTarget::Backend(backend);
		let rs = McpAuthorizationSet::new(RuleSets::from(
			self
				.policies_by_name
				.values()
				.filter_map(|p| {
					if p.target != t {
						return None;
					};
					match &p.policy {
						Policy::McpAuthorization(authz) => Some(authz.clone().into_inner()),
						_ => None,
					}
				})
				.collect_vec(),
		));
		let auth = self
			// This is a terrible approach!
			.policies_by_name
			.values()
			.filter_map(|p| {
				if p.target != t {
					return None;
				};
				match &p.policy {
					Policy::McpAuthentication(ba) => Some(ba.clone()),
					_ => None,
				}
			})
			.next();
		(rs, auth)
	}

	pub fn listeners(&self, bind: BindName) -> Option<ListenerSet> {
		// TODO: clone here is terrible!!!
		self.by_name.get(&bind).map(|b| b.listeners.clone())
	}

	pub fn all(&self) -> Vec<Arc<Bind>> {
		self.by_name.values().cloned().collect()
	}

	pub fn backend(&self, r: &BackendName) -> Option<Arc<Backend>> {
		self.backends_by_name.get(r).cloned()
	}

	#[instrument(
        level = Level::INFO,
        name="remove_bind",
        skip_all,
        fields(bind),
    )]
	pub fn remove_bind(&mut self, bind: BindName) {
		if let Some(old) = self.by_name.remove(&bind) {
			let _ = self.tx.send(Event::Remove(old));
		}
	}
	#[instrument(
        level = Level::INFO,
        name="remove_policy",
        skip_all,
        fields(bind),
    )]
	pub fn remove_policy(&mut self, pol: PolicyName) {
		if let Some(old) = self.policies_by_name.remove(&pol)
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
	pub fn remove_backend(&mut self, backend: BackendName) {
		self.backends_by_name.remove(&backend);
	}

	#[instrument(
        level = Level::INFO,
        name="remove_listener",
        skip_all,
        fields(listener),
    )]
	pub fn remove_listener(&mut self, listener: ListenerKey) {
		let Some(bind) = self
			.by_name
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
		let Some((_, bind, listener)) = self.by_name.iter().find_map(|(k, v)| {
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
		let Some((_, bind, listener)) = self.by_name.iter().find_map(|(k, v)| {
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
		self.by_name.insert(arc.key.clone(), arc.clone());
		// ok to have no subs
		let _ = self.tx.send(Event::Add(arc));
	}

	pub fn insert_backend(&mut self, b: Backend) {
		let name = b.name();
		if let Backend::AI(_, t) = &b
			&& t.providers.any(|p| p.tokenize)
		{
			preload_tokenizers()
		}
		let arc = Arc::new(b);
		self.backends_by_name.insert(name, arc);
	}

	#[instrument(
        level = Level::INFO,
        name="insert_policy",
        skip_all,
        fields(pol=%pol.name),
    )]
	pub fn insert_policy(&mut self, pol: TargetedPolicy) {
		let pol = Arc::new(pol);
		if let Some(old) = self.policies_by_name.insert(pol.name.clone(), pol.clone()) {
			// Remove the old target. We may add it back, though.
			if let Some(o) = self.policies_by_target.get_mut(&old.target) {
				o.remove(&pol.name);
			}
		}
		self
			.policies_by_target
			.entry(pol.target.clone())
			.or_default()
			.insert(pol.name.clone());
	}

	pub fn insert_listener(&mut self, mut lis: Listener, bind_name: BindName) {
		debug!(listener=%lis.name,bind=%bind_name, "insert listener");
		if let Some(b) = self.by_name.get(&bind_name) {
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
		debug!(listener=%ln,route=%r.key, "insert route");
		let Some((bind, lis)) = self
			.by_name
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
			.by_name
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
		let Some((res, res_name)) = res.split_once("/") else {
			trace!("unknown resource name {res}");
			return;
		};
		match res {
			"bind" => {
				self.remove_bind(strng::new(res_name));
			},
			"listener" => {
				self.remove_listener(strng::new(res_name));
			},
			"route" => {
				self.remove_route(strng::new(res_name));
			},
			"policy" => {
				self.remove_policy(strng::new(res_name));
			},
			"backend" => {
				self.remove_backend(strng::new(res_name));
			},
			"tcp_route" => {
				self.remove_tcp_route(strng::new(res_name));
			},
			_ => {
				error!("unknown resource kind {res}");
			},
		}
	}

	fn insert_xds(&mut self, res: ADPResource) -> anyhow::Result<()> {
		trace!("insert resource {res:?}");
		match res.kind {
			Some(XdsKind::Bind(w)) => self.insert_xds_bind(w),
			Some(XdsKind::Listener(w)) => self.insert_xds_listener(w),
			Some(XdsKind::Route(w)) => self.insert_xds_route(w),
			Some(XdsKind::TcpRoute(w)) => self.insert_xds_tcp_route(w),
			Some(XdsKind::Backend(w)) => self.insert_xds_backend(w),
			Some(XdsKind::Policy(w)) => self.insert_xds_policy(w),
			_ => Err(anyhow::anyhow!("unknown resource type")),
		}
	}

	fn insert_xds_bind(&mut self, raw: XdsBind) -> anyhow::Result<()> {
		let mut bind = Bind::try_from(&raw)?;
		// If XDS server pushes the same bind twice (which it shouldn't really do, but oh well),
		// we need to copy the listeners over.
		if let Some(old) = self.by_name.remove(&bind.key) {
			debug!("bind update, copy old listeners over");
			bind.listeners = Arc::unwrap_or_clone(old).listeners;
		}
		self.insert_bind(bind);
		Ok(())
	}
	fn insert_xds_listener(&mut self, raw: XdsListener) -> anyhow::Result<()> {
		let (lis, bind_name): (Listener, BindName) = (&raw).try_into()?;
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
		let backend: Backend = (&raw).try_into()?;
		self.insert_backend(backend);
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
	backends: Vec<Arc<Backend>>,
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
			.by_name
			.iter()
			.sorted_by_key(|k| k.0)
			.map(|k| k.1.clone())
			.collect();
		let policies: Vec<_> = store
			.policies_by_name
			.iter()
			.sorted_by_key(|k| k.0)
			.map(|k| k.1.clone())
			.collect();
		let backends: Vec<_> = store
			.backends_by_name
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
		backends: Vec<Backend>,
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
			old_backends.remove(&b.name());
			next_state.backends.insert(b.name());
			s.insert_backend(b);
		}
		for p in policies {
			old_pols.remove(&p.name);
			next_state.policies.insert(p.name.clone());
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
	pub binds: HashSet<BindName>,
	pub policies: HashSet<PolicyName>,
	pub backends: HashSet<BackendName>,
}

impl agent_xds::Handler<ADPResource> for StoreUpdater {
	fn handle(
		&self,
		updates: Box<&mut dyn Iterator<Item = XdsUpdate<ADPResource>>>,
	) -> Result<(), Vec<RejectedConfig>> {
		let mut state = self.state.write().unwrap();
		let handle = |res: XdsUpdate<ADPResource>| {
			match res {
				XdsUpdate::Update(w) => state.insert_xds(w.resource)?,
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
