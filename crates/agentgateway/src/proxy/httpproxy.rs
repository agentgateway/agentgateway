use std::collections::HashSet;
use std::net::SocketAddr;
use std::ops::{Deref, DerefMut};
use std::str::FromStr;
use std::sync::Arc;

use ::http::uri::PathAndQuery;
use ::http::{HeaderMap, header};
use anyhow::anyhow;
use frozen_collections::Len;
use headers::HeaderMapExt;
use hyper::body::Incoming;
use hyper::upgrade::OnUpgrade;
use hyper_util::rt::TokioIo;
use rand::RngExt;
use rand::seq::{IndexedRandom, IteratorRandom};
use tracing::{debug, trace};
use types::agent::*;
use types::discovery::*;

use crate::cel::{BackendContext, RequestTime};
use crate::client::{ApplicationTransport, Transport};
use crate::http::backendtls::BackendTLS;
use crate::http::ext_proc::{ExtProcRequest, InferenceRoutingDestinationMode};
use crate::http::filters::{AutoHostname, BackendRequestTimeout};
use crate::http::transformation_cel::Transformation;
use crate::http::{
	Authority, HeaderName, HeaderValue, Request, Response, Scheme, StatusCode, Uri, auth, filters,
	merge_in_headers, retry,
};
use crate::llm::{InputFormat, LLMInfo, LLMRequest, LLMResponse, RequestResult, RouteType};
use crate::proxy::tcpproxy::TCPProxy;
use crate::proxy::{
	ProxyError, ProxyResponse, ProxyResponseReason, WaypointService, dtrace, resolve_simple_backend,
};
use crate::store::{
	BackendPolicies, FrontendPolices, GatewayPolicies, LLMRequestPolicies, LLMResponsePolicies,
	ResponsePolicy, RoutePath,
};
use crate::telemetry::log;
use crate::telemetry::log::{AsyncLog, DropOnLog, LogBody, RequestLog, TraceSampler};
use crate::telemetry::trc::TraceParent;
use crate::transport::stream::{Extension, TCPConnectionInfo, TLSConnectionInfo};
use crate::types::{backend, frontend};
use crate::{ProxyInputs, store, *};

fn select_backend(route: &Route, _req: &Request) -> Option<RouteBackendReference> {
	route
		.backends
		.choose_weighted(&mut rand::rng(), |b| b.weight)
		.ok()
		.cloned()
}

#[derive(Debug)]
struct SelectedRouteChain {
	routes: Vec<Arc<Route>>,
	path_match: PathMatch,
	backend: Option<RouteBackendReference>,
}

fn select_route_chain(
	inputs: &ProxyInputs,
	target_address: SocketAddr,
	listener: &Listener,
	req: &Request,
) -> Result<SelectedRouteChain, ProxyError> {
	let (mut selected_route, mut path_match) =
		http::route::select_best_route(inputs.stores.clone(), target_address, listener, req)
			.ok_or(ProxyError::RouteNotFound)?;

	let mut routes = vec![selected_route.clone()];
	let mut seen = HashSet::from([selected_route.key.clone()]);
	loop {
		let Some(selected_backend) = select_backend(selected_route.as_ref(), req) else {
			return Ok(SelectedRouteChain {
				routes,
				path_match,
				backend: None,
			});
		};
		let RouteBackendTarget::RouteGroup(route_name) = &selected_backend.target else {
			return Ok(SelectedRouteChain {
				routes,
				path_match,
				backend: Some(selected_backend),
			});
		};

		let rg = {
			let binds = inputs.stores.binds.read();
			binds
				.lookup_route_group(route_name)
				.ok_or(ProxyError::RouteNotFound)?
		};
		(selected_route, path_match) =
			http::route::select_best_route_group(rg.as_ref(), req).ok_or(ProxyError::RouteNotFound)?;
		if !seen.insert(selected_route.key.clone()) {
			return Err(ProxyError::RouteCycleDetected);
		}
		routes.push(selected_route.clone());
	}
}

pub fn apply_logging_policy_to_log(log: &mut RequestLog, lp: &frontend::LoggingPolicy) {
	// Merge filter/fields into config for this request
	if lp.filter.is_some() {
		log.cel.filter = lp.filter.clone();
	}
	if lp.add.is_empty() && lp.remove.is_empty() {
		return;
	}
	if !lp.add.is_empty() {
		log.cel.fields.add = lp.add.clone();
	}
	if !lp.remove.is_empty() {
		log.cel.fields.remove = lp.remove.clone();
	}
}

async fn apply_request_policies(
	pol: &store::RoutePolicies,
	c: &PolicyClient,
	l: &mut RequestLog,
	req: &mut Request,
	rp: &mut ResponsePolicies,
) -> Result<(), ProxyResponse> {
	// CORS must run before authentication, authorization and rate limiting so that:
	// 1. Preflight OPTIONS requests short-circuit without requiring credentials
	// 2. CORS response headers are queued even if the request is later rejected,
	//    allowing browsers to read error responses instead of seeing a CORS error
	pol
		.cors
		.apply_without_response("cors", c, l, req, rp.headers())
		.await?;

	pol
		.oidc
		.apply_without_response("oidc", c, l, req, rp.headers())
		.await?;
	http::strip_request_cookies_by_prefix(req, http::oidc::RESERVED_COOKIE_PREFIX);

	pol
		.jwt
		.apply_without_response("jwt auth", c, l, req, rp.headers())
		.await?;
	pol
		.basic_auth
		.apply_without_response("basic auth", c, l, req, rp.headers())
		.await?;
	pol
		.api_key
		.apply_without_response("api key", c, l, req, rp.headers())
		.await?;

	pol
		.ext_authz
		.apply_without_response("ext authz", c, l, req, rp.headers())
		.await?;

	pol
		.authorization
		.apply_without_response("authorization", c, l, req, rp.headers())
		.await?;

	rp.llm_request_policies.local_rate_limit = pol
		.local_rate_limit
		.apply_selected("local rate limit", c, l, req, rp.headers())
		.await?;

	rp.llm_request_policies.remote_rate_limit = pol
		.remote_rate_limit
		.apply_selected("remote rate limit", c, l, req, rp.headers())
		.await?;

	// ExtProc uses RequestPolicy for conditional selection and CEL registration only.
	// The selected config is built into per-request state, which must be retained for
	// the response mutation phase instead of applying ExtProc directly.
	rp.ext_proc = pol
		.ext_proc
		.select("ext proc", req)
		.map(|p| p.build(c.clone()));
	if let Some(x) = rp.ext_proc.as_mut() {
		x.mutate_request(req).await?.apply(rp.headers())?;
		dtrace::snapshot!(Request, "ext proc", &req);
	}

	rp.transformation = pol
		.transformation
		.apply("transformation", c, l, req, rp.headers())
		.await?;

	pol
		.csrf
		.apply_without_response("csrf", c, l, req, rp.headers())
		.await?;

	// Insert route response header filter. This a response-only policy so we only insert it, no need
	// to run it.
	rp.route_response_header = pol.response_header_modifier.clone();
	pol
		.request_header_modifier
		.apply_without_response("request header modifier", c, l, req, rp.headers())
		.await?;

	// Enable Auto Hostname rewrite by default. This may be disabled by a URL Rewrite, or explicitly
	// setting hostname_rewrite = None
	if pol.hostname_rewrite.unwrap_or(HostRedirectOverride::Auto) == HostRedirectOverride::Auto {
		req.extensions_mut().insert(AutoHostname());
	}
	pol
		.url_rewrite
		.apply_without_response("url rewrite", c, l, req, rp.headers())
		.await?;
	pol
		.request_redirect
		.apply_without_response("request redirect", c, l, req, rp.headers())
		.await?;
	pol
		.direct_response
		.apply_without_response("direct response", c, l, req, rp.headers())
		.await?;

	// Mirror, timeout, and retry are handled separately.

	Ok(())
}

async fn apply_backend_policies(
	backend_info: auth::BackendInfo,
	backend_call: &BackendCall,
	req: &mut Request,
	log: &mut Option<&mut RequestLog>,
	rp: &mut ResponsePolicies,
) -> Result<(), ProxyResponse> {
	let BackendPolicies {
		backend_tls: _,
		backend_auth,
		a2a,
		http,
		// Doesn't currently have any options to set, todo
		tcp: _,
		// Applied elsewhere
		tunnel: _,
		// Applied elsewhere
		llm_provider: _,
		// Applied elsewhere
		llm: _,
		// Applied elsewhere
		mcp_authorization: _,
		// Applied elsewhere
		mcp_authentication: _,
		// Applied elsewhere
		inference_routing: _,
		request_header_modifier,
		response_header_modifier,
		request_redirect,
		transformation,
		// TODO: implement session persistence
		session_persistence: _,
		// Applied elsewhere
		request_mirror: _,
		// Applied elsewhere
		override_dest: _,
		// Applied elsewhere
		health: _,
	} = &backend_call.backend_policies;
	rp.backend_response_header = response_header_modifier.as_response_policy();

	let dh = backend::HTTP::default();
	http
		.as_ref()
		.unwrap_or(&dh)
		.apply(req, backend_call.http_version_override);

	if let Some(auth) = backend_auth {
		auth::apply_backend_auth(&backend_info, auth, req).await?;
		dtrace::snapshot!(Request, "backend auth", &req);
	}
	rp.backend_transformation = transformation
		.apply("backend transformation", log, req, rp.headers())
		.await?;
	if let Some(rhm) = request_header_modifier {
		rhm.apply_request(req).map_err(ProxyError::from)?;
		dtrace::snapshot!(Request, "backend request header modifier", &req);
	}
	if let Some(rr) = request_redirect {
		rr.apply(req)
			.map_err(ProxyError::from)?
			.apply(rp.headers())?;
	}

	if let Some(a2a) = a2a {
		let a2a_type = a2a::apply_to_request(a2a, req).await;
		if let a2a::RequestType::Call(method) = &a2a_type {
			log.add(|l| {
				l.a2a_method = Some(method.clone());
			});
		}
		if matches!(
			a2a_type,
			a2a::RequestType::Call(_) | a2a::RequestType::AgentCard(_)
		) {
			log.add(|l| {
				l.backend_protocol = Some(cel::BackendProtocol::a2a);
			});
		}
		rp.a2a_type = a2a_type;
	}

	Ok(())
}

async fn apply_gateway_policies(
	policies: &GatewayPolicies,
	client: PolicyClient,
	l: &mut RequestLog,
	req: &mut Request,
	response_policies: &mut ResponsePolicies,
) -> Result<(), ProxyResponse> {
	let c = &client;
	policies
		.oidc
		.apply_without_response("gateway oidc", c, l, req, response_policies.headers())
		.await?;
	http::strip_request_cookies_by_prefix(req, http::oidc::RESERVED_COOKIE_PREFIX);

	policies
		.jwt
		.apply_without_response("gateway jwt", c, l, req, response_policies.headers())
		.await?;

	policies
		.basic_auth
		.apply_without_response("gateway basic auth", c, l, req, response_policies.headers())
		.await?;
	policies
		.api_key
		.apply_without_response("gateway api key", c, l, req, response_policies.headers())
		.await?;

	policies
		.ext_authz
		.apply_without_response("gateway ext authz", c, l, req, response_policies.headers())
		.await?;

	// ExtProc uses RequestPolicy for conditional selection and CEL registration only.
	// The selected config is built into per-request state, which must be retained for
	// the response mutation phase instead of applying ExtProc directly.
	let mut ext_proc = policies
		.ext_proc
		.select("gateway ext proc", req)
		.map(|p| p.build(client.clone()));
	if let Some(x) = ext_proc.as_mut() {
		x.mutate_request(req)
			.await?
			.apply(response_policies.headers())?;
		dtrace::snapshot!(Request, "gateway ext proc", &req);
	}
	response_policies.gateway_ext_proc = ext_proc;

	response_policies.gateway_transformation = policies
		.transformation
		.apply(
			"gateway transformation",
			c,
			l,
			req,
			response_policies.headers(),
		)
		.await?;

	Ok(())
}

