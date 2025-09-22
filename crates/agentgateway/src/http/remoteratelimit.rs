use ::http::{HeaderMap, StatusCode};

use crate::cel::{Executor, Expression};
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

#[allow(warnings)]
#[allow(clippy::derive_partial_eq_without_eq)]
pub mod proto {
	tonic::include_proto!("envoy.service.ratelimit.v3");
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteRateLimit {
	pub domain: String,
	pub target: Arc<SimpleBackendReference>,
	pub descriptors: Arc<DescriptorSet>,
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
		.map(|i| cel::Expression::new(i.value).map(|v| Descriptor(i.key, v)))
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
	fn build_request(
		&self,
		exec: &Executor<'_>,
		limit_type: RateLimitType,
		cost: Option<u64>,
	) -> RateLimitRequest {
		let mut descriptors = Vec::with_capacity(self.descriptors.0.len());
		for entries in self
			.descriptors
			.0
			.iter()
			.filter(|e| e.limit_type == limit_type)
		{
			if let Some(rl_entries) = Self::eval_descriptor(exec, &entries.entries) {
				descriptors.push(RateLimitDescriptor {
					entries: rl_entries,
					limit: None,
					hits_addend: cost,
				});
			}
		}

		proto::RateLimitRequest {
			domain: self.domain.clone(),
			descriptors,
			// Ignored; we always set the per-descriptor one which allows distinguishing empty vs 0
			hits_addend: 0,
		}
	}
	pub async fn check_llm(
		&self,
		client: PolicyClient,
		req: &mut Request,
		exec: &Executor<'_>,
		cost: u64,
	) -> Result<(PolicyResponse, Option<LLMResponseAmend>), ProxyError> {
		if !self
			.descriptors
			.0
			.iter()
			.any(|d| d.limit_type == RateLimitType::Tokens)
		{
			// Nothing to do
			return Ok((PolicyResponse::default(), None));
		}
		let request = self.build_request(exec, RateLimitType::Tokens, Some(cost));
		let cr = self.check_internal(client.clone(), request.clone()).await;
		let r = LLMResponseAmend {
			base: self.clone(),
			client,
			request,
		};

		cr.and_then(|pr| (Self::apply(req, pr).map(|x| (x, Some(r)))))
	}

	pub async fn check(
		&self,
		client: PolicyClient,
		req: &mut Request,
		exec: &Executor<'_>,
	) -> Result<PolicyResponse, ProxyError> {
		// This is on the request path
		if !self
			.descriptors
			.0
			.iter()
			.any(|d| d.limit_type == RateLimitType::Requests)
		{
			// Nothing to do
			return Ok(PolicyResponse::default());
		}
		let request = self.build_request(exec, RateLimitType::Requests, None);
		let cr = self.check_internal(client, request).await?;
		Self::apply(req, cr)
	}

	async fn check_internal(
		&self,
		client: PolicyClient,
		request: proto::RateLimitRequest,
	) -> Result<proto::RateLimitResponse, ProxyError> {
		trace!("connecting to {:?}", self.target);
		let chan = GrpcReferenceChannel {
			target: self.target.clone(),
			client,
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

	fn eval_descriptor(exec: &Executor, entries: &Vec<Descriptor>) -> Option<Vec<Entry>> {
		let mut rl_entries = Vec::with_capacity(entries.len());
		for Descriptor(k, lookup) in entries {
			// We drop the entire set if we cannot eval one
			let value = exec.eval(lookup).ok()?;
			let cel::Value::String(value) = value else {
				return None;
			};
			let entry = Entry {
				key: k.clone(),
				value: value.to_string(),
			};
			rl_entries.push(entry);
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
		let hv = if h.raw_value.is_empty() {
			HeaderValue::from_bytes(h.key.as_bytes())
		} else {
			HeaderValue::from_bytes(&h.raw_value)
		};
		let Ok(hv) = hv else {
			continue;
		};
		hm.insert(hn, hv);
	}
}
