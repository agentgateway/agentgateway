use ::http::HeaderMap;

use crate::cel;
use crate::cel::Expression;
use crate::http::ext_proc::GrpcReferenceChannel;
use crate::http::localratelimit::RateLimitType;
use crate::http::proto_header::process_proto_headers;
use crate::http::remoteratelimit::proto::RateLimitDescriptor;
use crate::http::remoteratelimit::proto::rate_limit_descriptor::Entry;
use crate::http::remoteratelimit::proto::rate_limit_service_client::RateLimitServiceClient;
use crate::http::remoteratelimit::{Descriptor, FailureMode, RemoteRateLimit, proto};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::*;

#[cfg(test)]
#[path = "remoteratelimit_tests.rs"]
mod tests;

#[derive(Debug, Default, thiserror::Error)]
#[error("mcp rate limit exceeded")]
pub struct McpRateLimitError {
	pub response_headers: HeaderMap,
	/// When true, the rate limit service itself was unreachable (failClosed).
	/// This should map to 500, not 429.
	pub service_error: bool,
}

#[apply(schema!)]
#[serde(transparent)]
pub struct McpRemoteRateLimit(pub(crate) RemoteRateLimit);

impl McpRemoteRateLimit {
	pub fn domain(&self) -> &str {
		&self.0.domain
	}

	pub fn expressions(&self) -> impl Iterator<Item = &Expression> {
		self.0.expressions()
	}

	/// Check rate limiting using a request snapshot (for MCP backend-level rate limiting).
	/// Returns Ok with response headers to add on success, or Err with response headers
	/// if rate limited or the check failed.
	///
	/// When the rate limit service is unreachable, `failure_mode` controls behavior:
	/// - `FailOpen`: the request is allowed through (returns Ok).
	/// - `FailClosed`: the error propagates as `ProxyError::RateLimitFailed` (yields 500).
	pub async fn check(
		&self,
		client: PolicyClient,
		snapshot: &cel::RequestSnapshot,
		mcp: Option<&crate::mcp::ResourceType>,
	) -> Result<HeaderMap, McpRateLimitError> {
		if !self
			.0
			.descriptors
			.0
			.iter()
			.any(|d| d.limit_type == RateLimitType::Requests)
		{
			return Ok(HeaderMap::new());
		}
		let Some(request) =
			self.build_request_from_snapshot(snapshot, mcp, RateLimitType::Requests, None)
		else {
			return Ok(HeaderMap::new());
		};
		let cr = match self.check_internal(client, request).await {
			Ok(cr) => cr,
			Err(_) => return self.handle_service_error(),
		};
		if cr.overall_code != (proto::rate_limit_response::Code::Ok as i32) {
			warn!(
				target: "mcp::remoteratelimit",
				"mcp rate limit rejected: domain={}, code={}",
				self.0.domain, cr.overall_code
			);
			let mut response_headers = HeaderMap::new();
			process_proto_headers(&mut response_headers, cr.response_headers_to_add);
			return Err(McpRateLimitError {
				response_headers,
				service_error: false,
			});
		}
		let mut response_headers = HeaderMap::new();
		process_proto_headers(&mut response_headers, cr.response_headers_to_add);
		Ok(response_headers)
	}

	fn handle_service_error(&self) -> Result<HeaderMap, McpRateLimitError> {
		if self.0.failure_mode == FailureMode::FailOpen {
			Ok(HeaderMap::new())
		} else {
			Err(McpRateLimitError {
				response_headers: HeaderMap::new(),
				service_error: true,
			})
		}
	}

	fn build_request_from_snapshot(
		&self,
		snapshot: &cel::RequestSnapshot,
		mcp: Option<&crate::mcp::ResourceType>,
		limit_type: RateLimitType,
		cost: Option<u64>,
	) -> Option<proto::RateLimitRequest> {
		self.build_request_with(limit_type, cost, |entries| {
			let exec = match mcp {
				Some(mcp) => cel::Executor::new_mcp(Some(snapshot), mcp),
				None => cel::Executor::new_snapshot(snapshot),
			};
			Self::eval_descriptor_with_executor(&exec, entries)
		})
	}

	fn build_request_with(
		&self,
		limit_type: RateLimitType,
		cost: Option<u64>,
		eval_fn: impl Fn(&Arc<Vec<Descriptor>>) -> Option<Vec<Entry>>,
	) -> Option<proto::RateLimitRequest> {
		let rl = &self.0;
		let mut descriptors = Vec::with_capacity(rl.descriptors.0.len());
		let candidate_count = rl
			.descriptors
			.0
			.iter()
			.filter(|e| e.limit_type == limit_type)
			.count();
		trace!(
			"ratelimit build_request start: domain={}, type={:?}, cost={:?}, candidates={}",
			rl.domain, limit_type, cost, candidate_count
		);

		for desc_entry in rl
			.descriptors
			.0
			.iter()
			.filter(|e| e.limit_type == limit_type)
		{
			if let Some(rl_entries) = eval_fn(&desc_entry.entries) {
				if rl_entries.is_empty() {
					trace!(
						"ratelimit skipping descriptor with no entries for domain={}, type={:?}",
						rl.domain, limit_type,
					);
					continue;
				}
				let kv_pairs: Vec<String> = rl_entries
					.iter()
					.map(|e| format!("{}={}", e.key, e.value))
					.collect();
				trace!(
					"ratelimit evaluated descriptors (domain: {}, type: {:?}): {}",
					rl.domain,
					limit_type,
					kv_pairs.join(", ")
				);
				descriptors.push(RateLimitDescriptor {
					entries: rl_entries,
					limit: None,
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
					rl.domain,
					limit_type,
					attempted.join(", ")
				);
			}
		}

		if descriptors.is_empty() {
			trace!(
				"ratelimit all descriptors failed evaluation for domain={}, type={:?}, skipping rate-limit call",
				rl.domain, limit_type,
			);
			return None;
		}

		trace!(
			"ratelimit built request descriptors (domain: {}, type: {:?}): count={}",
			rl.domain,
			limit_type,
			descriptors.len()
		);

		Some(proto::RateLimitRequest {
			domain: rl.domain.clone(),
			descriptors,
			hits_addend: 0,
		})
	}

	fn eval_descriptor_with_executor(
		exec: &cel::Executor,
		entries: &Vec<Descriptor>,
	) -> Option<Vec<Entry>> {
		let mut rl_entries = Vec::with_capacity(entries.len());
		for Descriptor(k, lookup) in entries {
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

	async fn check_internal(
		&self,
		client: PolicyClient,
		request: proto::RateLimitRequest,
	) -> Result<proto::RateLimitResponse, ProxyError> {
		let rl = &self.0;
		trace!("connecting to {:?}", rl.target);
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
			target: rl.target.clone(),
			policies: rl.policies.clone(),
			client,
		};
		let mut client = RateLimitServiceClient::new(chan);
		let resp = client.should_rate_limit(request).await;
		trace!("check response: {:?}", resp);
		if let Err(ref error) = resp {
			let ignore = rl.failure_mode == FailureMode::FailOpen;
			warn!(
				"ratelimit service failed (domain: {}): {:?}; {}",
				rl.domain,
				error,
				if ignore {
					"failure will be ignored (failure_mode: failOpen)"
				} else {
					"denying request (failure_mode: failClosed)"
				}
			);
		}
		let cr = resp.map_err(|_| ProxyError::RateLimitFailed)?;
		Ok(cr.into_inner())
	}
}
