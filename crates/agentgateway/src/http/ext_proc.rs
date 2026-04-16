use std::convert::Infallible;

use ::http::HeaderMap;
use anyhow::anyhow;
use bytes::Bytes;
use http_body::{Body, Frame};
use http_body_util::BodyStream;
use itertools::Itertools;
use prost_wkt_types::Struct;
use proto::body_mutation::Mutation;
use proto::processing_request::Request;
use proto::processing_response::Response;
use serde_json::Value as JsonValue;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

use crate::cel::{Executor, Expression, RequestSnapshot};
use crate::client::ResolvedDestination;
use crate::http;
use crate::http::envoy_proto_common;
use crate::http::ext_proc::proto::{
	BodyMutation, BodyResponse, HeaderMutation, HeaderValueOption, HeadersResponse, HttpBody,
	HttpHeaders, HttpTrailers, ImmediateResponse, Metadata, ProcessingRequest, ProcessingResponse,
	processing_response,
};
use crate::http::{HeaderName, HeaderOrPseudo, PolicyResponse};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::types::agent::{BackendPolicy, SimpleBackendReference};
use crate::*;

/// The namespace key used for ext_proc attributes in ProcessingRequest.attributes
const EXTPROC_ATTRIBUTES_NAMESPACE: &str = "envoy.filters.http.ext_proc";

#[cfg(test)]
#[path = "ext_proc_tests.rs"]
mod tests;

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("failed to send request")]
	RequestSend,
	#[error("no more response messages")]
	NoMoreResponses,
	#[error("no more responses")]
	ResponseDropped,
	#[error("failed to buffer body: {0}")]
	BodyBuffer(String),
	#[error("failed to convert metadata value: {0}")]
	MetadataConversion(String),
	#[error(transparent)]
	InvalidHeaderName(#[from] http::header::InvalidHeaderName),
	#[error(transparent)]
	InvalidHeaderValue(#[from] http::header::InvalidHeaderValue),
}

#[apply(schema!)]
#[derive(Default, ::cel::DynamicType)]
pub struct ExtProcDynamicMetadata(serde_json::Map<String, JsonValue>);

#[allow(warnings)]
#[allow(clippy::derive_partial_eq_without_eq)]
pub mod proto {
	pub use protos::envoy::service::common::v3::{
		HeaderValue, HeaderValueOption, HttpStatus, Metadata, StatusCode, header_value_option,
	};
	pub use protos::envoy::service::ext_proc::v3::*;
}

#[apply(schema!)]
#[derive(Default, Copy, PartialEq, Eq)]
pub enum FailureMode {
	#[default]
	FailClosed,
	FailOpen,
}

#[apply(schema_ser!)]
pub struct InferenceRouting {
	pub target: Arc<SimpleBackendReference>,
	pub failure_mode: FailureMode,
}

#[derive(Debug, Default)]
pub struct InferencePoolRouter {
	ext_proc: Option<ExtProcInstance>,
}

#[derive(Debug, Default)]
pub struct InferenceRequestResult {
	pub destination: Option<SocketAddr>,
	pub policy_response: PolicyResponse,
	pub failed_open: bool,
}

impl InferenceRouting {
	pub fn build(&self, client: PolicyClient) -> InferencePoolRouter {
		InferencePoolRouter {
			ext_proc: Some(ExtProcInstance::new(
				client,
				Vec::new(),
				self.target.clone(),
				self.failure_mode,
				None,
				None,
				None,
			)),
		}
	}
}

impl InferencePoolRouter {
	pub async fn mutate_request(
		&mut self,
		req: &mut http::Request,
	) -> Result<InferenceRequestResult, ProxyError> {
		let Some(ext_proc) = &mut self.ext_proc else {
			return Ok(Default::default());
		};
		let r = std::mem::take(req);
		let (new_req, pr) = ext_proc.mutate_request(r).await?;
		let failed_open = ext_proc.did_fail_open();
		*req = new_req;
		let dest = req
			.headers()
			.get(HeaderName::from_static("x-gateway-destination-endpoint"))
			.and_then(|v| v.to_str().ok())
			.map(|v| v.parse::<SocketAddr>())
			.transpose()
			.map_err(|e| ProxyError::Processing(anyhow!("EPP returned invalid address: {e}")))?;
		Ok(InferenceRequestResult {
			destination: dest,
			policy_response: pr.unwrap_or_default(),
			failed_open,
		})
	}

	pub async fn mutate_response(
		&mut self,
		resp: &mut http::Response,
	) -> Result<PolicyResponse, ProxyError> {
		let rd = resp.extensions().get::<ResolvedDestination>().map(|d| d.0);
		let Some(ext_proc) = &mut self.ext_proc else {
			return Ok(Default::default());
		};
		let r = std::mem::take(resp);
		let (new_resp, pr) = ext_proc.mutate_response(r, None, rd).await?;
		*resp = new_resp;
		Ok(pr.unwrap_or_default())
	}
}

#[apply(schema!)]
pub struct ExtProc {
	/// Reference to the external processing service backend
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
	/// Behavior when the ext_proc service is unavailable or returns an error
	#[serde(default)]
	pub failure_mode: FailureMode,

	/// Additional metadata to send to the external processing service.
	/// Maps to the `metadata_context.filter_metadata` field in ProcessingRequest, and allows dynamic CEL expressions.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub metadata_context: Option<HashMap<String, HashMap<String, Arc<cel::Expression>>>>,

	/// Maps to the request `attributes` field in ProcessingRequest, and allows dynamic CEL expressions.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub request_attributes: Option<HashMap<String, Arc<cel::Expression>>>,
	/// Maps to the response `attributes` field in ProcessingRequest, and allows dynamic CEL expressions.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub response_attributes: Option<HashMap<String, Arc<cel::Expression>>>,
}

impl ExtProc {
	pub fn build(&self, client: PolicyClient) -> ExtProcRequest {
		ExtProcRequest {
			ext_proc: Some(ExtProcInstance::new(
				client,
				self.policies.clone(),
				self.target.clone(),
				self.failure_mode,
				self.metadata_context.clone(),
				self.request_attributes.clone(),
				self.response_attributes.clone(),
			)),
		}
	}

	pub fn expressions(&self) -> Box<dyn Iterator<Item = &Expression> + '_> {
		Box::new(
			self
				.metadata_context
				.iter()
				.flat_map(|m| {
					m.values()
						.flat_map(|inner| inner.values().map(AsRef::as_ref))
				})
				.chain(
					self
						.request_attributes
						.iter()
						.chain(self.response_attributes.iter())
						.flat_map(|m| m.values().map(AsRef::as_ref)),
				),
		)
	}
}