async fn apply_llm_request_policies(
	policies: &store::LLMRequestPolicies,
	client: PolicyClient,
	req: &mut Request,
	llm_req: &LLMRequest,
	response_headers: &mut HeaderMap,
) -> Result<store::LLMResponsePolicies, ProxyResponse> {
	let local_rate_limit = policies
		.local_rate_limit
		.as_deref()
		.into_iter()
		.flatten()
		.filter(|rate_limit| rate_limit.spec.limit_type == http::localratelimit::RateLimitType::Tokens)
		.cloned()
		.collect::<Vec<_>>();
	for lrl in &local_rate_limit {
		lrl.check_llm_request(llm_req)?;
	}
	let (rl_resp, response) = if let Some(rrl) = &policies.remote_rate_limit {
		// For the LLM request side, request either the count of the input tokens (if tokenization was done)
		// or 0.
		// Either way, we will 'true up' on the response side.
		rrl
			.check_llm(client, req, llm_req.input_tokens.unwrap_or_default())
			.await?
	} else {
		(http::PolicyResponse::default(), None)
	};
	rl_resp.apply(response_headers)?;
	Ok(store::LLMResponsePolicies {
		local_rate_limit,
		remote_rate_limit: response,
		prompt_guard: policies
			.llm
			.as_deref()
			.and_then(|llm| llm.prompt_guard.as_ref())
			.map(|g| g.response.clone())
			.unwrap_or_default(),
	})
}

#[derive(Clone)]
pub struct HTTPProxy {
	pub(super) bind_name: BindKey,
	pub(super) inputs: Arc<ProxyInputs>,
	pub(super) selected_listener: Option<Arc<Listener>>,
	pub(super) target_address: SocketAddr,
}

/// SnapshottedProxyResponse is just a marker to avoid accidentally returning a response that is not snapshotted.
#[derive(Debug)]
pub struct SnapshottedProxyResponse(ProxyResponse);

trait ResultWithSnapshot<T, E>
where
	E: Into<ProxyResponse>,
{
	fn snapshot_on_err(
		self,
		log: &mut RequestLog,
		req: &mut Request,
	) -> Result<T, SnapshottedProxyResponse>;
	fn maybe_snapshot_on_err(
		self,
		log: &mut RequestLog,
		req: &mut Option<Request>,
	) -> Result<T, SnapshottedProxyResponse>;
	fn explicitly_skip_snapshot(self) -> Result<T, SnapshottedProxyResponse>;
}

impl<T, E> ResultWithSnapshot<T, E> for Result<T, E>
where
	E: Into<ProxyResponse>,
{
	fn snapshot_on_err(
		self,
		log: &mut RequestLog,
		req: &mut Request,
	) -> Result<T, SnapshottedProxyResponse> {
		self.map_err(|e| {
			log.request_snapshot = log.cel.cel_context.maybe_snapshot_request(req, true);
			SnapshottedProxyResponse(e.into())
		})
	}
	fn maybe_snapshot_on_err(
		self,
		log: &mut RequestLog,
		req: &mut Option<Request>,
	) -> Result<T, SnapshottedProxyResponse> {
		self.map_err(|e| {
			if let Some(req) = req.as_mut() {
				log.request_snapshot = log.cel.cel_context.maybe_snapshot_request(req, true);
			}
			SnapshottedProxyResponse(e.into())
		})
	}
	fn explicitly_skip_snapshot(self) -> Result<T, SnapshottedProxyResponse> {
		self.map_err(|e| SnapshottedProxyResponse(e.into()))
	}
}

impl HTTPProxy {
	pub async fn proxy(
		&self,
		connection: Arc<Extension>,
		mut req: ::http::Request<Incoming>,
	) -> Response {
		let start = agent_core::Timestamp::now();

		dtrace::trace(|f| f.request_started());
		// Copy connection level attributes into request level attributes
		let tcp = connection
			.copy::<TCPConnectionInfo>(req.extensions_mut())
			.expect("tcp connection must be set")
			.clone();
		connection.copy::<TLSConnectionInfo>(req.extensions_mut());
		connection.copy::<cel::SourceContext>(req.extensions_mut());
		connection.copy::<WaypointService>(req.extensions_mut());
		req
			.extensions_mut()
			.insert(RequestTime(start.as_datetime()));
		let log = RequestLog::new(
			log::CelLogging::new(
				self.inputs.cfg.logging.clone(),
				self.inputs.cfg.metrics.clone(),
			),
			self.inputs.metrics.clone(),
			start,
			tcp.clone(),
		);
		let mut log: DropOnLog = log.into();

		// Setup ResponsePolicies outside of proxy_internal, so we have can unconditionally run them even on errors
		// or direct responses
		let mut response_policies = ResponsePolicies::default();
		let ret = self
			.proxy_internal(req, log.as_mut().unwrap(), &mut response_policies)
			.await
			.map_err(|e| e.0);

		log.with(|l| {
			l.error = ret.as_ref().err().and_then(|e| {
				if let ProxyResponse::Error(e) = e {
					Some(e.to_string())
				} else {
					None
				}
			})
		});
		let reason = match &ret {
			Ok(_) => ProxyResponseReason::Upstream,
			Err(e) => e.as_reason(),
		};
		let mut resp = ret.unwrap_or_else(|err| match err {
			ProxyResponse::Error(e) => e.into_response(),
			ProxyResponse::DirectResponse(dr) => *dr,
		});

		if let Some(l) = log.as_mut() {
			l.cel.ctx().maybe_buffer_response_body(&mut resp).await;
		}

		let mut resp = match response_policies
			.apply(
				&mut resp,
				log.as_mut().unwrap(),
				reason == ProxyResponseReason::Upstream,
			)
			.await
		{
			Ok(_) => resp,
			Err(e) => match e {
				ProxyResponse::Error(e) => e.into_response(),
				ProxyResponse::DirectResponse(dr) => *dr,
			},
		};
		if let Some(log) = log.as_mut() {
			dtrace::snapshot!(Response, "final response", log, &resp);
		}

		// Pass the log into the body so it finishes once the stream is entirely complete.
		// We will also record trailer info there.
		log.with(|l| {
			l.status = Some(resp.status());
			l.reason = Some(reason);
			l.retry_after = http::outlierdetection::retry_after(resp.status(), resp.headers());
			l.response_snapshot = l.cel.cel_context.maybe_snapshot_response(&mut resp);
		});

		if resp.status() == StatusCode::SWITCHING_PROTOCOLS {
			let Some(req_upgrade) = resp.extensions_mut().remove::<RequestUpgrade>() else {
				return ProxyError::UpgradeFailed(None, None).into_response();
			};
			handle_upgrade(req_upgrade, resp, log)
				.await
				.unwrap_or_else(|e| e.into_response())
		} else {
			resp.map(move |b| http::Body::new(LogBody::new(b, log)))
		}
	}

