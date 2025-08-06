use std::cell::RefCell;
use std::collections::HashMap;
use std::convert::Infallible;
use std::iter::Empty;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use ::http::uri::PathAndQuery;
use ::http::{HeaderMap, header};
use agent_core::drain::{DrainMode, DrainUpgrader, DrainWatcher, new};
use agent_core::{copy, drain};
use anyhow::anyhow;
use crossbeam::atomic::AtomicCell;
use futures::pin_mut;
use futures_util::{FutureExt, TryFutureExt};
use headers::HeaderMapExt;
use http_body::{Body, Frame, SizeHint};
use http_body_util::BodyExt;
use hyper::Error;
use hyper::body::Incoming;
use hyper::upgrade::OnUpgrade;
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto;
use hyper_util::server::graceful::{GracefulConnection, GracefulShutdown};
use hyper_util_fork::client::legacy::Error as HyperError;
use itertools::Itertools;
use rand::Rng;
use rand::seq::IndexedRandom;
use rustls::ServerConfig;
use secrecy::ExposeSecret;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::sync::watch::Receiver;
use tokio::task::{AbortHandle, JoinSet};
use tokio_stream::StreamExt;
use tonic::IntoRequest;
use tracing::{Instrument, debug, event, info, info_span, trace};
use types::agent::*;
use types::discovery::*;

use crate::cel::ContextBuilder;
use crate::client::{Client, Transport};
use crate::http::auth::BackendAuth;
use crate::http::backendtls::BackendTLS;
use crate::http::ext_authz::ExtAuthz;
use crate::http::ext_proc::ExtProc;
use crate::http::jwt::{Claims, TokenError};
use crate::http::transformation_cel::Transformation;
use crate::http::{
	Authority, HeaderName, HeaderValue, Request, Response, Scheme, StatusCode, Uri, auth, ext_proc,
	filters, get_host, merge_in_headers, retry,
};
use crate::llm::{LLMRequest, LLMResponse, RequestResult};
use crate::proxy::{ProxyError, resolve_simple_backend};
use crate::store::{BackendPolicies, Event, LLMRoutePolicies, RoutePolicies};
use crate::telemetry::log;
use crate::telemetry::log::{AsyncLog, DropOnLog, LogBody, RequestLog};
use crate::telemetry::metrics::TCPLabels;
use crate::telemetry::trc::TraceParent;
use crate::transport::stream::{Extension, Socket, TCPConnectionInfo, TLSConnectionInfo};
use crate::types::agent;
use crate::types::proto::ProtoError;
use crate::{ProxyInputs, *};

fn select_backend(route: &Route, _req: &Request) -> Option<RouteBackendReference> {
	route
		.backends
		.choose_weighted(&mut rand::rng(), |b| b.weight)
		.ok()
		.cloned()
}

async fn apply_request_policies(
	policies: &store::RoutePolicies,
	client: PolicyClient,
	log: &mut RequestLog,
	req: &mut Request,
) -> Result<http::PolicyResponse, ProxyError> {
	if let Some(j) = &policies.jwt {
		j.apply(log, req)
			.await
			.map_err(ProxyError::JwtAuthenticationFailure)?;
	}
	let ext_auth = if let Some(x) = &policies.ext_authz {
		x.check(client.clone(), req).await?
	} else {
		http::PolicyResponse::default()
	};
	if ext_auth.should_short_circuit() {
		return Ok(ext_auth);
	}

	for lrl in &policies.local_rate_limit {
		lrl.check_request(req)?;
	}
	let exec = log
		.cel
		.ctx()
		.build()
		.map_err(|_| ProxyError::ProcessingString("failed to build cel context".to_string()))?;
	let lrl = if let Some(rrl) = &policies.remote_rate_limit {
		rrl.check(client, req, &exec).await?
	} else {
		http::PolicyResponse::default()
	};
	let policy_resp = ext_auth.merge(lrl);
	if policy_resp.should_short_circuit() {
		return Ok(policy_resp);
	}

	if let Some(j) = &policies.transformation {
		j.apply_request(req, &exec)
			.map_err(|_| ProxyError::TransformationFailure)?;
	}

	Ok(policy_resp)
}

