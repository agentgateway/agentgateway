use rand::RngExt;
use std::net::SocketAddr;
use std::ops::{Deref, DerefMut};
use std::str::FromStr;
use std::sync::Arc;
use std::time::SystemTime;

use ::http::uri::PathAndQuery;
use ::http::{HeaderMap, header};
use anyhow::anyhow;
use headers::HeaderMapExt;
use hyper::body::Incoming;
use hyper::upgrade::OnUpgrade;
use hyper_util::rt::TokioIo;
use opentelemetry::KeyValue;
use opentelemetry::trace::Status as SpanStatus;
use rand::seq::IndexedRandom;
use tracing::{debug, trace};
use types::agent::*;
use types::discovery::*;

use crate::cel::{BackendContext, RequestStartTime};
use crate::client::{ApplicationTransport, Transport};
use crate::http::backendtls::BackendTLS;
use crate::http::ext_proc::ExtProcRequest;
use crate::http::filters::{AutoHostname, BackendRequestTimeout};
use crate::http::transformation_cel::Transformation;
use crate::http::{
	Authority, HeaderName, HeaderValue, PolicyResponse, Request, Response, Scheme, StatusCode, Uri,
	auth, filters, merge_in_headers, retry,
};
use crate::llm::{InputFormat, LLMRequest, RequestResult, RouteType};
use crate::proxy::{ProxyError, ProxyResponse, ProxyResponseReason, resolve_simple_backend};
use crate::store::{
	BackendPolicies, FrontendPolices, GatewayPolicies, LLMRequestPolicies, LLMResponsePolicies,
	RoutePath,
};
use crate::telemetry::log;
use crate::telemetry::log::{AsyncLog, DropOnLog, LogBody, RequestLog, SpanWriter, StartedSpan};
use crate::telemetry::operation_spans::{
	OperationSpanGuard, start_a2a_operation_span, start_llm_operation_span,
};
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

fn start_policy_span(
	span_writer: Option<&SpanWriter>,
	policy_name: &'static str,
) -> Option<StartedSpan> {
	span_writer.map(|sw| {
		sw.start(policy_name, |sb| {
			sb.with_attributes(vec![KeyValue::new("gateway.policy.name", policy_name)])
		})
	})
}

fn end_policy_span<T, E>(
	policy_span: &mut Option<StartedSpan>,
	result: &Result<T, E>,
	ok_is_deny: impl Fn(&T) -> bool,
) {
	let Some(policy_span) = policy_span.as_mut() else {
		return;
	};
	let (decision, error_type) = match result {
		Ok(ok) => (if ok_is_deny(ok) { "deny" } else { "allow" }, None),
		Err(_) => ("error", Some("policy_error")),
	};
	policy_span.set_attributes([KeyValue::new("gateway.policy.decision", decision)]);
	if let Some(error_type) = error_type {
		policy_span.set_attributes([KeyValue::new("error.type", error_type)]);
		policy_span.set_status(SpanStatus::error(error_type));
	}
	policy_span.end();
}

async fn apply_request_policies(
	policies: &store::RoutePolicies,
	client: PolicyClient,
	log: &mut RequestLog,
	req: &mut Request,
	span_writer: Option<SpanWriter>,
	response_policies: &mut ResponsePolicies,
) -> Result<(), ProxyResponse> {
	// CORS must run before authentication, authorization and rate limiting so that:
	// 1. Preflight OPTIONS requests short-circuit without requiring credentials
	// 2. CORS response headers are queued even if the request is later rejected,
	//    allowing browsers to read error responses instead of seeing a CORS error
	if let Some(c) = &policies.cors {
		c.apply(req)
			.map_err(ProxyError::from)?
			.apply(response_policies.headers())?;
	}

	if let Some(j) = &policies.jwt {
		j.apply(Some(log), req)
			.await
			.map_err(|e| ProxyResponse::from(ProxyError::JwtAuthenticationFailure(e)))?;
	}
	if let Some(b) = &policies.basic_auth {
		b.apply(req).await?;
	}
	if let Some(b) = &policies.api_key {
		b.apply(req).await?;
	}

	let ext_authz_resp = if let Some(x) = &policies.ext_authz {
		let mut policy_span = start_policy_span(span_writer.as_ref(), "policy.ext_authz");
		let policy_child_writer = policy_span
			.as_ref()
			.map(StartedSpan::child_writer)
			.or_else(|| span_writer.clone());
		let result = x.check(client.clone(), req, policy_child_writer).await;
		end_policy_span(&mut policy_span, &result, |pr: &PolicyResponse| {
			pr.direct_response.is_some()
		});
		result?
	} else {
		http::PolicyResponse::default()
	};
	ext_authz_resp.apply(response_policies.headers())?;
	if let Some(j) = &policies.authorization {
		j.apply(req)
			.map_err(|_| ProxyResponse::from(ProxyError::AuthorizationFailed))?;
	}

	if !policies.local_rate_limit.is_empty() {
		let mut policy_span = start_policy_span(span_writer.as_ref(), "policy.rate_limit.local");
		let mut local_rate_limit_result = Ok(());
		for lrl in &policies.local_rate_limit {
			if let Err(e) = lrl.check_request() {
				local_rate_limit_result = Err(e);
				break;
			}
		}
		end_policy_span(&mut policy_span, &local_rate_limit_result, |_| false);
		local_rate_limit_result?;
	}

	let remote_rate_limit_resp = if let Some(rrl) = &policies.remote_rate_limit {
		let mut policy_span = start_policy_span(span_writer.as_ref(), "policy.rate_limit.remote");
		let policy_child_writer = policy_span
			.as_ref()
			.map(StartedSpan::child_writer)
			.or_else(|| span_writer.clone());
		let result = rrl.check(client, req, policy_child_writer).await;
		end_policy_span(&mut policy_span, &result, |pr: &PolicyResponse| {
			pr.direct_response.is_some()
		});
		result?
	} else {
		http::PolicyResponse::default()
	};
	remote_rate_limit_resp.apply(response_policies.headers())?;

	let ext_proc_resp = if let Some(x) = response_policies.ext_proc.as_mut() {
		let mut policy_span = start_policy_span(span_writer.as_ref(), "policy.ext_proc.request");
		let result = x.mutate_request(req).await;
		end_policy_span(&mut policy_span, &result, |pr: &PolicyResponse| {
			pr.direct_response.is_some()
		});
		result?
	} else {
		http::PolicyResponse::default()
	};
	ext_proc_resp.apply(response_policies.headers())?;

	if let Some(j) = &policies.transformation {
		j.apply_request(req);
	}

	if let Some(csrf) = &policies.csrf {
		csrf
			.apply(req)
			.map_err(|_| ProxyError::CsrfValidationFailed)?
			.apply(response_policies.headers())?;
	}
	if let Some(rhm) = &policies.request_header_modifier {
		rhm.apply(req.headers_mut()).map_err(ProxyError::from)?;
	}

	// Enable Auto Hostname rewrite by default. This may be disabled by a URL Rewrite, or explicitly
	// setting hostname_rewrite = None
	if policies
		.hostname_rewrite
		.unwrap_or(HostRedirectOverride::Auto)
		== HostRedirectOverride::Auto
	{
		req.extensions_mut().insert(AutoHostname());
	}
	if let Some(r) = &policies.url_rewrite {
		r.apply(req).map_err(ProxyError::from)?;
	}
	if let Some(rr) = &policies.request_redirect {
		rr.apply(req)
			.map_err(ProxyError::from)?
			.apply(response_policies.headers())?;
	}
	if let Some(dr) = &policies.direct_response {
		PolicyResponse::default()
			.with_response(dr.apply().map_err(ProxyError::from)?)
			.apply(response_policies.headers())?;
	}

	// Mirror, timeout, and retry are handled separately.

	Ok(())
}