	async fn proxy_internal(
		&self,
		req: ::http::Request<Incoming>,
		log: &mut RequestLog,
		response_policies: &mut ResponsePolicies,
	) -> Result<Response, SnapshottedProxyResponse> {
		log.tls_info = req.extensions().get::<TLSConnectionInfo>().cloned();
		log.backend_protocol = Some(cel::BackendProtocol::http);

		let selected_listener = self.selected_listener.clone();
		let inputs = self.inputs.clone();
		let bind_name = self.bind_name.clone();
		debug!(bind=%bind_name, "route for bind");
		let mut req = req.map(http::Body::new);

		let Some(bind) = inputs.stores.read_binds().bind(&bind_name) else {
			return Err(ProxyResponse::Error(ProxyError::BindNotFound)).snapshot_on_err(log, &mut req);
		};

		sensitive_headers(&mut req);
		normalize_uri(log.tls_info.as_ref(), &mut req)
			.map_err(ProxyError::Processing)
			.snapshot_on_err(log, &mut req)?;
		let mut req_upgrade = hop_by_hop_headers(&mut req);

		let host = http::get_host(&req)
			.map(|s| s.to_string())
			.snapshot_on_err(log, &mut req)?;
		log.host = Some(host.clone());
		log.method = Some(req.method().clone());
		log.path = Some(
			req
				.uri()
				.path_and_query()
				.map(|pq| pq.to_string())
				.unwrap_or_else(|| req.uri().path().to_string()),
		);
		log.version = Some(req.version());
		dtrace::snapshot!(Request, "initial request", &req);

		// Now check if we actually have a listener - fail after tracing is set up
		let selected_listener = selected_listener
			.or_else(|| bind.listeners.best_match_http(&host))
			.ok_or(ProxyError::ListenerNotFound);
		let selected_listener = match selected_listener {
			Ok(l) => {
				debug!(bind=%bind_name, listener=%l.key, "selected listener");
				let frontend_policies = inputs.stores.read_binds().listener_frontend_policies(
					&l.name,
					Some(bind.address.port()),
					req
						.extensions()
						.get::<WaypointService>()
						.map(WaypointService::as_policy_ref),
				);

				self
					.handle_frontend_policies(&frontend_policies, log, &mut req)
					.await;
				l
			},
			Err(e) => {
				let frontend_policies = inputs
					.stores
					.read_binds()
					.frontend_policies(self.inputs.cfg.gateway_port_ref(bind.address.port()));
				self
					.handle_frontend_policies(&frontend_policies, log, &mut req)
					.await;
				return Err(ProxyResponse::Error(e)).snapshot_on_err(log, &mut req);
			},
		};
		log.bind_name = Some(bind_name.clone());
		log.listener_name = Some(selected_listener.name.clone());

		let gateway_policies = inputs
			.stores
			.read_binds()
			.gateway_policies(&selected_listener.name);
		gateway_policies.register_cel_expressions(log.cel.ctx());
		// This is unfortunate but we record the request twice possibly; we want to record it as early as possible
		// (for logging, etc) and also after we register the expressions since new fields may be available.
		log.cel.ctx().maybe_buffer_request_body(&mut req).await;

		apply_gateway_policies(
			&gateway_policies,
			self.policy_client(),
			log,
			&mut req,
			response_policies,
		)
		.await
		.snapshot_on_err(log, &mut req)?;
		dtrace::snapshot!(Request, "gateway policies", &req);

		Self::detect_misdirected(log, &bind, &req, &selected_listener)
			.snapshot_on_err(log, &mut req)?;

		let selected_route_chain =
			select_route_chain(&inputs, self.target_address, &selected_listener, &req)
				.snapshot_on_err(log, &mut req)?;
		let selected_route = selected_route_chain
			.routes
			.last()
			.expect("route chain always contains the initially selected route")
			.clone();
		let path_match = selected_route_chain.path_match.clone();
		log.route_name = Some(selected_route.name.clone());
		// Record the matched path for tracing/logging span names
		log.path_match = Some(match &path_match {
			PathMatch::Exact(p) => p.clone(),
			PathMatch::PathPrefix(p) => {
				if p == "/" {
					strng::literal!("/*")
				} else {
					strng::format!("{}/*", p)
				}
			},
			PathMatch::Regex(r) => r.as_str().into(),
			PathMatch::Invalid => strng::literal!("<invalid>"),
		});
		req.extensions_mut().insert(path_match);

		debug!(bind=%bind_name, listener=%selected_listener.key, route=%selected_route.key, "selected route");

		let route_path = RoutePath {
			listener: &selected_listener.name,
			service: selected_route_chain
				.routes
				.last()
				.and_then(|r| r.service_key.as_ref()),
			routes: selected_route_chain
				.routes
				.iter()
				.map(|route| &route.name)
				.collect(),
		};
		let route_inline_policies = selected_route_chain
			.routes
			.iter()
			.map(|route| route.inline_policies.as_slice())
			.collect::<Vec<_>>();

		let route_policies = inputs
			.stores
			.read_binds()
			.route_policies(&route_path, &route_inline_policies);
		// Register all expressions
		route_policies.register_cel_expressions(log.cel.ctx());
		log.retry_backoff = route_policies.retry.as_ref().and_then(|r| r.backoff);
		log.cel.ctx().maybe_buffer_request_body(&mut req).await;

		// Others are set only when they have gotten to the appropriate phase of the request, so we simulate
		// a middleware-style approach where if the request side never runs, neither does the response side.
		response_policies.timeout = route_policies.timeout.clone();

		apply_request_policies(
			&route_policies,
			&self.policy_client(),
			log,
			&mut req,
			response_policies,
		)
		.await
		.snapshot_on_err(log, &mut req)?;
		dtrace::snapshot!(Request, "route policies", &req);

		let selected_backend_ref = selected_route_chain
			.backend
			.ok_or(ProxyError::NoValidBackends)
			.snapshot_on_err(log, &mut req)?;
		let selected_backend =
			resolve_backend(selected_backend_ref, self.inputs.as_ref()).snapshot_on_err(log, &mut req)?;
		let backend_policies = get_backend_policies(
			self.inputs.as_ref(),
			&selected_backend.backend,
			&selected_backend.inline_policies,
			Some(route_path.clone()),
		);
		backend_policies.register_cel_expressions(log.cel.ctx());
		log.cel.ctx().maybe_buffer_request_body(&mut req).await;
		log.health_policy = backend_policies.health.clone();
		if let Some(ev) = &backend_policies.health
			&& let Some(expr) = &ev.unhealthy_expression
		{
			log.cel.ctx().register_expression(expr.as_ref());
		}
		log.backend_info = Some(selected_backend.backend.backend.backend_info());
		if let Some(bp) = selected_backend.backend.backend.backend_protocol() {
			log.backend_protocol = Some(bp)
		}

		let (head, body) = req.into_parts();
		for mirror in route_policies
			.request_mirror
			.iter()
			.chain(backend_policies.request_mirror.iter())
		{
			if !rand::rng().random_bool(mirror.percentage) {
				trace!(
					"skipping mirror, percentage {} not triggered",
					mirror.percentage
				);
				continue;
			}
			// TODO: mirror the body. For now, we just ignore the body
			let req = Request::from_parts(head.clone(), http::Body::empty());
			let inputs = inputs.clone();
			let policy_client = self.policy_client();
			let mirror = mirror.clone();
			tokio::task::spawn(async move {
				if let Err(e) = send_mirror(inputs, policy_client, mirror, req).await {
					warn!("error sending mirror request: {}", e);
				}
			});
		}

		const MAX_BUFFERED_BYTES: usize = 64 * 1024;
		let retries = route_policies.retry.clone();

		// LLM token rate limiting reuses the rate-limit policy selected above in the normal
		// request-policy flow. Conditional rate-limit expressions are evaluated only once there;
		// the LLM path must not re-run conditions against the provider-specific rewritten request.
		let mut llm_request_policies = std::mem::take(&mut response_policies.llm_request_policies);
		llm_request_policies.llm = route_policies.llm.clone();
		let llm_request_policies = Arc::new(llm_request_policies);

		// attempts is the total number of attempts, not the retries
		let attempts = retries.as_ref().map(|r| r.attempts.get() + 1).unwrap_or(1);
		let retry_backoff = retries.as_ref().and_then(|r| r.backoff);
		let request_timeout = response_policies
			.timeout
			.as_ref()
			.and_then(|t| t.request_timeout);
		let body = if attempts > 1 {
			// If we are going to attempt a retry we will need to track the incoming bytes for replay
			let body = http::retry::ReplayBody::try_new(body, MAX_BUFFERED_BYTES);
			if body.is_err() {
				debug!("initial body is too large to retry, disabling retries")
			}
			body
		} else {
			Err(body)
		};
		let mut next = match body {
			Ok(retry) => Some(retry),
			Err(body) => {
				trace!("no retries");
				// no retries at all, just send the request as normal
				let req = Request::from_parts(head, http::Body::new(body));
				return self
					.attempt_upstream(
						log,
						&mut req_upgrade,
						llm_request_policies,
						&selected_backend,
						backend_policies,
						response_policies,
						req,
					)
					.await;
			},
		};
		let mut last_res: Option<Result<Response, SnapshottedProxyResponse>> = None;
		for n in 0..attempts {
			let last = n == attempts - 1;
			let this = next.take().expect("next should be set");
			debug!("attempt {n}/{}", attempts - 1);
			if matches!(this.is_capped(), None | Some(true)) {
				// This could be either too much buffered, or it could mean we got a response before we read the request body.
				debug!("buffered too much to attempt a retry");
				return last_res.expect("should only be capped if we had a previous attempt");
			}
			if !last {
				// Stop cloning on our last
				next = Some(this.clone());
			}
			let mut head = head.clone();
			if n > 0 {
				log.retry_attempt = Some(n);
				head.headers.insert(
					HeaderName::from_static("x-retry-attempt"),
					HeaderValue::try_from(format!("{n}")).expect("number is always a valid header value"),
				);
			}
			let req = Request::from_parts(head, http::Body::new(this));
			let res = self
				.attempt_upstream(
					log,
					&mut req_upgrade,
					llm_request_policies.clone(),
					&selected_backend,
					backend_policies.clone(),
					response_policies,
					req,
				)
				.await;
			if last || !should_retry(&res, retries.as_ref().unwrap()) {
				if !last {
					debug!("response not retry-able");
				}
				return res;
			}
			debug!(
				backoff=?retry_backoff,
				"attempting another retry, last result was {} {:?}",
				res.is_err(),
				res.as_ref().map(|r| r.status())
			);
			last_res = Some(res);
			if let Some(bo) = retry_backoff {
				let fut = if let Some(request_timeout) = request_timeout {
					let deadline = tokio::time::Instant::from_std(log.start.as_instant() + request_timeout);
					tokio::time::timeout_at(deadline, tokio::time::sleep(bo)).await
				} else {
					tokio::time::sleep(bo).await;
					Ok(())
				};
				fut
					.map_err(|_| ProxyError::RequestTimeout)
					// This is safe because we guarantee in attempt_upstream to snapshot
					.explicitly_skip_snapshot()?
			}
		}
		unreachable!()
	}

	async fn handle_frontend_policies(
		&self,
		frontend_policies: &FrontendPolices,
		log: &mut RequestLog,
		req: &mut Request,
	) {
		frontend_policies.register_cel_expressions(log.cel.ctx());

		if let Some(lp) = &frontend_policies.access_log {
			apply_logging_policy_to_log(log, lp);
		}

		if let Some(mf) = &frontend_policies.metrics_fields
			&& !mf.add.is_empty()
		{
			log.cel.metric_fields = crate::telemetry::log::MetricFields {
				add: mf.add.clone(),
			};
		}

		if let Some(alp) = frontend_policies.access_log_otlp.as_deref() {
			log.otel_logger = alp
				.get_or_init(self.policy_client())
				.map(|l| Some(l.clone()))
				.unwrap_or_else(|e| {
					warn!("failed to initialize OTLP access logger: {e}");
					None
				});
		}

		let mut sampler = TraceSampler::default();
		if let Some(tp) = frontend_policies.tracing.as_deref() {
			// Apply sampling overrides if present
			if let Some(rs) = &tp.config.random_sampling {
				sampler.random_sampling = Some(rs.clone());
				log.cel.cel_context.register_expression(rs.as_ref());
			}
			if let Some(cs) = &tp.config.client_sampling {
				sampler.client_sampling = Some(cs.clone());
				log.cel.cel_context.register_expression(cs.as_ref());
			}
			// Re-apply request so any newly required attributes are captured before sampling
		}
		log.cel.ctx().maybe_buffer_request_body(req).await;

		let trace_parent = trc::TraceParent::from_request(req);
		let trace_sampled = sampler.trace_sampled(req, trace_parent.as_ref());

		// Use dynamic tracer from frontend policy if available, otherwise use static tracer
		if trace_sampled {
			log.tracer = if let Some(tp) = frontend_policies.tracing.as_deref() {
				debug!(
					resources_count=%tp.config.resources.len(),
					attrs_count=%tp.config.attributes.len(),
					"Using dynamic tracer from frontend policy"
				);

				tp.get_or_init(self.policy_client())
					.map(|t| Some(t.clone()))
					.unwrap_or_else(|e| {
						warn!("ignoring invalid tracing policy: {e}");
						None
					})
			} else {
				None
			};
			// Register CEL expressions from the tracer
			if let Some(tracer) = &log.tracer {
				log.cel.register(tracer.fields.as_ref());
			}

			// Now create outgoing span with the correct tracer already set
			let ns = match trace_parent {
				Some(tp) => {
					// Build a new span off the existing trace
					let ns = tp.new_span();
					log.incoming_span = Some(tp);
					ns
				},
				None => {
					// Build an entirely new trace
					let mut ns = TraceParent::new();
					ns.flags = 1;
					ns
				},
			};
			ns.insert_header(req);
			req.extensions_mut().insert(ns.clone());
			log.outgoing_span = Some(ns);
		}
	}

