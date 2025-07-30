use std::collections::HashMap;
use std::convert::Infallible;
use std::time::SystemTime;

use ::http::uri::Authority;
use ::http::{HeaderMap, StatusCode, Uri};
use anyhow::anyhow;
use bytes::Bytes;
use http_body::Frame;
use http_body_util::BodyStream;
use itertools::Itertools;
use prost_types::Timestamp;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

use crate::client::{Client, Transport};
use crate::control::AuthSource;
use crate::http::backendtls::BackendTLS;
use crate::http::ext_proc::GrpcReferenceChannel;
use crate::http::filters::DirectResponse;
use crate::http::remoteratelimit::proto::RateLimitDescriptor;
use crate::http::remoteratelimit::proto::rate_limit_descriptor::Entry;
use crate::http::remoteratelimit::proto::rate_limit_service_client::RateLimitServiceClient;
use crate::http::{HeaderName, HeaderValue, PolicyResponse, Request, Response};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::transport::stream::{TCPConnectionInfo, TLSConnectionInfo};
use crate::types::agent;
use crate::types::agent::{Backend, SimpleBackendReference, Target};
use crate::*;

#[allow(warnings)]
#[allow(clippy::derive_partial_eq_without_eq)]
pub mod proto {
	tonic::include_proto!("envoy.service.ratelimit.v3");
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteRateLimit {
	pub target: Arc<SimpleBackendReference>,
	pub descriptors: HashMap<String, Descriptor>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub enum Descriptor {
	#[serde(serialize_with = "ser_display", deserialize_with = "de_parse")]
	RequestHeader(#[cfg_attr(feature = "schema", schemars(with = "String"))] HeaderName),
	Static(Strng),
}

impl RemoteRateLimit {
	pub async fn check(
		&self,
		client: PolicyClient,
		req: &mut Request,
	) -> Result<PolicyResponse, ProxyError> {
		let mut entries = Vec::with_capacity(self.descriptors.len());
		for (k, lookup) in &self.descriptors {
			let value = match lookup {
				Descriptor::RequestHeader(k) => {
					let Some(hv) = req.headers().get(k) else {
						// If not found, its not a match
						return Ok(Default::default());
					};
					hv.to_str()
						.map_err(|_| ProxyError::InvalidRequest)?
						.to_string()
				},
				Descriptor::Static(v) => v.to_string(),
			};
			let entry = Entry {
				key: k.clone(),
				value,
			};
			entries.push(entry);
		}
		let request = proto::RateLimitRequest {
			domain: "crd".to_string(),
			descriptors: vec![RateLimitDescriptor {
				// TODO: do we ever need multiple
				entries,
				limit: None,
				hits_addend: None,
			}],
			hits_addend: 0,
		};

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
