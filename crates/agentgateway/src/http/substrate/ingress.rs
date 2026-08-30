use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU16;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use ::http::StatusCode;
use quick_cache::sync::Cache;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tonic::Code;

use super::{ActorRef, CACHE_CAPACITY, TRACE_POLICY_KIND, valid_resource_name};
use crate::http::{PolicyResponse, Request, Response};
use crate::proxy::dtrace::{Severity, pol_event};
use crate::proxy::httpproxy::PolicyClient;
use crate::proxy::{ProxyError, dtrace};
use crate::store::RequestPolicyTrait;
use crate::telemetry::log::RequestLog;
use crate::telemetry::metrics::{OutboundCallKind, OutboundCallSubtype};
use crate::types::agent::{SimpleBackendReferenceWithPolicies, Target};
use crate::*;

const ACTOR_DNS_SUFFIX: &str = ".actors.resources.substrate.ate.dev";
const RESUME_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_PARKING_BUDGET: Duration = Duration::from_secs(5);
const DEFAULT_PARKING_MAX: usize = 1024;
const DEFAULT_PARKING_RETRY_INTERVAL: Duration = Duration::from_millis(100);
const DEFAULT_PARKING_RETRY_FACTOR: f64 = 1.1;
const DEFAULT_ACTOR_PORT: u16 = 80;
const DEFAULT_CONNECT_TARGET_PORT: NonZeroU16 = NonZeroU16::new(8443).unwrap();
const TARGET_PORT_HEADER: &str = "x-ate-target-port";
pub(crate) const STALE_ASSIGNMENT_HEADER: &str = "x-ate-assignment-stale";

#[derive(Debug, Clone, thiserror::Error)]
enum ResumeError {
	#[error("{0:?}: {1}")]
	Status(Code, String),
	#[error("{0}")]
	InvalidResponse(String),
	#[error("request parking capacity exhausted")]
	ParkingFull,
}

impl ResumeError {
	fn into_proxy_error(self, actor: &ActorRef) -> ProxyError {
		let (status, body) = match self {
			Self::ParkingFull => (
				StatusCode::SERVICE_UNAVAILABLE,
				format!("actor {:?} request parking capacity exhausted", actor.name),
			),
			Self::Status(Code::NotFound, _) => (
				StatusCode::NOT_FOUND,
				format!("actor {:?} not found", actor.name),
			),
			Self::Status(Code::FailedPrecondition, message) => (
				StatusCode::SERVICE_UNAVAILABLE,
				format!("actor {:?} unavailable: {message}", actor.name),
			),
			Self::Status(Code::Unavailable, _) => (
				StatusCode::SERVICE_UNAVAILABLE,
				format!("actor {:?} unavailable", actor.name),
			),
			Self::Status(Code::DeadlineExceeded, _) => (
				StatusCode::GATEWAY_TIMEOUT,
				format!("actor {:?} request timed out", actor.name),
			),
			Self::Status(Code::PermissionDenied, _) => (
				StatusCode::FORBIDDEN,
				format!("actor {:?} access denied", actor.name),
			),
			Self::Status(Code::Unauthenticated, _) => (
				StatusCode::UNAUTHORIZED,
				format!("actor {:?} authentication required", actor.name),
			),
			Self::Status(Code::ResourceExhausted, _) => (
				StatusCode::TOO_MANY_REQUESTS,
				format!("actor {:?} rate limited", actor.name),
			),
			Self::Status(_, _) | Self::InvalidResponse(_) => (
				StatusCode::INTERNAL_SERVER_ERROR,
				format!("error resuming actor {:?}", actor.name),
			),
		};
		ProxyError::SubstrateIngressFailed(status, body)
	}
}

#[derive(Debug, Clone)]
struct CachedAssignment {
	target: SocketAddr,
	expires_at: Instant,
	generation: u64,
}

#[derive(Clone, Copy)]
enum ResolutionSource {
	Request,
	Cache,
	AteApi,
}

impl ResolutionSource {
	fn name(self) -> &'static str {
		match self {
			Self::Request => "request",
			Self::Cache => "cache",
			Self::AteApi => "ateApi",
		}
	}

	fn cached(self) -> bool {
		matches!(self, Self::Request | Self::Cache)
	}
}

type ResolutionResult =
	Result<(CachedAssignment, ResolutionSource), (ResumeError, ResolutionSource)>;

