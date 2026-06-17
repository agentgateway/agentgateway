use std::collections::HashSet;
use std::net::SocketAddr;
use std::ops::{Deref, DerefMut};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use ::http::uri::PathAndQuery;
use ::http::{HeaderMap, header};
use anyhow::anyhow;
use frozen_collections::Len;
use headers::HeaderMapExt;
use hyper::upgrade::OnUpgrade;
use hyper_util::rt::TokioIo;
use rand::RngExt;
use rand::seq::{IndexedRandom, IteratorRandom};
use tracing::{debug, trace};
use types::agent::*;
use types::discovery::*;

use crate::cel::{BackendContext, RequestTime};
use crate::client::{ApplicationTransport, HboneHeaders, HboneSourceRole, Transport};
use crate::http::backendtls::BackendTLS;
use crate::http::buffer::Buffer;
use crate::http::ext_proc::{ExtProcRequest, InferenceRoutingDestinationMode};
use crate::http::filters::{AutoHostname, BackendRequestTimeout};
use crate::http::transformation_cel::Transformation;
use crate::http::x_headers::TRACEPARENT;
use crate::http::{
	Authority, HeaderName, HeaderValue, Request, Response, Scheme, StatusCode, Uri, auth, filters,
	merge_in_headers, retry,
};
use crate::llm::{
	InputFormat, LLMInfo, LLMRequest, LLMResponse, RequestResult, RouteType, model_router,
};
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
use crate::telemetry::metrics::{OutboundCallKind, OutboundCallLabels, OutboundCallSubtype};
use crate::telemetry::trc::TraceParent;
use crate::transport::stream::{Extension, Socket, TCPConnectionInfo, TLSConnectionInfo};
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
	if !lp.add.is_empty() {
		log.cel.fields.add = lp.add.clone();
	}
	if !lp.remove.is_empty() {
		log.cel.fields.remove = lp.remove.clone();
	}
	if let Some(database) = &lp.database
		&& !database.add.is_empty()
	{
		log.cel.database_fields.add = database.add.clone();
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

	rp.buffer = pol.buffer.apply("buffer", c, l, req, rp.headers()).await?;

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

	// Select route response header filter now so request conditions are evaluated once, but run it
	// on the response path.
	rp.route_response_header = pol
		.response_header_modifier
		.select_response("response header modifier", req);
	pol
		.request_header_modifier
		.apply_without_response("request header modifier", c, l, req, rp.headers())
		.await?;

	// Enable Auto Hostname rewrite by default. This may be disabled by a URL Rewrite, or explicitly
	// setting hostname_rewrite = None
	let hostname_rewrite = pol.hostname_rewrite.select("hostname rewrite", req);
	if hostname_rewrite
		.as_deref()
		.copied()
		.unwrap_or(HostRedirectOverride::Auto)
		== HostRedirectOverride::Auto
	{
		req.extensions_mut().insert(AutoHostname {
			explicit: hostname_rewrite.is_some(),
			target: None,
		});
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
	client: PolicyClient,
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
		// Applied elsewhere (in mcp/handler.rs + mcp/session.rs)
		mcp_guardrails: _,
		// Applied elsewhere
		inference_routing: _,
		ext_authz,
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
	} = &*backend_call.backend_policies;
	rp.backend_response_header = response_header_modifier.as_response_policy();

	let dh = backend::HTTP::default();
	http
		.as_ref()
		.unwrap_or(&dh)
		.apply(req, backend_call.http_version_override);

	// Ext auth has no response-side
	let _ = ext_authz
		.apply("backend ext authz", &client, log, req, rp.headers())
		.await?;

	if let Some(auth) = backend_auth {
		auth::apply_backend_auth(&backend_info, auth, req).await?;
		dtrace::snapshot!(Request, "backend auth", &req);
	}
	rp.backend_transformation = transformation
		.apply("backend transformation", &client, log, req, rp.headers())
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

	// CORS must run before authentication so preflight requests can short-circuit
	// and rejected browser requests still receive the configured response headers.
	policies
		.cors
		.apply_without_response("gateway cors", c, l, req, response_policies.headers())
		.await?;

	policies
		.oidc
		.apply_without_response("gateway oidc", c, l, req, response_policies.headers())
		.await?;

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

	policies
		.authorization
		.apply_without_response(
			"gateway authorization",
			c,
			l,
			req,
			response_policies.headers(),
		)
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
	let prompt_guard = policies
		.llm
		.as_deref()
		.and_then(|llm| llm.prompt_guard.as_ref());
	Ok(store::LLMResponsePolicies {
		local_rate_limit,
		remote_rate_limit: response,
		request_traceparent: req.headers().get(TRACEPARENT).cloned(),
		prompt_guard: prompt_guard.map(|g| g.response.clone()).unwrap_or_default(),
		streaming_prompt_guard_enabled: prompt_guard.is_some_and(|g| g.streaming.is_enabled()),
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
			log.request_snapshot = log
				.cel
				.cel_context
				.maybe_snapshot_request(req, true)
				.map(Arc::new);
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
				log.request_snapshot = log
					.cel
					.cel_context
					.maybe_snapshot_request(req, true)
					.map(Arc::new);
			}
			SnapshottedProxyResponse(e.into())
		})
	}
	fn explicitly_skip_snapshot(self) -> Result<T, SnapshottedProxyResponse> {
		self.map_err(|e| SnapshottedProxyResponse(e.into()))
	}
}

impl HTTPProxy {
	pub async fn proxy(&self, connection: Arc<Extension>, mut req: Request) -> Response {
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
			self.inputs.model_catalog.clone(),
			start,
			tcp.clone(),
		);
		let mut log: DropOnLog = log.into();

