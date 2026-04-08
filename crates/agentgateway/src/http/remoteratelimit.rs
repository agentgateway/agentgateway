use crate::cel::{Executor, Expression};
use crate::http::envoy_proto_common;
use crate::http::ext_proc::GrpcReferenceChannel;
use crate::http::localratelimit::RateLimitType;
use crate::http::remoteratelimit::proto::rate_limit_descriptor::Entry;
use crate::http::remoteratelimit::proto::rate_limit_service_client::RateLimitServiceClient;
use crate::http::remoteratelimit::proto::{RateLimitDescriptor, RateLimitRequest};
use crate::http::{PolicyResponse, Request};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::types::agent::{BackendPolicy, SimpleBackendReference};
use crate::*;
use ::http::{HeaderMap, StatusCode};
use itertools::Itertools;

#[cfg(test)]
#[path = "remoteratelimit_tests.rs"]
mod tests;

#[allow(warnings)]
#[allow(clippy::derive_partial_eq_without_eq)]
pub mod proto {
	pub use protos::envoy::service::common::v3::HeaderValue;
	pub use protos::envoy::service::ratelimit::v3::*;
}

/// Defines how the proxy behaves when the remote rate limit service is
/// unavailable or returns an error.
///
/// Defaults to `FailClosed`. When failing closed, a 500 Internal Server Error
/// is returned when the service is unavailable. When failing open, requests are
/// allowed through despite the service failure.
///
/// # Configuration
///
/// Both camelCase (`failOpen`, `failClosed`) and PascalCase (`FailOpen`,
/// `FailClosed`) are accepted in configuration files
#[apply(schema!)]
#[derive(Default, Copy, PartialEq, Eq)]
pub enum FailureMode {
	/// Deny the request with a 500 status when the rate limit service is unavailable (default).
	#[default]
	#[serde(rename = "failClosed", alias = "FailClosed")]
	FailClosed,
	/// Allow the request through when the rate limit service is unavailable.
	#[serde(rename = "failOpen", alias = "FailOpen")]
	FailOpen,
}

#[apply(schema!)]
pub struct RemoteRateLimit {
	pub domain: String,
	#[serde(flatten)]
	pub target: Arc<SimpleBackendReference>,
	/// Policies to connect to the backend
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	#[serde(deserialize_with = "crate::types::local::de_from_local_backend_policy")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "Option<crate::types::local::SimpleLocalBackendPolicies>")
	)]
	pub policies: Vec<BackendPolicy>,
	pub descriptors: Arc<DescriptorSet>,
	/// Behavior when the remote rate limit service is unavailable or returns an error.
	/// Defaults to failClosed, denying requests with a 500 status on service failure.
	#[serde(default)]
	pub failure_mode: FailureMode,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor(pub String, pub cel::Expression);

#[apply(schema!)]
pub struct DescriptorSet(pub Vec<DescriptorEntry>);