struct AssignmentCache {
	entries: Cache<ActorRef, Result<CachedAssignment, ResumeError>>,
	next_generation: AtomicU64,
}

impl std::fmt::Debug for AssignmentCache {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("AssignmentCache").finish_non_exhaustive()
	}
}

impl AssignmentCache {
	fn remove_generation(&self, actor: &ActorRef, generation: u64) {
		self.entries.remove_if(
			actor,
			|entry| matches!(entry, Ok(cached) if cached.generation == generation),
		);
	}
}

#[derive(Clone)]
pub(crate) struct SubstrateRequestState {
	actor: ActorRef,
	actor_port: u16,
	ingress: SubstrateIngress,
	client: PolicyClient,
	current: Arc<Mutex<Option<CachedAssignment>>>,
}

fn default_cache_ttl() -> Duration {
	Duration::from_secs(5)
}

fn default_cache() -> Arc<AssignmentCache> {
	Arc::new(AssignmentCache {
		entries: Cache::new(CACHE_CAPACITY),
		next_generation: AtomicU64::new(0),
	})
}

fn default_parking_budget() -> Duration {
	DEFAULT_PARKING_BUDGET
}

fn default_parking_max() -> usize {
	DEFAULT_PARKING_MAX
}

fn default_parking_retry_interval() -> Duration {
	DEFAULT_PARKING_RETRY_INTERVAL
}

fn default_parking_retry_factor() -> f64 {
	DEFAULT_PARKING_RETRY_FACTOR
}

fn default_connect_target_port() -> NonZeroU16 {
	DEFAULT_CONNECT_TARGET_PORT
}

/// Bounds requests held while an actor is waiting for capacity to resume.
#[apply(schema!)]
pub struct RequestParking {
	/// Maximum time to wait for the actor to become routable.
	#[serde(default = "default_parking_budget", with = "serde_dur")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	pub budget: Duration,
	/// Maximum concurrent requests that may wait for actor resumption. Set to 0 to disable parking.
	#[serde(default = "default_parking_max")]
	pub max: usize,
	/// Initial delay between ResumeActor retries while parked.
	#[serde(default = "default_parking_retry_interval", with = "serde_dur")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	pub retry_interval: Duration,
	/// Multiplier applied to the delay after each parked retry.
	#[serde(default = "default_parking_retry_factor")]
	pub retry_factor: f64,
}

impl RequestParking {
	fn default_config() -> Self {
		Self {
			budget: default_parking_budget(),
			max: default_parking_max(),
			retry_interval: default_parking_retry_interval(),
			retry_factor: default_parking_retry_factor(),
		}
	}

	fn enabled(&self) -> bool {
		self.max > 0
	}

	fn budget(&self) -> Duration {
		if !self.enabled() {
			RESUME_TIMEOUT
		} else if !self.budget.is_zero() {
			self.budget
		} else {
			DEFAULT_PARKING_BUDGET
		}
	}

	fn retry_interval(&self) -> Duration {
		if self.retry_interval.is_zero() {
			DEFAULT_PARKING_RETRY_INTERVAL
		} else {
			self.retry_interval
		}
	}
}

impl Default for RequestParking {
	fn default() -> Self {
		Self::default_config()
	}
}

/// Resolves Substrate actor hostnames through the ate-api for dynamic route backends.
#[apply(schema!)]
pub struct SubstrateIngress {
	/// Backend that receives ResumeActor calls and policies used when connecting to it.
	#[serde(flatten)]
	pub target: SimpleBackendReferenceWithPolicies,
	/// Port on the resumed worker pod's atunnel CONNECT listener. Defaults to 8443.
	#[serde(default = "default_connect_target_port")]
	#[cfg_attr(feature = "schema", schemars(with = "std::num::NonZeroU16"))]
	pub connect_target_port: NonZeroU16,
	/// How long successful actor assignments are reused. Defaults to 5s; 0s disables reuse.
	#[serde(default = "default_cache_ttl", with = "serde_dur")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	pub cache_ttl: Duration,
	/// Bounded request parking while a suspended actor is waiting for worker capacity.
	#[serde(default)]
	pub request_parking: RequestParking,
	#[serde(skip, default = "default_cache")]
	#[cfg_attr(feature = "schema", schemars(skip))]
	cache: Arc<AssignmentCache>,
	#[serde(skip, default)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	parking_slots: Arc<OnceLock<Arc<Semaphore>>>,
}