fn apply_llm_request_policies(
	policies: &store::LLMRoutePolicies,
	req: &LLMRequest,
) -> Result<(), ProxyError> {
	for lrl in &policies.local_rate_limit {
		lrl.check_llm_request(req)?;
	}
	Ok(())
}

fn apply_request_filters(
	filters: &[RouteFilter],
	path_match: &PathMatch,
	req: &mut Request,
) -> Result<(Option<Response>, Option<HeaderMap>), filters::Error> {
	debug!("before request filters: {:?}", req);
	let mut header_map = None;
	for filter in filters {
		match filter {
			RouteFilter::RequestHeaderModifier(hm) => hm.apply(req.headers_mut())?,
			RouteFilter::UrlRewrite(rw) => rw.apply(req, path_match)?,
			RouteFilter::RequestRedirect(red) => {
				return Ok((Some(red.apply(req, path_match)?), header_map));
			},
			RouteFilter::DirectResponse(dr) => return Ok((Some(dr.apply(req)?), header_map)),
			RouteFilter::CORS(c) => {
				let res = c.apply(req)?;
				if let Some(dr) = res.direct_response {
					return Ok((Some(dr), None));
				} else if let Some(hm) = res.response_headers {
					header_map = Some(hm)
				}
			},
			// Response only
			RouteFilter::ResponseHeaderModifier { .. } => {},
			// This is handled elsewhere
			RouteFilter::RequestMirror(_) => {},
		}
	}
	Ok((None, header_map))
}

fn get_mirrors(filters: &[RouteFilter]) -> Vec<filters::RequestMirror> {
	let mut res = vec![];
	for filter in filters {
		if let RouteFilter::RequestMirror(m) = filter {
			res.push(m.clone())
		}
	}
	res
}

fn apply_response_filters(
	filters: &[RouteFilter],
	resp: &mut Response,
) -> Result<(), filters::Error> {
	for filter in filters {
		match filter {
			RouteFilter::ResponseHeaderModifier(rh) => rh.apply(resp.headers_mut())?,
			RouteFilter::RequestHeaderModifier { .. } => {},
			RouteFilter::UrlRewrite { .. } => {},
			RouteFilter::RequestRedirect { .. } => {},
			RouteFilter::RequestMirror(_) => {},
			RouteFilter::DirectResponse(_) => {},
			RouteFilter::CORS(_) => {},
		}
	}
	Ok(())
}

fn load_balance(
	pi: Arc<ProxyInputs>,
	svc: &Service,
	svc_port: u16,
	override_dest: Option<SocketAddr>,
) -> Option<(&Endpoint, Arc<Workload>)> {
	let state = &pi.stores;
	let workloads = &state.read_discovery().workloads;
	let target_port = svc.ports.get(&svc_port).copied();

	if target_port.is_none() {
		// Port doesn't exist on the service at all, this is invalid
		debug!("service {} does not have port {}", svc.hostname, svc_port);
		return None;
	};

	let endpoints = svc.endpoints.iter().filter_map(|ep| {
		let Some(wl) = workloads.find_uid(&ep.workload_uid) else {
			debug!("failed to fetch workload for {}", ep.workload_uid);
			return None;
		};
		if target_port.unwrap_or_default() == 0 && !ep.port.contains_key(&svc_port) {
			// Filter workload out, it doesn't have a matching port
			trace!(
				"filter endpoint {}, it does not have service port {}",
				ep.workload_uid, svc_port
			);
			return None;
		}
		if let Some(o) = override_dest {
			if !wl.workload_ips.contains(&o.ip()) {
				// We ignore port, assume its a bug to have a mismatch
				trace!(
					"filter endpoint {}, it was not the selected endpoint {}",
					ep.workload_uid, o
				);
				return None;
			}
		}
		Some((ep, wl))
	});

	let options = endpoints.collect_vec();
	options
		.choose_weighted(&mut rand::rng(), |(_, wl)| wl.capacity as u64)
		// This can fail if there are no weights, the sum is zero (not possible in our API), or if it overflows
		// The API has u32 but we sum into an u64, so it would take ~4 billion entries of max weight to overflow
		.ok()
		.cloned()
}

#[derive(Clone)]
pub struct HTTPProxy {
	pub(super) bind_name: BindName,
	pub(super) inputs: Arc<ProxyInputs>,
	pub(super) selected_listener: Option<Arc<Listener>>,
	pub(super) target_address: SocketAddr,
}