#[apply(schema!)]
pub struct DescriptorEntry {
	#[serde(deserialize_with = "de_descriptors")]
	#[cfg_attr(feature = "schema", schemars(with = "Vec<KV>"))]
	pub entries: Arc<Vec<Descriptor>>,
	#[serde(default)]
	#[serde(rename = "type")]
	pub limit_type: RateLimitType,
	/// cost determines the optional expression to determine the cost of the request.
	/// If unset, type `requests` defaults to `1`, and type `tokens` defaults to `total_tokens`.
	/// If the expression fails to evaluate, the descriptor is skipped.
	/// Costs are evaluated upon request completion.
	pub cost: Option<Arc<cel::Expression>>,
	/// limitOverride determines the optional expression to determine the limit of the request.
	/// This tells the remote server what limit to apply to the request.
	/// Note: this does not specify the *cost* of the request, which is done by the `cost` field.
	/// The expression must evaluate to a map with `unit` and `requestsPerUnit` keys. For example:
	/// `{"unit":"second","requestsPerUnit":100}`.
	/// Valid units: second, minute, hour, day, month, year
	/// If the expression fails to evaluate, the descriptor is skipped.
	pub limit_override: Option<Arc<cel::Expression>>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DescriptorLimitOverride {
	unit: String,
	#[serde(alias = "requests_per_unit")]
	requests_per_unit: u32,
}

#[derive(serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
struct KV {
	key: String,
	value: String,
}

fn de_descriptors<'de: 'a, 'a, D>(deserializer: D) -> Result<Arc<Vec<Descriptor>>, D::Error>
where
	D: Deserializer<'de>,
{
	let raw = Vec::<KV>::deserialize(deserializer)?;
	let parsed: Vec<_> = raw
		.into_iter()
		.map(|i| cel::Expression::new_strict(i.value).map(|v| Descriptor(i.key, v)))
		.collect::<Result<_, _>>()
		.map_err(|e| serde::de::Error::custom(e.to_string()))?;
	Ok(Arc::new(parsed))
}

#[derive(Debug)]
pub struct LLMResponseAmend {
	base: RemoteRateLimit,
	client: PolicyClient,
	request: proto::RateLimitRequest,
	descriptor_costs: Vec<Option<Arc<Expression>>>,
}

impl LLMResponseAmend {
	pub fn amend_tokens(mut self, default_tokens: i64, exec: &Executor) {
		let mut index = 0;
		self.request.descriptors.retain_mut(|d| {
			let cost = self.descriptor_costs[index].as_ref();
			d.hits_addend = if let Some(cost) = cost {
				// if there is a cost expression, run it.
				let Some(cost) = exec.eval(cost).ok().and_then(|v| v.as_unsigned().ok()) else {
					// Failed to evaluate: skip descriptor
					return false;
				};
				Some(cost as u64)
			} else {
				// We cannot currently do negative amendments, so if its negative just skip
				// The input is not the cost, but the delta, so if we get -5 we should have a cost of 5
				let Ok(tokens) = (default_tokens).try_into() else {
					return false;
				};
				Some(tokens)
			};
			index += 1;
			true
		});
		tokio::task::spawn(async move {
			let _ = self.base.check_internal(self.client, self.request).await;
		});
	}
}

impl RemoteRateLimit {
	/// Build a rate-limit request by evaluating all descriptor entries of the
	/// given `limit_type` against the incoming HTTP request.
	///
	/// Individual descriptors whose CEL expressions fail to evaluate are
	/// silently dropped (matching Envoy's per-descriptor "all-or-nothing"
	/// semantics). Returns `None` only when **no** descriptor could be
	/// successfully resolved, so the gRPC call is skipped entirely.
	fn build_request(
		&self,
		req: &http::Request,
		limit_type: RateLimitType,
		default_cost: Option<u64>,
	) -> Option<(RateLimitRequest, Vec<Option<Arc<cel::Expression>>>)> {
		let mut descriptors = Vec::with_capacity(self.descriptors.0.len());
		let exec = cel::Executor::new_request(req);
		let candidate_count = self
			.descriptors
			.0
			.iter()
			.filter(|e| e.limit_type == limit_type)
			.count();
		trace!(
			"ratelimit build_request start: domain={}, type={:?}, cost={:?}, candidates={}",
			self.domain, limit_type, default_cost, candidate_count
		);

		let mut descriptor_costs = vec![];
		for desc_entry in self
			.descriptors
			.0
			.iter()
			.filter(|e| e.limit_type == limit_type)
		{
			if let Some(rl_entries) = Self::eval_descriptor(&exec, &desc_entry.entries) {
				// Rate limit servers require each descriptor to have at least one entry.
				if rl_entries.is_empty() {
					trace!(
						"ratelimit skipping descriptor with no entries for domain={}, type={:?}",
						self.domain, limit_type,
					);
					continue;
				}
				// Trace evaluated descriptor key/value pairs for visibility
				let kv_pairs: Vec<String> = rl_entries
					.iter()
					.map(|e| format!("{}={}", e.key, e.value))
					.collect();
				trace!(
					"ratelimit evaluated descriptors (domain: {}, type: {:?}): {}",
					self.domain,
					limit_type,
					kv_pairs.join(", ")
				);
				let hits_addend = if desc_entry.cost.is_some() && limit_type == RateLimitType::Tokens {
					// Skip sending anything on the target request side; the cost computation is specified to be on the response (amend) side
					Some(0)
				} else {
					match eval_cost(&exec, desc_entry.cost.as_deref(), default_cost) {
						Ok(hits_addend) => hits_addend,
						Err(e) => {
							trace!(
								"ratelimit cost evaluation failed for domain={}, type={:?}, expr={:?}, error={}",
								self.domain, limit_type, desc_entry.cost, e
							);
							continue;
						},
					}
				};

				let limit = match Self::eval_limit_override(&exec, desc_entry.limit_override.as_deref()) {
					Ok(limit) => limit,
					Err(e) => {
						trace!(
							"ratelimit limit override evaluation failed for domain={}, type={:?}, expr={:?}, error={}",
							self.domain, limit_type, desc_entry.limit_override, e
						);
						continue;
					},
				};
				descriptors.push(RateLimitDescriptor {
					entries: rl_entries,
					limit,
					hits_addend,
				});
				descriptor_costs.push(desc_entry.cost.clone());
			} else {
				trace!(
					"ratelimit descriptor evaluation failed for domain={}, type={:?}, skipping descriptor: {}",
					self.domain,
					limit_type,
					desc_entry
						.entries
						.iter()
						.map(|d| format!("{}={:?}", d.0, d.1))
						.join(", ")
				);
			}
		}

		if descriptors.is_empty() {
			trace!(
				"ratelimit all descriptors failed evaluation for domain={}, type={:?}, skipping rate-limit call",
				self.domain, limit_type,
			);
			return None;
		}

		trace!(
			"ratelimit built request descriptors (domain: {}, type: {:?}): count={}",
			self.domain,
			limit_type,
			descriptors.len()
		);

		Some((
			proto::RateLimitRequest {
				domain: self.domain.clone(),
				descriptors,
				// Ignored; we always set the per-descriptor one which allows distinguishing empty vs 0
				hits_addend: 0,
			},
			descriptor_costs,
		))
	}
	pub async fn check_llm(
		&self,
		client: PolicyClient,
		req: &mut Request,
		default_cost: u64,
	) -> Result<(PolicyResponse, Option<LLMResponseAmend>), ProxyError> {
		if !self
			.descriptors
			.0
			.iter()
			.any(|d| d.limit_type == RateLimitType::Tokens)
		{
			// Nothing to do
			trace!(
				"ratelimit: no token descriptors configured for domain={}, skipping",
				self.domain
			);
			return Ok((PolicyResponse::default(), None));
		}
		// We usually don't have any information at this point.
		// If they have an explicit `cost` expression, it is specified to be on output; send only a '0' cost here.
		// If they have tokenization enabled, we have an explicit cost to send, so we can send it.
		// Else send '0'.
		let Some((request, descriptor_costs)) =
			self.build_request(req, RateLimitType::Tokens, Some(default_cost))
		else {
			return Ok((PolicyResponse::default(), None));
		};
		let cr = self.check_internal(client.clone(), request.clone()).await;
		let r = LLMResponseAmend {
			base: self.clone(),
			client,
			request,
			descriptor_costs,
		};

		match cr {
			Ok(resp) => Self::apply(req, resp).map(|x| (x, Some(r))),
			Err(e) => {
				if self.failure_mode == FailureMode::FailOpen {
					Ok((PolicyResponse::default(), Some(r)))
				} else {
					Err(e)
				}
			},
		}
	}