#[derive(Debug)]
pub struct ExtProcRequest {
	ext_proc: Option<ExtProcInstance>,
}

impl ExtProcRequest {
	pub async fn mutate_request(
		&mut self,
		req: &mut http::Request,
	) -> Result<PolicyResponse, ProxyError> {
		let Some(ext_proc) = &mut self.ext_proc else {
			return Ok(PolicyResponse::default());
		};
		let r = std::mem::take(req);
		let (new_req, pr) = ext_proc.mutate_request(r).await?;
		*req = new_req;
		Ok(pr.unwrap_or_default())
	}

	pub async fn mutate_response(
		&mut self,
		resp: &mut http::Response,
		request: Option<&RequestSnapshot>,
	) -> Result<PolicyResponse, ProxyError> {
		let Some(ext_proc) = &mut self.ext_proc else {
			return Ok(PolicyResponse::default());
		};
		let r = std::mem::take(resp);
		let (new_resp, pr) = ext_proc.mutate_response(r, request, None).await?;
		*resp = new_resp;
		Ok(pr.unwrap_or_default())
	}
}

// Very experimental support for ext_proc
#[derive(Debug)]
struct ExtProcInstance {
	failure_mode: FailureMode,
	skipped: bool,
	tx_req: Sender<ProcessingRequest>,
	rx_resp_for_request: Option<Receiver<ProcessingResponse>>,
	rx_resp_for_response: Option<Receiver<ProcessingResponse>>,
	metadata_context: Option<HashMap<String, HashMap<String, Arc<cel::Expression>>>>,
	req_attributes: Option<HashMap<String, Arc<cel::Expression>>>,
	resp_attributes: Option<HashMap<String, Arc<cel::Expression>>>,
}

impl ExtProcInstance {
	fn did_fail_open(&self) -> bool {
		self.skipped
	}

