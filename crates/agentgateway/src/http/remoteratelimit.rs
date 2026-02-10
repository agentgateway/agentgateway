use ::http::{HeaderMap, StatusCode};

use crate::cel::Expression;
use crate::http::ext_proc::GrpcReferenceChannel;
use crate::http::localratelimit::RateLimitType;
use crate::http::remoteratelimit::proto::rate_limit_descriptor::Entry;
use crate::http::remoteratelimit::proto::rate_limit_service_client::RateLimitServiceClient;
use crate::http::remoteratelimit::proto::{RateLimitDescriptor, RateLimitRequest};
use crate::http::{HeaderName, HeaderValue, PolicyResponse, Request};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::types::agent::SimpleBackendReference;
use crate::*;

#[cfg(test)]
#[path = "remoteratelimit_tests.rs"]
mod tests;

#[allow(warnings)]
#[allow(clippy::derive_partial_eq_without_eq)]
pub mod proto {
	tonic::include_proto!("envoy.service.ratelimit.v3");
}

/// Defines how the proxy behaves when the remote rate limit service is
/// unavailable or returns an error.
///
/// Defaults to `FailOpen`, matching Envoy's default behavior
/// (`failure_mode_deny=false`). When failing open, requests are allowed
/// through despite the service failure. When failing closed, a 500
/// Internal Server Error is returned.
///
/// # Configuration
///
/// Both camelCase (`failOpen`, `failClosed`) and PascalCase (`FailOpen`,
/// `FailClosed`) are accepted in configuration files for compatibility,
/// though camelCase is the preferred format.
#[apply(schema!)]
#[derive(Default, Copy, PartialEq, Eq)]
pub enum FailureMode {
	/// Allow the request through when the rate limit service is unavailable (default).
	#[default]
	#[serde(rename = "failOpen", alias = "FailOpen")]
	FailOpen,
	/// Deny the request with a 500 status when the rate limit service is unavailable.
	#[serde(rename = "failClosed", alias = "FailClosed")]
	FailClosed,
}

#[apply(schema!)]
pub struct RemoteRateLimit {
	pub domain: String,
	#[serde(flatten)]
	pub target: Arc<SimpleBackendReference>,
	pub descriptors: Arc<DescriptorSet>,
	/// Timeout for the request
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		with = "serde_dur_option"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub timeout: Option<Duration>,
	/// Behavior when the remote rate limit service is unavailable or returns an error.
	/// Defaults to failOpen, allowing requests through on service failure.
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
					warn!(
						"ratelimit service failed (domain: {}, failure_mode: failOpen): {:?}; allowing request",
						self.domain, e
					);
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
					warn!(
						"ratelimit service failed (domain: {}, failure_mode: failOpen): {:?}; allowing request",
						self.domain, e
					);
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
			client,
			timeout: self.timeout,
		};
		let mut client = RateLimitServiceClient::new(chan);
		let resp = client.should_rate_limit(request).await;
		trace!("check response: {:?}", resp);
		if let Err(ref error) = resp {
			warn!("rate limit request failed: {:?}", error);
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

	pub fn expressions(&self) -> impl Iterator<Item = &Expression> {
		self
			.descriptors
			.0
			.iter()
			.flat_map(|v| v.entries.iter().map(|v| &v.1))
	}
}