impl SubstrateIngress {
	async fn resume_actor(
		&self,
		client: &PolicyClient,
		actor: &ActorRef,
	) -> Result<SocketAddr, ResumeError> {
		let budget = self.request_parking.budget();
		let deadline = tokio::time::Instant::now() + budget;
		let result = async {
			let channel = self.target.grpc_channel(
				client.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Substrate),
			);
			let mut control = protos::ateapi::control_client::ControlClient::new(channel);
			let message = protos::ateapi::ResumeActorRequest {
				actor: Some(protos::ateapi::ObjectRef {
					atespace: actor.atespace.clone(),
					name: actor.name.clone(),
				}),
				boot: false,
			};
			let mut delay = self.request_parking.retry_interval();
			loop {
				let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
				if remaining.is_zero() {
					return Err(ResumeError::Status(
						Code::DeadlineExceeded,
						format!("ResumeActor timed out after {budget:?}"),
					));
				}
				let response = dtrace::scope_future(
					Some(TRACE_POLICY_KIND),
					tokio::time::timeout(remaining, control.resume_actor(message.clone())),
				)
				.await;
				match response {
					Ok(Ok(response)) => {
						let actor = response.into_inner().actor.ok_or_else(|| {
							ResumeError::InvalidResponse(
								"ResumeActor response did not include an actor".to_owned(),
							)
						})?;
						let assignment = actor
							.status
							.and_then(|status| status.worker_assignment)
							.ok_or_else(|| {
								ResumeError::InvalidResponse(
									"ResumeActor response did not include a worker assignment".to_owned(),
								)
							})?;
						let ip = assignment
							.worker_pod_ip
							.parse::<IpAddr>()
							.map_err(|error| {
								ResumeError::InvalidResponse(format!(
									"invalid worker_assignment.worker_pod_ip {:?}: {error}",
									assignment.worker_pod_ip
								))
							})?;
						return Ok(SocketAddr::new(ip, self.connect_target_port.get()));
					},
					Ok(Err(status)) if self.retryable_while_parked(status.code()) => {
						let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
						tokio::time::sleep(delay.min(remaining)).await;
						delay = delay.mul_f64(self.request_parking.retry_factor.max(1.0));
					},
					Ok(Err(status)) => {
						return Err(ResumeError::Status(
							status.code(),
							status.message().to_owned(),
						));
					},
					Err(_) => {
						return Err(ResumeError::Status(
							Code::DeadlineExceeded,
							format!("ResumeActor timed out after {budget:?}"),
						));
					},
				}
			}
		};
		result.await
	}

	fn acquire_parking_slot(&self) -> Result<Option<OwnedSemaphorePermit>, ResumeError> {
		if !self.request_parking.enabled() {
			return Ok(None);
		}
		let slots = self
			.parking_slots
			.get_or_init(|| Arc::new(Semaphore::new(self.request_parking.max)));
		slots
			.clone()
			.try_acquire_owned()
			.map(Some)
			.map_err(|_| ResumeError::ParkingFull)
	}

	fn retryable_while_parked(&self, code: Code) -> bool {
		matches!(code, Code::Aborted)
			|| (self.request_parking.enabled()
				&& matches!(
					code,
					Code::FailedPrecondition | Code::ResourceExhausted | Code::Unavailable
				))
	}

	async fn resolve(&self, client: &PolicyClient, actor: ActorRef) -> ResolutionResult {
		// Cached assignments need no parking slot. Every cache miss, including a
		// follower waiting for another request's in-flight resolution, does: this
		// is the bounded set of parked requests.
		if let Some(cached) = self.cache.entries.get(&actor) {
			match cached {
				Ok(cached) if cached.expires_at > Instant::now() => {
					return Ok((cached, ResolutionSource::Cache));
				},
				Ok(expired) => self.cache.remove_generation(&actor, expired.generation),
				Err(_) => {
					self.cache.entries.remove_if(&actor, |entry| entry.is_err());
				},
			}
		}
		let _parking_permit = self
			.acquire_parking_slot()
			.map_err(|error| (error, ResolutionSource::Request))?;
		loop {
			match self.cache.entries.get_value_or_guard_async(&actor).await {
				Ok(Ok(cached)) if cached.expires_at > Instant::now() => {
					return Ok((cached, ResolutionSource::Cache));
				},
				Ok(Ok(expired)) => {
					self.cache.remove_generation(&actor, expired.generation);
				},
				Ok(Err(error)) => {
					self.cache.entries.remove_if(&actor, |entry| entry.is_err());
					return Err((error, ResolutionSource::Cache));
				},
				Err(guard) => {
					let result = self
						.resume_actor(client, &actor)
						.await
						.map(|target| CachedAssignment {
							target,
							expires_at: Instant::now() + self.cache_ttl,
							generation: self.cache.next_generation.fetch_add(1, Ordering::Relaxed),
						});
					let _ = guard.insert(result.clone());
					match &result {
						Err(_) => {
							self.cache.entries.remove_if(&actor, |entry| entry.is_err());
						},
						Ok(cached) if self.cache_ttl.is_zero() => {
							self.cache.remove_generation(&actor, cached.generation);
						},
						Ok(_) => {},
					}
					return result
						.map(|assignment| (assignment, ResolutionSource::AteApi))
						.map_err(|error| (error, ResolutionSource::AteApi));
				},
			}
		}
	}
}