	fn new(
		client: PolicyClient,
		policies: Vec<BackendPolicy>,
		target: Arc<SimpleBackendReference>,
		failure_mode: FailureMode,
		metadata_context: Option<HashMap<String, HashMap<String, Arc<cel::Expression>>>>,
		req_attributes: Option<HashMap<String, Arc<cel::Expression>>>,
		resp_attributes: Option<HashMap<String, Arc<cel::Expression>>>,
	) -> ExtProcInstance {
		trace!("connecting to {:?}", target);
		let chan = GrpcReferenceChannel {
			target,
			client,
			policies: Arc::new(policies),
		};
		let mut c = proto::external_processor_client::ExternalProcessorClient::new(chan);
		let (tx_req, rx_req) = tokio::sync::mpsc::channel(10);
		let (tx_resp, mut rx_resp) = tokio::sync::mpsc::channel(10);
		let req_stream = tokio_stream::wrappers::ReceiverStream::new(rx_req);
		tokio::task::spawn(async move {
			// Spawn a task to handle processing requests.
			// Incoming requests get send to tx_req and will be piped through here.
			let responses = match c.process(req_stream).await {
				Ok(r) => r,
				Err(e) => {
					warn!(?failure_mode, "failed to initialize endpoint picker: {e:?}");
					return;
				},
			};
			trace!("initial stream established");
			let mut responses = responses.into_inner();
			while let Ok(Some(item)) = responses.message().await {
				trace!("received response item {item:?}");
				let _ = tx_resp.send(item).await;
			}
		});
		let (tx_resp_for_request, rx_resp_for_request) = tokio::sync::mpsc::channel(1);
		let (tx_resp_for_response, rx_resp_for_response) = tokio::sync::mpsc::channel(1);
		tokio::task::spawn(async move {
			while let Some(item) = rx_resp.recv().await {
				match &item.response {
					Some(processing_response::Response::ResponseBody(_))
					| Some(processing_response::Response::ResponseHeaders(_))
					| Some(processing_response::Response::ResponseTrailers(_)) => {
						let _ = tx_resp_for_response.send(item).await;
					},
					Some(processing_response::Response::RequestBody(_))
					| Some(processing_response::Response::RequestHeaders(_))
					| Some(processing_response::Response::RequestTrailers(_)) => {
						let _ = tx_resp_for_request.send(item).await;
					},
					Some(processing_response::Response::ImmediateResponse(_)) => {
						// In this case we aren't sure which is going to handle things...
						// Send to both
						let _ = tx_resp_for_request.send(item.clone()).await;
						let _ = tx_resp_for_response.send(item).await;
					},
					None => {},
				}
			}
		});
		Self {
			skipped: Default::default(),
			failure_mode,
			tx_req,
			rx_resp_for_request: Some(rx_resp_for_request),
			rx_resp_for_response: Some(rx_resp_for_response),
			metadata_context,
			req_attributes,
			resp_attributes,
		}
	}

	async fn send_request(&mut self, req: ProcessingRequest) -> Result<(), Error> {
		self.tx_req.send(req).await.map_err(|_| Error::RequestSend)
	}