fn process_headers(hm: &mut HeaderMap, headers: Vec<proto::HeaderValue>) {
	for h in headers {
		let Ok(hn) = HeaderName::from_bytes(h.key.as_bytes()) else {
			continue;
		};
		let hv = if !h.value.is_empty() {
			HeaderValue::from_bytes(h.value.as_bytes())
		} else if !h.raw_value.is_empty() {
			HeaderValue::from_bytes(&h.raw_value)
		} else {
			continue;
		};
		let Ok(hv) = hv else {
			continue;
		};
		hm.insert(hn, hv);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::http::tests_common::request_for_uri;

	#[test]
	fn failure_mode_defaults_to_fail_open() {
		let mode = FailureMode::default();
		assert_eq!(mode, FailureMode::FailOpen);
	}

	#[test]
	fn failure_mode_serde_roundtrip() {
		// Test failOpen
		let json = serde_json::to_string(&FailureMode::FailOpen).unwrap();
		assert_eq!(json, r#""failOpen""#);
		let deserialized: FailureMode = serde_json::from_str(&json).unwrap();
		assert_eq!(deserialized, FailureMode::FailOpen);

		// Test failClosed
		let json = serde_json::to_string(&FailureMode::FailClosed).unwrap();
		assert_eq!(json, r#""failClosed""#);
		let deserialized: FailureMode = serde_json::from_str(&json).unwrap();
		assert_eq!(deserialized, FailureMode::FailClosed);
	}

	#[test]
	fn failure_mode_accepts_pascal_case_alias() {
		// Test FailOpen (PascalCase alias for compatibility)
		let deserialized: FailureMode = serde_json::from_str(r#""FailOpen""#).unwrap();
		assert_eq!(deserialized, FailureMode::FailOpen);

		// Test FailClosed (PascalCase alias for compatibility)
		let deserialized: FailureMode = serde_json::from_str(r#""FailClosed""#).unwrap();
		assert_eq!(deserialized, FailureMode::FailClosed);

		// Serialization still uses camelCase (not the alias)
		let json = serde_json::to_string(&FailureMode::FailOpen).unwrap();
		assert_eq!(json, r#""failOpen""#);
	}

	#[test]
	fn apply_ok_response_passes_through() {
		let mut req = request_for_uri("http://example.com/test");
		let response = proto::RateLimitResponse {
			overall_code: proto::rate_limit_response::Code::Ok as i32,
			statuses: vec![],
			response_headers_to_add: vec![],
			request_headers_to_add: vec![proto::HeaderValue {
				key: "x-ratelimit-remaining".to_string(),
				value: "99".to_string(),
				raw_value: vec![],
			}],
			raw_body: vec![],
			dynamic_metadata: None,
			quota: None,
		};
		let result = RemoteRateLimit::apply(&mut req, response).unwrap();
		// Should not have a direct response (request is allowed)
		assert!(result.direct_response.is_none());
		// Request header should have been added
		assert_eq!(req.headers().get("x-ratelimit-remaining").unwrap(), "99");
	}

	#[test]
	fn apply_over_limit_response_returns_429() {
		let mut req = request_for_uri("http://example.com/test");
		let response = proto::RateLimitResponse {
			overall_code: proto::rate_limit_response::Code::OverLimit as i32,
			statuses: vec![],
			response_headers_to_add: vec![proto::HeaderValue {
				key: "retry-after".to_string(),
				value: "60".to_string(),
				raw_value: vec![],
			}],
			request_headers_to_add: vec![],
			raw_body: b"rate limit exceeded".to_vec(),
			dynamic_metadata: None,
			quota: None,
		};
		let result = RemoteRateLimit::apply(&mut req, response).unwrap();
		// Should have a direct response with 429
		let direct = result.direct_response.unwrap();
		assert_eq!(direct.status(), StatusCode::TOO_MANY_REQUESTS);
		assert_eq!(direct.headers().get("retry-after").unwrap(), "60");
	}

	#[test]
	fn rate_limit_failed_maps_to_500() {
		let err = ProxyError::RateLimitFailed;
		let response = err.into_response();
		assert_eq!(
			response.status(),
			StatusCode::INTERNAL_SERVER_ERROR,
			"RateLimitFailed should map to 500, not 429"
		);
	}

	#[test]
	fn rate_limit_exceeded_maps_to_429() {
		let err = ProxyError::RateLimitExceeded {
			limit: 10,
			remaining: 0,
			reset_seconds: 60,
		};
		let response = err.into_response();
		assert_eq!(
			response.status(),
			StatusCode::TOO_MANY_REQUESTS,
			"RateLimitExceeded should map to 429"
		);
	}

	#[test]
	fn config_with_failure_mode_deserializes() {
		let yaml = r#"
domain: "test"
host: "127.0.0.1:8081"
failureMode: failOpen
descriptors:
  - entries:
      - key: "user"
        value: '"test-user"'
    type: "requests"
"#;
		let rrl: RemoteRateLimit = serde_yaml::from_str(yaml).unwrap();
		assert_eq!(rrl.failure_mode, FailureMode::FailOpen);
		assert_eq!(rrl.domain, "test");
	}

	#[test]
	fn config_with_fail_closed_deserializes() {
		let yaml = r#"
domain: "test"
host: "127.0.0.1:8081"
failureMode: failClosed
descriptors:
  - entries:
      - key: "user"
        value: '"test-user"'
    type: "requests"
"#;
		let rrl: RemoteRateLimit = serde_yaml::from_str(yaml).unwrap();
		assert_eq!(rrl.failure_mode, FailureMode::FailClosed);
	}

	#[test]
	fn config_with_pascal_case_aliases_deserializes() {
		// Test FailOpen (PascalCase alias)
		let yaml = r#"
domain: "test"
host: "127.0.0.1:8081"
failureMode: FailOpen
descriptors:
  - entries:
      - key: "user"
        value: '"test-user"'
    type: "requests"
"#;
		let rrl: RemoteRateLimit = serde_yaml::from_str(yaml).unwrap();
		assert_eq!(rrl.failure_mode, FailureMode::FailOpen);

		// Test FailClosed (PascalCase alias)
		let yaml = r#"
domain: "test"
host: "127.0.0.1:8081"
failureMode: FailClosed
descriptors:
  - entries:
      - key: "user"
        value: '"test-user"'
    type: "requests"
"#;
		let rrl: RemoteRateLimit = serde_yaml::from_str(yaml).unwrap();
		assert_eq!(rrl.failure_mode, FailureMode::FailClosed);
	}

	#[test]
	fn config_without_failure_mode_defaults_to_fail_open() {
		let yaml = r#"
domain: "test"
host: "127.0.0.1:8081"
descriptors:
  - entries:
      - key: "user"
        value: '"test-user"'
    type: "requests"
"#;
		let rrl: RemoteRateLimit = serde_yaml::from_str(yaml).unwrap();
		assert_eq!(
			rrl.failure_mode,
			FailureMode::FailOpen,
			"Missing failureMode should default to failOpen"
		);
	}
}