impl HTTPProxy {
	pub async fn proxy(
		&self,
		connection: Arc<Extension>,
		mut req: ::http::Request<Incoming>,
	) -> Response {
		let start = Instant::now();

		// Copy connection level attributes into request level attributes
		connection.copy::<TCPConnectionInfo>(req.extensions_mut());
		connection.copy::<TLSConnectionInfo>(req.extensions_mut());

		let tcp = connection
			.get::<TCPConnectionInfo>()
			.expect("tcp connection must be set");
		let mut log: DropOnLog = RequestLog::new(
			log::CelLogging::new(
				self.inputs.cfg.logging.clone(),
				self.inputs.cfg.tracing.clone(),
			),
			self.inputs.metrics.clone(),
			start,
			tcp.clone(),
		)
		.into();
		let ret = self
			.proxy_internal(connection, req, log.as_mut().unwrap())
			.await;

		log.with(|l| l.error = ret.as_ref().err().map(|e| e.to_string()));
		let resp = ret.unwrap_or_else(|err| err.as_response());

		// Pass the log into the body so it finishes once the stream is entirely complete.
		// We will also record trailer info there.
		log.with(|l| {
			l.status = Some(resp.status());
			l.cel.ctx().with_response(&resp)
		});

		resp.map(move |b| http::Body::new(LogBody::new(b, log)))
	}
	async fn proxy_internal(
		&self,
		connection: Arc<Extension>,
		mut req: ::http::Request<Incoming>,
		log: &mut RequestLog,
	) -> Result<Response, ProxyError> {
		log.tls_info = connection.get::<TLSConnectionInfo>().cloned();
		log
			.cel
			.ctx()
			.with_source(&log.tcp_info, log.tls_info.as_ref());
		let selected_listener = self.selected_listener.clone();
		let upstream = self.inputs.upstream.clone();
		let inputs = self.inputs.clone();
		let bind_name = self.bind_name.clone();
		debug!(bind=%bind_name, "route for bind");
		self
			.inputs
			.metrics
			.downstream_connection
			.get_or_create(&TCPLabels {
				bind: Some(&self.bind_name).into(),
				// For HTTP, this will be empty
				gateway: selected_listener.as_ref().map(|l| &l.gateway_name).into(),
				listener: selected_listener.as_ref().map(|l| &l.name).into(),
				protocol: if log.tls_info.is_some() {
					BindProtocol::https
				} else {
					BindProtocol::http
				},
			})
			.inc();
		let Some(listeners) = ({
			let state = inputs.stores.read_binds();
			state.listeners(bind_name.clone())
		}) else {
			return Err(ProxyError::BindNotFound);
		};

		let mut req = req.map(http::Body::new);

		normalize_uri(&connection, &mut req).map_err(ProxyError::Processing)?;
		sensitive_headers(&mut req);
		let mut req_upgrade = hop_by_hop_headers(&mut req);

		let host = http::get_host(&req)?.to_string();
		log.host = Some(host.clone());
		log.method = Some(req.method().clone());
		log.path = Some(req.uri().path().to_string());
		log.version = Some(req.version());
		let needs_body = log.cel.ctx().with_request(&req);
		if needs_body {
			if let Ok(body) = crate::http::inspect_body(req.body_mut()).await {
				log.cel.ctx().with_request_body(body);
			}
		}

		let trace_parent = trc::TraceParent::from_request(&req);
		let trace_sampled = log.trace_sampled(trace_parent.as_ref());
		if trace_sampled {
			log.tracer = self.inputs.tracer.clone();
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
			ns.insert_header(&mut req);
			req.extensions_mut().insert(ns.clone());
			log.outgoing_span = Some(ns);
		}

		if let Some(tracer) = &log.tracer {
			log.cel.register(tracer.fields.as_ref());
		}

		let selected_listener = selected_listener
			.or_else(|| listeners.best_match(&host))
			.ok_or(ProxyError::ListenerNotFound)?;
		log.gateway_name = Some(selected_listener.gateway_name.clone());
		log.listener_name = Some(selected_listener.name.clone());

		debug!(bind=%bind_name, listener=%selected_listener.key, "selected listener");

		let (selected_route, path_match) = http::route::select_best_route(
			inputs.stores.clone(),
			inputs.cfg.network.clone(),
			inputs.cfg.self_addr.clone(),
			self.target_address,
			selected_listener.clone(),
			&req,
		)
		.ok_or(ProxyError::RouteNotFound)?;
		log.route_rule_name = selected_route.rule_name.clone();
		log.route_name = Some(selected_route.route_name.clone());

		debug!(bind=%bind_name, listener=%selected_listener.key, route=%selected_route.key, "selected route");

		let route_policies = inputs.stores.read_binds().route_policies(
			selected_route.key.clone(),
			selected_route.route_name.clone(),
			selected_listener.gateway_name.clone(),
		);
		// Register all expressions
		route_policies.register_cel_expressions(&req, log.cel.ctx());
		// This is unfortunate but we record the request twice possibly; we want to record it as early as possible
		// so we can do logging, etc when we find no routes.
		// But we may find new expressions that now need the request.
		// it is zero-cost at runtime to do it twice so NBD.
		let needs_body = log.cel.ctx().with_request(&req);
		if needs_body {
			if let Ok(body) = crate::http::inspect_body(req.body_mut()).await {
				log.cel.ctx().with_request_body(body);
			}
		}

		let ext_authz_response =
			apply_request_policies(&route_policies, self.policy_client(), log, &mut req).await?;
		if let Some(dr) = ext_authz_response.direct_response {
			return Ok(dr);
		}
		let mut response_polices = ResponsePolicies::from(route_policies.transformation.clone());
		merge_in_headers(
			ext_authz_response.response_headers,
			&mut response_polices.response_headers,
		);

		let (direct_response_route, response_headers_route) = apply_request_filters(
			selected_route.as_ref().filters.as_slice(),
			&path_match,
			&mut req,
		)?;
		if let Some(resp) = direct_response_route {
			return Ok(resp);
		}
		merge_in_headers(
			response_headers_route,
			&mut response_polices.response_headers,
		);

		let selected_backend =
			select_backend(selected_route.as_ref(), &req).ok_or(ProxyError::NoValidBackends)?;
		let selected_backend = resolve_backend(selected_backend, self.inputs.as_ref())?;
		let (direct_response, response_headers_backend) =
			apply_request_filters(selected_backend.filters.as_slice(), &path_match, &mut req)?;
		if let Some(resp) = direct_response {
			return Ok(resp);
		}
		merge_in_headers(
			response_headers_backend,
			&mut response_polices.response_headers,
		);

		let mut mirrors = get_mirrors(selected_route.as_ref().filters.as_slice());
		mirrors.extend(get_mirrors(selected_backend.filters.as_slice()));
		let (head, body) = req.into_parts();
		for mirror in mirrors {
			if !rand::rng().random_bool(mirror.percentage) {
				trace!(
					"skipping mirror, percentage {} not triggered",
					mirror.percentage
				);
				continue;
			}
			// TODO: mirror the body. For now, we just ignore the body
			let req = Request::from_parts(head.clone(), http::Body::empty());
			let upstream = self.inputs.upstream.clone();
			let inputs = inputs.clone();
			let policy_client = self.policy_client();
			tokio::task::spawn(async move {
				if let Err(e) = send_mirror(inputs, policy_client, mirror, req).await {
					warn!("error sending mirror request: {}", e);
				}
			});
		}

		const MAX_BUFFERED_BYTES: usize = 64 * 1024;
		let retries = match &selected_route.policies {
			Some(TrafficPolicy { retry, .. }) => retry,
			_ => &None,
		};
		let late_route_policies: Arc<LLMRoutePolicies> = Arc::new(route_policies.into());
		let response_polices = Arc::new(response_polices);
		// attempts is the total number of attempts, not the retries
		let attempts = retries.as_ref().map(|r| r.attempts.get() + 1).unwrap_or(1);
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
				let mut req = Request::from_parts(head, http::Body::new(body));
				return self
					.attempt_upstream(
						log,
						&mut req_upgrade,
						late_route_policies,
						upstream,
						&selected_backend,
						&selected_route,
						response_polices,
						req,
					)
					.await;
			},
		};
		let mut last_res: Option<Result<Response, ProxyError>> = None;
		for n in 0..attempts {
			let last = n == attempts - 1;
			let this = next.take().expect("next should be set");
			debug!("attempt {n}/{}", attempts - 1);
			if matches!(this.is_capped(), None | Some(true)) {
				// This could be either too much buffered, or it could mean we got a response before we read the request body.
				debug!("buffered too much to attempt a retry");
				return last_res.expect("should only be capped if we had a previous attempt");
			}
			// If we don't need
			last_res = None;
			if !last {
				// Stop cloning on our last
				next = Some(this.clone());
			}
			let mut head = head.clone();
			if n > 0 {
				log.retry_attempt = Some(n);
				head.headers.insert(
					HeaderName::from_static("x-retry-attempt"),
					HeaderValue::try_from(format!("{n}"))
						.map_err(|e| ProxyError::ProcessingString(e.to_string()))?,
				);
			}
			let mut req = Request::from_parts(head, http::Body::new(this));
			let res = self
				.attempt_upstream(
					log,
					&mut req_upgrade,
					late_route_policies.clone(),
					upstream.clone(),
					&selected_backend,
					&selected_route,
					response_polices.clone(),
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
				"attempting another retry, last result was {} {:?}",
				res.is_err(),
				res.as_ref().map(|r| r.status())
			);
			last_res = Some(res);
		}
		unreachable!()
	}

	#[allow(clippy::too_many_arguments)]
	async fn attempt_upstream(
		&self,
		log: &mut RequestLog,
		req_upgrade: &mut Option<RequestUpgrade>,
		route_policies: Arc<store::LLMRoutePolicies>,
		upstream: Client,
		selected_backend: &RouteBackend,
		selected_route: &Route,
		response_policies: Arc<ResponsePolicies>,
		mut req: Request,
	) -> Result<Response, ProxyError> {
		let inputs = self.inputs.clone();

		let call = make_backend_call(
			self.inputs.clone(),
			&route_policies,
			&selected_backend.backend,
			None,
			req,
			Some(log),
		)
		.await?;

		let timeout = match &selected_route.policies {
			Some(TrafficPolicy { timeout, .. }) => timeout.effective_timeout(),
			_ => None,
		};

		// Setup timeout
		let (call_result, body_timeout) = if let Some(timeout) = timeout {
			let deadline = tokio::time::Instant::from_std(log.start + timeout);
			let fut = tokio::time::timeout_at(deadline, call);
			(fut.await, http::timeout::BodyTimeout::Deadline(deadline))
		} else {
			(Ok(call.await), http::timeout::BodyTimeout::None)
		};

		// Run the actual call
		let mut resp = match call_result {
			Ok(Ok(resp)) => resp,
			Ok(Err(e)) => {
				return Err(e);
			},
			Err(_) => {
				return Err(ProxyError::RequestTimeout);
			},
		};
		if resp.status() == StatusCode::SWITCHING_PROTOCOLS {
			return handle_upgrade(req_upgrade, resp).await;
		}

		// Handle response filters
		apply_response_filters(selected_route.filters.as_slice(), &mut resp)?;
		apply_response_filters(selected_backend.filters.as_slice(), &mut resp)?;
		response_policies.apply(&mut resp, log)?;

		// for now we do not have any body timeout. Maybe we should add it
		// let resp = body_timeout.apply(resp);

		// gRPC status can be in the initial headers or a trailer, add if they are here
		maybe_set_grpc_status(&log.grpc_status, resp.headers());

		Ok(resp)
	}

	async fn process_backend_call(
		&self,
		log: &mut RequestLog,
		mut req: &mut Request,
		backend_call: &BackendCall,
	) -> Result<(), ProxyError> {
		Ok(())
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
		filters: b.filters,
	})
}