	pub async fn mutate_request(
		&mut self,
		req: http::Request,
	) -> Result<(http::Request, Option<PolicyResponse>), Error> {
		let headers = req_to_header_map(&req);

		let exec = cel::Executor::new_request(&req);
		// request_attributes should only be sent on first ProcessingRequest
		// this will need to be modified if we configure which Requests to send
		// Wrap metadata_context in Arc for cheap cloning across body chunks
		let metadata_context = self.metadata_context.as_ref().map(|meta| {
			Arc::new(Metadata {
				filter_metadata: meta
					.iter()
					.filter_map(|(n, e)| {
						eval_to_struct(&exec, e).map(|v| (n.clone(), v)).ok() // TODO(mk): where best to log convertion issues
					})
					.collect(),
			})
		});
		let attributes = self
			.req_attributes
			.as_ref()
			.and_then(|attrs| {
				eval_to_struct(&exec, attrs)
					.map(|v| HashMap::from([(EXTPROC_ATTRIBUTES_NAMESPACE.to_string(), v)]))
					.ok()
			})
			.unwrap_or_default();

		let failure_mode = self.failure_mode;
		let end_of_stream = req.body().is_end_stream();

		// Send the request headers to ext_proc.
		if let Err(e) = self
			.send_request(ProcessingRequest {
				request: Some(Request::RequestHeaders(HttpHeaders {
					headers,
					end_of_stream,
				})),
				metadata_context: metadata_context.as_deref().cloned(),
				attributes,
				protocol_config: Default::default(),
				observability_mode: false,
			})
			.await
		{
			if failure_mode == FailureMode::FailOpen {
				trace!("fail open triggered");
				self.skipped = true;
				return Ok((req, None));
			}
			return Err(e);
		}

		// At this point, we start body handling. Fail open + streaming bodies is a disaster,
		// as we could silently corrupt data.
		// We behave approximately like Envoy here (https://github.com/envoyproxy/envoy/pull/41276); after
		// the headers are sent we drop fail_open.
		// In practice, this means that the server was running fine at the start of the request which covers
		// all but edge cases around the server dying mid-request.
		let (parts, body) = req.into_parts();
		let had_body = !end_of_stream;
		let tx = self.tx_req.clone();
		if had_body {
			tokio::task::spawn(Self::handle_body_stream(
				metadata_context,
				body,
				tx,
				Request::RequestBody,
				Request::RequestTrailers,
			));
		}

		// Now we need to build the new body. This is going to be streamed in from the ext_proc server.
		let (mut tx_chunk, rx_chunk) = tokio::sync::mpsc::channel(1);

		let upstream_body = http_body_util::StreamBody::new(ReceiverStream::new(rx_chunk));
		let mut req = http::Request::from_parts(parts, http::Body::new(upstream_body));
		req.headers_mut().remove(http::header::CONTENT_LENGTH);
		let mut rx = self
			.rx_resp_for_request
			.take()
			.expect("mutate_request called twice");
		loop {
			let Some(presp) = rx.recv().await else {
				if !had_body && failure_mode == FailureMode::FailOpen {
					trace!("fail open triggered");
					self.skipped = true;
					return Ok((req, None));
				}
				trace!("done receiving request");
				return Err(Error::NoMoreResponses);
			};
			if let Some(resp) = to_immediate_response(&presp) {
				trace!("got immediate response in request handler");
				return Ok((req, Some(resp)));
			}
			let (headers_done, eos) =
				handle_response_for_request_mutation(had_body, Some(&mut req), &mut tx_chunk, presp).await;
			if headers_done {
				if !eos {
					trace!("spawn body!");
					// Moving rest of body handling to async
					tokio::task::spawn(async move {
						loop {
							let Some(presp) = rx.recv().await else {
								trace!("done receiving request");
								return;
							};
							let (_, eos) =
								handle_response_for_request_mutation(had_body, None, &mut tx_chunk, presp).await;
							if eos || !had_body {
								trace!("request EOS!");
								drop(tx_chunk);
								return;
							}
						}
					});
				}
				return Ok((req, None));
			}
		}
	}

	async fn handle_body_stream(
		metadata_context: Option<Arc<Metadata>>,
		body: http::Body,
		tx: Sender<ProcessingRequest>,
		body_fn: fn(HttpBody) -> Request,
		trail_fn: fn(HttpTrailers) -> Request,
	) {
		let mut stream = BodyStream::new(body);
		while let Some(Ok(frame)) = stream.next().await {
			let request = Some(if frame.is_data() {
				let frame = frame.into_data().expect("already checked");
				trace!("sending body chunk...",);
				body_fn(HttpBody {
					body: frame.into(),
					end_of_stream: false,
				})
			} else if frame.is_trailers() {
				let frame = frame.into_trailers().expect("already checked");
				trail_fn(HttpTrailers {
					trailers: to_header_map(&frame),
				})
			} else {
				// http_body::Frame only has data and trailers variants
				unreachable!("Frame is neither data nor trailers")
			});
			let Ok(()) = tx
				.send(ProcessingRequest {
					request,
					metadata_context: metadata_context.as_deref().cloned(),
					attributes: Default::default(),
					protocol_config: Default::default(),
					observability_mode: false,
				})
				.await
			else {
				return;
			};
		}

		// Send end of stream marker - try to unwrap Arc to avoid final clone
		let final_metadata = metadata_context.and_then(Arc::into_inner);
		let _ = tx
			.send(ProcessingRequest {
				request: Some(body_fn(HttpBody {
					body: Default::default(),
					end_of_stream: true,
				})),
				metadata_context: final_metadata,
				attributes: Default::default(),
				protocol_config: Default::default(),
				observability_mode: false,
			})
			.await;
		trace!("body request done");
	}