	fn detect_misdirected(
		log: &RequestLog,
		bind: &Bind,
		req: &Request,
		selected_listener: &Listener,
	) -> Result<(), ProxyError> {
		if log.tls_info.is_none() {
			// Only applicable for HTTPS
			return Ok(());
		}
		// From the spec:
		// * If another Listener has an exact match or more specific wildcard entry,
		//   the Gateway SHOULD return a 421.
		// * If the current Listener (selected by SNI matching during ClientHello)
		//   does not match the Host:
		//     * If another Listener does match the Host, the Gateway SHOULD return a
		//       421.
		//     * If no other Listener matches the Host, the Gateway MUST return a
		//       404.
		let host = http::get_host(req).map_err(|_| ProxyError::RouteNotFound)?;
		// Use protocol-filtered matching: since we're in a TLS context (checked
		// above), only compare against other TLS-capable listeners. Without this
		// filter, an HTTP listener with the same wildcard hostname could be
		// returned by best_match(), causing a spurious 421 when BindProtocol::auto
		// serves both HTTP and HTTPS listeners on the same bind.
		let new_best_listener = bind
			.listeners
			.best_match_tls(host)
			.filter(|l| l.key != selected_listener.key);

		// "If another listener has a more specific match..."
		if let Some(new_best) = new_best_listener {
			debug!(
				"misdirected, more specific match for {host} ({})",
				new_best.key
			);
			return Err(ProxyError::MisdirectedRequest);
		}
		let host_matches_listener = selected_listener.matches(host);
		// "If the current Listener does not match the host..."
		if !host_matches_listener {
			debug!(
				"misdirected, host {host} no longer matches ({})",
				selected_listener.key
			);
			Err(ProxyError::RouteNotFound)
		} else {
			Ok(())
		}
	}

	#[allow(clippy::too_many_arguments)]
	async fn attempt_upstream(
		&self,
		log: &mut RequestLog,
		req_upgrade: &mut Option<RequestUpgrade>,
		route_policies: Arc<store::LLMRequestPolicies>,
		selected_backend: &RouteBackend,
		backend_policies: BackendPolicies,
		response_policies: &mut ResponsePolicies,
		mut req: Request,
	) -> Result<Response, SnapshottedProxyResponse> {
		if let Some(backend_timeout) = response_policies
			.timeout
			.as_ref()
			.and_then(|t| t.backend_request_timeout)
		{
			req
				.extensions_mut()
				.insert(BackendRequestTimeout(backend_timeout));
		}
		let mut req_opt = Some(req);
		let timeout = response_policies
			.timeout
			.as_ref()
			.and_then(|t| t.request_timeout);
		let start = log.start;
		let call = make_backend_call(
			self.inputs.clone(),
			route_policies.clone(),
			&selected_backend.backend.backend,
			backend_policies,
			MustSnapshot::new(&mut req_opt),
			Some(log),
			response_policies,
		);

		// Setup timeout
		let call_result = if let Some(timeout) = timeout {
			let deadline = tokio::time::Instant::from_std(start.as_instant() + timeout);
			let fut = tokio::time::timeout_at(deadline, call);
			fut.await
		} else {
			Ok(call.await)
		};

		// Run the actual call
		let mut resp = match call_result {
			Ok(Ok(resp)) => resp,
			Ok(Err(e)) => {
				return Err(e).maybe_snapshot_on_err(log, &mut req_opt)?;
			},
			Err(_) => {
				return Err(ProxyResponse::Error(ProxyError::RequestTimeout))
					.maybe_snapshot_on_err(log, &mut req_opt)?;
			},
		};
		if resp.status() == StatusCode::SWITCHING_PROTOCOLS {
			let Some(upgrade) = req_upgrade.take() else {
				return Err(ProxyResponse::Error(ProxyError::UpgradeFailed(None, None)))
					.maybe_snapshot_on_err(log, &mut req_opt)?;
			};
			resp.extensions_mut().insert(upgrade);
		}

		// gRPC status can be in the initial headers or a trailer, add if they are here
		maybe_set_grpc_status(&log.grpc_status, resp.headers());

		Ok(resp)
	}

	fn policy_client(&self) -> PolicyClient {
		PolicyClient {
			inputs: self.inputs.clone(),
		}
	}
}

fn resolve_backend(b: RouteBackendReference, pi: &ProxyInputs) -> Result<RouteBackend, ProxyError> {
	let backend_ref = b
		.target
		.as_backend_reference()
		.ok_or(ProxyError::InvalidBackendType)?;
	let backend = super::resolve_backend(&backend_ref, pi)?;
	Ok(RouteBackend {
		weight: b.weight,
		backend,
		inline_policies: b.inline_policies,
	})
}

async fn handle_upgrade(
	req_upgrade_type: RequestUpgrade,
	mut resp: Response,
	log: DropOnLog,
) -> Result<Response, ProxyError> {
	let RequestUpgrade {
		upgrade_type,
		upgrade,
	} = req_upgrade_type;
	let resp_upgrade_type = get_upgrade_type(resp.headers());
	if Some(&upgrade_type) != resp_upgrade_type.as_ref() {
		return Err(ProxyError::UpgradeFailed(
			Some(upgrade_type),
			resp_upgrade_type,
		));
	}
	let response_upgraded = resp
		.extensions_mut()
		.remove::<OnUpgrade>()
		.ok_or_else(|| ProxyError::ProcessingString("no upgrade".to_string()))?
		.await
		.map_err(|e| ProxyError::ProcessingString(format!("upgrade failed: {e:?}")))?;
	tokio::task::spawn(async move {
		let req = match upgrade.await {
			Ok(u) => u,
			Err(e) => {
				error!("upgrade error: {e}");
				return;
			},
		};
		let mut server = TokioIo::new(response_upgraded);
		if let Some(log) = log.as_ref()
			&& let Some(llm_req) = log.llm_request.as_ref()
			&& llm_req.input_format == InputFormat::Realtime
		{
			let llm = log.llm_response.clone();
			let llm_info = LLMInfo::new(llm_req.clone(), LLMResponse::default());
			llm.store(Some(llm_info));
			let mut server = parse::websocket::parser(server, llm).await;
			let _ = agent_core::copy::copy_bidirectional(
				&mut TokioIo::new(req),
				&mut server,
				&agent_core::copy::ConnectionResult {},
			)
			.await;
		} else {
			let _ = agent_core::copy::copy_bidirectional(
				&mut TokioIo::new(req),
				&mut server,
				&agent_core::copy::ConnectionResult {},
			)
			.await;
		}
		// Make sure we only emit log after we are done with the entire connection
		drop(log);
	});
	Ok(resp)
}

pub async fn build_transport(
	inputs: &ProxyInputs,
	backend_call: &BackendCall,
	backend_tls: Option<BackendTLS>,
	backend_tunnel: Option<&backend::Tunnel>,
	backend_http_version_override: Option<::http::Version>,
) -> Result<Transport, ProxyError> {
	let backend_tls = backend_tls.map(|btls| btls.config_for(backend_http_version_override));
	let app_transport = if let Some(tls) = backend_tls {
		ApplicationTransport::Tls(tls)
	} else {
		ApplicationTransport::Plaintext
	};
	if let Some(tun) = backend_tunnel {
		let backend = super::resolve_simple_backend_with_policies(&tun.proxy, inputs)?;
		let pols = crate::proxy::tcpproxy::get_backend_policies(inputs, &backend, &[], None);
		let call = TCPProxy::build_backend_call(&mut None, None, inputs, &backend.backend, pols)?;
		let tunnel_backend_tls = call.backend_policies.backend_tls.clone();
		let tunnel_auth = call.backend_policies.backend_auth.clone();
		// This is a bounded recursion; this code is only called when backend_tunnel is set, and in this call
		// we never set it.
		let transport = Box::pin(build_transport(
			inputs,
			&call,
			tunnel_backend_tls,
			None,
			// Currently we only support HTTP/1.1
			Some(::http::Version::HTTP_11),
		))
		.await?;
		trace!("built tunnel to {:?}", call.target);
		let token = if let Some(auth) = tunnel_auth {
			Some(auth::apply_tunnel_auth(&auth)?)
		} else {
			None
		};
		let tc = client::TunnelConfig {
			transport: Box::new(transport),
			target: call.target,
			token,
		};
		return Ok(Transport::Tunnel(app_transport, tc));
	}

	// Check if we need double hbone
	if let (
		Some((gw_addr, gw_identity)),
		Some((InboundProtocol::HBONE, waypoint_identities)),
		Some(ca),
	) = (
		&backend_call.network_gateway,
		&backend_call.transport_override,
		&inputs.ca,
	) {
		if ca.get_identity().await.is_ok() {
			// Extract gateway IP from the gateway address
			let gateway_ip = match &gw_addr.destination {
				types::discovery::gatewayaddress::Destination::Address(net_addr) => net_addr.address,
				types::discovery::gatewayaddress::Destination::Hostname(_) => {
					warn!("hostname-based gateway addresses not yet supported");
					return Ok(app_transport.into());
				},
			};

			let gateway_socket_addr = SocketAddr::new(gateway_ip, gw_addr.hbone_mtls_port);

			tracing::debug!(
				"using double hbone through gateway {:?} at {}",
				gw_addr,
				gateway_socket_addr
			);
			return Ok(Transport::DoubleHbone {
				gateway_address: gateway_socket_addr,
				gateway_identity: gw_identity.clone(),
				waypoint_identities: waypoint_identities.clone(),
				inner: app_transport,
			});
		} else {
			warn!("wanted double hbone but CA is not available");
			return Ok(app_transport.into());
		}
	}

	Ok(match (&backend_call.transport_override, &inputs.ca) {
		// Use legacy mTLS if they did not define a TLS policy. We could do double TLS but Istio doesn't,
		// so maintain bug-for-bug parity
		(Some((InboundProtocol::LegacyIstioMtls, idents)), Some(ca))
			if matches!(app_transport, ApplicationTransport::Plaintext) =>
		{
			if let Ok(id) = ca.get_identity().await {
				Some(
					id.legacy_mtls(idents.clone())
						.map_err(|e| ProxyError::Processing(anyhow!("{e}")))?,
				)
				.into()
			} else {
				warn!("wanted TLS but CA is not available");
				app_transport.into()
			}
		},
		(Some((InboundProtocol::HBONE, idents)), Some(ca)) => {
			if ca.get_identity().await.is_ok() {
				Transport::Hbone(app_transport, idents.clone())
			} else {
				warn!("wanted TLS but CA is not available");
				app_transport.into()
			}
		},
		(_, _) => app_transport.into(),
	})
}

fn get_backend_policies(
	inputs: &ProxyInputs,
	// Backend, and policies specifically inlined on this backend object
	backend: &BackendWithPolicies,
	inline_policies: &[BackendTrafficPolicy],
	path: Option<RoutePath>,
) -> BackendPolicies {
	inputs.stores.read_binds().backend_policies(
		backend.backend.target_ref(),
		// Precedence: Selector < Backend inline < backendRef inline
		// Note this differs from the logical chain of objects (Route -> backendRef -> backend),
		// because a backendRef is actually more specific: its one *specific usage* of the backend.
		// For example, we may say to use TLS for a Backend, but in a specific TLSRoute backendRef we disable
		// as it is already TLS.
		&[&backend.inline_policies, inline_policies],
		path,
	)
}

pub struct MustSnapshot<'a>(&'a mut Option<Request>);