async fn handle_upgrade(
	req_upgrade_type: &mut Option<RequestUpgrade>,
	mut resp: Response,
) -> Result<Response, ProxyError> {
	let Some(RequestUpgrade {
		upgade_type,
		upgrade,
	}) = std::mem::take(req_upgrade_type)
	else {
		return Err(ProxyError::UpgradeFailed(None, None));
	};
	let resp_upgrade_type = upgrade_type(resp.headers());
	if Some(&upgade_type) != resp_upgrade_type.as_ref() {
		return Err(ProxyError::UpgradeFailed(
			Some(upgade_type),
			resp_upgrade_type,
		));
	}
	let mut response_upgraded = resp
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
		let _ = agent_core::copy::copy_bidirectional(
			&mut TokioIo::new(req),
			&mut TokioIo::new(response_upgraded),
			&agent_core::copy::ConnectionResult {},
		)
		.await;
	});
	Ok(resp)
}

async fn build_transport(
	inputs: &ProxyInputs,
	backend_call: &BackendCall,
	backend_tls: Option<BackendTLS>,
) -> Result<Transport, ProxyError> {
	Ok(
		match (&backend_call.transport_override, backend_tls, &inputs.ca) {
			// Use legacy mTLS if they did not define a TLS policy. We could do double TLS but Istio doesn't,
			// so maintain bug-for-bug parity
			(Some((InboundProtocol::LegacyIstioMtls, ident)), None, Some(ca)) => {
				if let Ok(id) = ca.get_identity().await {
					Some(
						id.legacy_mtls(vec![ident.clone()])
							.map_err(|e| ProxyError::Processing(anyhow!("{e}")))?,
					)
					.into()
				} else {
					warn!("wanted TLS but CA is not available");
					Transport::Plaintext
				}
			},
			(Some((InboundProtocol::HBONE, ident)), btls, Some(ca)) => {
				if let Ok(id) = ca.get_identity().await {
					Transport::Hbone(btls, ident.clone())
				} else {
					warn!("wanted TLS but CA is not available");
					Transport::Plaintext
				}
			},
			(_, pol, _) => pol.into(),
		},
	)
}

