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
use crate::http::ext_authz::proto::attribute_context::HttpRequest;
use crate::http::ext_authz::proto::authorization_client::AuthorizationClient;
use crate::http::ext_authz::proto::check_response::HttpResponse;
use crate::http::ext_authz::proto::{
	AttributeContext, CheckRequest, DeniedHttpResponse, HeaderValueOption, OkHttpResponse,
};
use crate::http::ext_proc::GrpcReferenceChannel;
use crate::http::ext_proc::proto::{
	BodyMutation, BodyResponse, HeadersResponse, HttpBody, HttpHeaders, HttpTrailers,
	ProcessingRequest, ProcessingResponse,
};
use crate::http::filters::DirectResponse;
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
	tonic::include_proto!("envoy.service.auth.v3");
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtAuthz {
	pub target: Arc<SimpleBackendReference>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub context: Option<HashMap<String, String>>, // TODO: gRPC vs HTTP, fail open, include body,
}

impl ExtAuthz {
	pub async fn check(
		&self,
		client: PolicyClient,
		req: &mut Request,
	) -> Result<PolicyResponse, ProxyError> {
		trace!("connecting to {:?}", self.target);
		let chan = GrpcReferenceChannel {
			target: self.target.clone(),
			client,
		};
		let mut client = AuthorizationClient::new(chan);
		let tcp_info = req.extensions().get::<TCPConnectionInfo>().unwrap();
		let tls_info = req.extensions().get::<TLSConnectionInfo>();

		// Convert headers to the format expected by Envoy
		let mut headers = std::collections::HashMap::new();
		for (name, value) in req.headers() {
			headers.insert(name.to_string(), value.to_str().unwrap_or("").to_string());
		}

		// Get request body if available - for now we'll use an empty string
		// since reading the body would consume it and we might need it later
		let body = "".to_string();

		let request = crate::http::ext_authz::proto::attribute_context::Request {
			time: Some(Timestamp::from(
				SystemTime::now() - tcp_info.start.elapsed(),
			)),
			http: Some(HttpRequest {
				// TODO: maybe get the trace span...?
				id: "".to_string(),
				// id: format!("req-{}", SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos()),
				method: req.method().to_string(),
				headers,
				path: req.uri().path().to_string(),
				host: req.uri().host().unwrap_or("").to_string(),
				scheme: req
					.uri()
					.scheme()
					.map(|s| s.to_string())
					.unwrap_or_else(|| "http".to_string()),
				protocol: format!("{:?}", req.version())
					.replace("2.0", "2")
					.replace("3.0", "3"),
				// Always empty per spec
				query: "".to_string(),
				// Always empty per spec
				fragment: "".to_string(),
				// TODO
				size: body.len() as i64,
				body,
				raw_body: vec![], // Raw body bytes if needed
			}),
		};

		// Build source and destination information - simplified for now
		let source = None; // TODO: Implement proper source address mapping
		let destination = None; // TODO: Implement proper destination address mapping

		// Build TLS session info if available
		let tls_session =
			tls_info.map(
				|_tls_info| crate::http::ext_authz::proto::attribute_context::TlsSession {
					sni: "".to_string(), // Could be extracted from TLS info if available
				},
			);

		let authz_req = CheckRequest {
			attributes: Some(AttributeContext {
				source,
				destination,
				request: Some(request),
				context_extensions: self.context.clone().unwrap_or_default(),
				tls_session,
			}),
		};
		let resp = client.check(authz_req).await;
		trace!("check response: {:?}", resp);
		let cr = resp.map_err(|_| ProxyError::AuthorizationFailed)?;
		let cr = cr.into_inner();
		let status = cr.status.map(|status| status.code).unwrap();
		if status != 0 {
			debug!("status denied: {status}");
			return Err(ProxyError::AuthorizationFailed);
		}
		let mut res = PolicyResponse::default();
		let Some(resp) = cr.http_response else {
			return Ok(res);
		};

		match resp {
			HttpResponse::DeniedResponse(DeniedHttpResponse {
				status,
				headers,
				body,
			}) => {
				let status = status
					.and_then(|s| StatusCode::from_u16(s.code as u16).ok())
					.unwrap_or(StatusCode::FORBIDDEN);
				let mut rb = ::http::response::Builder::new().status(status);
				if let Some(hm) = rb.headers_mut() {
					process_headers(hm, headers)
				}
				let resp = rb
					.body(http::Body::from(body))
					.map_err(|e| ProxyError::Processing(e.into()))?;
				res.direct_response = Some(resp);
			},
			HttpResponse::OkResponse(OkHttpResponse {
				headers,
				headers_to_remove,
				response_headers_to_add,
				query_parameters_to_set,
				query_parameters_to_remove,
				..
			}) => {
				// Handle headers to remove
				for header_name in headers_to_remove {
					req.headers_mut().remove(header_name);
				}

				process_headers(req.headers_mut(), headers);
				for param in query_parameters_to_set {
					// TODO
				}
				for param_name in query_parameters_to_remove {
					// TODO
				}
				if !response_headers_to_add.is_empty() {
					let mut hm = HeaderMap::new();
					process_headers(&mut hm, response_headers_to_add);
					res.response_headers = Some(hm);
				}
			},
		}
		Ok(res)
	}
}

fn process_headers(hm: &mut HeaderMap, headers: Vec<HeaderValueOption>) {
	for header in headers {
		let Some(h) = header.header else { continue };
		let append = header.append.unwrap_or_default();
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
		if append {
			hm.append(hn, hv);
		} else {
			hm.insert(hn, hv);
		}
	}
}
