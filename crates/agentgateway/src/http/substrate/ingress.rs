use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU16;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ::http::StatusCode;
use quick_cache::sync::Cache;
use tonic::Code;

use super::{ActorRef, CACHE_CAPACITY, TRACE_POLICY_KIND, valid_resource_name};
use crate::http::{PolicyResponse, Request, Response};
use crate::proxy::dtrace::{Severity, pol_event};
use crate::proxy::httpproxy::PolicyClient;
use crate::proxy::{ProxyError, dtrace};
use crate::store::RequestPolicyTrait;
use crate::telemetry::log::RequestLog;
use crate::types::agent::{SimpleBackendReferenceWithPolicies, Target};
use crate::*;

const ACTOR_DNS_SUFFIX: &str = ".actors.resources.substrate.ate.dev";
const RESUME_TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) const STALE_ASSIGNMENT_HEADER: &str = "x-ate-assignment-stale";

#[derive(Debug, Clone, thiserror::Error)]
enum ResumeError {
	#[error("{0:?}: {1}")]
	Status(Code, String),
	#[error("{0}")]
	InvalidResponse(String),
}

impl ResumeError {
	fn into_proxy_error(self, actor: &ActorRef) -> ProxyError {
		let (status, body) = match self {
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
	ingress: SubstrateIngress,
	client: PolicyClient,
	current: Arc<Mutex<Option<CachedAssignment>>>,
}

fn default_target_port() -> NonZeroU16 {
	NonZeroU16::new(80).unwrap()
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

/// Resolves Substrate actor hostnames through the ate-api for dynamic route backends.
#[apply(schema!)]
pub struct SubstrateIngress {
	/// Backend that receives ResumeActor calls and policies used when connecting to it.
	#[serde(flatten)]
	pub target: SimpleBackendReferenceWithPolicies,
	/// Port on the resumed actor pod. Defaults to 80.
	#[serde(default = "default_target_port")]
	#[cfg_attr(feature = "schema", schemars(with = "std::num::NonZeroU16"))]
	pub target_port: NonZeroU16,
	/// How long successful actor assignments are reused. Defaults to 5s; 0s disables reuse.
	#[serde(default = "default_cache_ttl", with = "serde_dur")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	pub cache_ttl: Duration,
	#[serde(skip, default = "default_cache")]
	#[cfg_attr(feature = "schema", schemars(skip))]
	cache: Arc<AssignmentCache>,
}

impl SubstrateIngress {
	async fn resume_actor(
		&self,
		client: &PolicyClient,
		actor: &ActorRef,
	) -> Result<SocketAddr, ResumeError> {
		tokio::time::timeout(RESUME_TIMEOUT, async {
			let channel = self.target.grpc_channel(client.clone());
			let mut control = protos::ateapi::control_client::ControlClient::new(channel);
			let message = protos::ateapi::ResumeActorRequest {
				actor: Some(protos::ateapi::ObjectRef {
					atespace: actor.atespace.clone(),
					name: actor.name.clone(),
				}),
				boot: false,
			};
			let mut delay = Duration::from_millis(200);
			for attempt in 0..7 {
				let response = {
					let _scope = dtrace::start_scope(TRACE_POLICY_KIND);
					control.resume_actor(message.clone()).await
				};
				match response {
					Ok(response) => {
						let actor = response.into_inner().actor.ok_or_else(|| {
							ResumeError::InvalidResponse(
								"ResumeActor response did not include an actor".to_owned(),
							)
						})?;
						let ip = actor.ateom_pod_ip.parse::<IpAddr>().map_err(|error| {
							ResumeError::InvalidResponse(format!(
								"invalid ateom_pod_ip {:?}: {error}",
								actor.ateom_pod_ip
							))
						})?;
						return Ok(SocketAddr::new(ip, self.target_port.get()));
					},
					Err(status) if status.code() == Code::Aborted && attempt < 6 => {
						tokio::time::sleep(delay).await;
						delay = delay.mul_f64(1.5);
					},
					Err(status) => {
						return Err(ResumeError::Status(
							status.code(),
							status.message().to_owned(),
						));
					},
				}
			}
			unreachable!()
		})
		.await
		.unwrap_or_else(|_| {
			Err(ResumeError::Status(
				Code::DeadlineExceeded,
				"ResumeActor timed out after 15s".to_owned(),
			))
		})
	}

	async fn resolve(&self, client: &PolicyClient, actor: ActorRef) -> ResolutionResult {
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

impl RequestPolicyTrait for SubstrateIngress {
	async fn apply(
		&self,
		client: &PolicyClient,
		log: &mut RequestLog,
		req: &mut Request,
	) -> Result<PolicyResponse, crate::proxy::ProxyResponse> {
		let host = crate::http::get_host(req).unwrap_or_default();
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
		req.extensions_mut().insert(SubstrateRequestState {
			actor,
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

	use ::http::Method;
	use protos::ateapi::control_server::{Control, ControlServer};
	use protos::ateapi::{Actor, GetActorRequest, ResumeActorRequest, ResumeActorResponse};
	use tonic::{Request as GrpcRequest, Response as GrpcResponse, Status};
	use wiremock::matchers::{header, method};
	use wiremock::{Mock, MockServer, ResponseTemplate};

	use super::STALE_ASSIGNMENT_HEADER;
	use crate::strng;
	use crate::test_helpers::proxymock::{
		basic_named_route, send_request, setup_proxy_test, simple_bind,
	};
	use crate::types::agent::{Backend, ResourceName};

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
					ateom_pod_ip: self.pod_ip.clone(),
					..Default::default()
				}),
			}))
		}
	}

	#[tokio::test]
	async fn stale_assignment_is_refreshed_then_cached() {
		let actor = MockServer::start().await;
		let actor_calls = Arc::new(AtomicUsize::new(0));
		let responder_calls = actor_calls.clone();
		Mock::given(method("GET"))
			.and(header(
				"host",
				"my-actor.my-space.actors.resources.substrate.ate.dev",
			))
			.respond_with(move |_: &wiremock::Request| {
				if responder_calls.fetch_add(1, Ordering::Relaxed) == 0 {
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

		let dynamic = Backend::Dynamic(ResourceName::new("dynamic".into(), "".into()), ());
		let mut proxy = setup_proxy_test("{}")
			.unwrap()
			.with_raw_backend(dynamic.into())
			.with_bind(simple_bind())
			.with_route(basic_named_route(strng::literal!("/dynamic")));
		proxy
			.attach_route_policy(serde_json::json!({
				"substrateIngress": {
					"host": control.address.to_string(),
					"targetPort": actor.address().port(),
					"cacheTtl": "5s"
				}
			}))
			.await;
		let client = proxy.serve_http("bind".into());

		for _ in 0..2 {
			let response = send_request(
				client.clone(),
				Method::GET,
				"http://my-actor.my-space.actors.resources.substrate.ate.dev/",
			)
			.await;
			assert_eq!(response.status(), ::http::StatusCode::OK);
		}
		assert_eq!(actor_calls.load(Ordering::Relaxed), 3);
		assert_eq!(control_calls.load(Ordering::Relaxed), 2);
	}
}
