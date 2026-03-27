use crate::cel::Expression;
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

#[cfg(test)]
#[path = "remoteratelimit_tests.rs"]
mod tests;

#[allow(warnings)]
#[allow(clippy::derive_partial_eq_without_eq)]
pub mod proto {
	pub use protos::envoy::service::common::v3::HeaderValue;
	pub use protos::envoy::service::ratelimit::v3::rate_limit_descriptor::RateLimitOverride;
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

/// Dynamic rate limit override configuration.
/// This maps to
/// When set, evaluates the CEL expression against the request context
/// to extract rate limit override values from ext-proc metadata.
#[derive(Debug, Clone)]
pub struct LimitOverride {
	/// Parsed CEL expression (e.g., "extproc.rate_limit")
	pub expression: Arc<cel::Expression>,
}

#[apply(schema!)]
pub struct DescriptorEntry {
	#[serde(deserialize_with = "de_descriptors")]
	#[cfg_attr(feature = "schema", schemars(with = "Vec<KV>"))]
	pub entries: Arc<Vec<Descriptor>>,
	#[serde(default)]
	#[serde(rename = "type")]
	pub limit_type: RateLimitType,
	/// Optional limit override from metadata (set via XDS, not local config)
	#[serde(skip)]
	pub limit_override: Option<LimitOverride>,
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
}

impl LLMResponseAmend {
	pub fn amend_tokens(mut self, tokens: i64) {
		// We cannot currently do negative amendments, so if its negative just skip
		// The input is not the cost, but the delta, so if we get -5 we should have a cost of 5
		let Ok(tokens) = (tokens).try_into() else {
			return;
		};
		self
			.request
			.descriptors
			.iter_mut()
			.for_each(|d| d.hits_addend = Some(tokens));
		// Ignore the response
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
		cost: Option<u64>,
	) -> Option<RateLimitRequest> {
		let mut descriptors = Vec::with_capacity(self.descriptors.0.len());
		let candidate_count = self
			.descriptors
			.0
			.iter()
			.filter(|e| e.limit_type == limit_type)
			.count();
		trace!(
			"ratelimit build_request start: domain={}, type={:?}, cost={:?}, candidates={}",
			self.domain, limit_type, cost, candidate_count
		);

		for desc_entry in self
			.descriptors
			.0
			.iter()
			.filter(|e| e.limit_type == limit_type)
		{
			if let Some(rl_entries) = Self::eval_descriptor(req, &desc_entry.entries) {
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

				// evaluate limit override if configured
				let limit = desc_entry
					.limit_override
					.as_ref()
					.and_then(|lo| Self::eval_limit_override(req, lo));

				descriptors.push(RateLimitDescriptor {
					entries: rl_entries,
					limit,
					hits_addend: cost,
				});
			} else {
				let attempted: Vec<String> = desc_entry
					.entries
					.iter()
					.map(|d| format!("{}={:?}", d.0, d.1))
					.collect();
				trace!(
					"ratelimit descriptor evaluation failed for domain={}, type={:?}, skipping descriptor: {}",
					self.domain,
					limit_type,
					attempted.join(", ")
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

		Some(proto::RateLimitRequest {
			domain: self.domain.clone(),
			descriptors,
			// Ignored; we always set the per-descriptor one which allows distinguishing empty vs 0
			hits_addend: 0,
		})
	}
	pub async fn check_llm(
		&self,
		client: PolicyClient,
		req: &mut Request,
		cost: u64,
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
		let Some(request) = self.build_request(req, RateLimitType::Tokens, Some(cost)) else {
			return Ok((PolicyResponse::default(), None));
		};
		let cr = self.check_internal(client.clone(), request.clone()).await;
		let r = LLMResponseAmend {
			base: self.clone(),
			client,
			request,
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
		let Some(request) = self.build_request(req, RateLimitType::Requests, None) else {
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
		let descriptor_summaries: Vec<String> = request
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
			.collect();
		trace!(
			"ratelimit request summary (domain: {}): descriptors={} {}",
			request.domain,
			request.descriptors.len(),
			descriptor_summaries.join(" | ")
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

	fn eval_descriptor(req: &Request, entries: &Vec<Descriptor>) -> Option<Vec<Entry>> {
		let mut rl_entries = Vec::with_capacity(entries.len());
		let exec = cel::Executor::new_request(req);
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

	/// Evaluate a limit override expression against the request context.
	/// returns `Some(RateLimitOverride)` if the expression evaluates successfully
	/// and contains valid `requests_per_unit` and `unit` fields.
	/// returns `None` if evaluation fails, metadata is missing, or fields are invalid.
	fn eval_limit_override(
		req: &Request,
		limit_override: &LimitOverride,
	) -> Option<proto::RateLimitOverride> {
		let exec = cel::Executor::new_request(req);
		match exec.eval(&limit_override.expression) {
			Ok(value) => {
				// the CEL result should be a map with requests_per_unit and unit
				let map = match value.json() {
					Ok(serde_json::Value::Object(m)) => m,
					Ok(other) => {
						trace!("ratelimit limit_override: expected object, got {:?}", other);
						return None;
					},
					Err(e) => {
						trace!("ratelimit limit_override: failed to convert to JSON: {}", e);
						return None;
					},
				};

				// extract requests_per_unit
				let requests_per_unit = match map.get("requests_per_unit") {
					Some(serde_json::Value::Number(n)) => n.as_u64().and_then(|v| v.try_into().ok()),
					_ => {
						trace!("ratelimit limit_override: missing or invalid requests_per_unit field");
						return None;
					},
				}?;

				// extract unit and convert to RateLimitUnit enum
				let unit = match map.get("unit") {
					Some(serde_json::Value::String(s)) => Self::parse_rate_limit_unit(s),
					Some(serde_json::Value::Number(n)) => {
						// Allow numeric unit values too
						n.as_i64().and_then(|v| v.try_into().ok())
					},
					_ => {
						trace!("ratelimit limit_override: missing or invalid unit field");
						return None;
					},
				}?;

				trace!(
					"ratelimit limit_override: evaluated to requests_per_unit={}, unit={}",
					requests_per_unit, unit
				);

				Some(proto::RateLimitOverride {
					requests_per_unit,
					unit,
				})
			},
			Err(e) => {
				trace!(
					"ratelimit limit_override: failed to evaluate expression {:?}: {}",
					limit_override.expression, e
				);
				None
			},
		}
	}

	/// parse a unit string to its corresponding RateLimitUnit enum value.
	fn parse_rate_limit_unit(s: &str) -> Option<i32> {
		match s.to_uppercase().as_str() {
			"SECOND" => Some(proto::RateLimitUnit::Second as i32),
			"MINUTE" => Some(proto::RateLimitUnit::Minute as i32),
			"HOUR" => Some(proto::RateLimitUnit::Hour as i32),
			"DAY" => Some(proto::RateLimitUnit::Day as i32),
			"MONTH" => Some(proto::RateLimitUnit::Month as i32),
			"YEAR" => Some(proto::RateLimitUnit::Year as i32),
			_ => {
				trace!("ratelimit limit_override: unknown unit string: {}", s);
				None
			},
		}
	}

	pub fn expressions(&self) -> impl Iterator<Item = &Expression> {
		self.descriptors.0.iter().flat_map(|v| {
			let entry_exprs = v.entries.iter().map(|e| &e.1);
			let override_expr = v.limit_override.iter().map(|lo| lo.expression.as_ref());
			entry_exprs.chain(override_expr)
		})
	}
}

fn process_headers(hm: &mut HeaderMap, headers: Vec<proto::HeaderValue>) {
	for h in headers {
		let _ = envoy_proto_common::apply_header_value(hm, &h);
	}
}