async fn make_backend_call(
	inputs: Arc<ProxyInputs>,
	route_policies: &store::LLMRoutePolicies,
	backend: &Backend,
	default_policies: Option<BackendPolicies>,
	mut req: Request,
	mut log: Option<&mut RequestLog>,
) -> Result<Pin<Box<dyn Future<Output = Result<Response, ProxyError>> + Send>>, ProxyError> {
	let client = inputs.upstream.clone();

	let policy_target = PolicyTarget::Backend(backend.name());
	let policies = inputs.stores.read_binds().backend_policies(policy_target);
	let mut maybe_inference = policies.build_inference(PolicyClient {
		inputs: inputs.clone(),
	});
	let override_dest = maybe_inference.mutate_request(&mut req).await?;
	log.add(|l| l.inference_pool = override_dest);

	let backend_call = match backend {
		Backend::AI(_, ai) => {
			let (target, default_policies) = match &ai.host_override {
				Some(target) => (
					target.clone(),
					Some(BackendPolicies {
						backend_tls: None,
						backend_auth: None,
						a2a: None,
						llm: None,
						inference_routing: None,
						// Attach LLM provider, but don't use default setup
						llm_provider: Some((ai.provider.clone(), false, ai.tokenize)),
					}),
				),
				None => {
					let (tgt, mut pol) = ai.provider.default_connector();
					pol.llm_provider = Some((ai.provider.clone(), true, ai.tokenize));
					(tgt, Some(pol))
				},
			};
			BackendCall {
				target,
				default_policies,
				http_version_override: None,
				transport_override: None,
			}
		},
		Backend::Service(svc, port) => {
			let port = *port;
			let (ep, wl) = load_balance(inputs.clone(), svc.as_ref(), port, override_dest)
				.ok_or(ProxyError::NoHealthyEndpoints)?;
			let svc_target_port = svc.ports.get(&port).copied().unwrap_or_default();
			let target_port = if let Some(&ep_target_port) = ep.port.get(&port) {
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
			let Some(ip) = wl.workload_ips.first() else {
				return Err(ProxyError::NoHealthyEndpoints);
			};
			let dest = SocketAddr::from((*ip, target_port));
			BackendCall {
				target: Target::Address(dest),
				http_version_override,
				transport_override: Some((wl.protocol, wl.identity())),
				default_policies,
			}
		},
		Backend::Opaque(_, target) => BackendCall {
			target: target.clone(),
			http_version_override: None,
			transport_override: None,
			default_policies,
		},
		Backend::Dynamic {} => {
			let port = req
				.extensions()
				.get::<TCPConnectionInfo>()
				.unwrap()
				.local_addr
				.port();
			let target = Target::try_from((get_host(&req)?, port)).map_err(ProxyError::Processing)?;
			BackendCall {
				target: target.clone(),
				http_version_override: None,
				transport_override: None,
				default_policies,
			}
		},
		Backend::MCP(name, backend) => {
			let inputs = inputs.clone();
			let backend = backend.clone();
			let name = name.clone();
			let mcp_response_log = log.map(|l| l.mcp_status.clone()).expect("must be set");
			return Ok(Box::pin(async move {
				inputs
					.clone()
					.mcp_state
					.serve(inputs, name, backend, req, mcp_response_log)
					.map(Ok)
					.await
			}));
		},
		Backend::Invalid => return Err(ProxyError::BackendDoesNotExist),
	};

	match backend_call.http_version_override {
		Some(::http::Version::HTTP_2) => {
			req.headers_mut().remove(http::header::TRANSFER_ENCODING);
			*req.version_mut() = ::http::Version::HTTP_2;
		},
		Some(::http::Version::HTTP_11) => {
			*req.version_mut() = ::http::Version::HTTP_11;
		},
		_ => {},
	};
	log.add(|l| l.endpoint = Some(backend_call.target.clone()));

	let policies = match backend_call.default_policies.clone() {
		Some(def) => def.merge(policies),
		None => policies,
	};

	// Apply auth before LLM request setup, so the providers can assume auth is in standardized header
	auth::apply_backend_auth(policies.backend_auth.as_ref(), &mut req).await?;
	let a2a_type = a2a::apply_to_request(policies.a2a.as_ref(), &mut req).await;
	if let a2a::RequestType::Call(method) = a2a_type {
		log.add(|l| l.a2a_method = Some(method));
	}

	let (mut req, llm_request) =
		if let Some((llm, use_default_policies, tokenize)) = &policies.llm_provider {
			let r = llm
				.process_request(client, policies.llm.as_ref(), req, *tokenize, &mut log)
				.await
				.map_err(|e| ProxyError::Processing(e.into()))?;
			let (mut req, llm_request) = match r {
				RequestResult::Success(r, lr) => (r, lr),
				RequestResult::Rejected(dr) => return Ok(Box::pin(async move { Ok(dr) })),
			};
			if *use_default_policies {
				llm
					.setup_request(&mut req, &llm_request)
					.map_err(ProxyError::Processing)?;
			}
			apply_llm_request_policies(route_policies, &llm_request)?;
			log.add(|l| l.llm_request = Some(llm_request.clone()));
			(req, Some(llm_request))
		} else {
			(req, None)
		};
	// Some auth types (AWS) need to be applied after all request processing
	auth::apply_late_backend_auth(policies.backend_auth.as_ref(), &mut req).await?;
	let transport = build_transport(&inputs, &backend_call, policies.backend_tls.clone()).await?;
	let call = client::Call {
		req,
		target: backend_call.target,
		transport,
	};
	let mut upstream = inputs.upstream.clone();
	let llm_response_log = log.as_ref().map(|l| l.llm_response.clone());
	let include_completion_in_log = log
		.as_ref()
		.map(|l| l.cel.cel_context.needs_llm_completion())
		.unwrap_or_default();
	let rate_limit = route_policies.local_rate_limit.clone();
	Ok(Box::pin(async move {
		let mut resp = upstream.call(call).await?;
		a2a::apply_to_response(policies.a2a.as_ref(), a2a_type, &mut resp)
			.await
			.map_err(ProxyError::Processing)?;
		let mut resp =
			if let (Some((llm, _, _)), Some(llm_request)) = (policies.llm_provider, llm_request) {
				llm
					.process_response(
						llm_request,
						rate_limit,
						llm_response_log.expect("must be set"),
						include_completion_in_log,
						resp,
					)
					.await
					.map_err(|e| ProxyError::Processing(e.into()))?
			} else {
				resp
			};
		maybe_inference.mutate_response(&mut resp).await?;
		Ok(resp)
	}))
}

fn should_retry(res: &Result<Response, ProxyError>, pol: &retry::Policy) -> bool {
	match res {
		Ok(resp) => pol.codes.contains(&resp.status()),
		Err(e) => e.is_retryable(),
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

struct RequestUpgrade {
	upgade_type: HeaderValue,
	upgrade: OnUpgrade,
}

fn hop_by_hop_headers(req: &mut Request) -> Option<RequestUpgrade> {
	let trailers = req
		.headers()
		.get(header::TE)
		.and_then(|h| h.to_str().ok())
		.map(|s| s.contains("trailers"))
		.unwrap_or(false);
	let upgrade_type = upgrade_type(req.headers());
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
			upgade_type: t,
			upgrade: u,
		})
	} else {
		None
	}
}