	pub async fn mutate_response(
		&mut self,
		req: http::Response,
		request: Option<&RequestSnapshot>,
		resolved_destination_metadata: Option<SocketAddr>,
	) -> Result<(http::Response, Option<PolicyResponse>), Error> {
		if self.skipped {
			return Ok((req, None));
		}
		let headers = resp_to_header_map(&req);

		let exec = cel::Executor::new_response(request, &req);
		// Wrap metadata_context in Arc for cheap cloning across body chunks
		let metadata_context = if self.metadata_context.is_none()
			&& let Some(rd) = resolved_destination_metadata
		{
			Some(Arc::new(Metadata {
				filter_metadata: HashMap::from([(
					// This is gross, but the GIE project unfairly favors Envoy, so we have to adapt to its limitations.
					"envoy.lb".to_string(),
					serde_json::from_value(serde_json::json!({"x-gateway-destination-endpoint-served": rd}))
						.unwrap(),
				)]),
			}))
		} else {
			self.metadata_context.as_ref().map(|meta| {
				Arc::new(Metadata {
					filter_metadata: meta
						.iter()
						.filter_map(|(n, e)| eval_to_struct(&exec, e).map(|v| (n.clone(), v)).ok())
						.collect(),
				})
			})
		};
		// response_attributes should only be sent on first ProcessingRequest
		// this will need to be modified if we configure which Requests to send
		let attributes = self
			.resp_attributes
			.as_ref()
			.and_then(|attrs| {
				eval_to_struct(&exec, attrs)
					.map(|v| HashMap::from([(EXTPROC_ATTRIBUTES_NAMESPACE.to_string(), v)]))
					.ok()
			})
			.unwrap_or_default();
		let (parts, body) = req.into_parts();
		let end_of_stream = body.is_end_stream();
		let had_body = !end_of_stream;

		// Send the response headers to ext_proc.
		// No response side fail_open handling.
		self
			.send_request(ProcessingRequest {
				request: Some(Request::ResponseHeaders(HttpHeaders {
					headers,
					end_of_stream,
				})),
				metadata_context: metadata_context.as_deref().cloned(),
				attributes,
				protocol_config: Default::default(),
				observability_mode: false,
			})
			.await?;

		// The EPP will await for our headers and body. The body is going to be streaming in.
		// We will spin off a task that is going to pipe the body to the ext_proc server as we read it.
		let tx = self.tx_req.clone();
		if had_body {
			tokio::task::spawn(Self::handle_body_stream(
				metadata_context,
				body,
				tx,
				Request::ResponseBody,
				Request::ResponseTrailers,
			));
		}

		// Now we need to build the new body. This is going to be streamed in from the ext_proc server.
		let (mut tx_chunk, rx_chunk) = tokio::sync::mpsc::channel(1);

		let body = http_body_util::StreamBody::new(ReceiverStream::new(rx_chunk));
		let mut resp = http::Response::from_parts(parts, http::Body::new(body));
		resp.headers_mut().remove(http::header::CONTENT_LENGTH);
		let mut rx = self
			.rx_resp_for_response
			.take()
			.expect("mutate_response called twice");
		loop {
			let Some(presp) = rx.recv().await else {
				trace!("done receiving response");
				return Err(Error::NoMoreResponses);
			};
			if let Some(dr) = to_immediate_response(&presp) {
				trace!("got immediate response in request handler");
				return Ok((resp, Some(dr)));
			}
			let (headers_done, eos) =
				handle_response_for_response_mutation(had_body, Some(&mut resp), &mut tx_chunk, presp)
					.await;
			if headers_done {
				if !eos {
					trace!("spawn body!");
					// Moving rest of body handling to async
					tokio::task::spawn(async move {
						loop {
							let Some(presp) = rx.recv().await else {
								trace!("done receiving response");
								return;
							};
							let (_, eos) =
								handle_response_for_response_mutation(had_body, None, &mut tx_chunk, presp).await;
							if eos || !had_body {
								trace!("response EOS!");
								drop(tx_chunk);
								return;
							}
						}
					});
				}
				return Ok((resp, None));
			}
		}
	}
}