	pub async fn check(
		&self,
		client: PolicyClient,
		req: &mut Request,
	) -> Result<PolicyResponse, ProxyError> {
		// This is on the request path
		if !self
			.descriptors
			.0
			.iter()
			.any(|d| d.limit_type == RateLimitType::Requests)
		{
			// Nothing to do
			trace!(
				"ratelimit: no request descriptors configured for domain={}, skipping",
				self.domain
			);
			return Ok(PolicyResponse::default());
		}
		let Some((request, _)) = self.build_request(req, RateLimitType::Requests, None) else {
			return Ok(PolicyResponse::default());
		};
		match self.check_internal(client, request).await {
			Ok(cr) => Self::apply(req, cr),
			Err(e) => {
				if self.failure_mode == FailureMode::FailOpen {
					Ok(PolicyResponse::default())
				} else {
					Err(e)
				}
			},
		}
	}

	async fn check_internal(
		&self,
		client: PolicyClient,
		request: proto::RateLimitRequest,
	) -> Result<proto::RateLimitResponse, ProxyError> {
		trace!("connecting to {:?}", self.target);
		trace!(
			"ratelimit request summary (domain: {}): descriptors={} {}",
			request.domain,
			request.descriptors.len(),
			request
				.descriptors
				.iter()
				.map(|d| {
					let kvs: Vec<String> = d
						.entries
						.iter()
						.map(|e| format!("{}={}", e.key, e.value))
						.collect();
					format!("[hits_addend={:?}; {}]", d.hits_addend, kvs.join(", "))
				})
				.join(" | ")
		);
		let chan = GrpcReferenceChannel {
			target: self.target.clone(),
			policies: Arc::new(self.policies.clone()),
			client,
		};
		let mut client = RateLimitServiceClient::new(chan);
		let resp = client.should_rate_limit(request).await;
		trace!("check response: {:?}", resp);
		if let Err(ref error) = resp {
			let ignore = self.failure_mode == FailureMode::FailOpen;
			warn!(
				"ratelimit service failed (domain: {}): {:?}; {}",
				self.domain,
				error,
				if ignore {
					"failure will be ignored (failure_mode: failOpen)"
				} else {
					"denying request (failure_mode: failClosed)"
				}
			);
		}
		let cr = resp.map_err(|_| ProxyError::RateLimitFailed)?;

		let cr = cr.into_inner();
		Ok(cr)
	}