		// Setup ResponsePolicies outside of proxy_internal, so we have can unconditionally run them even on errors
		// or direct responses
		let mut response_policies = ResponsePolicies::default();
		let is_grpc_request = http::is_grpc_request(&req);
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
			ProxyResponse::Error(e) => e.into_response_with_grpc(is_grpc_request),
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
				ProxyResponse::Error(e) => e.into_response_with_grpc(is_grpc_request),
				ProxyResponse::DirectResponse(dr) => *dr,
			},
		};
		if let Some(log) = log.as_mut() {
			dtrace::snapshot!(Response, "final response", log, &resp);
		}

		log.with(|l| {
			if let Some(start) = l.response_processing_start {
				l.response_processing_duration = Some(start.elapsed());
			}
		});

		// Pass the log into the body so it finishes once the stream is entirely complete.
		// We will also record trailer info there.
		log.with(|l| set_final_response_fields(l, &reason, &mut resp));

		if let Some(connect) = resp.extensions_mut().remove::<ConnectTunnel>() {
			handle_connect_tunnel(connect, resp, log)
		} else if resp.status() == StatusCode::SWITCHING_PROTOCOLS {
			let Some(req_upgrade) = resp.extensions_mut().remove::<RequestUpgrade>() else {
				return ProxyError::UpgradeFailed(None, None).into_response_with_grpc(is_grpc_request);
			};
			let realtime_guard_context = resp.extensions_mut().remove::<RealtimeGuardContext>();
			handle_upgrade(req_upgrade, resp, log, realtime_guard_context)
				.await
				.unwrap_or_else(|e| e.into_response_with_grpc(is_grpc_request))
		} else {
			resp.map(move |b| http::Body::new(LogBody::new(b, log)))
		}
	}

	async fn proxy_internal(
		&self,
		mut req: Request,
		log: &mut RequestLog,
		response_policies: &mut ResponsePolicies,
	) -> Result<Response, SnapshottedProxyResponse> {
		log.tls_info = req.extensions().get::<TLSConnectionInfo>().cloned();
		log.backend_protocol = Some(cel::BackendProtocol::http);

		let selected_listener = self.selected_listener.clone();
		let inputs = self.inputs.clone();
		let bind_name = self.bind_name.clone();
		debug!(bind=%bind_name, "route for bind");

		let Some(bind) = inputs.stores.read_binds().bind(&bind_name) else {
			return Err(ProxyResponse::Error(ProxyError::BindNotFound)).snapshot_on_err(log, &mut req);
		};
		log.bind_name = Some(bind_name.clone());
		cel::ProxyContext::mutate(&mut req, |ctx| {
			ctx.bind = Some(bind_name.clone());
		});

		sensitive_headers(&mut req);
		normalize_uri(log.tls_info.as_ref(), &mut req)
			.map_err(ProxyError::Processing)
			.snapshot_on_err(log, &mut req)?;
		let connect_upgrade = if req.method() == ::http::Method::CONNECT {
			req.extensions_mut().remove::<OnUpgrade>()
		} else {
			None
		};
		let mut req_upgrade = hop_by_hop_headers(&mut req);

		let host = http::get_host(&req)
			.map(|s| s.to_string())
			.snapshot_on_err(log, &mut req)?;
		log.host = Some(host.clone());
		log.method = Some(req.method().clone());
		log.path = Some(
			if req.method() == ::http::Method::CONNECT && req.uri().path().is_empty() {
				"/".to_string()
			} else {
				req
					.uri()
					.path_and_query()
					.map(|pq| pq.to_string())
					.unwrap_or_else(|| req.uri().path().to_string())
			},
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
				log.listener_name = Some(l.name.clone());
				cel::ProxyContext::mutate(&mut req, |ctx| {
					ctx.gateway = Some(cel::ProxyGatewayContext {
						namespace: l.name.gateway_namespace.clone(),
						name: l.name.gateway_name.clone(),
					});
					ctx.listener = Some(cel::ProxyListenerContext {
						name: l.name.listener_name.clone(),
					});
				});
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
				if req.method() == ::http::Method::CONNECT {
					let mode = frontend_policies
						.connect
						.as_ref()
						.map(|p| p.mode)
						.unwrap_or(frontend::ConnectMode::Deny);
					match mode {
						frontend::ConnectMode::Deny | frontend::ConnectMode::Tunnel => {
							return Err(ProxyResponse::Error(ProxyError::MethodNotAllowed))
								.snapshot_on_err(log, &mut req);
						},
						frontend::ConnectMode::Route => {},
					}
				}
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
		cel::ProxyContext::mutate(&mut req, |ctx| {
			ctx.route = Some(cel::ProxyRouteContext {
				namespace: selected_route.name.namespace.clone(),
				name: selected_route.name.name.clone(),
				kind: selected_route.name.kind.clone(),
				rule: selected_route.name.rule_name.clone(),
			});
		});
		req.extensions_mut().insert(path_match);

		debug!(bind=%bind_name, listener=%selected_listener.key, route=%selected_route.key, "selected route");

		let selected_llm_backend = if let Some(router) = &selected_route.llm_router {
			match router.resolve(&mut req).await {
				model_router::ResolveResult::DirectResponse(resp) => {
					return Err(ProxyResponse::DirectResponse(Box::new(resp))).snapshot_on_err(log, &mut req);
				},
				model_router::ResolveResult::Backend(backend) => Some(backend),
			}
		} else {
			None
		};

		let mut route_inline_policy_storage;
		let route_inlines = if let Some(selected_llm_backend) = &selected_llm_backend {
			// LLM routing may add route policies to the final selected route. Clone the
			// inline policy lists so we can append those policies without mutating config.
			route_inline_policy_storage = selected_route_chain
				.routes
				.iter()
				.map(|route| route.inline_policies.clone())
				.collect::<Vec<_>>();
			if let Some(inline_policies) = route_inline_policy_storage.last_mut() {
				inline_policies.extend(selected_llm_backend.route_policies.clone());
			}
			route_inline_policy_storage
				.iter()
				.map(Vec::as_slice)
				.collect::<Vec<_>>()
		} else {
			// Most requests do not use LLM routing, so borrow the existing inline policy
			// lists directly and avoid cloning policy config.
			selected_route_chain
				.routes
				.iter()
				.map(|route| route.inline_policies.as_slice())
				.collect()
		};
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
			route_inlines,
		};
		let route_policies = inputs.stores.read_binds().route_policies(&route_path);
		// Register all expressions
		route_policies.register_cel_expressions(log.cel.ctx());
		let mut route_retry = route_policies.retry.select("retry", &req);
		log.retry_backoff = route_retry.as_ref().and_then(|r| r.backoff);
		// Evaluate the retry precondition (if any) against the request before it is consumed.
		if let Some(retry) = route_retry.as_ref()
			&& let Some(pre) = retry.precondition.as_ref()
		{
			let exec = cel::Executor::new_request(&req);
			if !exec.eval_bool(pre.as_ref()) {
				debug!("retry precondition not met, disabling retries");
				route_retry = None;
			}
		}
		log.cel.ctx().maybe_buffer_request_body(&mut req).await;

		// Others are set only when they have gotten to the appropriate phase of the request, so we simulate
		// a middleware-style approach where if the request side never runs, neither does the response side.
		response_policies.timeout = route_policies
			.timeout
			.select("timeout", &req)
			.as_deref()
			.cloned();

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

		let selected_backend_ref = selected_llm_backend
			.map(|selected| selected.backend)
			.or(selected_route_chain.backend)
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
		log.backend_info = Some(selected_backend.backend.backend.backend_info());
		if let Some(bp) = selected_backend.backend.backend.backend_protocol() {
			log.backend_protocol = Some(bp)
		}

		if req.method() == ::http::Method::CONNECT {
			let connect_upgrade = connect_upgrade
				.ok_or_else(|| ProxyError::ProcessingString("CONNECT missing upgrade".to_string()))
				.snapshot_on_err(log, &mut req)?;
			return self
				.connect_tunnel(
					log,
					connect_upgrade,
					&selected_backend,
					backend_policies,
					response_policies,
					&mut req,
				)
				.await
				.snapshot_on_err(log, &mut req);
		}

		let route_request_mirrors = route_policies.request_mirror.select("request mirror", &req);
		let route_llm = route_policies.llm.select("llm", &req);
		let (head, body) = req.into_parts();
		for mirror in route_request_mirrors
			.iter()
			.flat_map(|mirrors| mirrors.iter())
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
		let retries = route_retry;

		// LLM token rate limiting reuses the rate-limit policy selected above in the normal
		// request-policy flow. Conditional rate-limit expressions are evaluated only once there;
		// the LLM path must not re-run conditions against the provider-specific rewritten request.
		let mut llm_request_policies = std::mem::take(&mut response_policies.llm_request_policies);
		llm_request_policies.llm = route_llm;
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
			let mut res = self
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
			if last
				|| !should_retry(
					&res,
					retries.as_ref().unwrap(),
					log.request_snapshot.as_deref(),
				) {
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
			finalize_attempt_for_retry(log, &mut res);
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

	async fn connect_tunnel(
		&self,
		log: &mut RequestLog,
		upgrade: OnUpgrade,
		selected_backend: &RouteBackend,
		backend_policies: BackendPolicies,
		response_policies: &mut ResponsePolicies,
		req: &mut Request,
	) -> Result<Response, ProxyResponse> {
		let backend_call = build_connect_backend_call(
			self.inputs.as_ref(),
			&selected_backend.backend.backend,
			backend_policies.into(),
			log,
			req,
		)?;
		let backend_info = auth::BackendInfo {
			target: selected_backend.backend.backend.target(),
			call_target: backend_call.target.clone(),
			inputs: self.inputs.clone(),
		};
		{
			let mut maybe_log = Some(&mut *log);
			apply_backend_policies(
				backend_info,
				self.policy_client(),
				&backend_call,
				req,
				&mut maybe_log,
				response_policies,
			)
			.await?;
		}
		log.endpoint = Some(backend_call.target.clone());
		set_backend_cel_context(req, Some(&log));
		log.request_snapshot = snapshot_connect_request(log, req).map(Arc::new);

		// CONNECT establishes a raw byte tunnel after any configured backend transport
		// has been established. Backend TLS applies to the gateway-to-backend leg.
		let transport = build_backend_transport(
			&self.inputs,
			&backend_call,
			req
				.extensions()
				.get::<WaypointService>()
				.is_some()
				.then_some(HboneSourceRole::Waypoint),
		)
		.await?;
		let upstream = self
			.inputs
			.upstream
			.connect_raw(backend_call.target, transport)
			.await?;
		let mut resp = ::http::Response::builder()
			.status(StatusCode::OK)
			.body(http::Body::empty())
			.map_err(ProxyError::Http)?;
		resp.extensions_mut().insert(ConnectTunnel {
			upgrade,
			upstream: Arc::new(Mutex::new(Some(upstream))),
		});
		Ok(resp)
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
		let upgrade_req_headers = req.headers().clone();
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
			backend_policies.into(),
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

		// If ext_proc returned an ImmediateResponse during the request body phase, it stored
		// the response and dropped the body channel.  Return it regardless of whether the
		// upstream call succeeded (with empty body) or failed (body parse error).
		if let Some(resp) = response_policies.take_ext_proc_body_immediate_response() {
			return Err(ProxyResponse::DirectResponse(Box::new(resp)))
				.maybe_snapshot_on_err(log, &mut req_opt);
		}

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
			// Store prompt guard + policy client for the WebSocket upgrade path.
			if let Some(prompt_guard) = route_policies
				.llm
				.as_deref()
				.and_then(|p| p.prompt_guard.clone())
				.filter(|g| g.streaming.is_enabled())
			{
				resp.extensions_mut().insert(RealtimeGuardContext {
					prompt_guard,
					policy_client: self.policy_client(),
					req_headers: upgrade_req_headers,
				});
			}
		}

		// gRPC status can be in the initial headers or a trailer, add if they are here
		maybe_set_grpc_status(&log.grpc_status, resp.headers());

		Ok(resp)
	}

	fn policy_client(&self) -> PolicyClient {
		PolicyClient::new(self.inputs.clone())
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
	realtime_guard_context: Option<RealtimeGuardContext>,
) -> Result<Response, ProxyError> {
	let RequestUpgrade {
		upgrade_type,
		upgrade,
	} = req_upgrade_type;
	let resp_upgrade_type = get_upgrade_type(resp.headers());
	// Case insensitive: https://www.rfc-editor.org/rfc/rfc6455.html#section-4.2.1
	if !resp_upgrade_type
		.as_ref()
		.is_some_and(|rt| upgrade_type.as_bytes().eq_ignore_ascii_case(rt.as_bytes()))
	{
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
			if let Some(guard_context) = realtime_guard_context {
				parse::websocket::guarded_realtime_proxy(
					TokioIo::new(req),
					server,
					guard_context.prompt_guard,
					guard_context.policy_client,
					llm,
					guard_context.req_headers,
				)
				.await;
				return;
			}
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

fn handle_connect_tunnel(connect: ConnectTunnel, resp: Response, log: DropOnLog) -> Response {
	tokio::task::spawn(async move {
		let Some(mut upstream) = connect
			.upstream
			.lock()
			.expect("CONNECT upstream lock")
			.take()
		else {
			return;
		};
		let downstream = match connect.upgrade.await {
			Ok(u) => u,
			Err(e) => {
				error!("CONNECT upgrade error: {e}");
				return;
			},
		};
		let mut downstream = TokioIo::new(downstream);
		let _ = agent_core::copy::copy_bidirectional(
			&mut downstream,
			&mut upstream,
			&agent_core::copy::ConnectionResult {},
		)
		.await;
		drop(log);
	});
	resp
}

fn snapshot_connect_request(
	log: &mut RequestLog,
	req: &mut Request,
) -> Option<cel::RequestSnapshot> {
	let mut snapshot = log.cel.cel_context.maybe_snapshot_request(req, false)?;
	if snapshot.path.path().is_empty()
		&& let Some(authority) = snapshot.host.clone()
	{
		let scheme = if log.tls_info.is_some() {
			Scheme::HTTPS
		} else {
			Scheme::HTTP
		};
		let mut parts = ::http::uri::Parts::default();
		parts.scheme = Some(scheme.clone());
		parts.authority = Some(authority);
		parts.path_and_query = Some(PathAndQuery::from_static("/"));
		if let Ok(uri) = Uri::from_parts(parts) {
			snapshot.path = uri;
			snapshot.scheme = Some(scheme);
		}
	}
	Some(snapshot)
}

/// Build the `x-istio-source` / `x-forwarded-network` / `baggage` headers added
/// to outbound HBONE CONNECTs.
pub(crate) fn build_hbone_headers(
	inputs: &ProxyInputs,
	source: Option<HboneSourceRole>,
) -> HboneHeaders {
	let baggage = inputs
		.stores
		.read_discovery()
		.self_workload
		.get()
		.map(format_baggage)
		.map(strng::new);
	HboneHeaders {
		source,
		forwarded_network: inputs.cfg.network.clone(),
		baggage,
	}
}

/// Format matches Istio's `baggageFormat`.
fn format_baggage(w: &Workload) -> String {
	format!(
		"k8s.cluster.name={},k8s.namespace.name={},k8s.deployment.name={},service.name={},service.version={}",
		w.cluster_id, w.namespace, w.workload_name, w.canonical_name, w.canonical_revision,
	)
}

pub async fn build_transport(
	inputs: &ProxyInputs,
	backend_call: &BackendCall,
	hbone_source: Option<HboneSourceRole>,
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
		let call = TCPProxy::build_backend_call(&mut None, None, inputs, &backend.backend, pols, None)?;
		let tunnel_backend_tls = call.backend_policies.backend_tls.clone();
		let tunnel_auth = call.backend_policies.backend_auth.clone();
		// This is a bounded recursion; this code is only called when backend_tunnel is set, and in this call
		// we never set it.
		let transport = Box::pin(build_transport(
			inputs,
			&call,
			hbone_source,
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

	// Check if we should route through a waypoint proxy (ingress_use_waypoint)
	if let (Some(wp), Some(ca)) = (&backend_call.waypoint, &inputs.ca) {
		if ca.get_identity().await.is_ok() {
			tracing::debug!("using HBONE waypoint at {} for service", wp.address);
			return Ok(Transport::HboneWaypoint {
				waypoint_address: wp.address,
				identities: wp.identities.clone(),
				inner: app_transport,
			});
		} else {
			return Err(ProxyError::Processing(anyhow::anyhow!(
				"ingress_use_waypoint: wanted HBONE to waypoint but CA is not available"
			)));
		}
	}

	// Check if we need double hbone
	if let (
		Some((gw_addr, gw_identities)),
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
				gateway_identities: gw_identities.clone(),
				waypoint_identities: waypoint_identities.clone(),
				inner: app_transport,
				headers: build_hbone_headers(inputs, hbone_source),
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
				Transport::Hbone(
					app_transport,
					backend_call.hbone_port,
					idents.clone(),
					build_hbone_headers(inputs, hbone_source),
				)
			} else {
				warn!("wanted TLS but CA is not available");
				app_transport.into()
			}
		},
		(_, _) => app_transport.into(),
	})
}

async fn build_backend_transport(
	inputs: &ProxyInputs,
	backend_call: &BackendCall,
	hbone_source: Option<HboneSourceRole>,
) -> Result<Transport, ProxyError> {
	build_transport(
		inputs,
		backend_call,
		hbone_source,
		backend_call.backend_policies.backend_tls.clone(),
		backend_call.backend_policies.tunnel.as_ref(),
		backend_call
			.backend_policies
			.http
			.as_ref()
			.and_then(|h| h.version)
			.or(backend_call.http_version_override),
	)
	.await
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
				l.request_snapshot = l
					.cel
					.cel_context
					.maybe_snapshot_request(&mut req, clear)
					.map(Arc::new);
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

fn target_from_request(req: &Request) -> Result<Target, ProxyError> {
	let host = http::get_host(req)?;
	let port = req
		.uri()
		.port_u16()
		.unwrap_or_else(|| match req.uri().scheme() {
			Some(s) if *s == Scheme::HTTPS => 443,
			_ => 80,
		});
	Ok(Target::from((host, port)))
}

#[allow(clippy::too_many_arguments)]
async fn apply_inference_routing(
	policies: &BackendPolicies,
	policy_client: PolicyClient,
	req: &mut Request,
	log: &mut Option<&mut RequestLog>,
	response_policies: &mut ResponsePolicies,
) -> Result<(http::ext_proc::InferencePoolRouter, ServiceCallOverride), ProxyResponse> {
	let mut maybe_inference = policies.build_inference(policy_client);
	let inference_result = Box::pin(maybe_inference.mutate_request(req)).await?;
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

	Ok((maybe_inference, service_override))
}

/// Builds a call for an already-resolved simple backend without re-entering
/// full AI or MCP backend dispatch.
#[allow(clippy::too_many_arguments)]
async fn build_simple_backend_call(
	inputs: &ProxyInputs,
	policy_client: PolicyClient,
	backend: &SimpleBackend,
	policies: Arc<BackendPolicies>,
	req: &mut Request,
	log: &mut Option<&mut RequestLog>,
	response_policies: &mut ResponsePolicies,
	hbone_source: Option<HboneSourceRole>,
) -> Result<(BackendCall, http::ext_proc::InferencePoolRouter), ProxyResponse> {
	let (maybe_inference, service_override) =
		apply_inference_routing(&policies, policy_client, req, log, response_policies).await?;
	let backend_call = match backend {
		SimpleBackend::Service(svc, port) => {
			// If user explicitly set auto hostname, we support it for Service.
			// Otherwise, the implicit default only applies to AgentgatewayBackend.
			if let Some(auto) = req.extensions_mut().get_mut::<AutoHostname>()
				&& auto.explicit
			{
				auto.target = Some(svc.hostname.clone());
			}
			build_service_call(
				inputs,
				policies,
				log,
				service_override,
				svc,
				port,
				req.uri().host(),
				hbone_source,
			)?
		},
		SimpleBackend::Opaque(_, target) => BackendCall::from_shared(target.clone(), policies),
		SimpleBackend::Aws(_, config) => {
			http::modify_req_uri(req, |uri| {
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

			req
				.extensions_mut()
				.insert(auth::aws::DefaultAwsServiceName(
					config.service_name().to_string(),
				));

			let default_policies = BackendPolicies {
				backend_tls: Some(http::backendtls::SYSTEM_TRUST.clone()),
				backend_auth: Some(auth::BackendAuth::Aws(auth::AwsAuth::Implicit {
					service_name: Some(config.service_name().to_string()),
					assume_role: None,
					source_credentials_cache: Default::default(),
					assume_role_cache: Default::default(),
				})),
				..Default::default()
			};
			BackendCall::new(
				Target::Hostname(config.get_host().into(), 443),
				default_policies.merge(policies.as_ref().clone()),
			)
		},
		SimpleBackend::Invalid => {
			return Err(ProxyError::BackendDoesNotExist.into());
		},
	};
	Ok((backend_call, maybe_inference))
}

#[allow(clippy::too_many_arguments)]
async fn make_backend_call(
	inputs: Arc<ProxyInputs>,
	route_policies: Arc<store::LLMRequestPolicies>,
	backend: &Backend,
	base_policies: Arc<BackendPolicies>,
	mut req: MustSnapshot<'_>,
	mut log: Option<&mut RequestLog>,
	response_policies: &mut ResponsePolicies,
) -> Result<Response, ProxyResponse> {
	let policy_client = PolicyClient::new(inputs.clone());
	let hbone_source = req
		.extensions()
		.get::<WaypointService>()
		.map(|_| HboneSourceRole::Waypoint)
		.or(Some(HboneSourceRole::Gateway));

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
					Arc::new(base_policies.as_ref().clone().merge(policies)),
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

	let (mut backend_call, mut maybe_inference) = match backend {
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

			let provider_defaults = BackendPolicies {
				llm_provider: Some(provider.clone()),
				..Default::default()
			};
			if let Some(provider_backend) = &provider.provider_backend {
				let provider_backend =
					super::resolve_simple_backend_with_policies(provider_backend, inputs.as_ref())?;
				let provider_backend_policies = inputs.stores.read_binds().sub_backend_policies(
					provider_backend.backend.target(),
					Some(&provider_backend.inline_policies),
				);
				let effective_policies = provider_defaults
					.merge(policies.as_ref().clone())
					.merge(sub_backend_policies)
					.merge(provider_backend_policies);

				build_simple_backend_call(
					&inputs,
					policy_client.clone(),
					&provider_backend.backend,
					effective_policies.into(),
					&mut req,
					&mut log,
					response_policies,
					hbone_source,
				)
				.await?
			} else {
				let provider_defaults = match &provider.host_override {
					Some(_) => provider_defaults,
					None => {
						let mut pol = provider
							.provider
							.default_connector_policies()
							.ok_or_else(|| {
								ProxyError::ProcessingString(
									"custom providers require an explicit host override or provider backend"
										.to_string(),
								)
							})?;
						pol.llm_provider = Some(provider.clone());
						pol
					},
				};
				// Defaults for the provider < Backend level policies < Sub Backend
				let effective_policies = Arc::new(
					provider_defaults
						.merge(policies.as_ref().clone())
						.merge(sub_backend_policies),
				);
				// Resolve the LLM route before picking the connection target: some providers serve
				// routes from different hosts (e.g. Bedrock rerank uses bedrock-agent-runtime).
				let route_type = route_policies
					.clone()
					.merge_backend_policies(effective_policies.llm.clone())
					.llm
					.as_ref()
					.map(|policy| policy.resolve_route(req.uri().path()))
					.unwrap_or(llm::RouteType::Completions);
				let target = match &provider.host_override {
					Some(target) => target.clone(),
					None => provider
						.provider
						.default_connector_target(route_type)
						.ok_or_else(|| {
							ProxyError::ProcessingString(
								"custom providers require an explicit host override or provider backend"
									.to_string(),
							)
						})?,
				};
				let (maybe_inference, _) = apply_inference_routing(
					&effective_policies,
					policy_client.clone(),
					&mut req,
					&mut log,
					response_policies,
				)
				.await?;
				(
					BackendCall::from_shared(target, effective_policies),
					maybe_inference,
				)
			}
		},
		Backend::Service(svc, port) => {
			let simple = SimpleBackend::Service(svc.clone(), *port);
			build_simple_backend_call(
				&inputs,
				policy_client.clone(),
				&simple,
				policies,
				&mut req,
				&mut log,
				response_policies,
				hbone_source,
			)
			.await?
		},
		Backend::Opaque(name, target) => {
			let simple = SimpleBackend::Opaque(name.clone(), target.clone());
			build_simple_backend_call(
				&inputs,
				policy_client.clone(),
				&simple,
				policies,
				&mut req,
				&mut log,
				response_policies,
				hbone_source,
			)
			.await?
		},
		Backend::Aws(name, config) => {
			let simple = SimpleBackend::Aws(name.clone(), config.clone());
			build_simple_backend_call(
				&inputs,
				policy_client.clone(),
				&simple,
				policies,
				&mut req,
				&mut log,
				response_policies,
				hbone_source,
			)
			.await?
		},
		Backend::Dynamic(_, _) if policies.inference_routing.is_some() => {
			return Err(
				ProxyError::ProcessingString(
					"inferenceRouting is not supported with dynamic backends".to_string(),
				)
				.into(),
			);
		},
		Backend::Dynamic(_, _) => {
			let backend_call = BackendCall::from_shared(target_from_request(&req)?, policies);
			(backend_call, http::ext_proc::InferencePoolRouter::default())
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
				.serve(inputs, name, backend, policies.as_ref().clone(), req, log)
				.await;
			return res.map_err(ProxyResponse::from);
		},
		Backend::Invalid => return Err(ProxyResponse::from(ProxyError::BackendDoesNotExist)),
	};
	log.add(|l| l.health_policy = backend_call.backend_policies.health.clone());
	if let Some(log) = log.as_mut() {
		backend_call
			.backend_policies
			.register_cel_expressions(log.cel.ctx());
	}
	// Apply auth before LLM request setup, so the providers can assume auth is in standardized header
	// Apply auth as early as possible so any ext_proc or transformations won't be repeated on retries in case it fails.
	let backend_info = auth::BackendInfo {
		target: backend.target(),
		call_target: backend_call.target.clone(),
		inputs: inputs.clone(),
	};
	apply_backend_policies(
		backend_info.clone(),
		PolicyClient::new(inputs.clone()),
		&backend_call,
		&mut req,
		&mut log,
		response_policies,
	)
	.await?;

	// For Dynamic backends, re-resolve the target from the (now potentially transformed)
	// request URI. This allows policies like `:authority` overrides (e.g., VPC endpoint
	// routing) to take effect on the actual upstream connection target.
	if matches!(backend, Backend::Dynamic(_, _)) {
		backend_call.target = target_from_request(&req)?;
	}

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
			let llm_provider = llm.provider.provider().to_string();
			dtrace::trace(|trace| {
				trace.llm_route_resolved(llm_provider.clone(), format!("{route_type:?}"))
			});
			// First, we process the incoming request. This entails translating to the relevant provider,
			// and parsing the request to build the LLMRequest for logging/etc, and applying LLM policies like
			// prompt enrichment, prompt guard, etc.
			match route_type {
				RouteType::Completions
				| RouteType::Messages
				| RouteType::Responses
				| RouteType::AnthropicTokenCount
				| RouteType::Embeddings
				| RouteType::Rerank
				| RouteType::Detect => {
					let request_body_limit = crate::http::buffer_limit(&req);
					let req = req.map(|b| {
						dtrace::TracingBody::maybe_wrap("llm request before translation", b, request_body_limit)
					});
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
						RouteType::Rerank => Box::pin(llm.provider.process_rerank_request(
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
					dtrace::trace(|trace| {
						trace.llm_request_detected(
							llm_provider.clone(),
							format!("{:?}", llm_request.input_format),
							llm_request.native_format.map(|f| format!("{f:?}")),
							llm_request.request_model.to_string(),
							llm_request.streaming,
						);
					});
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
								native_format: Some(llm::custom::ProviderFormat::Realtime),
								cache_convention: llm::CacheTokenConvention::pending(),
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
	if let Some(llm) = &backend_call.backend_policies.llm_provider {
		llm.provider.strip_browser_cors_headers(&mut req);
	}
	apply_auto_hostname(&mut req, &backend_call.target)?;
	// Some auth types (AWS) need to be applied after all request processing
	auth::apply_late_backend_auth(
		backend_call.backend_policies.backend_auth.as_ref(),
		&mut req,
	)
	.await?;
	let transport = build_backend_transport(&inputs, &backend_call, hbone_source).await?;
	dtrace::snapshot!(Request, "final request", &req);
	let request_body_limit = crate::http::buffer_limit(&req);
	let req = req.map(|b| dtrace::TracingBody::maybe_wrap("final request", b, request_body_limit));
	let call = client::Call {
		req,
		target: backend_call.target,
		transport,
	};
	dtrace::trace(|trace| trace.backend_call_started(&call.target));
	let upstream = inputs.upstream.clone();
	let llm_response_log = log.as_ref().map(|l| l.llm_response.clone());
	let include_completion_in_log = log
		.as_ref()
		.map(|l| l.cel.cel_context.needs_llm_completion())
		.unwrap_or_default();
	let a2a_type = response_policies.a2a_type.clone();

	let outbound_subtype = if backend_call.backend_policies.llm_provider.is_some() {
		OutboundCallSubtype::Llm
	} else {
		OutboundCallSubtype::Http
	};
	let outbound_start = std::time::Instant::now();
	log.add(|l| {
		if l.request_processing_duration.is_none() {
			l.request_processing_duration = Some(l.request_processing_start.elapsed());
		}
	});
	let resp = upstream.call(call).await;
	let outbound_end = Instant::now();
	log.add(|l| {
		l.metrics
			.upstream_call_duration
			.get_or_create(&OutboundCallLabels {
				kind: OutboundCallKind::Primary,
				subtype: outbound_subtype,
			})
			.observe((outbound_end - outbound_start).as_secs_f64());
		l.upstream_duration = Some(outbound_end - outbound_start);
		if resp.is_ok() {
			l.response_processing_start = Some(outbound_end);
		}
	});
	dtrace::trace(|trace| match &resp {
		Ok(resp) => trace.backend_call_completed(
			Some(outbound_start),
			Instant::now(),
			Some(resp.status().as_u16()),
			None,
		),
		Err(err) => trace.backend_call_completed(
			Some(outbound_start),
			Instant::now(),
			None,
			Some(err.to_string()),
		),
	});
	let mut resp = resp?;
	if let Some(log) = log.as_ref() {
		resp
			.extensions_mut()
			.insert(cel::ProxyContext::from_std_durations(
				log.request_processing_duration,
				log.upstream_duration,
				log.response_processing_duration,
			));
		dtrace::snapshot!(Response, "raw response", log, &resp);
	}
	a2a::apply_to_response(
		backend_call.backend_policies.a2a.as_ref(),
		a2a_type,
		&mut resp,
	)
	.await
	.map_err(ProxyError::Processing)?;
	let mut resp = if let (Some(llm), Some(llm_request)) = (
		backend_call.backend_policies.llm_provider.clone(),
		llm_request,
	) {
		llm
			.provider
			.process_response(
				policy_client.clone(),
				llm_request,
				llm_response_policies,
				log.as_ref().expect("must be set").request_snapshot.clone(),
				llm_response_log.expect("must be set"),
				include_completion_in_log,
				Some(&inputs.model_catalog),
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
	let response_body_limit = crate::http::response_buffer_limit(&resp);
	let resp = resp.map(|b| dtrace::TracingBody::maybe_wrap("response", b, response_body_limit));
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

fn connect_authority_target(req: &Request) -> Result<Target, ProxyError> {
	let authority = req.uri().authority().ok_or(ProxyError::InvalidRequest)?;
	let port = authority.port_u16().ok_or(ProxyError::InvalidRequest)?;
	Ok(Target::from((authority.host(), port)))
}

fn build_connect_backend_call(
	inputs: &ProxyInputs,
	backend: &Backend,
	policies: Arc<BackendPolicies>,
	log: &mut RequestLog,
	req: &Request,
) -> Result<BackendCall, ProxyError> {
	match backend {
		Backend::Service(svc, port) => {
			let mut maybe_log = Some(log);
			build_service_call(
				inputs,
				policies,
				&mut maybe_log,
				ServiceCallOverride::default(),
				svc,
				port,
				req.uri().host(),
				None,
			)
		},
		Backend::Opaque(_, target) => Ok(BackendCall::from_shared(target.clone(), policies)),
		Backend::Dynamic(_, _) => Ok(BackendCall::from_shared(
			connect_authority_target(req)?,
			policies,
		)),
		Backend::Invalid => Err(ProxyError::BackendDoesNotExist),
		Backend::AI(_, _) | Backend::MCP(_, _) | Backend::Aws(_, _) => {
			Err(ProxyError::InvalidBackendType)
		},
	}
}

#[allow(clippy::too_many_arguments)]
pub fn build_service_call(
	inputs: &ProxyInputs,
	backend_policies: Arc<BackendPolicies>,
	log: &mut Option<&mut RequestLog>,
	service_override: ServiceCallOverride,
	svc: &Arc<Service>,
	port: &u16,
	request_host: Option<&str>,
	hbone_source: Option<HboneSourceRole>,
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
			hbone_port: agent_hbone::DEFAULT_HBONE_PORT,
			network_gateway: None,
			waypoint: None,
			backend_policies,
		});
	}

	let discovery = inputs.stores.read_discovery();
	let workloads = &discovery.workloads;
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
	let mut network_gateway = if wl.network != inputs.cfg.network {
		resolve_network_gateway(
			inputs,
			&discovery,
			wl.as_ref(),
			"selected service workload is remote",
		)
	} else {
		None
	};
	let mut transport_override = Some((wl.protocol, workload_and_service_sans(&wl, svc)));

	// Check if the service has ingress_use_waypoint set and a waypoint configured.
	// When set, route traffic through the waypoint instead of directly to the workload.
	// Skip when we are acting as a waypoint: ingress_use_waypoint should only affect
	// ingress gateways, never waypoint-to-waypoint traffic.
	let waypoint = if svc.ingress_use_waypoint && hbone_source != Some(HboneSourceRole::Waypoint) {
		if let Some(wp) = &svc.waypoint {
			let default_wp_port = if wp.hbone_mtls_port > 0 {
				wp.hbone_mtls_port
			} else {
				agent_hbone::DEFAULT_HBONE_PORT
			};
			// Resolve the waypoint to a concrete backing workload so we know its IP (not the
			// service VIP) and its network — the latter lets us detect a remote waypoint and
			// route to it through the network gateway via double HBONE.
			let waypoint_info = match &wp.destination {
				types::discovery::gatewayaddress::Destination::Address(net_addr) => {
					if let Some(wp_wl) = discovery.workloads.find_address(net_addr) {
						// A pod IP: use the workload identity directly (no service SANs available).
						Some((
							SocketAddr::new(net_addr.address, default_wp_port),
							vec![wp_wl.identity()],
							wp_wl,
						))
					} else if let Some(wp_svc) = discovery.services.get_by_vip(net_addr) {
						// A service VIP: resolve a backing endpoint to get a real workload IP.
						resolve_service_endpoint(&discovery, &wp_svc, default_wp_port)
					} else {
						None
					}
				},
				types::discovery::gatewayaddress::Destination::Hostname(nh) => discovery
					.services
					.get_by_namespaced_host(&NamespacedHostname {
						namespace: nh.namespace.clone(),
						hostname: nh.hostname.clone(),
					})
					.and_then(|wp_svc| resolve_service_endpoint(&discovery, &wp_svc, default_wp_port)),
			};
			match waypoint_info {
				Some((address, identities, wp_wl)) => {
					if wp_wl.network != inputs.cfg.network {
						// Remote waypoint: reach it through the network gateway via double HBONE.
						// The inner HBONE identities are the waypoint's; the outer ones are the
						// gateway's, resolved below.
						if let Some(gw) = resolve_network_gateway(
							inputs,
							&discovery,
							wp_wl.as_ref(),
							"ingress_use_waypoint selected a remote waypoint workload",
						) {
							network_gateway = Some(gw);
							transport_override = Some((InboundProtocol::HBONE, identities));
							tracing::debug!(
								service = %svc.hostname,
								waypoint = %address.ip(),
								waypoint_port = %address.port(),
								"ingress_use_waypoint: routing to remote waypoint through network gateway"
							);
						}
						None
					} else {
						tracing::debug!(
							service = %svc.hostname,
							waypoint = %address.ip(),
							waypoint_port = %address.port(),
							"ingress_use_waypoint: routing through waypoint"
						);
						Some(WaypointTarget {
							address,
							service_hostname: svc.hostname.clone(),
							identities,
						})
					}
				},
				None => {
					tracing::warn!(
						service = %svc.hostname,
						"ingress_use_waypoint set but waypoint could not be resolved"
					);
					None
				},
			}
		} else {
			None
		}
	} else {
		None
	};

	// Waypoint: target = service hostname (becomes the inner CONNECT authority).
	// Double HBONE: target = service hostname (resolved by the gateway).
	let target = if let Some(wp) = &waypoint {
		Target::Hostname(wp.service_hostname.clone(), port)
	} else if network_gateway.is_some() {
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

	let hbone_port = if wl.hbone_mtls_port > 0 {
		wl.hbone_mtls_port
	} else {
		agent_hbone::DEFAULT_HBONE_PORT
	};

	Ok(BackendCall {
		target,
		http_version_override,
		transport_override,
		hbone_port,
		network_gateway,
		waypoint,
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

/// Selects a healthy endpoint for `svc` on `port` and returns its reachable socket address
/// (a real workload IP, not the service VIP), the accepted SPIFFE identities for mTLS
/// verification (workload identity + service SANs, matching ztunnel), and the backing workload.
fn resolve_service_endpoint(
	discovery: &store::DiscoveryStore,
	svc: &Service,
	port: u16,
) -> Option<(SocketAddr, Vec<Identity>, Arc<Workload>)> {
	let (ep, _handle, wl) = svc
		.endpoints
		.select_endpoint(&discovery.workloads, svc, port, None)?;
	// TODO: plumb `_handle` through the waypoint/gateway transports so endpoint selection
	// keeps EWMA, eviction, and latency feedback.
	let resolved_port = select_service_target_port(ep.as_ref(), svc, port, None, false)?;
	let ip = *wl.workload_ips.first()?;
	let identities = workload_and_service_sans(&wl, svc);
	Some((SocketAddr::new(ip, resolved_port), identities, wl))
}

/// Resolves the network gateway used to reach `remote_wl` from the local network via double
/// HBONE. Returns the gateway address and the accepted SPIFFE identities for the outer tunnel.
fn resolve_network_gateway(
	inputs: &ProxyInputs,
	discovery: &store::DiscoveryStore,
	remote_wl: &Workload,
	reason: &'static str,
) -> Option<(GatewayAddress, Vec<Identity>)> {
	let Some(gw_addr) = &remote_wl.network_gateway else {
		tracing::warn!(
			source_network = %inputs.cfg.network,
			dest_network = %remote_wl.network,
			"workload on remote network but no gateway configured"
		);
		return None;
	};
	let resolved = match &gw_addr.destination {
		types::discovery::gatewayaddress::Destination::Address(net_addr) => {
			let Some(gw_wl) = discovery.workloads.find_address(net_addr) else {
				tracing::warn!(
					gateway_address = ?net_addr,
					"network gateway address not found in workload store for remote workload"
				);
				return None;
			};
			// A direct gateway address gives us only the workload; no service SANs available.
			Some((gw_addr.clone(), vec![gw_wl.identity()]))
		},
		types::discovery::gatewayaddress::Destination::Hostname(hostname) => {
			let resolved = discovery
				.services
				.get_by_namespaced_host(&NamespacedHostname {
					namespace: hostname.namespace.clone(),
					hostname: hostname.hostname.clone(),
				})
				.and_then(|gw_svc| {
					let (addr, identities, gw_wl) =
						resolve_service_endpoint(discovery, &gw_svc, gw_addr.hbone_mtls_port)?;
					Some((
						GatewayAddress {
							destination: types::discovery::gatewayaddress::Destination::Address(NetworkAddress {
								network: gw_wl.network.clone(),
								address: addr.ip(),
							}),
							hbone_mtls_port: addr.port(),
						},
						identities,
					))
				});
			if resolved.is_none() {
				tracing::warn!(
					gateway_hostname = ?hostname,
					"no service / endpoint / IP for hostname-based network gateway"
				);
			}
			resolved
		},
	};
	if let Some((resolved_gw_addr, _)) = &resolved {
		tracing::debug!(
			source_network = %inputs.cfg.network,
			dest_network = %remote_wl.network,
			gateway = ?resolved_gw_addr,
			reason,
			"picked workload on remote network, using double hbone"
		);
	}
	resolved
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

fn set_final_response_fields(
	log: &mut RequestLog,
	reason: &ProxyResponseReason,
	resp: &mut Response,
) {
	log.status = Some(resp.status());
	log.reason = Some(*reason);
	log.retry_after = http::outlierdetection::retry_after(resp.status(), resp.headers());
	log.response_snapshot = log.cel.cel_context.maybe_snapshot_response(resp);
}

fn finalize_attempt_for_retry(
	log: &mut RequestLog,
	res: &mut Result<Response, SnapshottedProxyResponse>,
) {
	let (status, retry_after, response_snapshot) = match res {
		Ok(resp) => (
			Some(resp.status()),
			http::outlierdetection::retry_after(resp.status(), resp.headers()),
			log.cel.cel_context.maybe_snapshot_response(resp),
		),
		Err(SnapshottedProxyResponse(_)) => (None, None, None),
	};
	let end_time = agent_core::Timestamp::now();
	// This is an intermediate retry snapshot, so a best-effort clone is fine here.
	let mut llm_response: Option<crate::cel::LLMContext> =
		log.llm_response.load_clone().map(|llm_info| {
			crate::cel::LLMContext::from_llm_info(llm_info, Some(log.model_catalog.as_ref()))
		});
	if let Some(llm_response) = llm_response.as_mut() {
		llm_response.set_token_timing(log.start.as_instant(), end_time.as_instant());
	}
	let mcp = log.mcp_status.load_clone();
	log.finalize_request_handle_for_attempt(
		end_time,
		status,
		retry_after,
		response_snapshot.as_ref(),
		llm_response.as_ref(),
		mcp.as_ref(),
	);
}

fn should_retry(
	res: &Result<Response, SnapshottedProxyResponse>,
	pol: &retry::Policy,
	req_snapshot: Option<&cel::RequestSnapshot>,
) -> bool {
	match res {
		Ok(resp) => {
			if pol.codes.contains(&resp.status()) {
				return true;
			}
			// A condition can match responses that status codes alone cannot, e.g. APIs that
			// return a 200 but signal failure via a header.
			if let Some(cond) = pol.condition.as_ref() {
				let exec = cel::Executor::new_response(req_snapshot, resp);
				return exec.eval_bool(cond.as_ref());
			}
			false
		},
		Err(SnapshottedProxyResponse(ProxyResponse::Error(e))) => e.is_retryable(),
		Err(SnapshottedProxyResponse(ProxyResponse::DirectResponse(_))) => false,
	}
}

#[cfg(test)]
mod tests {
	use std::collections::{HashMap, HashSet};
	use std::net::SocketAddr;
	use std::sync::Arc;

	use ::http::Method;
	use serde_json::json;
	use wiremock::{Mock, ResponseTemplate};

	use super::{
		apply_auto_hostname, apply_llm_request_policies, hop_by_hop_headers,
		resolved_workload_target_hostname, select_service_target_port,
	};
	use crate::http::filters::AutoHostname;
	use crate::llm::policy::{
		PromptGuard, PromptGuardStreamingMode, RegexRule, RegexRules, RequestRejection, ResponseGuard,
		ResponseGuardKind,
	};
	use crate::store::LLMRequestPolicies;
	use crate::test_helpers::proxymock;
	use crate::types::agent::{Backend, ResourceName, Target};
	use crate::types::discovery::{AppProtocol, Endpoint, HealthStatus, Service};
	use crate::types::local::LocalAIBackend;
	use crate::{http, llm};

	fn retry_policy(codes: &[u16], condition: Option<&str>) -> crate::http::retry::Policy {
		crate::http::retry::Policy {
			attempts: std::num::NonZeroU8::new(2).unwrap(),
			backoff: None,
			codes: codes
				.iter()
				.map(|c| ::http::StatusCode::from_u16(*c).unwrap())
				.collect(),
			precondition: None,
			condition: condition
				.map(|e| std::sync::Arc::new(crate::cel::Expression::new_strict(e).unwrap())),
		}
	}

	fn response_regex_guard() -> ResponseGuard {
		ResponseGuard {
			rejection: RequestRejection::default(),
			kind: ResponseGuardKind::Regex(RegexRules {
				action: Default::default(),
				rules: vec![RegexRule::Regex {
					pattern: regex::Regex::new("secret").unwrap(),
				}],
			}),
		}
	}

	fn llm_request() -> llm::LLMRequest {
		llm::LLMRequest {
			input_tokens: None,
			input_format: llm::InputFormat::Completions,
			native_format: Some(llm::custom::ProviderFormat::Completions),
			cache_convention: llm::CacheTokenConvention::pending(),
			request_model: "test-model".into(),
			provider: "test-provider".into(),
			streaming: true,
			params: Default::default(),
			prompt: None,
		}
	}

	async fn response_prompt_guards_for_streaming_mode(
		streaming: PromptGuardStreamingMode,
	) -> crate::store::LLMResponsePolicies {
		let policy = llm::Policy {
			prompt_guard: Some(PromptGuard {
				streaming,
				request: vec![],
				response: vec![response_regex_guard()],
			}),
			..Default::default()
		};
		let policies = LLMRequestPolicies {
			llm: Some(Arc::new(policy)),
			..Default::default()
		};
		let mut req = ::http::Request::builder()
			.body(http::Body::empty())
			.unwrap();
		let mut response_headers = ::http::HeaderMap::new();

		apply_llm_request_policies(
			&policies,
			crate::test_helpers::policy_client(),
			&mut req,
			&llm_request(),
			&mut response_headers,
		)
		.await
		.expect("LLM request policies should apply")
	}

	#[tokio::test]
	async fn apply_llm_request_policies_skips_streaming_guardrails_when_disabled() {
		let policies =
			response_prompt_guards_for_streaming_mode(PromptGuardStreamingMode::Disabled).await;

		assert_eq!(policies.prompt_guard.len(), 1);
		assert!(!policies.streaming_prompt_guard_enabled);
	}

	#[tokio::test]
	async fn apply_llm_request_policies_includes_streaming_guardrails_when_enabled() {
		let policies =
			response_prompt_guards_for_streaming_mode(PromptGuardStreamingMode::Enabled).await;

		assert_eq!(policies.prompt_guard.len(), 1);
		assert!(policies.streaming_prompt_guard_enabled);
	}

	#[test]
	fn should_retry_matches_status_codes() {
		let pol = retry_policy(&[503], None);
		let resp = ::http::Response::builder()
			.status(503)
			.body(http::Body::empty())
			.unwrap();
		assert!(super::should_retry(&Ok(resp), &pol, None));

		let pol = retry_policy(&[503], None);
		let resp = ::http::Response::builder()
			.status(200)
			.body(http::Body::empty())
			.unwrap();
		assert!(!super::should_retry(&Ok(resp), &pol, None));
	}

	#[test]
	fn should_retry_condition_matches_on_200() {
		// Simulate an API that returns 200 but signals failure via a header.
		let pol = retry_policy(&[], Some(r#"response.headers["x-req-failed"] == "true""#));
		let failed = ::http::Response::builder()
			.status(200)
			.header("x-req-failed", "true")
			.body(http::Body::empty())
			.unwrap();
		assert!(super::should_retry(&Ok(failed), &pol, None));

		let pol = retry_policy(&[], Some(r#"response.headers["x-req-failed"] == "true""#));
		let ok = ::http::Response::builder()
			.status(200)
			.body(http::Body::empty())
			.unwrap();
		assert!(!super::should_retry(&Ok(ok), &pol, None));
	}

	#[test]
	fn apply_auto_hostname_rewrites_authority_when_enabled() {
		let mut req = ::http::Request::builder()
			.uri("http://original.example.com/")
			.body(http::Body::empty())
			.unwrap();
		req.extensions_mut().insert(AutoHostname {
			explicit: false,
			target: None,
		});

		apply_auto_hostname(
			&mut req,
			&Target::Hostname("backend.example.com".into(), 80),
		)
		.expect("auto hostname rewrite should succeed");

		assert_eq!(
			req.uri().authority().map(|a| a.as_str()),
			Some("backend.example.com")
		);
	}

	#[test]
	fn apply_auto_hostname_preserves_authority_when_disabled() {
		let mut req = ::http::Request::builder()
			.uri("http://original.example.com/")
			.body(http::Body::empty())
			.unwrap();

		apply_auto_hostname(
			&mut req,
			&Target::Hostname("backend.example.com".into(), 80),
		)
		.expect("disabled auto hostname rewrite should succeed");

		assert_eq!(
			req.uri().authority().map(|a| a.as_str()),
			Some("original.example.com")
		);
	}

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
			ingress_use_waypoint: false,
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

	#[tokio::test]
	async fn llm_retry_evicts_failed_priority_group_before_next_attempt() {
		let primary = wiremock::MockServer::start().await;
		Mock::given(wiremock::matchers::any())
			.respond_with(ResponseTemplate::new(429))
			.mount(&primary)
			.await;

		let fallback = wiremock::MockServer::start().await;
		Mock::given(wiremock::matchers::any())
			.respond_with(ResponseTemplate::new(200).set_body_raw(
				include_bytes!("../llm/tests/response/completions/basic.json").to_vec(),
				"application/json",
			))
			.mount(&fallback)
			.await;

		let mut bind = proxymock::setup_proxy_test("{}").expect("proxy test harness");
		let local_backend: LocalAIBackend = serde_json::from_value(json!({
			"groups": [
				{
					"providers": [{
						"name": "primary",
						"hostOverride": primary.address().to_string(),
						"provider": {
							"openAI": {
								"model": null
							}
						},
						"policies": {
							"health": {
								"unhealthyExpression": "response.code == 429",
								"eviction": {
									"duration": "1s"
								}
							}
						}
					}]
				},
				{
					"providers": [{
						"name": "fallback",
						"hostOverride": fallback.address().to_string(),
						"provider": {
							"openAI": {
								"model": null
							}
						}
					}]
				}
			]
		}))
		.expect("local AI backend");
		let backend = Backend::AI(
			ResourceName::new("llm".into(), "".into()),
			local_backend.translate().expect("translated backend"),
		);
		bind
			.pi
			.stores
			.binds
			.write()
			.insert_backend(backend.name(), backend.into());
		bind = bind
			.with_bind(proxymock::simple_bind())
			.with_route(proxymock::basic_named_route("/llm".into()));
		bind
			.attach_route_policy(json!({
				"retry": {
					"attempts": 1,
					"backoff": "10ms",
					"codes": [429]
				},
				"ai": {
					"routes": {
						"/v1/chat/completions": "completions"
					}
				}
			}))
			.await;
		let io = bind.serve_http(proxymock::BIND_KEY);

		let res = proxymock::send_request_body(
			io,
			Method::POST,
			"http://lo/v1/chat/completions",
			include_bytes!("../llm/tests/requests/completions/basic.json"),
		)
		.await;

		assert_eq!(res.status(), 200);

		let primary_requests = primary
			.received_requests()
			.await
			.expect("primary request recording");
		assert_eq!(primary_requests.len(), 1);

		let fallback_requests = fallback
			.received_requests()
			.await
			.expect("fallback request recording");
		assert_eq!(fallback_requests.len(), 1);
		assert_eq!(
			fallback_requests[0]
				.headers
				.get("x-retry-attempt")
				.and_then(|v| v.to_str().ok()),
			Some("1")
		);
	}
}

pub fn maybe_set_grpc_status(status: &AsyncLog<u8>, headers: &HeaderMap) {
	if let Some(parsed) = parse_grpc_status(headers) {
		status.store(Some(parsed));
	}
}

pub fn parse_grpc_status(headers: &HeaderMap) -> Option<u8> {
	headers
		.get("grpc-status")
		.and_then(|status| std::str::from_utf8(status.as_bytes()).ok())
		.and_then(|status| status.parse().ok())
}

async fn send_mirror(
	inputs: Arc<ProxyInputs>,
	upstream: PolicyClient,
	mirror: filters::RequestMirror,
	mut req: Request,
) -> Result<(), ProxyError> {
	req.headers_mut().remove(http::header::CONTENT_LENGTH);
	let backend = super::resolve_simple_backend(&mirror.backend, inputs.as_ref())?;
	let _ = upstream
		.with_outbound(OutboundCallKind::Mirror, OutboundCallSubtype::Http)
		.call(req, backend)
		.await?;
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

#[derive(Clone)]
struct ConnectTunnel {
	upgrade: OnUpgrade,
	upstream: Arc<Mutex<Option<Socket>>>,
}

#[derive(Clone)]
struct RealtimeGuardContext {
	prompt_guard: crate::llm::policy::PromptGuard,
	policy_client: PolicyClient,
	req_headers: ::http::HeaderMap,
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
	if let ::http::Version::HTTP_10 | ::http::Version::HTTP_11 = req.version() {
		let host = req.headers_mut().remove(http::header::HOST);
		if req.uri().authority().is_none() {
			let mut parts = std::mem::take(req.uri_mut()).into_parts();
			let host = host
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
	}
	debug!("request after normalization: {req:?}");
	Ok(())
}

fn apply_auto_hostname(req: &mut Request, target: &Target) -> Result<(), ProxyError> {
	let Some(auto) = req.extensions().get::<filters::AutoHostname>() else {
		return Ok(());
	};
	let ext_host = auto.target.as_ref().cloned();
	let backend_host = if let Target::Hostname(h, _) = target {
		Some(h)
	} else {
		None
	};
	let Some(host) = ext_host.as_ref().or(backend_host) else {
		return Ok(());
	};

	http::modify_req_uri(req, |uri| {
		uri.authority = Some(Authority::from_str(host)?);
		Ok(())
	})
	.map_err(ProxyError::Processing)
}

pub struct BackendCall {
	pub target: Target,
	pub http_version_override: Option<::http::Version>,
	pub transport_override: Option<(InboundProtocol, Vec<Identity>)>,
	pub hbone_port: u16,
	pub network_gateway: Option<(GatewayAddress, Vec<Identity>)>, /* For double hbone: (gateway_address, gateway_identities) */
	pub waypoint: Option<WaypointTarget>,                         // For ingress waypoint routing
	pub backend_policies: Arc<BackendPolicies>,
}

impl BackendCall {
	pub fn new(target: Target, backend_policies: BackendPolicies) -> Self {
		Self::from_shared(target, Arc::new(backend_policies))
	}

	pub fn from_shared(target: Target, backend_policies: Arc<BackendPolicies>) -> Self {
		Self {
			target,
			http_version_override: None,
			transport_override: None,
			hbone_port: agent_hbone::DEFAULT_HBONE_PORT,
			network_gateway: None,
			waypoint: None,
			backend_policies,
		}
	}
}

/// Information needed to route through a waypoint proxy.
pub struct WaypointTarget {
	/// The socket address of the waypoint (IP:hbone_port).
	pub address: SocketAddr,
	/// Destination service hostname used as the inner HBONE CONNECT authority.
	pub service_hostname: Strng,
	/// Identities for mTLS verification (service SANs).
	pub identities: Vec<Identity>,
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
	buffer: ResponsePolicy<Buffer>,
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

	fn take_ext_proc_body_immediate_response(&self) -> Option<http::Response> {
		for ep in [&self.ext_proc, &self.gateway_ext_proc]
			.into_iter()
			.flatten()
		{
			if let Some(resp) = ep.take_body_immediate_response() {
				return Some(resp);
			}
		}
		None
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
		self.buffer.apply("buffer", l, resp, rh).await?;
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
				x.mutate_response(resp, l.request_snapshot.as_deref())
					.await?
					.apply(&mut self.response_headers)?;
				dtrace::snapshot!(Response, "ext proc", l, &resp);
			};
			if let Some(x) = self.gateway_ext_proc.as_mut() {
				x.mutate_response(resp, l.request_snapshot.as_deref())
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
	pub outbound: Option<OutboundCallLabels>,
}

impl PolicyClient {
	pub fn new(inputs: Arc<ProxyInputs>) -> PolicyClient {
		PolicyClient {
			inputs,
			outbound: None,
		}
	}

	pub fn with_outbound(
		&self,
		kind: OutboundCallKind,
		subtype: OutboundCallSubtype,
	) -> PolicyClient {
		PolicyClient {
			inputs: self.inputs.clone(),
			outbound: Some(OutboundCallLabels { kind, subtype }),
		}
	}

	fn observe_outbound(&self, start: std::time::Instant) {
		if let Some(labels) = &self.outbound {
			self
				.inputs
				.metrics
				.upstream_call_duration
				.get_or_create(labels)
				.observe(start.elapsed().as_secs_f64());
		}
	}

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
		let start = std::time::Instant::now();
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
		let res = self
			.internal_call_with_policies(req, backend.backend, pols)
			.await;
		self.observe_outbound(start);
		res
	}

	pub async fn call(
		&self,
		req: Request,
		backend: SimpleBackendWithPolicies,
	) -> Result<Response, ProxyError> {
		let start = std::time::Instant::now();
		let backend = BackendWithPolicies::from(backend);
		let pols = get_backend_policies(&self.inputs, &backend, &[], None);
		let res = self
			.internal_call_with_policies(req, backend.backend, pols)
			.await;
		self.observe_outbound(start);
		res
	}

	pub async fn call_with_explicit_policies(
		&self,
		req: Request,
		backend: &SimpleBackend,
		policies: BackendPolicies,
	) -> Result<Response, ProxyError> {
		let start = std::time::Instant::now();
		let backend = Backend::from(backend.clone());
		let res = self
			.internal_call_with_policies(req, backend, policies)
			.await;
		self.observe_outbound(start);
		res
	}

	pub async fn call_with_explicit_policies_list(
		&self,
		req: Request,
		backend: Backend,
		policies: Vec<BackendTrafficPolicy>,
	) -> Result<Response, ProxyError> {
		let start = std::time::Instant::now();
		let pols = self
			.inputs
			.stores
			.read_binds()
			.inline_backend_policies(&policies);
		let res = self.internal_call_with_policies(req, backend, pols).await;
		self.observe_outbound(start);
		res
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
				pols.into(),
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
		let start = std::time::Instant::now();
		let res = Box::pin(self.inputs.upstream.simple_call(req)).await;
		self.observe_outbound(start);
		res
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
			service_port: 0,
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
			llm_router: None,
			inline_policies: Vec::new(),
		}
	}

	fn route_without_backends(name: &str, path: &str) -> Route {
		Route {
			key: strng::new(name),
			service_key: None,
			service_port: 0,
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
			llm_router: None,
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