impl<'a> MustSnapshot<'a> {
	pub fn new(req: &'a mut Option<Request>) -> Self {
		Self(req)
	}
	pub fn take_and_snapshot_clearing_extensions(
		self,
		log: Option<&mut &mut RequestLog>,
	) -> Result<Request, ProxyError> {
		self.take_and_snapshot(log, true)
	}
	pub fn take_and_snapshot_without_clearing_extensions(
		self,
		log: Option<&mut &mut RequestLog>,
	) -> Result<Request, ProxyError> {
		self.take_and_snapshot(log, false)
	}
	fn take_and_snapshot(
		self,
		mut log: Option<&mut &mut RequestLog>,
		clear: bool,
	) -> Result<Request, ProxyError> {
		if let Some(mut req) = self.0.take() {
			if let Some(l) = log.take() {
				// Do not clear extensions
				l.request_snapshot = l.cel.cel_context.maybe_snapshot_request(&mut req, clear);
			};
			Ok(req)
		} else {
			Err(ProxyError::ProcessingString(
				"request already snapshot".into(),
			))
		}
	}
}

impl Deref for MustSnapshot<'_> {
	type Target = Request;
	fn deref(&self) -> &Self::Target {
		self.0.as_ref().expect("unreachable")
	}
}
impl DerefMut for MustSnapshot<'_> {
	fn deref_mut(&mut self) -> &mut Self::Target {
		self.0.as_mut().expect("unreachable")
	}
}

async fn make_backend_call(
	inputs: Arc<ProxyInputs>,
	route_policies: Arc<store::LLMRequestPolicies>,
	backend: &Backend,
	base_policies: BackendPolicies,
	mut req: MustSnapshot<'_>,
	mut log: Option<&mut RequestLog>,
	response_policies: &mut ResponsePolicies,
) -> Result<Response, ProxyResponse> {
	let policy_client = PolicyClient {
		inputs: inputs.clone(),
	};

	// The MCP backend aggregates multiple backends into a single backend.
	// In some cases, we want to treat this as a normal backend, so we swap it out.
	let (backend, policies) = match backend {
		Backend::MCP(_, mcp_backend) => {
			if let Some(be) =
				inputs
					.clone()
					.mcp_state
					.should_passthrough(&base_policies, mcp_backend, &req)
			{
				let target = super::resolve_simple_backend_with_policies(&be, inputs.as_ref())?;
				let tgt = target.backend.target();
				let policies = inputs
					.stores
					.read_binds()
					.sub_backend_policies(tgt, Some(&target.inline_policies));

				(
					&Backend::from(target.backend),
					base_policies.merge(policies),
				)
			} else {
				(backend, base_policies)
			}
		},
		_ => (backend, base_policies),
	};

	log.add(|l| {
		l.backend_info = Some(backend.backend_info());
		if let Some(bp) = backend.backend_protocol() {
			l.backend_protocol = Some(bp)
		}
	});

	let mut maybe_inference = policies.build_inference(policy_client.clone());
	let inference_result = maybe_inference.mutate_request(&mut req).await?;
	inference_result
		.policy_response
		.apply(response_policies.headers())?;
	log.add(|l| l.inference_pool = inference_result.destination);

	// Use inference override if present, otherwise check for stateful MCP pinning.
	// In practice, these don't conflict: inference is for AI backends, MCP pinning is for MCP backends.
	let service_override = ServiceCallOverride {
		destination: inference_result.destination.or(policies.override_dest),
		destination_passthrough: inference_result.destination.is_some()
			&& matches!(
				inference_result.destination_mode,
				InferenceRoutingDestinationMode::Passthrough
			),
		inference_failed_open: inference_result.failed_open,
	};

	let backend_call = match backend {
		Backend::AI(n, ai) => {
			let (provider, handle) = ai.select_provider().ok_or(ProxyError::NoHealthyEndpoints)?;
			log.add(move |l| l.request_handle = Some(handle));
			let sub_backend_name = BackendTargetRef::Backend {
				name: n.name.as_ref(),
				namespace: n.namespace.as_ref(),
				section: Some(provider.name.as_ref()),
			};
			let sub_backend_policies = inputs
				.stores
				.read_binds()
				.sub_backend_policies(sub_backend_name, Some(&provider.inline_policies));

			let (target, provider_defaults) = match &provider.host_override {
				Some(target) => (
					target.clone(),
					BackendPolicies {
						// Attach LLM provider, but don't use default setup
						llm_provider: Some(provider.clone()),
						..Default::default()
					},
				),
				None => {
					let (tgt, mut pol) = provider.provider.default_connector();
					pol.llm_provider = Some(provider.clone());
					(tgt, pol)
				},
			};
			// Defaults for the provider < Backend level policies < Sub Backend
			let effective_policies = provider_defaults
				.merge(policies)
				.merge(sub_backend_policies);
			BackendCall {
				target,
				backend_policies: effective_policies,
				http_version_override: None,
				transport_override: None,
				network_gateway: None,
			}
		},
		Backend::Service(svc, port) => build_service_call(
			&inputs,
			policies,
			&mut log,
			service_override,
			svc,
			port,
			req.uri().host(),
		)?,
		Backend::Opaque(_, target) => BackendCall {
			target: target.clone(),
			http_version_override: None,
			transport_override: None,
			network_gateway: None,
			backend_policies: policies,
		},
		Backend::Aws(_, config) => {
			http::modify_req_uri(&mut req, |uri| {
				let host_with_port = format!("{}:443", config.get_host());
				uri.authority =
					Some(Authority::try_from(host_with_port.as_str()).map_err(anyhow::Error::msg)?);
				uri.path_and_query = Some(PathAndQuery::from_str(&config.get_path())?);
				Ok(())
			})
			.map_err(ProxyError::Processing)?;

			req.extensions_mut().insert(llm::bedrock::AwsRegion {
				region: config.region().to_string(),
			});
			req.extensions_mut().insert(llm::bedrock::AwsServiceName {
				name: config.service_name(),
			});

			let default_policies = BackendPolicies {
				backend_tls: Some(http::backendtls::SYSTEM_TRUST.clone()),
				backend_auth: Some(auth::BackendAuth::Aws(auth::AwsAuth::Implicit {})),
				..Default::default()
			};
			BackendCall {
				target: Target::Hostname(config.get_host().into(), 443),
				backend_policies: default_policies.merge(policies),
				http_version_override: None,
				transport_override: None,
				network_gateway: None,
			}
		},
		Backend::Dynamic(_, _) => {
			let host = http::get_host(&req)?;
			let port = req
				.uri()
				.port_u16()
				.unwrap_or_else(|| match req.uri().scheme() {
					Some(s) if *s == Scheme::HTTPS => 443,
					_ => 80,
				});
			let target = Target::from((host, port));
			BackendCall {
				target,
				http_version_override: None,
				transport_override: None,
				network_gateway: None,
				backend_policies: policies,
			}
		},
		Backend::MCP(name, backend) => {
			let inputs = inputs.clone();
			let backend = backend.clone();
			set_backend_cel_context(&mut req, log.as_ref());
			let name = name.clone();
			let Some(log) = log else {
				return Err(
					ProxyError::ProcessingString("invalid: log required for MCP".to_string()).into(),
				);
			};
			let res = inputs
				.clone()
				.mcp_state
				.serve(inputs, name, backend, policies, req, log)
				.await;
			return res.map_err(ProxyResponse::from);
		},
		Backend::Invalid => return Err(ProxyResponse::from(ProxyError::BackendDoesNotExist)),
	};

	// Apply auth before LLM request setup, so the providers can assume auth is in standardized header
	// Apply auth as early as possible so any ext_proc or transformations won't be repeated on retries in case it fails.
	let backend_info = auth::BackendInfo {
		target: backend.target(),
		call_target: backend_call.target.clone(),
		inputs: inputs.clone(),
	};
	apply_backend_policies(
		backend_info.clone(),
		&backend_call,
		&mut req,
		&mut log,
		response_policies,
	)
	.await?;

	log.add(|l| {
		l.endpoint = Some(backend_call.target.clone());
	});

	let llm_request_policies =
		route_policies.merge_backend_policies(backend_call.backend_policies.llm.clone());

	set_backend_cel_context(&mut req, log.as_ref());

	let (mut req, llm_response_policies, llm_request) =
		if let Some(llm) = &backend_call.backend_policies.llm_provider {
			// LLM requires CEL execution after the snapshot so we do not clear extensions
			let mut req = req.take_and_snapshot_without_clearing_extensions(log.as_mut())?;
			let route_type = llm_request_policies
				.llm
				.as_ref()
				.map(|policy| policy.resolve_route(req.uri().path()))
				.unwrap_or(llm::RouteType::Completions);
			trace!("llm: route {} to {route_type:?}", req.uri().path());
			// First, we process the incoming request. This entails translating to the relevant provider,
			// and parsing the request to build the LLMRequest for logging/etc, and applying LLM policies like
			// prompt enrichment, prompt guard, etc.
			match route_type {
				RouteType::Completions
				| RouteType::Messages
				| RouteType::Responses
				| RouteType::AnthropicTokenCount
				| RouteType::Embeddings
				| RouteType::Detect => {
					let r = match route_type {
						RouteType::Completions => Box::pin(llm.provider.process_completions_request(
							&backend_info,
							llm_request_policies.llm.as_deref(),
							req,
							llm.tokenize,
							&mut log,
						))
						.await
						.map_err(|e| ProxyError::Processing(e.into()))?,
						RouteType::Messages => Box::pin(llm.provider.process_messages_request(
							&backend_info,
							llm_request_policies.llm.as_deref(),
							req,
							llm.tokenize,
							&mut log,
						))
						.await
						.map_err(|e| ProxyError::Processing(e.into()))?,
						RouteType::Responses => Box::pin(llm.provider.process_responses_request(
							&backend_info,
							llm_request_policies.llm.as_deref(),
							req,
							llm.tokenize,
							&mut log,
						))
						.await
						.map_err(|e| ProxyError::Processing(e.into()))?,
						RouteType::Embeddings => Box::pin(llm.provider.process_embeddings_request(
							&backend_info,
							llm_request_policies.llm.as_deref(),
							req,
							llm.tokenize,
							&mut log,
						))
						.await
						.map_err(|e| ProxyError::Processing(e.into()))?,
						RouteType::AnthropicTokenCount => Box::pin(llm.provider.process_count_tokens_request(
							&backend_info,
							req,
							llm_request_policies.llm.as_deref(),
							&mut log,
						))
						.await
						.map_err(|e| ProxyError::Processing(e.into()))?,
						RouteType::Detect => Box::pin(llm.provider.process_detect_request(
							&backend_info,
							llm_request_policies.llm.as_deref(),
							req,
							&mut log,
						))
						.await
						.map_err(|e| ProxyError::Processing(e.into()))?,
						_ => unreachable!(),
					};
					let (mut req, llm_request) = match r {
						RequestResult::Success(r, lr) => (r, lr),
						RequestResult::Rejected(dr) => return Err(ProxyResponse::DirectResponse(Box::new(dr))),
					};
					// If a user doesn't configure explicit overrides for connecting to a provider, setup default
					// paths, TLS, etc.
					llm
						.provider
						.setup_request(
							&mut req,
							route_type,
							Some(&llm_request),
							llm.path_override.as_deref(),
							llm.path_prefix.as_deref(),
							llm.host_override.is_some(),
						)
						.map_err(ProxyError::Processing)?;

					// Apply all policies (rate limits, prompt guards, enrichment)
					// count_tokens skips policies (no tokens generated, no prompts to manipulate)
					let response_policies = if route_type == RouteType::AnthropicTokenCount {
						LLMResponsePolicies::default()
					} else {
						apply_llm_request_policies(
							&llm_request_policies,
							policy_client.clone(),
							&mut req,
							&llm_request,
							&mut response_policies.response_headers,
						)
						.await?
					};
					log.add(|l| l.llm_request = Some(llm_request.clone()));
					(req, response_policies, Some(llm_request))
				},
				RouteType::Models => {
					return Ok(
						::http::Response::builder()
							.status(::http::StatusCode::NOT_IMPLEMENTED)
							.header(::http::header::CONTENT_TYPE, "application/json")
							.body(http::Body::from(format!(
								"{{\"error\":\"Route '{route_type:?}' not implemented\"}}"
							)))
							.expect("Failed to build response"),
					);
				},
				RouteType::Passthrough | RouteType::Realtime => {
					// For passthrough, we only need to setup the response so we get default TLS, hostname, etc set.
					// We do not need LLM policies nor token-based rate limits, etc.
					// For realtime we do the same and handle everything in the Websocket handler
					llm
						.provider
						.setup_request(
							&mut req,
							route_type,
							None,
							llm.path_override.as_deref(),
							llm.path_prefix.as_deref(),
							llm.host_override.is_some(),
						)
						.map_err(ProxyError::Processing)?;
					if route_type == RouteType::Realtime {
						let request_model = http::as_url(req.uri())
							.map_err(ProxyError::Processing)?
							.query_pairs()
							.find(|(k, _v)| k == "model")
							.map(|(_, v)| strng::new(v))
							.unwrap_or_default();
						log.add(|l| {
							l.llm_request = Some(LLMRequest {
								input_format: InputFormat::Realtime,
								request_model,
								streaming: true,
								provider: llm.provider.provider(),
								input_tokens: None,
								params: Default::default(),
								prompt: Default::default(),
							})
						});
					}
					(req, LLMResponsePolicies::default(), None)
				},
			}
		} else {
			(
				// Clearing extensions is fine; the HTTP codepath doesn't require usage after this point.
				req.take_and_snapshot_clearing_extensions(log.as_mut())?,
				LLMResponsePolicies::default(),
				None,
			)
		};
	// Some auth types (AWS) need to be applied after all request processing
	auth::apply_late_backend_auth(
		backend_call.backend_policies.backend_auth.as_ref(),
		&mut req,
	)
	.await?;
	let transport = build_transport(
		&inputs,
		&backend_call,
		backend_call.backend_policies.backend_tls.clone(),
		backend_call.backend_policies.tunnel.as_ref(),
		backend_call
			.backend_policies
			.http
			.as_ref()
			.and_then(|h| h.version)
			.or(backend_call.http_version_override),
	)
	.await?;
	dtrace::snapshot!(Request, "final request", &req);
	let call = client::Call {
		req,
		target: backend_call.target,
		transport,
	};
	let backend_call_start = dtrace::timed_start();
	dtrace::trace(|trace| trace.backend_call_started(&call.target));
	let upstream = inputs.upstream.clone();
	let llm_response_log = log.as_ref().map(|l| l.llm_response.clone());
	let include_completion_in_log = log
		.as_ref()
		.map(|l| l.cel.cel_context.needs_llm_completion())
		.unwrap_or_default();
	let a2a_type = response_policies.a2a_type.clone();

	let resp = upstream.call(call).await;
	dtrace::trace(|trace| match &resp {
		Ok(resp) => trace.backend_call_completed(
			backend_call_start,
			Instant::now(),
			Some(resp.status().as_u16()),
			None,
		),
		Err(err) => trace.backend_call_completed(
			backend_call_start,
			Instant::now(),
			None,
			Some(err.to_string()),
		),
	});
	let mut resp = resp?;
	a2a::apply_to_response(
		backend_call.backend_policies.a2a.as_ref(),
		a2a_type,
		&mut resp,
	)
	.await
	.map_err(ProxyError::Processing)?;
	let mut resp = if let (Some(llm), Some(llm_request)) =
		(backend_call.backend_policies.llm_provider, llm_request)
	{
		llm
			.provider
			.process_response(
				policy_client.clone(),
				llm_request,
				llm_response_policies,
				llm_response_log.expect("must be set"),
				include_completion_in_log,
				resp,
			)
			.await
			.map_err(|e| ProxyError::Processing(e.into()))?
	} else {
		resp
	};
	// TODO: we currently do not support ImmediateResponse from inference router
	let _ = maybe_inference.mutate_response(&mut resp).await?;
	if let Some(log) = log.as_ref() {
		dtrace::snapshot!(Response, "backend response ready", log, &resp);
	}
	Ok(resp)
}