	fn apply(req: &mut Request, cr: proto::RateLimitResponse) -> Result<PolicyResponse, ProxyError> {
		let mut res = PolicyResponse::default();
		// if not OK, we directly respond
		if cr.overall_code != (proto::rate_limit_response::Code::Ok as i32) {
			let mut rb = ::http::response::Builder::new().status(StatusCode::TOO_MANY_REQUESTS);
			if let Some(hm) = rb.headers_mut() {
				process_headers(hm, cr.response_headers_to_add)
			}
			let resp = rb
				.body(http::Body::from(cr.raw_body))
				.map_err(|e| ProxyError::Processing(e.into()))?;
			res.direct_response = Some(resp);
			return Ok(res);
		}

		process_headers(req.headers_mut(), cr.request_headers_to_add);
		if !cr.response_headers_to_add.is_empty() {
			let mut hm = HeaderMap::new();
			process_headers(&mut hm, cr.response_headers_to_add);
			res.response_headers = Some(hm);
		}
		Ok(res)
	}

	fn eval_limit_override(
		exec: &cel::Executor<'_>,
		limit_override: Option<&Expression>,
	) -> anyhow::Result<Option<proto::rate_limit_descriptor::RateLimitOverride>> {
		let Some(expr) = limit_override else {
			return Ok(None);
		};

		let raw = exec
			.eval(expr)?
			.json()
			.map_err(|_| cel::Error::JsonConvert)?;
		let override_config: DescriptorLimitOverride = serde_json::from_value(raw)?;
		let unit = match override_config.unit.to_ascii_lowercase().as_str() {
			"second" => proto::RateLimitUnit::Second,
			"minute" => proto::RateLimitUnit::Minute,
			"hour" => proto::RateLimitUnit::Hour,
			"day" => proto::RateLimitUnit::Day,
			"month" => proto::RateLimitUnit::Month,
			"year" => proto::RateLimitUnit::Year,
			unit => anyhow::bail!("invalid limit override unit: {unit}"),
		};
		Ok(Some(proto::rate_limit_descriptor::RateLimitOverride {
			requests_per_unit: override_config.requests_per_unit,
			unit: unit as i32,
		}))
	}

	fn eval_descriptor(exec: &cel::Executor<'_>, entries: &Vec<Descriptor>) -> Option<Vec<Entry>> {
		let mut rl_entries = Vec::with_capacity(entries.len());
		for Descriptor(k, lookup) in entries {
			// We drop the entire set if we cannot eval one; emit trace to aid debugging
			match exec.eval(lookup) {
				Ok(value) => {
					let Ok(string_value) = value.as_string() else {
						trace!(
							"ratelimit descriptor value not convertible to string: key={}, expr={:?}",
							k, lookup
						);
						return None;
					};
					let entry = Entry {
						key: k.clone(),
						value: string_value,
					};
					rl_entries.push(entry);
				},
				Err(e) => {
					trace!(
						"ratelimit failed to evaluate expression: key={}, expr={:?}, error={}",
						k, lookup, e
					);
					return None;
				},
			}
		}
		Some(rl_entries)
	}

	pub fn expressions(&self) -> impl Iterator<Item = &Expression> {
		self.descriptors.0.iter().flat_map(|v| {
			v.entries
				.iter()
				.map(|entry| &entry.1)
				.chain(v.cost.iter().map(|expr| expr.as_ref()))
				.chain(v.limit_override.iter().map(|expr| expr.as_ref()))
		})
	}
}

fn process_headers(hm: &mut HeaderMap, headers: Vec<proto::HeaderValue>) {
	for h in headers {
		let _ = envoy_proto_common::apply_header_value(hm, &h);
	}
}

fn eval_cost(
	exec: &cel::Executor<'_>,
	cost: Option<&Expression>,
	default_cost: Option<u64>,
) -> anyhow::Result<Option<u64>> {
	match cost {
		Some(expr) => Ok(Some(exec.eval(expr)?.as_unsigned()?.try_into()?)),
		None => Ok(default_cost),
	}
}