fn to_immediate_response(rp: &ProcessingResponse) -> Option<PolicyResponse> {
	match &rp.response {
		Some(Response::ImmediateResponse(ir)) => {
			let ImmediateResponse {
				status,
				headers,
				body,
				grpc_status: _,
				details: _,
			} = ir;
			let mut rb =
				::http::response::Builder::new().status(status.map(|s| s.code).unwrap_or(200) as u16);

			if let Some(hm) = rb.headers_mut() {
				apply_header_mutations(hm, headers.as_ref());
			}
			let resp = rb
				.body(http::Body::from(body.to_string()))
				.map_err(|e| ProxyError::Processing(e.into()))
				.unwrap();
			Some(crate::http::PolicyResponse {
				direct_response: Some(resp),
				response_headers: None,
			})
		},
		_ => None,
	}
}

// handle_response_for_request_mutation handles a single ext_proc response. If it returns 'true' we are done processing.
async fn handle_response_for_request_mutation(
	had_body: bool,
	mut req: Option<&mut http::Request>,
	body_tx: &mut Sender<Result<Frame<Bytes>, Infallible>>,
	presp: ProcessingResponse,
) -> (bool, bool) {
	if let Some(dm) = &presp.dynamic_metadata {
		if let Some(req) = req.as_mut() {
			if let Err(e) = extract_dynamic_metadata(req, dm) {
				warn!("Failed to extract ext_proc dynamic metadata: {}", e);
			}
		} else if !dm.fields.is_empty() {
			warn!(
				"ext_proc server sent dynamic_metadata after headers were processed; \
					 metadata cannot be attached and will be ignored. Consider sending \
					 metadata in the RequestHeaders response instead."
			);
		}
	}

	let res = matches!(presp.response, Some(Response::RequestHeaders(_)));
	let cr = match presp.response {
		Some(Response::RequestHeaders(HeadersResponse { response: None })) => {
			trace!("no headers");
			return (true, !had_body);
		},
		Some(Response::RequestHeaders(HeadersResponse { response: Some(cr) })) => {
			trace!("got request headers back");
			cr
		},
		Some(Response::RequestBody(BodyResponse { response: None })) => {
			trace!("got empty request body back");
			return (false, true);
		},
		Some(Response::RequestBody(BodyResponse { response: Some(cr) })) => {
			trace!("got request body back");
			cr
		},
		Some(Response::ImmediateResponse(_)) => {
			if req.is_none() {
				trace!("immediate response received after request sent; will apply only on the response");
			}
			// Handled out of this function.
			return (true, true);
		},
		msg => {
			// In theory, there can trailers too. EPP never sends them
			warn!("ignoring response during request {msg:?}");
			return (false, false);
		},
	};
	if let Some(req) = req {
		apply_header_mutations_request(req, cr.header_mutation.as_ref());
	}
	if let Some(BodyMutation { mutation: Some(b) }) = cr.body_mutation {
		match b {
			Mutation::StreamedResponse(bb) => {
				let eos = bb.end_of_stream;
				let by = bytes::Bytes::from(bb.body);
				let _ = body_tx.send(Ok(Frame::data(by.clone()))).await;

				trace!(eos, "got stream request body");
				return (res, eos);
			},
			Mutation::Body(_) => {
				warn!("Body() not valid for streaming mode, skipping...");
			},
			Mutation::ClearBody(_) => {
				warn!("ClearBody() not valid for streaming mode, skipping...");
			},
		}
	} else if !had_body {
		trace!("got headers back and do not expect body; we are done");
		return (res, true);
	}
	trace!("still waiting for response...");
	(res, false)
}

fn apply_header_with_action(headers: &mut HeaderMap, hk: &HeaderName, hvo: &HeaderValueOption) {
	let Some(_) = hvo.header else {
		return;
	};

	// Skip content-length as the EPP sets it to invalid values
	// https://github.com/kubernetes-sigs/gateway-api-inference-extension/issues/943
	if hk == http::header::CONTENT_LENGTH {
		debug!("skipping invalid content-length");
		return;
	}

	let _ = envoy_proto_common::apply_header_value_option(headers, hk, hvo);
}