fn set_backend_cel_context(req: &mut http::Request, log: Option<&&mut RequestLog>) {
	if let Some(l) = log
		&& let Some(bp) = l.backend_protocol
		&& let Some(bi) = &l.backend_info
	{
		req.extensions_mut().insert(BackendContext {
			name: bi.backend_name.clone(),
			backend_type: bi.backend_type,
			protocol: bp,
		});
	}
}

pub fn build_service_call(
	inputs: &ProxyInputs,
	backend_policies: BackendPolicies,
	log: &mut Option<&mut RequestLog>,
	service_override: ServiceCallOverride,
	svc: &Arc<Service>,
	port: &u16,
	request_host: Option<&str>,
) -> Result<BackendCall, ProxyError> {
	let port = *port;
	let http_version_override = if svc.port_is_http2(port) {
		Some(::http::Version::HTTP_2)
	} else if svc.port_is_http1(port) {
		Some(::http::Version::HTTP_11)
	} else {
		None
	};
	if let Some(destination) = service_override.destination
		&& service_override.destination_passthrough
	{
		return Ok(BackendCall {
			target: Target::Address(destination),
			http_version_override,
			transport_override: None,
			network_gateway: None,
			backend_policies,
		});
	}

	let workloads = &inputs.stores.read_discovery().workloads;
	let (ep, handle, wl) = svc
		.endpoints
		.select_endpoint(workloads, svc.as_ref(), port, service_override.destination)
		.ok_or(ProxyError::NoHealthyEndpoints)?;

	let target_port = select_service_target_port(
		ep.as_ref(),
		svc.as_ref(),
		port,
		service_override.destination,
		service_override.inference_failed_open,
	)
	.ok_or(ProxyError::NoHealthyEndpoints)?;

	log.add(move |l| l.request_handle = Some(handle));

	// Check if we need double hbone (workload on remote network with gateway)
	let network_gateway = if wl.network != inputs.cfg.network {
		if let Some(gw_addr) = &wl.network_gateway {
			// Look up the gateway workload to get its identity
			let gateway_workload = match &gw_addr.destination {
				types::discovery::gatewayaddress::Destination::Address(net_addr) => {
					workloads.find_address(net_addr)
				},
				types::discovery::gatewayaddress::Destination::Hostname(_hostname) => {
					// TODO: Implement hostname resolution for gateway
					// For now, we don't support hostname-based gateways
					tracing::warn!("hostname-based network gateways not yet supported");
					None
				},
			};

			if let Some(gw_wl) = gateway_workload {
				tracing::debug!(
					source_network = % inputs.cfg.network,
					dest_network = % wl.network,
					gateway = ? gw_addr,
					"picked workload on remote network, using double hbone"
				);
				Some((gw_addr.clone(), gw_wl.identity()))
			} else {
				tracing::warn!(
					"network gateway {:?} not found for remote workload",
					gw_addr
				);
				None
			}
		} else {
			tracing::warn ! (
			source_network = % inputs.cfg.network,
			dest_network = % wl.network,
			"workload on remote network but no gateway configured"
			);
			None
		}
	} else {
		None
	};

	// For double HBONE, use hostname-based target so the gateway can resolve it
	let target = if network_gateway.is_some() {
		tracing::debug!(
			hostname=%svc.hostname,
			port=%port,
			"using hostname-based target for double hbone"
		);
		// Use the original service port, not the target port; the gateway will resolve it
		Target::Hostname(svc.hostname.clone(), port)
	} else {
		// TODO: this should only be used with DNS resolution type! maybe?
		if wl.workload_ips.is_empty()
			&& let Some(hostname) = resolved_workload_target_hostname(&wl.hostname, request_host)
		{
			Target::Hostname(hostname.into(), target_port)
		} else {
			// For direct connections, we need the workload IP
			let Some(ip) = wl.workload_ips.first() else {
				return Err(ProxyError::NoHealthyEndpoints);
			};
			let dest = SocketAddr::from((*ip, target_port));
			Target::Address(dest)
		}
	};

	Ok(BackendCall {
		target,
		http_version_override,
		transport_override: Some((wl.protocol, workload_and_service_sans(&wl, svc))),
		network_gateway,
		backend_policies,
	})
}

fn select_service_target_port(
	ep: &Endpoint,
	svc: &Service,
	svc_port: u16,
	override_dest: Option<SocketAddr>,
	inference_failed_open: bool,
) -> Option<u16> {
	let svc_target_port = svc.ports.get(&svc_port).copied().unwrap_or_default();
	if let Some(ov) = override_dest {
		// Use the explicit override. select_endpoint ensures this is actually in the endpoint.
		return Some(ov.port());
	}
	if inference_failed_open
		&& let Some(target_port) = ep.port.values().choose(&mut rand::rng()).copied()
	{
		return Some(target_port);
	}
	if let Some(&ep_target_port) = ep.port.get(&svc_port) {
		// prefer endpoint port mapping
		return Some(ep_target_port);
	}
	if svc_target_port > 0 {
		// otherwise, see if the service has this port
		return Some(svc_target_port);
	}
	None
}

/// Combines workload identity with service SANs.
fn workload_and_service_sans(wl: &Workload, svc: &Service) -> Vec<Identity> {
	let wl_id = wl.identity();
	let mut ids = Vec::with_capacity(1 + svc.subject_alt_names.len());
	ids.push(wl_id.clone());
	for id in &svc.subject_alt_names {
		if *id != wl_id {
			ids.push(id.clone());
		}
	}
	ids
}

fn resolved_workload_target_hostname<'a>(
	workload_hostname: &'a str,
	request_host: Option<&'a str>,
) -> Option<&'a str> {
	if workload_hostname.is_empty() {
		return None;
	}

	if let Some(wildcard_suffix) = workload_hostname.strip_prefix("*.") {
		let suffix = format!(".{wildcard_suffix}");
		request_host.filter(|host| host.ends_with(&suffix))
	} else {
		Some(workload_hostname)
	}
}

fn should_retry(res: &Result<Response, SnapshottedProxyResponse>, pol: &retry::Policy) -> bool {
	match res {
		Ok(resp) => pol.codes.contains(&resp.status()),
		Err(SnapshottedProxyResponse(ProxyResponse::Error(e))) => e.is_retryable(),
		Err(SnapshottedProxyResponse(ProxyResponse::DirectResponse(_))) => false,
	}
}

#[cfg(test)]
mod tests {
	use std::collections::{HashMap, HashSet};
	use std::net::SocketAddr;

	use super::{hop_by_hop_headers, resolved_workload_target_hostname, select_service_target_port};
	use crate::http;
	use crate::types::discovery::{AppProtocol, Endpoint, HealthStatus, Service};