async fn apply_backend_policies(
	backend_info: auth::BackendInfo,
	backend_call: &BackendCall,
	req: &mut Request,
	log: &mut Option<&mut RequestLog>,
	response_policies: &mut ResponsePolicies,
) -> Result<(), ProxyResponse> {
	let BackendPolicies {
		backend_tls: _,
		backend_auth,
		a2a,
		http,
		// Doesn't currently have any options to set, todo
		tcp: _,
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
		// TODO: implement session persistence
		session_persistence: _,
		// Applied elsewhere
		request_mirror: _,
		// Applied elsewhere
		override_dest: _,
	} = &backend_call.backend_policies;
	response_policies.backend_response_header = response_header_modifier.clone();

	let dh = backend::HTTP::default();
	http
		.as_ref()
		.unwrap_or(&dh)
		.apply(req, backend_call.http_version_override);

	if let Some(auth) = backend_auth {
		auth::apply_backend_auth(&backend_info, auth, req).await?;
	}
	if let Some(rhm) = request_header_modifier {
		rhm.apply(req.headers_mut()).map_err(ProxyError::from)?;
	}
	if let Some(rr) = request_redirect {
		rr.apply(req)
			.map_err(ProxyError::from)?
			.apply(response_policies.headers())?;
	}

	if let Some(a2a) = a2a {
		let a2a_type = a2a::apply_to_request(a2a, req).await;
		if let a2a::RequestType::Call(method) = a2a_type {
			log.add(|l| {
				l.a2a_method = Some(method);
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
		response_policies.a2a_type = a2a_type;
	}

	Ok(())
}

async fn apply_gateway_policies(
	policies: &GatewayPolicies,
	client: PolicyClient,
	log: &mut RequestLog,
	req: &mut Request,
	ext_proc: Option<&mut ExtProcRequest>,
	span_writer: Option<SpanWriter>,
	response_headers: &mut HeaderMap,
) -> Result<(), ProxyResponse> {
	if let Some(j) = &policies.jwt {
		j.apply(Some(log), req)
			.await
			.map_err(|e| ProxyResponse::from(ProxyError::JwtAuthenticationFailure(e)))?;
	}
	if let Some(b) = &policies.basic_auth {
		b.apply(req).await?;
	}
	if let Some(b) = &policies.api_key {
		b.apply(req).await?;
	}

	let ext_authz_resp = if let Some(x) = &policies.ext_authz {
		let mut policy_span = start_policy_span(span_writer.as_ref(), "policy.ext_authz");
		let policy_child_writer = policy_span
			.as_ref()
			.map(StartedSpan::child_writer)
			.or(span_writer.clone());
		let result = x.check(client.clone(), req, policy_child_writer).await;
		end_policy_span(&mut policy_span, &result, |pr: &PolicyResponse| {
			pr.direct_response.is_some()
		});
		result?
	} else {
		http::PolicyResponse::default()
	};
	ext_authz_resp.apply(response_headers)?;

	let ext_proc_resp = if let Some(x) = ext_proc {
		let mut policy_span = start_policy_span(span_writer.as_ref(), "policy.ext_proc.request");
		let result = x.mutate_request(req).await;
		end_policy_span(&mut policy_span, &result, |pr: &PolicyResponse| {
			pr.direct_response.is_some()
		});
		result?
	} else {
		http::PolicyResponse::default()
	};
	ext_proc_resp.apply(response_headers)?;

	if let Some(j) = &policies.transformation {
		j.apply_request(req);
	}

	Ok(())
}

async fn apply_llm_request_policies(
	policies: &store::LLMRequestPolicies,
	client: PolicyClient,
	req: &mut Request,
	llm_req: &LLMRequest,
	span_writer: Option<SpanWriter>,
	response_headers: &mut HeaderMap,
) -> Result<store::LLMResponsePolicies, ProxyResponse> {
	if !policies.local_rate_limit.is_empty() {
		let mut policy_span = start_policy_span(span_writer.as_ref(), "policy.rate_limit.local");
		let mut local_rate_limit_result = Ok(());
		for lrl in &policies.local_rate_limit {
			if let Err(e) = lrl.check_llm_request(llm_req) {
				local_rate_limit_result = Err(e);
				break;
			}
		}
		end_policy_span(&mut policy_span, &local_rate_limit_result, |_| false);
		local_rate_limit_result?;
	}
	let (rl_resp, response) = if let Some(rrl) = &policies.remote_rate_limit {
		// For the LLM request side, request either the count of the input tokens (if tokenization was done)
		// or 0.
		// Either way, we will 'true up' on the response side.
		let mut policy_span = start_policy_span(span_writer.as_ref(), "policy.rate_limit.remote");
		let policy_child_writer = policy_span
			.as_ref()
			.map(StartedSpan::child_writer)
			.or_else(|| span_writer.clone());
		let result = rrl
			.check_llm(
				client,
				req,
				llm_req.input_tokens.unwrap_or_default(),
				policy_child_writer,
			)
			.await;
		end_policy_span(
			&mut policy_span,
			&result,
			|(pr, _): &(PolicyResponse, _)| pr.direct_response.is_some(),
		);
		result?
	} else {
		(http::PolicyResponse::default(), None)
	};
	rl_resp.apply(response_headers)?;
	Ok(store::LLMResponsePolicies {
		local_rate_limit: policies.local_rate_limit.clone(),
		remote_rate_limit: response,
		prompt_guard: policies
			.llm
			.as_deref()
			.and_then(|llm| llm.prompt_guard.as_ref())
			.map(|g| g.response.clone())
			.unwrap_or_default(),
		span_writer,
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
			log.request_snapshot = log.cel.cel_context.maybe_snapshot_request(req);
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
				log.request_snapshot = log.cel.cel_context.maybe_snapshot_request(req);
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
		let start = Instant::now();
		let start_time = agent_core::telemetry::render_current_time();

		// Copy connection level attributes into request level attributes
		connection.copy::<TCPConnectionInfo>(req.extensions_mut());
		connection.copy::<TLSConnectionInfo>(req.extensions_mut());

		let tcp = connection
			.get::<TCPConnectionInfo>()
			.expect("tcp connection must be set");
		let tls = connection.get::<TLSConnectionInfo>();
		let src = cel::SourceContext {
			address: tcp.peer_addr.ip(),
			port: tcp.peer_addr.port(),
			tls: tls.and_then(|t| t.src_identity.clone()),
		};
		req.extensions_mut().insert(src);
		req
			.extensions_mut()
			.insert(RequestStartTime(start_time.clone()));
		let log = RequestLog::new(
			log::CelLogging::new(
				self.inputs.cfg.logging.clone(),
				self.inputs.cfg.tracing.clone(),
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
			.proxy_internal(
				connection,
				req,
				log.as_mut().unwrap(),
				&mut response_policies,
			)
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
		connection: Arc<Extension>,
		req: ::http::Request<Incoming>,
		log: &mut RequestLog,
		response_policies: &mut ResponsePolicies,
	) -> Result<Response, SnapshottedProxyResponse> {
		log.tls_info = connection.get::<TLSConnectionInfo>().cloned();
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
		normalize_uri(&connection, &mut req)
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

		// Now check if we actually have a listener - fail after tracing is set up
		let selected_listener = selected_listener
			.or_else(|| bind.listeners.best_match(&host))
			.ok_or(ProxyError::ListenerNotFound);
		let selected_listener = match selected_listener {
			Ok(l) => {
				debug!(bind=%bind_name, listener=%l.key, "selected listener");
				let frontend_policies = inputs
					.stores
					.read_binds()
					.listener_frontend_policies(&l.name);

				self
					.handle_frontend_policies(&frontend_policies, log, &mut req)
					.await;
				l
			},
			Err(e) => {
				let frontend_policies = inputs
					.stores
					.read_binds()
					.frontend_policies(self.inputs.cfg.gateway_ref());
				self
					.handle_frontend_policies(&frontend_policies, log, &mut req)
					.await;
				return Err(ProxyResponse::Error(e)).snapshot_on_err(log, &mut req);
			},
		};
		log.bind_name = Some(bind_name.clone());
		log.listener_name = Some(selected_listener.name.clone());

		let mut gateway_policies = inputs
			.stores
			.read_binds()
			.gateway_policies(&selected_listener.name);
		gateway_policies.register_cel_expressions(log.cel.ctx());
		// This is unfortunate but we record the request twice possibly; we want to record it as early as possible
		// (for logging, etc) and also after we register the expressions since new fields may be available.
		log.cel.ctx().maybe_buffer_request_body(&mut req).await;

		let mut response_headers = HeaderMap::new();
		let mut maybe_gateway_ext_proc = gateway_policies
			.ext_proc
			.take()
			.map(|c| c.build(self.policy_client(), log.span_writer()));
		apply_gateway_policies(
			&gateway_policies,
			self.policy_client(),
			log,
			&mut req,
			maybe_gateway_ext_proc.as_mut(),
			log.span_writer(),
			&mut response_headers,
		)
		.await
		.snapshot_on_err(log, &mut req)?;

		Self::detect_misdirected(log, bind, &req, &selected_listener).snapshot_on_err(log, &mut req)?;

		let (selected_route, path_match) = http::route::select_best_route(
			inputs.stores.clone(),
			inputs.cfg.network.clone(),
			inputs.cfg.self_addr.clone(),
			self.target_address,
			&selected_listener,
			&req,
		)
		.ok_or(ProxyError::RouteNotFound)
		.snapshot_on_err(log, &mut req)?;
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
			PathMatch::Regex(r, _) => r.as_str().into(),
		});
		req.extensions_mut().insert(path_match);

		debug!(bind=%bind_name, listener=%selected_listener.key, route=%selected_route.key, "selected route");

		let route_path = RoutePath {
			route: &selected_route.name,
			listener: &selected_listener.name,
		};

		let mut route_policies = inputs
			.stores
			.read_binds()
			.route_policies(&route_path, &selected_route.inline_policies);
		// Register all expressions
		route_policies.register_cel_expressions(log.cel.ctx());
		log.cel.ctx().maybe_buffer_request_body(&mut req).await;

		let maybe_ext_proc = route_policies
			.ext_proc
			.take()
			.map(|c| c.build(self.policy_client(), log.span_writer()));
		response_policies.route_response_header = route_policies.response_header_modifier.clone();
		// backend_response_header is set much later
		response_policies.timeout = route_policies.timeout.clone();
		response_policies.transformation = route_policies.transformation.clone();
		response_policies.gateway_transformation = gateway_policies.transformation.clone();
		response_policies.ext_proc = maybe_ext_proc;
		response_policies.gateway_ext_proc = maybe_gateway_ext_proc;

		apply_request_policies(
			&route_policies,
			self.policy_client(),
			log,
			&mut req,
			log.span_writer(),
			response_policies,
		)
		.await
		.snapshot_on_err(log, &mut req)?;

		let selected_backend = select_backend(selected_route.as_ref(), &req)
			.ok_or(ProxyError::NoValidBackends)
			.snapshot_on_err(log, &mut req)?;
		let selected_backend =
			resolve_backend(selected_backend, self.inputs.as_ref()).snapshot_on_err(log, &mut req)?;
		let backend_policies = get_backend_policies(
			self.inputs.as_ref(),
			&selected_backend.backend,
			&selected_backend.inline_policies,
			Some(route_path.clone()),
		);
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
		let late_route_policies: Arc<LLMRequestPolicies> = Arc::new(route_policies.into());
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
						late_route_policies,
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
					late_route_policies.clone(),
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
					let deadline = tokio::time::Instant::from_std(log.start + request_timeout);
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
		if let Some(tp) = frontend_policies.tracing.as_deref() {
			// Apply sampling overrides if present
			if let Some(rs) = &tp.config.random_sampling {
				log.cel.tracing_sampler.random_sampling = Some(rs.clone());
				log.cel.cel_context.register_expression(rs.as_ref());
			}
			if let Some(cs) = &tp.config.client_sampling {
				log.cel.tracing_sampler.client_sampling = Some(cs.clone());
				log.cel.cel_context.register_expression(cs.as_ref());
			}
			// Re-apply request so any newly required attributes are captured before sampling
		}
		log.cel.ctx().maybe_buffer_request_body(req).await;

		let trace_parent = trc::TraceParent::from_request(req);
		let trace_sampled = log.trace_sampled(req, trace_parent.as_ref());

		// Tracing is configured through frontend tracing policies only.
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
						debug!("failed to initialize tracing policy tracer: {e:?}");
						None
					})
			} else {
				debug!("No frontend tracing policy found; tracing exporter disabled");
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
		bind: Arc<Bind>,
		req: &Request,
		selected_listener: &Arc<Listener>,
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
		let new_best_listener = bind
			.listeners
			.best_match(host)
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
			let deadline = tokio::time::Instant::from_std(start + timeout);
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
	let backend = super::resolve_backend(&b.backend, pi)?;
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
	backend_http_version_override: Option<::http::Version>,
) -> Result<Transport, ProxyError> {
	let backend_tls = backend_tls.map(|btls| btls.config_for(backend_http_version_override));
	let app_transport = if let Some(tls) = backend_tls {
		ApplicationTransport::Tls(tls)
	} else {
		ApplicationTransport::Plaintext
	};

	// Check if we need double hbone
	if let (
		Some((gw_addr, gw_identity)),
		Some((InboundProtocol::HBONE, waypoint_identity)),
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
				waypoint_identity: waypoint_identity.clone(),
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
		(Some((InboundProtocol::LegacyIstioMtls, ident)), Some(ca))
			if matches!(app_transport, ApplicationTransport::Plaintext) =>
		{
			if let Ok(id) = ca.get_identity().await {
				Some(
					id.legacy_mtls(vec![ident.clone()])
						.map_err(|e| ProxyError::Processing(anyhow!("{e}")))?,
				)
				.into()
			} else {
				warn!("wanted TLS but CA is not available");
				app_transport.into()
			}
		},
		(Some((InboundProtocol::HBONE, ident)), Some(ca)) => {
			if ca.get_identity().await.is_ok() {
				Transport::Hbone(app_transport, ident.clone())
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
	inline_policies: &[BackendPolicy],
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
	pub fn take_and_snapshot(
		self,
		mut log: Option<&mut &mut RequestLog>,
	) -> Result<Request, ProxyError> {
		if let Some(mut req) = self.0.take() {
			if let Some(l) = log.take() {
				l.request_snapshot = l.cel.cel_context.maybe_snapshot_request(&mut req);
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
	let span_writer = log
		.as_ref()
		.and_then(|l| l.span_writer())
		.or_else(|| req.extensions().get::<SpanWriter>().cloned());
	let mut operation_span_guard: Option<OperationSpanGuard> = None;
	let mut operation_span_writer = span_writer.clone();

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

	let mut maybe_inference = policies.build_inference(policy_client.clone(), span_writer.clone());
	let (inference_override, ext_proc_resp) = maybe_inference.mutate_request(&mut req).await?;
	ext_proc_resp.apply(response_policies.headers())?;
	log.add(|l| l.inference_pool = inference_override);

	// Use inference override if present, otherwise check for stateful MCP pinning.
	// In practice, these don't conflict: inference is for AI backends, MCP pinning is for MCP backends.
	let override_dest = inference_override.or(policies.override_dest);

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
			if let Some(po) = &provider.path_override {
				http::modify_req_uri(&mut req, |p| {
					p.path_and_query = Some(PathAndQuery::from_str(po)?);
					Ok(())
				})
				.map_err(ProxyError::Processing)?;
			}
			BackendCall {
				target,
				backend_policies: effective_policies,
				http_version_override: None,
				transport_override: None,
				network_gateway: None,
			}
		},
		Backend::Service(svc, port) => {
			build_service_call(&inputs, policies, &mut log, override_dest, svc, port)?
		},
		Backend::Opaque(_, target) => BackendCall {
			target: target.clone(),
			http_version_override: None,
			transport_override: None,
			network_gateway: None,
			backend_policies: policies,
		},
		Backend::Dynamic(_, _) => {
			let port = req
				.extensions()
				.get::<TCPConnectionInfo>()
				.unwrap()
				.local_addr
				.port();
			let target =
				Target::try_from((http::get_host(&req)?, port)).map_err(ProxyError::Processing)?;
			BackendCall {
				target: target.clone(),
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
			if let Some(sw) = span_writer.clone() {
				req.extensions_mut().insert(sw);
			}
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

	if operation_span_guard.is_none()
		&& let Some(sw) = span_writer.as_ref()
		&& let a2a::RequestType::Call(method) = &response_policies.a2a_type
	{
		let agent_name = log
			.as_ref()
			.and_then(|l| l.backend_info.as_ref().map(|bi| bi.backend_name.as_ref()));
		let guard = start_a2a_operation_span(sw, method, agent_name);
		operation_span_writer = Some(guard.child_writer());
		operation_span_guard = Some(guard);
	}

	let (mut req, llm_response_policies, llm_request) =
		if let Some(llm) = &backend_call.backend_policies.llm_provider {
			let mut req = req.take_and_snapshot(log.as_mut())?;
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
				| RouteType::Embeddings => {
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
						_ => unreachable!(),
					};
					let (mut req, llm_request) = match r {
						RequestResult::Success(r, lr) => (r, lr),
						RequestResult::Rejected(dr) => return Err(ProxyResponse::DirectResponse(Box::new(dr))),
					};
					if operation_span_guard.is_none()
						&& let Some(sw) = span_writer.as_ref()
					{
						let guard = start_llm_operation_span(sw, &llm_request);
						operation_span_writer = Some(guard.child_writer());
						operation_span_guard = Some(guard);
					}
					// If a user doesn't configure explicit overrides for connecting to a provider, setup default
					// paths, TLS, etc.
					llm
						.provider
						.setup_request(
							&mut req,
							route_type,
							Some(&llm_request),
							llm.use_default_policies(),
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
							operation_span_writer.clone(),
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
						.setup_request(&mut req, route_type, None, true)
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
				req.take_and_snapshot(log.as_mut())?,
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
		backend_call
			.backend_policies
			.http
			.as_ref()
			.and_then(|h| h.version)
			.or(backend_call.http_version_override),
	)
	.await?;
	let call = client::Call {
		req,
		target: backend_call.target,
		transport,
	};
	let upstream_target = call.target.to_string();
	let upstream_name = format!("upstream.http {}", upstream_target);
	let upstream_target_meta = call.target.clone();
	let upstream_method = call.req.method().to_string();
	let upstream_path = call.req.uri().path().to_string();
	let upstream_http_version = call.req.version();
	let upstream_retry_attempt = call
		.req
		.headers()
		.get("x-retry-attempt")
		.and_then(|v| v.to_str().ok())
		.map(str::to_string);
	let upstream_mcp_session = call
		.req
		.headers()
		.get("mcp-session-id")
		.and_then(|v| v.to_str().ok())
		.map(str::to_string);
	let upstream_span_meta = call.req.extensions().get::<UpstreamSpanMeta>().cloned();
	let upstream_start = SystemTime::now();
	let upstream = inputs.upstream.clone();
	let llm_response_log = log.as_ref().map(|l| l.llm_response.clone());
	let include_completion_in_log = log
		.as_ref()
		.map(|l| l.cel.cel_context.needs_llm_completion())
		.unwrap_or_default();
	let a2a_type = response_policies.a2a_type.clone();

	let mut resp = match upstream.call(call).await {
		Ok(resp) => {
			if let Some(sw) = operation_span_writer.as_ref() {
				sw.write(upstream_name.clone(), |sb| {
					let attrs = upstream_span_attributes(UpstreamSpanAttributesInput {
						upstream_target: &upstream_target_meta,
						upstream_method: &upstream_method,
						upstream_path: &upstream_path,
						upstream_version: upstream_http_version,
						status: Some(resp.status()),
						error_type: None,
						retry_attempt: upstream_retry_attempt.as_deref(),
						mcp_session: upstream_mcp_session.as_deref(),
						span_meta: upstream_span_meta.as_ref(),
					});
					sb.with_start_time(upstream_start)
						.with_attributes(attrs)
						.with_status(upstream_span_status(Some(resp.status()), None))
				});
			}
			resp
		},
		Err(err) => {
			if let Some(sw) = operation_span_writer.as_ref() {
				sw.write(upstream_name.clone(), |sb| {
					let attrs = upstream_span_attributes(UpstreamSpanAttributesInput {
						upstream_target: &upstream_target_meta,
						upstream_method: &upstream_method,
						upstream_path: &upstream_path,
						upstream_version: upstream_http_version,
						status: None,
						error_type: Some("ProxyError"),
						retry_attempt: upstream_retry_attempt.as_deref(),
						mcp_session: upstream_mcp_session.as_deref(),
						span_meta: upstream_span_meta.as_ref(),
					});
					sb.with_start_time(upstream_start)
						.with_attributes(attrs)
						.with_status(upstream_span_status(None, Some("ProxyError")))
				});
			}
			if let Some(guard) = operation_span_guard.as_mut() {
				guard.mark_error("proxy_error");
			}
			return Err(err.into());
		},
	};
	if let Err(err) = a2a::apply_to_response(
		backend_call.backend_policies.a2a.as_ref(),
		a2a_type,
		&mut resp,
	)
	.await
	{
		if let Some(guard) = operation_span_guard.as_mut() {
			guard.mark_error("processing_error");
		}
		return Err(ProxyError::Processing(err).into());
	}
	let mut resp = if let (Some(llm), Some(llm_request)) =
		(backend_call.backend_policies.llm_provider, llm_request)
	{
		match llm
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
		{
			Ok(resp) => resp,
			Err(err) => {
				if let Some(guard) = operation_span_guard.as_mut() {
					guard.mark_error("processing_error");
				}
				return Err(ProxyError::Processing(err.into()).into());
			},
		}
	} else {
		resp
	};
	// TODO: we currently do not support ImmediateResponse from inference router
	if let Err(err) = maybe_inference.mutate_response(&mut resp).await {
		if let Some(guard) = operation_span_guard.as_mut() {
			guard.mark_error("processing_error");
		}
		return Err(err.into());
	}
	if let Some(guard) = operation_span_guard.as_mut() {
		guard.mark_status(resp.status());
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
	override_dest: Option<SocketAddr>,
	svc: &Arc<Service>,
	port: &u16,
) -> Result<BackendCall, ProxyError> {
	let port = *port;
	let workloads = &inputs.stores.read_discovery().workloads;
	let (ep, handle, wl) = svc
		.endpoints
		.select_endpoint(workloads, svc.as_ref(), port, override_dest)
		.ok_or(ProxyError::NoHealthyEndpoints)?;

	let svc_target_port = svc.ports.get(&port).copied().unwrap_or_default();
	let target_port = if let Some(ov) = override_dest {
		// use the explicit override. select_endpoint ensures this is actually in the endpoint
		ov.port()
	} else if let Some(&ep_target_port) = ep.port.get(&port) {
		// prefer endpoint port mapping
		ep_target_port
	} else if svc_target_port > 0 {
		// otherwise, see if the service has this port
		svc_target_port
	} else {
		return Err(ProxyError::NoHealthyEndpoints);
	};

	let http_version_override = if svc.port_is_http2(port) {
		Some(::http::Version::HTTP_2)
	} else if svc.port_is_http1(port) {
		Some(::http::Version::HTTP_11)
	} else {
		None
	};

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
			hostname = % svc.hostname,
			port = % port,
			"using hostname-based target for double hbone"
		);
		Target::Hostname(svc.hostname.clone(), port)
	} else {
		// TODO: support a mode like ServiceEntry DYNAMIC_DNS. Need a way to signal this, though; perhaps:
		// wl.workload_ips.is_empty() && wl.hostname.starts_with("*.")
		// For direct connections, we need the workload IP
		let Some(ip) = wl.workload_ips.first() else {
			return Err(ProxyError::NoHealthyEndpoints);
		};
		let dest = SocketAddr::from((*ip, target_port));
		Target::Address(dest)
	};

	Ok(BackendCall {
		target,
		http_version_override,
		transport_override: Some((wl.protocol, wl.identity())),
		network_gateway,
		backend_policies,
	})
}

fn should_retry(res: &Result<Response, SnapshottedProxyResponse>, pol: &retry::Policy) -> bool {
	match res {
		Ok(resp) => pol.codes.contains(&resp.status()),
		Err(SnapshottedProxyResponse(ProxyResponse::Error(e))) => e.is_retryable(),
		Err(SnapshottedProxyResponse(ProxyResponse::DirectResponse(_))) => false,
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
	let upgrade_type = get_upgrade_type(req.headers());
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
fn normalize_uri(connection: &Extension, req: &mut Request) -> anyhow::Result<()> {
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
			if connection.get::<TLSConnectionInfo>().is_some() {
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
	pub transport_override: Option<(InboundProtocol, Identity)>,
	pub network_gateway: Option<(GatewayAddress, Identity)>, /* For double hbone: (gateway_address, gateway_identity) */
	pub backend_policies: BackendPolicies,
}

#[derive(Debug, Default)]
struct ResponsePolicies {
	timeout: Option<http::timeout::Policy>,
	route_response_header: Option<filters::HeaderModifier>,
	backend_response_header: Option<filters::HeaderModifier>,
	transformation: Option<Transformation>,
	gateway_transformation: Option<Transformation>,
	response_headers: HeaderMap,
	ext_proc: Option<ExtProcRequest>,
	gateway_ext_proc: Option<ExtProcRequest>,
	a2a_type: a2a::RequestType,
}

impl ResponsePolicies {
	pub fn headers(&mut self) -> &mut HeaderMap {
		&mut self.response_headers
	}

	pub async fn apply(
		&mut self,
		resp: &mut Response,
		log: &mut RequestLog,
		is_upstream_response: bool,
	) -> Result<(), ProxyResponse> {
		if let Some(rhm) = &self.route_response_header {
			rhm.apply(resp.headers_mut()).map_err(ProxyError::from)?;
		}
		if let Some(rhm) = &self.backend_response_header {
			rhm.apply(resp.headers_mut()).map_err(ProxyError::from)?;
		}
		if let Some(j) = &self.transformation {
			j.apply_response(resp, log.request_snapshot.as_ref());
		}
		if let Some(j) = &self.gateway_transformation {
			j.apply_response(resp, log.request_snapshot.as_ref());
		}

		// ext_proc is only intended to run on responses from upstream
		if is_upstream_response {
			let span_writer = log.span_writer();
			let ext_proc_resp = if let Some(x) = self.ext_proc.as_mut() {
				let mut policy_span = start_policy_span(span_writer.as_ref(), "policy.ext_proc.response");
				let result = x.mutate_response(resp, log.request_snapshot.as_ref()).await;
				end_policy_span(&mut policy_span, &result, |pr: &PolicyResponse| {
					pr.direct_response.is_some()
				});
				result?
			} else {
				PolicyResponse::default()
			};
			ext_proc_resp.apply(&mut self.response_headers)?;
			let gateway_ext_proc_resp = if let Some(x) = self.gateway_ext_proc.as_mut() {
				let mut policy_span = start_policy_span(span_writer.as_ref(), "policy.ext_proc.response");
				let result = x.mutate_response(resp, log.request_snapshot.as_ref()).await;
				end_policy_span(&mut policy_span, &result, |pr: &PolicyResponse| {
					pr.direct_response.is_some()
				});
				result?
			} else {
				PolicyResponse::default()
			};
			gateway_ext_proc_resp.apply(&mut self.response_headers)?;
		}

		merge_in_headers(Some(self.response_headers.clone()), resp.headers_mut());
		Ok(())
	}
}

#[derive(Debug, Clone)]
pub struct PolicyClient {
	pub inputs: Arc<ProxyInputs>,
}

#[derive(Debug, Clone)]
pub(crate) struct UpstreamSpanMeta {
	pub mcp_target: Option<String>,
}

struct UpstreamSpanAttributesInput<'a> {
	upstream_target: &'a Target,
	upstream_method: &'a str,
	upstream_path: &'a str,
	upstream_version: ::http::Version,
	status: Option<StatusCode>,
	error_type: Option<&'a str>,
	retry_attempt: Option<&'a str>,
	mcp_session: Option<&'a str>,
	span_meta: Option<&'a UpstreamSpanMeta>,
}

fn upstream_span_attributes(input: UpstreamSpanAttributesInput<'_>) -> Vec<KeyValue> {
	let UpstreamSpanAttributesInput {
		upstream_target,
		upstream_method,
		upstream_path,
		upstream_version,
		status,
		error_type,
		retry_attempt,
		mcp_session,
		span_meta,
	} = input;
	let (server_address, server_port, network_transport) = match upstream_target {
		Target::Address(addr) => (addr.ip().to_string(), Some(i64::from(addr.port())), "tcp"),
		Target::Hostname(hostname, port) => (hostname.to_string(), Some(i64::from(*port)), "tcp"),
		Target::UnixSocket(path) => (path.display().to_string(), None, "unix"),
	};
	let mut attrs = vec![
		KeyValue::new("server.address", server_address),
		KeyValue::new("http.request.method", upstream_method.to_string()),
		KeyValue::new("url.path", upstream_path.to_string()),
		KeyValue::new("network.protocol.name", "http"),
		KeyValue::new("network.transport", network_transport),
	];
	if let Some(server_port) = server_port {
		attrs.push(KeyValue::new("server.port", server_port));
	}
	match upstream_version {
		::http::Version::HTTP_11 => {
			attrs.push(KeyValue::new("network.protocol.version", "1.1"));
		},
		::http::Version::HTTP_2 => {
			attrs.push(KeyValue::new("network.protocol.version", "2"));
		},
		_ => {},
	}
	if let Some(status) = status {
		attrs.push(KeyValue::new(
			"http.response.status_code",
			i64::from(status.as_u16()),
		));
	}
	if let Some(error_type) = error_type {
		attrs.push(KeyValue::new("error.type", error_type.to_string()));
	}
	if let Some(retry_attempt) = retry_attempt {
		attrs.push(KeyValue::new(
			"gateway.retry_attempt",
			retry_attempt.to_string(),
		));
	}
	if let Some(mcp_session) = mcp_session {
		attrs.push(KeyValue::new("mcp.session.id", mcp_session.to_string()));
	}
	if let Some(meta) = span_meta
		&& let Some(mcp_target) = &meta.mcp_target
	{
		attrs.push(KeyValue::new("mcp.target", mcp_target.clone()));
	}
	attrs
}

fn upstream_span_status(status: Option<StatusCode>, error_type: Option<&str>) -> SpanStatus {
	if let Some(error_type) = error_type {
		return SpanStatus::error(error_type.to_string());
	}
	if let Some(status) = status
		&& status.is_server_error()
	{
		return SpanStatus::error(format!("http {}", status.as_u16()));
	}
	SpanStatus::Unset
}

impl PolicyClient {
	pub async fn call_reference(
		&self,
		req: Request,
		backend_ref: &SimpleBackendReference,
	) -> Result<Response, ProxyError> {
		self.call_reference_and_span(req, backend_ref, None).await
	}

	pub async fn call_reference_and_span(
		&self,
		mut req: Request,
		backend_ref: &SimpleBackendReference,
		span_writer: Option<SpanWriter>,
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
		let pols = get_backend_policies(&self.inputs, &backend, &[], None);
		self
			.internal_call_with_policies(req, backend.backend, pols, span_writer)
			.await
	}

	pub async fn call(
		&self,
		req: Request,
		backend: SimpleBackendWithPolicies,
	) -> Result<Response, ProxyError> {
		self.call_and_span(req, backend, None).await
	}

	pub async fn call_and_span(
		&self,
		req: Request,
		backend: SimpleBackendWithPolicies,
		span_writer: Option<SpanWriter>,
	) -> Result<Response, ProxyError> {
		let backend = BackendWithPolicies::from(backend);
		let pols = get_backend_policies(&self.inputs, &backend, &[], None);
		self
			.internal_call_with_policies(req, backend.backend, pols, span_writer)
			.await
	}

	pub async fn call_with_default_policies(
		&self,
		req: Request,
		backend: &SimpleBackend,
		defaults: BackendPolicies,
	) -> Result<Response, ProxyError> {
		self
			.call_with_default_policies_and_span(req, backend, defaults, None)
			.await
	}

	pub async fn call_with_default_policies_and_span(
		&self,
		req: Request,
		backend: &SimpleBackend,
		defaults: BackendPolicies,
		span_writer: Option<SpanWriter>,
	) -> Result<Response, ProxyError> {
		let backend = Backend::from(backend.clone()).into();
		let pols = defaults.merge(get_backend_policies(&self.inputs, &backend, &[], None));
		self
			.internal_call_with_policies(req, backend.backend, pols, span_writer)
			.await
	}

	pub async fn call_with_explicit_policies(
		&self,
		req: Request,
		backend: SimpleBackend,
		policies: Vec<BackendPolicy>,
	) -> Result<Response, ProxyError> {
		self
			.call_with_explicit_policies_and_span(req, backend, policies, None)
			.await
	}

	pub async fn call_with_explicit_policies_and_span(
		&self,
		req: Request,
		backend: SimpleBackend,
		policies: Vec<BackendPolicy>,
		span_writer: Option<SpanWriter>,
	) -> Result<Response, ProxyError> {
		let backend = Backend::from(backend);
		let pols = self
			.inputs
			.stores
			.read_binds()
			.inline_backend_policies(&policies);
		self
			.internal_call_with_policies(req, backend, pols, span_writer)
			.await
	}

	fn internal_call_with_policies<'a>(
		&'a self,
		req: Request,
		backend: Backend,
		pols: BackendPolicies,
		span_writer: Option<SpanWriter>,
	) -> Pin<Box<dyn Future<Output = Result<Response, ProxyError>> + Send + '_>> {
		let mut req = Some(req);
		Box::pin(async move {
			if let Some(sw) = span_writer
				&& let Some(req) = req.as_mut()
			{
				req.extensions_mut().insert(sw);
			}
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
mod upstream_span_attr_tests {
	use super::*;

	fn has_key(attrs: &[KeyValue], key: &str) -> bool {
		attrs.iter().any(|kv| kv.key.as_str() == key)
	}

	fn has_str_value(attrs: &[KeyValue], key: &str, expected: &str) -> bool {
		attrs
			.iter()
			.any(|kv| kv.key.as_str() == key && kv.value.as_str() == expected)
	}

	#[test]
	fn upstream_span_attributes_include_mcp_fields() {
		let attrs = upstream_span_attributes(UpstreamSpanAttributesInput {
			upstream_target: &Target::Address("127.0.0.1:1234".parse().expect("socket addr")),
			upstream_method: "POST",
			upstream_path: "/mcp",
			upstream_version: ::http::Version::HTTP_2,
			status: Some(StatusCode::OK),
			error_type: None,
			retry_attempt: Some("2"),
			mcp_session: Some("sess-1"),
			span_meta: Some(&UpstreamSpanMeta {
				mcp_target: Some("calendar".to_string()),
			}),
		});
		assert!(has_str_value(&attrs, "network.protocol.name", "http"));
		assert!(has_str_value(&attrs, "mcp.target", "calendar"));
		assert!(has_str_value(&attrs, "mcp.session.id", "sess-1"));
		assert!(has_str_value(&attrs, "gateway.retry_attempt", "2"));
		assert!(has_key(&attrs, "http.response.status_code"));
	}

	#[test]
	fn upstream_span_attributes_include_error_type_on_failure() {
		let attrs = upstream_span_attributes(UpstreamSpanAttributesInput {
			upstream_target: &Target::Address("127.0.0.1:1234".parse().expect("socket addr")),
			upstream_method: "GET",
			upstream_path: "/foo",
			upstream_version: ::http::Version::HTTP_11,
			status: None,
			error_type: Some("ProxyError"),
			retry_attempt: None,
			mcp_session: None,
			span_meta: None,
		});
		assert!(has_str_value(&attrs, "network.protocol.name", "http"));
		assert!(has_str_value(&attrs, "error.type", "ProxyError"));
	}
}