fn apply_header_mutations(headers: &mut HeaderMap, h: Option<&HeaderMutation>) {
	if let Some(hm) = h {
		for rm in &hm.remove_headers {
			headers.remove(rm);
		}
		for set in &hm.set_headers {
			let Some(h) = &set.header else { continue };
			let Ok(hk) = HeaderName::try_from(h.key.as_str()) else {
				warn!("invalid header key: {}", h.key);
				continue;
			};
			apply_header_with_action(headers, &hk, set);
		}
	}
}

fn apply_header_mutations_request(req: &mut http::Request, h: Option<&HeaderMutation>) {
	if let Some(hm) = h {
		for rm in &hm.remove_headers {
			req.headers_mut().remove(rm);
		}
		for set in &hm.set_headers {
			let Some(h) = &set.header else { continue };
			match HeaderOrPseudo::try_from(h.key.as_str()) {
				Ok(HeaderOrPseudo::Header(hk)) => {
					apply_header_with_action(req.headers_mut(), &hk, set);
				},
				Ok(_) => {
					let mut rr = crate::http::RequestOrResponse::Request(req);
					let _ = envoy_proto_common::apply_pseudo_header_option(&mut rr, set);
				},
				Err(e) => {
					warn!("invalid header key: {} {e}", h.key);
				},
			}
		}
	}
}

fn apply_header_mutations_response(resp: &mut http::Response, h: Option<&HeaderMutation>) {
	if let Some(hm) = h {
		for rm in &hm.remove_headers {
			resp.headers_mut().remove(rm);
		}
		for set in &hm.set_headers {
			let Some(h) = &set.header else { continue };
			match crate::http::HeaderOrPseudo::try_from(h.key.as_str()) {
				Ok(crate::http::HeaderOrPseudo::Header(hk)) => {
					apply_header_with_action(resp.headers_mut(), &hk, set);
				},
				Ok(_) => {
					let mut rr = crate::http::RequestOrResponse::Response(resp);
					let _ = envoy_proto_common::apply_pseudo_header_option(&mut rr, set);
				},
				Err(e) => {
					warn!("invalid header key: {} {e}", h.key);
				},
			}
		}
	}
}

// handle_response_for_response_mutation handles a single ext_proc response. If it returns 'true' we are done processing.
async fn handle_response_for_response_mutation(
	had_body: bool,
	resp: Option<&mut http::Response>,
	body_tx: &mut Sender<Result<Frame<Bytes>, Infallible>>,
	presp: ProcessingResponse,
) -> (bool, bool) {
	let res = matches!(presp.response, Some(Response::ResponseHeaders(_)));
	let cr = match presp.response {
		Some(Response::ResponseHeaders(HeadersResponse { response: None })) => {
			trace!("no headers");
			return (res, false);
		},
		Some(Response::ResponseHeaders(HeadersResponse { response: Some(cr) })) => cr,
		Some(Response::ResponseBody(BodyResponse { response: Some(cr) })) => cr,
		Some(Response::ResponseBody(BodyResponse { response: None })) => {
			trace!("got empty response body back");
			return (res, true);
		},
		msg => {
			// In theory, there can trailers too. EPP never sends them
			warn!("ignoring {msg:?}");
			return (res, false);
		},
	};
	if let Some(resp) = resp {
		apply_header_mutations_response(resp, cr.header_mutation.as_ref());
	}
	if let Some(BodyMutation { mutation: Some(b) }) = cr.body_mutation {
		match b {
			Mutation::StreamedResponse(bb) => {
				let eos = bb.end_of_stream;
				let by = bytes::Bytes::from(bb.body);
				let _ = body_tx.send(Ok(Frame::data(by.clone()))).await;
				trace!(%eos, "got body chunk");
				return (res, eos);
			},
			Mutation::Body(_) => {
				warn!("Body() not valid for streaming mode, skipping...");
			},
			Mutation::ClearBody(_) => {
				warn!("ClearBody() not valid for streaming mode, skipping...");
			},
		}
	} else if !had_body {
		trace!("got headers back and do not expect body; we are done");
		return (res, true);
	}
	trace!("still waiting for response for response...");
	(res, false)
}