impl SubstrateRequestState {
	/// The authority sent to atunnel when proxying a raw CONNECT tunnel. atunnel
	/// authenticates the router connection and uses this stable actor DNS name
	/// plus port to select the currently active actor process.
	pub(crate) fn connect_authority(&self) -> String {
		format!(
			"{}.{}{}:{}",
			self.actor.name, self.actor.atespace, ACTOR_DNS_SUFFIX, self.actor_port
		)
	}

	pub(crate) async fn resolve_target(&self) -> Result<Target, crate::proxy::ProxyResponse> {
		if let Some(current) = self.current.lock().unwrap().as_ref() {
			pol_event!(
				TRACE_POLICY_KIND,
				Severity::Info,
				details = serde_json::json!({
					"operation": "resumeActor",
					"actor": self.actor.name,
					"atespace": self.actor.atespace,
					"source": ResolutionSource::Request.name(),
					"cached": true,
					"lookedUp": false,
					"target": current.target.to_string(),
				}),
			);
			return Ok(Target::Address(current.target));
		}
		match self.ingress.resolve(&self.client, self.actor.clone()).await {
			Ok((assignment, source)) => {
				let target = assignment.target;
				pol_event!(
					TRACE_POLICY_KIND,
					Severity::Info,
					details = serde_json::json!({
						"operation": "resumeActor",
						"actor": self.actor.name,
						"atespace": self.actor.atespace,
						"source": source.name(),
						"cached": source.cached(),
						"lookedUp": matches!(source, ResolutionSource::AteApi),
						"target": target.to_string(),
					}),
				);
				*self.current.lock().unwrap() = Some(assignment);
				Ok(Target::Address(target))
			},
			Err((error, source)) => {
				pol_event!(
					TRACE_POLICY_KIND,
					Severity::Error,
					details = serde_json::json!({
						"operation": "resumeActor",
						"actor": self.actor.name,
						"atespace": self.actor.atespace,
						"source": source.name(),
						"cached": source.cached(),
						"lookedUp": matches!(source, ResolutionSource::AteApi),
						"error": error.to_string(),
					}),
				);
				match &error {
					ResumeError::Status(code, message) => warn!(
						actor = self.actor.name,
						atespace = self.actor.atespace,
						grpc.code = ?code,
						grpc.message = message,
						"substrate ResumeActor failed"
					),
					ResumeError::InvalidResponse(message) => warn!(
						actor = self.actor.name,
						atespace = self.actor.atespace,
						error = message,
						"substrate ResumeActor returned an invalid response"
					),
					ResumeError::ParkingFull => warn!(
						actor = self.actor.name,
						atespace = self.actor.atespace,
						"substrate request parking capacity exhausted"
					),
				}
				Err(error.into_proxy_error(&self.actor).into())
			},
		}
	}

	pub(crate) fn evict(&self) {
		if let Some(current) = self.current.lock().unwrap().take() {
			self
				.ingress
				.cache
				.remove_generation(&self.actor, current.generation);
		}
	}
}

pub(crate) fn is_stale_assignment(response: &Response) -> bool {
	response.status() == StatusCode::MISDIRECTED_REQUEST
		&& response
			.headers()
			.get(STALE_ASSIGNMENT_HEADER)
			.is_some_and(|value| value == "true")
}

pub(crate) fn stale_assignment_unavailable() -> Response {
	::http::Response::builder()
		.status(StatusCode::SERVICE_UNAVAILABLE)
		.body(crate::http::Body::empty())
		.expect("a static status-only response is valid")
}