fn upgrade_type(headers: &HeaderMap) -> Option<HeaderValue> {
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
	if let ::http::Version::HTTP_10 | ::http::Version::HTTP_11 = req.version() {
		if req.uri().authority().is_none() {
			let mut parts = std::mem::take(req.uri_mut()).into_parts();
			// TODO: handle absolute HTTP/1.1 form
			let host = req
				.headers()
				.get(http::header::HOST)
				.and_then(|h| h.to_str().ok())
				.and_then(|h| h.parse::<Authority>().ok())
				.ok_or_else(|| anyhow::anyhow!("no authority or host"))?;
			req.headers_mut().remove(http::header::HOST);

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
	}
	debug!("request after normalization: {req:?}");
	Ok(())
}

struct BackendCall {
	target: Target,
	http_version_override: Option<::http::Version>,
	transport_override: Option<(InboundProtocol, Identity)>,
	default_policies: Option<BackendPolicies>,
}

#[derive(Debug)]
struct ResponsePolicies {
	transformation: Option<Transformation>,
	response_headers: HeaderMap,
}

impl ResponsePolicies {
	pub fn from(transformation: Option<Transformation>) -> ResponsePolicies {
		Self {
			transformation,
			response_headers: HeaderMap::new(),
		}
	}
	pub fn apply(&self, resp: &mut Response, log: &mut RequestLog) -> Result<(), ProxyError> {
		if let Some(j) = &self.transformation {
			j.apply_response(resp, log.cel.ctx())
				.map_err(|_| ProxyError::TransformationFailure)?;
		}
		merge_in_headers(Some(self.response_headers.clone()), resp.headers_mut());
		Ok(())
	}
}

#[derive(Debug, Clone)]
pub struct PolicyClient {
	pub inputs: Arc<ProxyInputs>,
}

impl PolicyClient {
	pub async fn call_reference(
		&self,
		mut req: Request,
		backend_ref: &SimpleBackendReference,
	) -> Result<Response, ProxyError> {
		let backend = resolve_simple_backend(backend_ref, self.inputs.as_ref())?;
		trace!("resolved {:?} to {:?}", backend_ref, &backend);

		http::modify_req_uri(&mut req, |uri| {
			if uri.authority.is_none() {
				// If host is not set, set it to the backend
				uri.authority = Some(Authority::try_from(backend.hostport())?);
			}
			if uri.scheme.is_none() {
				// Default to HTTP, if the policy is TLS it will get set correctly later
				uri.scheme = Some(Scheme::HTTP);
			}
			Ok(())
		})
		.map_err(ProxyError::Processing)?;
		self.call(req, backend).await
	}
	pub async fn call(&self, req: Request, backend: SimpleBackend) -> Result<Response, ProxyError> {
		make_backend_call(
			self.inputs.clone(),
			&LLMRoutePolicies::default(),
			&backend.into(),
			None,
			req,
			None,
		)
		.await?
		.await
	}
	pub async fn call_with_default_policies(
		&self,
		req: Request,
		backend: &SimpleBackend,
		defaults: BackendPolicies,
	) -> Result<Response, ProxyError> {
		Box::pin(
			make_backend_call(
				self.inputs.clone(),
				&LLMRoutePolicies::default(),
				&backend.clone().into(),
				Some(defaults),
				req,
				None,
			)
			.await?,
		)
		.await
	}

	pub async fn simple_call(&self, req: Request) -> Result<Response, ProxyError> {
		self.inputs.upstream.simple_call(req).await
	}
}
trait OptLogger {
	fn add<F>(&mut self, f: F)
	where
		F: Fn(&mut RequestLog);
}

impl OptLogger for Option<&mut RequestLog> {
	fn add<F>(&mut self, f: F)
	where
		F: Fn(&mut RequestLog),
	{
		if let Some(log) = self.as_mut() {
			f(log)
		}
	}
}