	#[test]
	fn resolved_workload_target_hostname_uses_explicit_workload_hostname() {
		assert_eq!(
			resolved_workload_target_hostname("api.example.com", Some("caller.example.com")),
			Some("api.example.com")
		);
		assert_eq!(
			resolved_workload_target_hostname("api.example.com", None),
			Some("api.example.com")
		);
	}

	#[test]
	fn resolved_workload_target_hostname_uses_request_host_for_matching_wildcard() {
		assert_eq!(
			resolved_workload_target_hostname("*.example.com", Some("api.example.com")),
			Some("api.example.com")
		);
		assert_eq!(
			resolved_workload_target_hostname("*.example.com", Some("deep.api.example.com")),
			Some("deep.api.example.com")
		);
	}

	#[test]
	fn resolved_workload_target_hostname_rejects_non_matching_wildcard() {
		assert_eq!(
			resolved_workload_target_hostname("*.example.com", Some("example.com")),
			None
		);
		assert_eq!(
			resolved_workload_target_hostname("*.example.com", Some("api.other.com")),
			None
		);
		assert_eq!(
			resolved_workload_target_hostname("*.example.com", None),
			None
		);
	}

	fn multi_port_inference_service() -> Service {
		Service {
			name: "gateway-pool".into(),
			namespace: "default".into(),
			hostname: "gateway-pool.default.inference.cluster.local".into(),
			vips: Vec::new(),
			ports: HashMap::from([(8000, 8000), (8001, 8001)]),
			app_protocols: HashMap::from([(8000, AppProtocol::Http2), (8001, AppProtocol::Http2)]),
			endpoints: Default::default(),
			subject_alt_names: Vec::new(),
			waypoint: None,
			load_balancer: None,
			ip_families: None,
		}
	}

	#[tokio::test]
	async fn select_service_target_port_uses_override_destination_when_present() {
		let endpoint = Endpoint {
			workload_uid: "wl-1".into(),
			port: HashMap::from([(8000, 8000), (8001, 8001)]),
			status: HealthStatus::Healthy,
		};
		let service = multi_port_inference_service();
		let override_dest = SocketAddr::from(([10, 0, 0, 1], 8001));

		assert_eq!(
			select_service_target_port(&endpoint, &service, 8000, Some(override_dest), true),
			Some(8001)
		);
	}

	#[tokio::test]
	async fn select_service_target_port_uses_canonical_port_without_inference_fail_open() {
		let endpoint = Endpoint {
			workload_uid: "wl-1".into(),
			port: HashMap::from([(8000, 8000), (8001, 8001)]),
			status: HealthStatus::Healthy,
		};
		let service = multi_port_inference_service();

		assert_eq!(
			select_service_target_port(&endpoint, &service, 8000, None, false),
			Some(8000)
		);
	}

	#[tokio::test]
	async fn select_service_target_port_can_reach_all_ports_after_inference_fail_open() {
		let endpoint = Endpoint {
			workload_uid: "wl-1".into(),
			port: HashMap::from([(8000, 8000), (8001, 8001)]),
			status: HealthStatus::Healthy,
		};
		let service = multi_port_inference_service();
		let mut seen = HashSet::new();

		for _ in 0..64 {
			let target_port = select_service_target_port(&endpoint, &service, 8000, None, true)
				.expect("expected a target port");
			seen.insert(target_port);
			if seen.len() == 2 {
				break;
			}
		}

		assert_eq!(seen, HashSet::from([8000, 8001]));
	}

	#[test]
	fn hop_by_hop_headers_removes_connection_nominated_headers() {
		let mut req = ::http::Request::builder()
			.uri("http://app/")
			.header("connection", "x-internal-auth, x-original-url")
			.header("x-internal-auth", "1")
			.header("x-original-url", "/admin")
			.body(http::Body::empty())
			.expect("request should build");

		assert!(hop_by_hop_headers(&mut req).is_none());
		assert!(!req.headers().contains_key("connection"));
		assert!(!req.headers().contains_key("x-internal-auth"));
		assert!(!req.headers().contains_key("x-original-url"));
	}

	#[test]
	fn hop_by_hop_headers_preserves_upgrade_and_trailers_after_stripping() {
		let mut req = ::http::Request::builder()
			.uri("http://app/")
			.header("connection", "keep-alive, upgrade, x-original-url")
			.header("upgrade", "websocket")
			.header("te", "trailers")
			.header("x-original-url", "/admin")
			.body(http::Body::empty())
			.expect("request should build");

		assert!(hop_by_hop_headers(&mut req).is_none());
		assert_eq!(
			req
				.headers()
				.get("connection")
				.and_then(|v| v.to_str().ok()),
			Some("upgrade")
		);
		assert_eq!(
			req.headers().get("upgrade").and_then(|v| v.to_str().ok()),
			Some("websocket")
		);
		assert_eq!(
			req.headers().get("te").and_then(|v| v.to_str().ok()),
			Some("trailers")
		);
		assert!(!req.headers().contains_key("x-original-url"));
	}
}

pub fn maybe_set_grpc_status(status: &AsyncLog<u8>, headers: &HeaderMap) {
	if let Some(s) = headers.get("grpc-status") {
		let parsed = std::str::from_utf8(s.as_bytes())
			.ok()
			.and_then(|s| s.parse::<u8>().ok());
		status.store(parsed);
	}
}

async fn send_mirror(
	inputs: Arc<ProxyInputs>,
	upstream: PolicyClient,
	mirror: filters::RequestMirror,
	mut req: Request,
) -> Result<(), ProxyError> {
	req.headers_mut().remove(http::header::CONTENT_LENGTH);
	let backend = super::resolve_simple_backend(&mirror.backend, inputs.as_ref())?;
	let _ = upstream.call(req, backend).await?;
	Ok(())
}

// Hop-by-hop headers. These are removed when sent to the backend.
// As of RFC 7230, hop-by-hop headers are required to appear in the
// Connection header field. These are the headers defined by the
// obsoleted RFC 2616 (section 13.5.1) and are used for backward
// compatibility.
static HOP_HEADERS: [HeaderName; 9] = [
	header::CONNECTION,
	// non-standard but still sent by libcurl and rejected by e.g. google
	HeaderName::from_static("proxy-connection"),
	HeaderName::from_static("keep-alive"),
	header::PROXY_AUTHENTICATE,
	header::PROXY_AUTHORIZATION,
	header::TE,
	header::TRAILER,
	header::TRANSFER_ENCODING,
	header::UPGRADE,
];

fn connection_header_tokens(headers: &HeaderMap) -> Vec<HeaderName> {
	headers
		.get_all(header::CONNECTION)
		.into_iter()
		.filter_map(|value| value.to_str().ok())
		.flat_map(|value| value.split(','))
		.map(str::trim)
		.filter(|token| !token.is_empty())
		.filter_map(|token| HeaderName::from_bytes(token.as_bytes()).ok())
		.collect()
}

#[derive(Clone)]
struct RequestUpgrade {
	upgrade_type: HeaderValue,
	upgrade: OnUpgrade,
}

fn hop_by_hop_headers(req: &mut Request) -> Option<RequestUpgrade> {
	let trailers = req
		.headers()
		.get(header::TE)
		.and_then(|h| h.to_str().ok())
		.map(|s| s.contains("trailers"))
		.unwrap_or(false);
	let connection_headers = connection_header_tokens(req.headers());
	let upgrade_type = get_upgrade_type(req.headers());
	for h in connection_headers {
		req.headers_mut().remove(h);
	}
	for h in HOP_HEADERS.iter() {
		req.headers_mut().remove(h);
	}
	// If the incoming request supports trailers, the downstream one will as well
	if trailers {
		req.headers_mut().typed_insert(headers::Te::trailers());
	}
	// After stripping all the hop-by-hop connection headers above, add back any
	// necessary for protocol upgrades, such as for websockets.
	if let Some(upgrade_type) = upgrade_type.clone() {
		req
			.headers_mut()
			.typed_insert(headers::Connection::upgrade());
		req.headers_mut().insert(header::UPGRADE, upgrade_type);
	}
	let on_upgrade = req.extensions_mut().remove::<OnUpgrade>();
	if let Some(t) = upgrade_type
		&& let Some(u) = on_upgrade
	{
		Some(RequestUpgrade {
			upgrade_type: t,
			upgrade: u,
		})
	} else {
		None
	}
}

fn get_upgrade_type(headers: &HeaderMap) -> Option<HeaderValue> {
	if let Some(con) = headers.typed_get::<headers::Connection>() {
		if con.contains(http::header::UPGRADE) {
			headers.get(http::header::UPGRADE).cloned()
		} else {
			None
		}
	} else {
		None
	}
}

fn sensitive_headers(req: &mut Request) {
	for (name, value) in req.headers_mut() {
		if name == http::header::AUTHORIZATION {
			value.set_sensitive(true)
		}
	}
}

// The http library will not put the authority into req.uri().authority for HTTP/1. Normalize so
// the rest of the code doesn't need to worry about it
fn normalize_uri(tls: Option<&TLSConnectionInfo>, req: &mut Request) -> anyhow::Result<()> {
	debug!("request before normalization: {req:?}");
	if let ::http::Version::HTTP_10 | ::http::Version::HTTP_11 = req.version()
		&& req.uri().authority().is_none()
	{
		let mut parts = std::mem::take(req.uri_mut()).into_parts();
		// TODO: handle absolute HTTP/1.1 form
		let host = req
			.headers_mut()
			.remove(http::header::HOST)
			// TODO(https://github.com/hyperium/http/pull/811) actually make this shared
			.and_then(|h| Authority::try_from(h.as_bytes()).ok())
			.ok_or_else(|| anyhow::anyhow!("no authority or host"))?;

		parts.authority = Some(host);
		if parts.path_and_query.is_some() {
			// TODO: or always do this?
			if tls.is_some() {
				parts.scheme = Some(Scheme::HTTPS);
			} else {
				parts.scheme = Some(Scheme::HTTP);
			}
		}
		*req.uri_mut() = Uri::from_parts(parts)?
	}
	debug!("request after normalization: {req:?}");
	Ok(())
}

pub struct BackendCall {
	pub target: Target,
	pub http_version_override: Option<::http::Version>,
	pub transport_override: Option<(InboundProtocol, Vec<Identity>)>,
	pub network_gateway: Option<(GatewayAddress, Identity)>, /* For double hbone: (gateway_address, gateway_identity) */
	pub backend_policies: BackendPolicies,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ServiceCallOverride {
	pub destination: Option<SocketAddr>,
	pub destination_passthrough: bool,
	pub inference_failed_open: bool,
}

#[derive(Debug, Default)]
struct ResponsePolicies {
	timeout: Option<http::timeout::Policy>,
	route_response_header: ResponsePolicy<filters::HeaderModifier>,
	backend_response_header: ResponsePolicy<filters::HeaderModifier>,
	transformation: ResponsePolicy<Transformation>,
	backend_transformation: ResponsePolicy<Transformation>,
	gateway_transformation: ResponsePolicy<Transformation>,
	response_headers: HeaderMap,
	ext_proc: Option<ExtProcRequest>,
	gateway_ext_proc: Option<ExtProcRequest>,
	// Populated by the standard request-policy flow after conditional rate-limit policies are
	// evaluated. The later LLM path uses these selected policies and does not re-evaluate conditions.
	llm_request_policies: LLMRequestPolicies,
	a2a_type: a2a::RequestType,
}

impl ResponsePolicies {
	pub fn headers(&mut self) -> &mut HeaderMap {
		&mut self.response_headers
	}