impl RequestPolicyTrait for SubstrateIngress {
	async fn apply(
		&self,
		client: &PolicyClient,
		log: &mut RequestLog,
		req: &mut Request,
	) -> Result<PolicyResponse, crate::proxy::ProxyResponse> {
		let connect_authority = req
			.extensions()
			.get::<crate::cel::SourceContext>()
			.and_then(|source| {
				let mut values = source.connect_headers.get_all(::http::header::HOST).iter();
				let authority = values.next()?.to_str().ok()?;
				(values.next().is_none()).then_some(authority)
			});
		// CONNECT re-entry retains the outer authority in SourceContext. A direct
		// CONNECT routed by AgentGateway has no such re-entry, so its request URI
		// is the authoritative source (and preserves its non-default port).
		let authority = connect_authority
			.map(ToOwned::to_owned)
			.or_else(|| {
				req
					.uri()
					.authority()
					.map(|authority| authority.as_str().to_owned())
			})
			.unwrap_or_else(|| crate::http::get_host(req).unwrap_or_default().to_owned());
		let authority = authority
			.parse::<::http::uri::Authority>()
			.map_err(|error| {
				ProxyError::SubstrateIngressFailed(
					StatusCode::NOT_FOUND,
					format!("invalid actor authority {authority:?}: {error}"),
				)
			})?;
		let host = authority.host();
		let actor_port = authority.port_u16().unwrap_or(DEFAULT_ACTOR_PORT);
		let host = host.strip_suffix('.').unwrap_or(host);
		let parsed = host
			.strip_suffix(ACTOR_DNS_SUFFIX)
			.and_then(|prefix| prefix.split_once('.'))
			.filter(|(_, atespace)| !atespace.contains('.'));
		let Some((name, atespace)) =
			parsed.filter(|(name, atespace)| valid_resource_name(name) && valid_resource_name(atespace))
		else {
			return Err(
				ProxyError::SubstrateIngressFailed(
					StatusCode::NOT_FOUND,
					format!("invalid host {host:?}: expected <actor>.<atespace>{ACTOR_DNS_SUFFIX}"),
				)
				.into(),
			);
		};

		let actor = ActorRef {
			atespace: atespace.to_owned(),
			name: name.to_owned(),
		};
		log.ate_actor_id = Some(actor.name.clone());
		log.ate_atespace = Some(actor.atespace.clone());
		// Ordinary atunnel ingress uses this header to select the actor port and
		// strips it before forwarding. Raw CONNECT carries the port in its
		// authority instead, which atunnel parses directly.
		if req.method() != ::http::Method::CONNECT {
			req.headers_mut().insert(
				::http::header::HeaderName::from_static(TARGET_PORT_HEADER),
				::http::HeaderValue::from(actor_port),
			);
		}
		req.extensions_mut().insert(SubstrateRequestState {
			actor,
			actor_port,
			ingress: self.clone(),
			client: client.clone(),
			current: Arc::new(Mutex::new(None)),
		});
		Ok(PolicyResponse::default())
	}
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;
	use std::sync::atomic::{AtomicUsize, Ordering};
	use std::time::{Duration, Instant};

	use ::http::Method;
	use protos::ateapi::control_server::{Control, ControlServer};
	use protos::ateapi::{
		Actor, ActorStatus, GetActorRequest, ResumeActorRequest, ResumeActorResponse,
	};
	use tonic::{Request as GrpcRequest, Response as GrpcResponse, Status};
	use wiremock::matchers::{header, method};
	use wiremock::{Mock, MockServer, ResponseTemplate};

	use super::STALE_ASSIGNMENT_HEADER;
	use crate::strng;
	use crate::test_helpers::proxymock::{
		basic_named_route, send_request, setup_proxy_test, simple_bind,
	};
	use crate::types::agent::{Backend, ResourceName};

	#[test]
	fn default_connect_target_port_matches_atunnel_connect_ingress() {
		assert_eq!(super::default_connect_target_port().get(), 8443);
	}

	#[derive(Clone)]
	struct MockControl {
		pod_ip: String,
		calls: Arc<AtomicUsize>,
	}

	#[tonic::async_trait]
	impl Control for MockControl {
		async fn get_actor(
			&self,
			_request: GrpcRequest<GetActorRequest>,
		) -> Result<GrpcResponse<Actor>, Status> {
			Err(Status::unimplemented("not used"))
		}