fn req_to_header_map(req: &http::Request) -> Option<proto::HeaderMap> {
	let mut pseudo = crate::http::get_request_pseudo_headers(req);
	let has_scheme = pseudo
		.iter()
		.any(|(p, _)| matches!(p, crate::http::HeaderOrPseudo::Scheme));
	if !has_scheme {
		// Default to http when scheme is not explicitly present on the request URI
		pseudo.push((crate::http::HeaderOrPseudo::Scheme, "http".to_string()));
	}
	let pseudo_header_pairs: Vec<(String, String)> = pseudo
		.into_iter()
		.map(|(p, v)| (p.to_string(), v))
		.collect();
	to_header_map_extra(
		req.headers(),
		&pseudo_header_pairs
			.iter()
			.map(|(k, v)| (k.as_str(), v.as_str()))
			.collect::<Vec<_>>(),
	)
}

fn resp_to_header_map(res: &http::Response) -> Option<proto::HeaderMap> {
	to_header_map_extra(res.headers(), &[(":status", res.status().as_str())])
}

fn to_header_map(headers: &http::HeaderMap) -> Option<proto::HeaderMap> {
	to_header_map_extra(headers, &[])
}

fn to_header_map_extra(
	headers: &http::HeaderMap,
	additional_headers: &[(&str, &str)],
) -> Option<proto::HeaderMap> {
	let h = headers
		.iter()
		.map(|(k, v)| proto::HeaderValue {
			key: k.to_string(),
			value: String::new(),
			raw_value: v.as_bytes().to_vec(),
		})
		.chain(additional_headers.iter().map(|(k, v)| proto::HeaderValue {
			key: k.to_string(),
			value: v.to_string(),
			raw_value: vec![],
		}))
		.collect_vec();
	Some(proto::HeaderMap { headers: h })
}

fn eval_expression(exec: &Executor, v: &Expression) -> Result<prost_wkt_types::Value, ProxyError> {
	let res = exec.eval(v).map_err(|e| ProxyError::Processing(e.into()))?;
	let js = res
		.json()
		.map_err(|_| ProxyError::Processing(cel::Error::JsonConvert.into()))?;
	envoy_proto_common::json_to_prost_value(js)
}

fn eval_to_struct(
	exec: &Executor<'_>,
	expressions: &HashMap<String, Arc<cel::Expression>>,
) -> Result<prost_wkt_types::Struct, ProxyError> {
	Ok(Struct {
		fields: expressions
			.iter()
			.filter_map(|(key, expr)| match eval_expression(exec, expr) {
				Ok(result) => Some((key.clone(), result)),
				Err(error) => {
					warn!(%key, %error, "failed to evaluate metadata_context CEL expression");
					None
				},
			})
			.collect(),
	})
}

pub(crate) fn extract_dynamic_metadata(
	req: &mut http::Request,
	metadata: &prost_wkt_types::Struct,
) -> Result<(), Error> {
	// Get or create metadata container, merging with existing metadata
	let mut dynamic_metadata = req
		.extensions_mut()
		.remove::<ExtProcDynamicMetadata>()
		.unwrap_or_default();

	// Merge new fields into existing metadata
	for (key, value) in &metadata.fields {
		let json_val = envoy_proto_common::prost_value_to_json(value)
			.map_err(|e| Error::MetadataConversion(format!("failed to convert key '{}': {}", key, e)))?;
		dynamic_metadata.0.insert(key.clone(), json_val);
	}

	if !dynamic_metadata.0.is_empty() {
		req.extensions_mut().insert(dynamic_metadata);
	}

	Ok(())
}

#[derive(Clone, Debug)]
pub struct GrpcReferenceChannel {
	pub target: Arc<SimpleBackendReference>,
	pub client: PolicyClient,
	pub policies: Arc<Vec<BackendPolicy>>,
}

impl tower::Service<::http::Request<tonic::body::Body>> for GrpcReferenceChannel {
	type Response = http::Response;
	type Error = ProxyError;
	type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

	fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
		Ok(()).into()
	}

	fn call(&mut self, req: ::http::Request<tonic::body::Body>) -> Self::Future {
		let client = self.client.clone();
		let target = self.target.clone();
		let policies = self.policies.clone();
		let req = req.map(http::Body::new);
		Box::pin(async move {
			client
				.call_reference_with_policies(req, &target, policies.as_slice())
				.await
		})
	}
}