	pub async fn apply(
		&mut self,
		resp: &mut Response,
		l: &mut RequestLog,
		is_upstream_response: bool,
	) -> Result<(), ProxyResponse> {
		let rh = &mut self.response_headers;

		self
			.route_response_header
			.apply("response header modifier", l, resp, rh)
			.await?;
		self
			.backend_response_header
			.apply("backend response header modifier", l, resp, rh)
			.await?;
		self
			.transformation
			.apply("transformation", l, resp, rh)
			.await?;
		self
			.backend_transformation
			.apply("backend transformation", l, resp, rh)
			.await?;
		self
			.gateway_transformation
			.apply("gateway transformation", l, resp, rh)
			.await?;

		// ext_proc is only intended to run on responses from upstream
		if is_upstream_response {
			if let Some(x) = self.ext_proc.as_mut() {
				x.mutate_response(resp, l.request_snapshot.as_ref())
					.await?
					.apply(&mut self.response_headers)?;
				dtrace::snapshot!(Response, "ext proc", l, &resp);
			};
			if let Some(x) = self.gateway_ext_proc.as_mut() {
				x.mutate_response(resp, l.request_snapshot.as_ref())
					.await?
					.apply(&mut self.response_headers)?;
				dtrace::snapshot!(Response, "gateway ext proc", l, &resp);
			}
		}

		if !self.response_headers.is_empty() {
			merge_in_headers(Some(self.response_headers.clone()), resp.headers_mut());
			dtrace::snapshot!(Response, "response headers", l, &resp);
		}

		Ok(())
	}
}

#[derive(Debug, Clone)]
pub struct TunnelClient {
	pub inputs: Arc<ProxyInputs>,
}
#[derive(Debug, Clone)]
pub struct PolicyClient {
	pub inputs: Arc<ProxyInputs>,
}

impl PolicyClient {
	pub async fn call_reference(
		&self,
		req: Request,
		backend_ref: &SimpleBackendReference,
	) -> Result<Response, ProxyError> {
		self
			.call_reference_with_policies(req, backend_ref, &[])
			.await
	}

	pub async fn call_reference_with_policies(
		&self,
		mut req: Request,
		backend_ref: &SimpleBackendReference,
		policies: &[BackendTrafficPolicy],
	) -> Result<Response, ProxyError> {
		let backend = resolve_simple_backend(backend_ref, self.inputs.as_ref())?;
		trace!("resolved {:?} to {:?}", backend_ref, &backend);

		http::modify_req_uri(&mut req, |uri| {
			if uri.authority.is_none() {
				// If host is not set, set it to the backend
				uri.authority = Some(Authority::try_from(backend.backend.hostport())?);
			}
			if uri.scheme.is_none() {
				// Default to HTTP, if the policy is TLS it will get set correctly later
				uri.scheme = Some(Scheme::HTTP);
			}
			Ok(())
		})
		.map_err(ProxyError::Processing)?;

		let backend = BackendWithPolicies::from(backend);
		let pols = get_backend_policies(&self.inputs, &backend, policies, None);
		self
			.internal_call_with_policies(req, backend.backend, pols)
			.await
	}

	pub async fn call(
		&self,
		req: Request,
		backend: SimpleBackendWithPolicies,
	) -> Result<Response, ProxyError> {
		let backend = BackendWithPolicies::from(backend);
		let pols = get_backend_policies(&self.inputs, &backend, &[], None);
		self
			.internal_call_with_policies(req, backend.backend, pols)
			.await
	}

	pub async fn call_with_explicit_policies(
		&self,
		req: Request,
		backend: &SimpleBackend,
		policies: BackendPolicies,
	) -> Result<Response, ProxyError> {
		let backend = Backend::from(backend.clone());
		self
			.internal_call_with_policies(req, backend, policies)
			.await
	}

	pub async fn call_with_explicit_policies_list(
		&self,
		req: Request,
		backend: SimpleBackend,
		policies: Vec<BackendTrafficPolicy>,
	) -> Result<Response, ProxyError> {
		let backend = Backend::from(backend);
		let pols = self
			.inputs
			.stores
			.read_binds()
			.inline_backend_policies(&policies);
		self.internal_call_with_policies(req, backend, pols).await
	}

	fn internal_call_with_policies<'a>(
		&'a self,
		req: Request,
		backend: Backend,
		pols: BackendPolicies,
	) -> Pin<Box<dyn Future<Output = Result<Response, ProxyError>> + Send + '_>> {
		let mut req = Some(req);
		Box::pin(async move {
			make_backend_call(
				self.inputs.clone(),
				Arc::new(LLMRequestPolicies::default()),
				&backend,
				pols,
				MustSnapshot::new(&mut req),
				// Here we don't have a log to pass. MCP and LLM flows expect there to always be a log.
				// As such, we ensure we ONLY call this with Simple backend type which cannot be MCP/LLM
				None,
				&mut Default::default(),
			)
			.await
			.map_err(ProxyResponse::downcast)
		})
	}

	pub async fn simple_call(&self, req: Request) -> Result<Response, ProxyError> {
		Box::pin(self.inputs.upstream.simple_call(req)).await
	}
}
trait OptLogger {
	fn add<F>(&mut self, f: F)
	where
		F: FnOnce(&mut RequestLog);
}

impl OptLogger for Option<&mut RequestLog> {
	fn add<F>(&mut self, f: F)
	where
		F: FnOnce(&mut RequestLog),
	{
		if let Some(log) = self.as_mut() {
			f(log)
		}
	}
}

#[cfg(test)]
mod route_chain_tests {
	use agent_core::strng;

	use super::*;
	use crate::test_helpers::proxymock;

	fn route(name: &str, path: &str, target: RouteBackendTarget) -> Route {
		Route {
			key: strng::new(name),
			service_key: None,
			name: RouteName {
				name: strng::new(name),
				namespace: strng::EMPTY,
				rule_name: None,
				kind: Some(strng::literal!("HTTPRoute")),
			},
			hostnames: Vec::new(),
			matches: vec![RouteMatch {
				headers: Vec::new(),
				path: PathMatch::PathPrefix(strng::new(path)),
				method: None,
				query: Vec::new(),
			}],
			backends: vec![RouteBackendReference {
				weight: 1,
				target,
				inline_policies: Vec::new(),
			}],
			inline_policies: Vec::new(),
		}
	}

	fn route_without_backends(name: &str, path: &str) -> Route {
		Route {
			key: strng::new(name),
			service_key: None,
			name: RouteName {
				name: strng::new(name),
				namespace: strng::EMPTY,
				rule_name: None,
				kind: Some(strng::literal!("HTTPRoute")),
			},
			hostnames: Vec::new(),
			matches: vec![RouteMatch {
				headers: Vec::new(),
				path: PathMatch::PathPrefix(strng::new(path)),
				method: None,
				query: Vec::new(),
			}],
			backends: Vec::new(),
			inline_policies: Vec::new(),
		}
	}

	fn request(path: &str) -> Request {
		::http::Request::builder()
			.uri(format!("http://example.com{path}"))
			.header(header::HOST, "example.com")
			.body(http::Body::empty())
			.unwrap()
	}

	fn bind() -> Bind {
		Bind {
			key: proxymock::BIND_KEY,
			address: "127.0.0.1:0".parse().unwrap(),
			listeners: ListenerSet::from_list([Listener {
				key: proxymock::LISTENER_KEY,
				name: Default::default(),
				hostname: Default::default(),
				protocol: ListenerProtocol::HTTP,
			}]),
			protocol: BindProtocol::http,
			tunnel_protocol: Default::default(),
		}
	}

	#[test]
	fn select_route_chain_follows_delegated_routes() {
		let backend: SocketAddr = "127.0.0.1:8080".parse().unwrap();
		let child = route(
			"child",
			"/",
			BackendReference::Backend(strng::format!("/{}", backend)).into(),
		);
		let parent = route(
			"parent",
			"/foo",
			RouteBackendTarget::RouteGroup(child.key.clone()),
		);
		let bind = bind();
		let listener = bind.listeners.get_exactly_one().unwrap();
		let proxy = proxymock::setup_proxy_test("{}")
			.unwrap()
			.with_backend(backend)
			.with_bind(bind)
			.with_route(parent)
			.with_route_group(child.key.clone(), vec![child.clone()]);

		let selected = select_route_chain(
			proxy.inputs().as_ref(),
			listener_address(),
			&listener,
			&request("/foo"),
		)
		.expect("delegated route should resolve");

		assert_eq!(selected.routes.len(), 2);
		assert_eq!(selected.routes[0].name.name.as_str(), "parent");
		assert_eq!(selected.routes[1].name.name.as_str(), "child");
		match &selected.path_match {
			PathMatch::PathPrefix(prefix) => assert_eq!(prefix.as_str(), "/"),
			other => panic!("expected delegated path prefix match, got {other:?}"),
		}
		match selected.backend.unwrap().target {
			RouteBackendTarget::Backend(name) => assert_eq!(name.as_str(), format!("/{}", backend)),
			other => panic!("expected backend target, got {other:?}"),
		}
	}

	#[test]
	fn select_route_chain_rejects_cycles() {
		let parent = route(
			"parent",
			"/",
			RouteBackendTarget::RouteGroup(strng::literal!("child")),
		);
		let child = route(
			"child",
			"/",
			RouteBackendTarget::RouteGroup(strng::literal!("parent")),
		);
		let bind = bind();
		let listener = bind.listeners.get_exactly_one().unwrap();
		let proxy = proxymock::setup_proxy_test("{}")
			.unwrap()
			.with_bind(bind)
			.with_route(parent.clone())
			.with_route(child.clone())
			.with_route_group(strng::literal!("child"), vec![child])
			.with_route_group(strng::literal!("parent"), vec![parent]);

		let err = select_route_chain(
			proxy.inputs().as_ref(),
			listener_address(),
			&listener,
			&request("/"),
		)
		.expect_err("cycle should fail");
		assert!(matches!(err, ProxyError::RouteCycleDetected));
	}

	#[test]
	fn select_route_chain_allows_backendless_terminal_route() {
		let bind = bind();
		let listener = bind.listeners.get_exactly_one().unwrap();
		let proxy = proxymock::setup_proxy_test("{}")
			.unwrap()
			.with_bind(bind)
			.with_route(route_without_backends("direct", "/"));

		let selected = select_route_chain(
			proxy.inputs().as_ref(),
			listener_address(),
			&listener,
			&request("/"),
		)
		.expect("backendless route should still resolve");

		assert_eq!(selected.routes.len(), 1);
		assert!(selected.backend.is_none());
		match &selected.path_match {
			PathMatch::PathPrefix(prefix) => assert_eq!(prefix.as_str(), "/"),
			other => panic!("expected path prefix match, got {other:?}"),
		}
	}

	fn listener_address() -> SocketAddr {
		"127.0.0.1:80".parse().unwrap()
	}
}