		async fn resume_actor(
			&self,
			request: GrpcRequest<ResumeActorRequest>,
		) -> Result<GrpcResponse<ResumeActorResponse>, Status> {
			let actor = request.into_inner().actor.unwrap();
			if actor.name != "my-actor" || actor.atespace != "my-space" {
				return Err(Status::invalid_argument("wrong actor"));
			}
			self.calls.fetch_add(1, Ordering::Relaxed);
			Ok(GrpcResponse::new(ResumeActorResponse {
				actor: Some(Actor {
					status: Some(ActorStatus {
						state: 0,
						worker_assignment: Some(protos::ateapi::WorkerAssignment {
							worker_pod_ip: self.pod_ip.clone(),
						}),
					}),
					..Default::default()
				}),
			}))
		}
	}

	#[tokio::test]
	async fn stale_assignment_retries_wait_for_assignment_convergence() {
		let actor = MockServer::start().await;
		let actor_calls = Arc::new(AtomicUsize::new(0));
		let responder_calls = actor_calls.clone();
		Mock::given(method("GET"))
			.and(header(
				"host",
				"my-actor.my-space.actors.resources.substrate.ate.dev",
			))
			.respond_with(move |_: &wiremock::Request| {
				if responder_calls.fetch_add(1, Ordering::Relaxed) < 2 {
					ResponseTemplate::new(421).insert_header(STALE_ASSIGNMENT_HEADER, "true")
				} else {
					ResponseTemplate::new(200)
				}
			})
			.mount(&actor)
			.await;
		let control_calls = Arc::new(AtomicUsize::new(0));
		let control = crate::test_helpers::spawn_service(ControlServer::new(MockControl {
			pod_ip: actor.address().ip().to_string(),
			calls: control_calls.clone(),
		}))
		.await;

		let dynamic = Backend::Dynamic(ResourceName::new("dynamic".into(), "".into()), None);
		let mut proxy = setup_proxy_test("{}")
			.unwrap()
			.with_raw_backend(dynamic.into())
			.with_bind(simple_bind())
			.with_route(basic_named_route(strng::literal!("/dynamic")));
		proxy
			.attach_route_policy(serde_json::json!({
				"substrateIngress": {
					"host": control.address.to_string(),
					"connectTargetPort": actor.address().port(),
					"cacheTtl": "5s"
				}
			}))
			.await;
		let client = proxy.serve_http("bind".into());

		let started = Instant::now();
		for _ in 0..2 {
			let response = send_request(
				client.clone(),
				Method::GET,
				"http://my-actor.my-space.actors.resources.substrate.ate.dev/",
			)
			.await;
			assert_eq!(response.status(), ::http::StatusCode::OK);
		}
		assert!(started.elapsed() >= Duration::from_millis(200));
		assert_eq!(actor_calls.load(Ordering::Relaxed), 4);
		assert_eq!(control_calls.load(Ordering::Relaxed), 3);
	}

	#[tokio::test]
	async fn stale_assignment_is_not_exposed_after_retries_are_exhausted() {
		let actor = MockServer::start().await;
		Mock::given(method("GET"))
			.and(header(
				"host",
				"my-actor.my-space.actors.resources.substrate.ate.dev",
			))
			.respond_with(ResponseTemplate::new(421).insert_header(STALE_ASSIGNMENT_HEADER, "true"))
			.mount(&actor)
			.await;
		let control = crate::test_helpers::spawn_service(ControlServer::new(MockControl {
			pod_ip: actor.address().ip().to_string(),
			calls: Arc::new(AtomicUsize::new(0)),
		}))
		.await;

		let dynamic = Backend::Dynamic(ResourceName::new("dynamic".into(), "".into()), None);
		let mut proxy = setup_proxy_test("{}")
			.unwrap()
			.with_raw_backend(dynamic.into())
			.with_bind(simple_bind())
			.with_route(basic_named_route(strng::literal!("/dynamic")));
		proxy
			.attach_route_policy(serde_json::json!({
				"substrateIngress": {
					"host": control.address.to_string(),
					"connectTargetPort": actor.address().port(),
				}
			}))
			.await;
		let response = send_request(
			proxy.serve_http("bind".into()),
			Method::GET,
			"http://my-actor.my-space.actors.resources.substrate.ate.dev/",
		)
		.await;

		assert_eq!(response.status(), ::http::StatusCode::SERVICE_UNAVAILABLE);
		assert!(response.headers().get(STALE_ASSIGNMENT_HEADER).is_none());
	}
}
