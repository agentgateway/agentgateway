use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};

use ::http::HeaderMap;
use anyhow::anyhow;
use bytes::Bytes;
use http_body::{Body, Frame};
use http_body_util::BodyStream;
use itertools::Itertools;
use proto::body_mutation::Mutation;
use proto::processing_request::Request;
use proto::processing_response::Response;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

use crate::http::ext_proc::proto::{
	BodyMutation, BodyResponse, HeaderMutation, HeadersResponse, HttpBody, HttpHeaders, HttpTrailers,
	ImmediateResponse, ProcessingRequest, ProcessingResponse, processing_response,
};
use crate::http::{HeaderName, HeaderValue, PolicyResponse};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::types::agent::SimpleBackendReference;
use crate::*;

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
	#[error(transparent)]
	InvalidHeaderName(#[from] http::header::InvalidHeaderName),
	#[error(transparent)]
	InvalidHeaderValue(#[from] http::header::InvalidHeaderValue),
}

#[cfg(test)]
#[path = "ext_proc_tests.rs"]
mod tests;

#[allow(warnings)]
#[allow(clippy::derive_partial_eq_without_eq)]
pub mod proto {
	tonic::include_proto!("envoy.service.ext_proc.v3");
}

#[apply(schema!)]
#[derive(Default, Copy, PartialEq, Eq)]
pub enum FailureMode {
	#[default]
	FailClosed,
	FailOpen,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceRouting {
	pub target: Arc<SimpleBackendReference>,
	pub failure_mode: FailureMode,
}

#[derive(Debug, Default)]
pub struct InferencePoolRouter {
	ext_proc: Option<ExtProcInstance>,
}

impl InferenceRouting {
	pub fn build(&self, client: PolicyClient) -> InferencePoolRouter {
		InferencePoolRouter {
			ext_proc: Some(ExtProcInstance::new(
				client,
				self.target.clone(),
				self.failure_mode,
			)),
		}
	}
}

impl InferencePoolRouter {
	pub async fn mutate_request(
		&mut self,
		req: &mut http::Request,
	) -> Result<(Option<SocketAddr>, PolicyResponse), ProxyError> {
		let Some(ext_proc) = &mut self.ext_proc else {
			return Ok((None, Default::default()));
		};
		let r = std::mem::take(req);
		let (new_req, pr) = ext_proc.mutate_request(r).await?;
		*req = new_req;
		let dest = req
			.headers()
			.get(HeaderName::from_static("x-gateway-destination-endpoint"))
			.and_then(|v| v.to_str().ok())
			.map(|v| v.parse::<SocketAddr>())
			.transpose()
			.map_err(|e| ProxyError::Processing(anyhow!("EPP returned invalid address: {e}")))?;
		Ok((dest, pr.unwrap_or_default()))
	}

	pub async fn mutate_response(
		&mut self,
		resp: &mut http::Response,
	) -> Result<PolicyResponse, ProxyError> {
		let Some(ext_proc) = &mut self.ext_proc else {
			return Ok(Default::default());
		};
		let r = std::mem::take(resp);
		let (new_resp, pr) = ext_proc.mutate_response(r).await?;
		*resp = new_resp;
		Ok(pr.unwrap_or_default())
	}
}

#[apply(schema_ser!)]
pub struct ExtProc {
	pub target: Arc<SimpleBackendReference>,
	#[serde(default)]
	pub failure_mode: FailureMode,
}

impl ExtProc {
	pub fn build(&self, client: PolicyClient) -> ExtProcRequest {
		ExtProcRequest {
			ext_proc: Some(ExtProcInstance::new(
				client,
				self.target.clone(),
				self.failure_mode,
			)),
		}
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
	) -> Result<PolicyResponse, ProxyError> {
		let Some(ext_proc) = &mut self.ext_proc else {
			return Ok(PolicyResponse::default());
		};
		let r = std::mem::take(resp);
		let (new_resp, pr) = ext_proc.mutate_response(r).await?;
		*resp = new_resp;
		Ok(pr.unwrap_or_default())
	}
}

// Very experimental support for ext_proc
#[derive(Debug)]
struct ExtProcInstance {
	failure_mode: FailureMode,
	skipped: Arc<AtomicBool>,
	tx_req: Sender<ProcessingRequest>,
	rx_resp_for_request: Option<Receiver<ProcessingResponse>>,
	rx_resp_for_response: Option<Receiver<ProcessingResponse>>,
}

impl ExtProcInstance {
	fn new(
		client: PolicyClient,
		target: Arc<SimpleBackendReference>,
		failure_mode: FailureMode,
	) -> ExtProcInstance {
		trace!("connecting to {:?}", target);
		let chan = GrpcReferenceChannel { target, client };
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
				trace!("received response item {item:?}");
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
		let buffer = http::buffer_limit(&req);
		let (parts, body) = req.into_parts();

		// For fail open we need a copy of the body. There is definitely a better way to do this, but for
		// now its good enough?
		let (body_copy, body) = if self.failure_mode == FailureMode::FailOpen {
			let buffered = http::read_body_with_limit(body, buffer)
				.await
				.map_err(|e| Error::BodyBuffer(e.to_string()))?;
			(Some(buffered.clone()), http::Body::from(buffered))
		} else {
			(None, body)
		};

		let end_of_stream = body.is_end_stream();
		let preq = processing_request(Request::RequestHeaders(HttpHeaders {
			headers,
			attributes: Default::default(),
			end_of_stream,
		}));
		let had_body = !end_of_stream;

		// Send the request headers to ext_proc.
		self.send_request(preq).await?;
		// The EPP will await for our headers and body. The body is going to be streaming in.
		// We will spin off a task that is going to pipe the body to the ext_proc server as we read it.
		let tx = self.tx_req.clone();

		if had_body {
			tokio::task::spawn(async move {
				let mut stream = BodyStream::new(body);
				while let Some(Ok(frame)) = stream.next().await {
					let preq = if frame.is_data() {
						let frame = frame.into_data().expect("already checked");
						trace!("sending request body chunk...",);
						processing_request(Request::RequestBody(HttpBody {
							body: frame.into(),
							end_of_stream: false,
						}))
					} else if frame.is_trailers() {
						let frame = frame.into_trailers().expect("already checked");
						processing_request(Request::RequestTrailers(HttpTrailers {
							trailers: to_header_map(&frame),
						}))
					} else {
						panic!("unknown type")
					};
					trace!("sending request body chunk...");
					let Ok(()) = tx.send(preq).await else {
						// TODO: on error here we need a way to signal to the outer task to fail fast
						return;
					};
				}
				// Now that the body is done, send end of stream
				let preq = processing_request(Request::RequestBody(HttpBody {
					body: Default::default(),
					end_of_stream: true,
				}));
				let _ = tx.send(preq).await;

				trace!("body request done");
			});
		}
		// Now we need to build the new body. This is going to be streamed in from the ext_proc server.
		let (tx_chunk, rx_chunk) = tokio::sync::mpsc::channel(1);

		let body = http_body_util::StreamBody::new(ReceiverStream::new(rx_chunk));
		let mut req = http::Request::from_parts(parts, http::Body::new(body));
		req.headers_mut().remove(http::header::CONTENT_LENGTH);
		let (tx_done, rx_done) = tokio::sync::oneshot::channel();
		let mut rx = self
			.rx_resp_for_request
			.take()
			.expect("mutate_request called twice");
		let failure_mode = self.failure_mode;
		let skipped = self.skipped.clone();
		tokio::task::spawn(async move {
			let mut req = Some(req);
			let mut tx_done = Some(tx_done);
			let mut tx_chunkh = Some(tx_chunk);
			loop {
				// Loop through all the ext_proc responses and process them
				let Some(presp) = rx.recv().await else {
					trace!("done receiving request");
					if failure_mode == FailureMode::FailOpen
						&& let Some(req) = req.take()
						&& let Some(tx_done) = tx_done.take()
					{
						trace!("fail open triggered");
						skipped.store(true, Ordering::SeqCst);
						let (parts, _) = req.into_parts();
						let new_req = http::Request::from_parts(parts, http::Body::from(body_copy.unwrap()));
						let _ = tx_done.send(Ok((new_req, None)));
						tx_chunkh.take();
					}
					return;
				};
				if let Some(resp) = to_immediate_response(&presp) {
					trace!("got immediate response in request handler");
					let _ = tx_done
						.take()
						.unwrap()
						.send(Ok((http::Request::default(), Some(resp))));
					tx_chunkh.take();
					return;
				}
				let Some(tx_chunk) = tx_chunkh.as_mut() else {
					return;
				};
				let r = handle_response_for_request_mutation(had_body, req.as_mut(), tx_chunk, presp).await;
				match r {
					Ok((headers_done, eos)) => {
						if headers_done
							&& let Some(req) = req.take()
							&& let Some(tx_done) = tx_done.take()
						{
							trace!("request complete!");
							let _ = tx_done.send(Ok((req, None)));
						}
						if eos || !had_body {
							trace!("request EOS!");
							tx_chunkh.take();
						}
					},
					Err(e) => {
						warn!("error {e:?}");
						return;
					},
				}
			}
		});
		rx_done.await.map_err(|_| Error::ResponseDropped)?
	}
	pub async fn mutate_response(
		&mut self,
		req: http::Response,
	) -> Result<(http::Response, Option<PolicyResponse>), Error> {
		if self.skipped.load(Ordering::SeqCst) {
			return Ok((req, None));
		}
		let headers = resp_to_header_map(&req);
		let (parts, body) = req.into_parts();

		let end_of_stream = body.is_end_stream();
		let preq = processing_request(Request::ResponseHeaders(HttpHeaders {
			headers,
			attributes: Default::default(),
			end_of_stream,
		}));
		let had_body = !end_of_stream;

		// Send the request headers to ext_proc.
		self.send_request(preq).await?;
		// The EPP will await for our headers and body. The body is going to be streaming in.
		// We will spin off a task that is going to pipe the body to the ext_proc server as we read it.
		let tx = self.tx_req.clone();

		if had_body {
			tokio::task::spawn(async move {
				let mut stream = BodyStream::new(body);
				while let Some(Ok(frame)) = stream.next().await {
					let preq = if frame.is_data() {
						let frame = frame.into_data().expect("already checked");
						processing_request(Request::ResponseBody(HttpBody {
							body: frame.into(),
							end_of_stream: false,
						}))
					} else if frame.is_trailers() {
						let frame = frame.into_trailers().expect("already checked");
						processing_request(Request::ResponseTrailers(HttpTrailers {
							trailers: to_header_map(&frame),
						}))
					} else {
						panic!("unknown type")
					};
					trace!("sending response body chunk...");
					let Ok(()) = tx.send(preq).await else {
						// TODO: on error here we need a way to signal to the outer task to fail fast
						return;
					};
				}
				// Now that the body is done, send end of stream
				let preq = processing_request(Request::ResponseBody(HttpBody {
					body: Default::default(),
					end_of_stream: true,
				}));
				let _ = tx.send(preq).await;
				trace!("body response done");
			});
		}
		// Now we need to build the new body. This is going to be streamed in from the ext_proc server.
		let (tx_chunk, rx_chunk) = tokio::sync::mpsc::channel(1);
		let body = http_body_util::StreamBody::new(ReceiverStream::new(rx_chunk));
		let mut resp = http::Response::from_parts(parts, http::Body::new(body));
		resp.headers_mut().remove(http::header::CONTENT_LENGTH);
		let (tx_done, rx_done) = tokio::sync::oneshot::channel();
		let mut rx = self
			.rx_resp_for_response
			.take()
			.expect("mutate_request called twice");
		tokio::task::spawn(async move {
			let mut resp = Some(resp);
			let mut tx_done = Some(tx_done);
			let mut tx_chunkh = Some(tx_chunk);
			loop {
				// Loop through all the ext_proc responses and process them
				let Some(presp) = rx.recv().await else {
					trace!("done receiving response");
					return;
				};
				if let Some(resp) = to_immediate_response(&presp) {
					trace!("got immediate response in response handler");
					let _ = tx_done
						.take()
						.unwrap()
						.send(Ok((http::Response::default(), Some(resp))));
					tx_chunkh.take();
					return;
				}
				let Some(tx_chunk) = tx_chunkh.as_mut() else {
					trace!("body done, skipping");
					return;
				};
				let r =
					handle_response_for_response_mutation(had_body, resp.as_mut(), tx_chunk, presp).await;
				match r {
					Ok((headers_done, eos)) => {
						if headers_done
							&& let Some(resp) = resp.take()
							&& let Some(tx_done) = tx_done.take()
						{
							trace!("response complete!");
							let _ = tx_done.send(Ok((resp, None)));
						}
						if eos || !had_body {
							trace!("response EOS!");
							tx_chunkh.take();
						}
					},
					Err(e) => {
						warn!("error {e:?}");
						return;
						// return tx_done.take().expect("must be called once").send(Err(e));
					},
				}
			}
		});
		rx_done.await.map_err(|_| Error::ResponseDropped)?
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
				let _ = apply_header_mutations(hm, headers.as_ref());
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
	req: Option<&mut http::Request>,
	body_tx: &mut Sender<Result<Frame<Bytes>, Infallible>>,
	presp: ProcessingResponse,
) -> Result<(bool, bool), Error> {
	let res = matches!(presp.response, Some(Response::RequestHeaders(_)));
	let cr = match presp.response {
		Some(Response::RequestHeaders(HeadersResponse { response: None })) => {
			trace!("no headers");
			return Ok((res, false));
		},
		Some(Response::RequestHeaders(HeadersResponse { response: Some(cr) })) => {
			trace!("got request headers back");
			cr
		},
		Some(Response::RequestBody(BodyResponse { response: None })) => {
			trace!("got empty request body back");
			return Ok((res, true));
		},
		Some(Response::RequestBody(BodyResponse { response: Some(cr) })) => {
			trace!("got request body back");
			cr
		},
		msg => {
			// In theory, there can trailers too. EPP never sends them
			warn!("ignoring response during request {msg:?}");
			return Ok((res, false));
		},
	};
	if let Some(req) = req {
		apply_header_mutations(req.headers_mut(), cr.header_mutation.as_ref())?;
	}
	if let Some(BodyMutation { mutation: Some(b) }) = cr.body_mutation {
		match b {
			Mutation::StreamedResponse(bb) => {
				let eos = bb.end_of_stream;
				let by = bytes::Bytes::from(bb.body);
				let _ = body_tx.send(Ok(Frame::data(by.clone()))).await;

				trace!(eos, "got stream request body");
				return Ok((res, eos));
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
		return Ok((res, true));
	}
	trace!("still waiting for response...");
	Ok((res, false))
}

fn apply_header_mutations(
	headers: &mut HeaderMap,
	h: Option<&HeaderMutation>,
) -> Result<(), Error> {
	if let Some(h) = h {
		for rm in &h.remove_headers {
			headers.remove(rm);
		}
		for set in &h.set_headers {
			let Some(h) = &set.header else {
				continue;
			};
			let hk = HeaderName::try_from(h.key.as_str())?;
			if hk == http::header::CONTENT_LENGTH {
				debug!("skipping invalid content-length");
				// The EPP actually sets content-length to an invalid value, so don't respect it.
				// https://github.com/kubernetes-sigs/gateway-api-inference-extension/issues/943
				continue;
			}
			headers.insert(hk, HeaderValue::from_bytes(h.raw_value.as_slice())?);
		}
	}
	Ok(())
}

// handle_response_for_response_mutation handles a single ext_proc response. If it returns 'true' we are done processing.
async fn handle_response_for_response_mutation(
	had_body: bool,
	resp: Option<&mut http::Response>,
	body_tx: &mut Sender<Result<Frame<Bytes>, Infallible>>,
	presp: ProcessingResponse,
) -> Result<(bool, bool), Error> {
	let res = matches!(presp.response, Some(Response::ResponseHeaders(_)));
	let cr = match presp.response {
		Some(Response::ResponseHeaders(HeadersResponse { response: None })) => {
			trace!("no headers");
			return Ok((res, false));
		},
		Some(Response::ResponseHeaders(HeadersResponse { response: Some(cr) })) => cr,
		Some(Response::ResponseBody(BodyResponse { response: Some(cr) })) => cr,
		Some(Response::ResponseBody(BodyResponse { response: None })) => {
			trace!("got empty response body back");
			return Ok((res, true));
		},
		msg => {
			// In theory, there can trailers too. EPP never sends them
			warn!("ignoring {msg:?}");
			return Ok((res, false));
		},
	};
	if let Some(resp) = resp {
		apply_header_mutations(resp.headers_mut(), cr.header_mutation.as_ref())?;
	}
	if let Some(BodyMutation { mutation: Some(b) }) = cr.body_mutation {
		match b {
			Mutation::StreamedResponse(bb) => {
				let eos = bb.end_of_stream;
				let by = bytes::Bytes::from(bb.body);
				let _ = body_tx.send(Ok(Frame::data(by.clone()))).await;
				trace!(%eos, "got body chunk");
				return Ok((res, eos));
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
		return Ok((res, true));
	}
	trace!("still waiting for response for response...");
	Ok((res, false))
}

fn req_to_header_map(req: &http::Request) -> Option<proto::HeaderMap> {
	to_header_map_extra(
		req.headers(),
		&[
			(":path", req.uri().path()),
			(":method", req.method().as_str()),
			(":scheme", req.uri().scheme_str().unwrap_or("http")),
			(
				":authority",
				req.uri().authority().map(|a| a.as_str()).unwrap_or(""),
			),
		],
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
	extra: &[(&str, &str)],
) -> Option<proto::HeaderMap> {
	let h = headers
		.iter()
		.map(|(k, v)| proto::HeaderValue {
			key: k.to_string(),
			raw_value: v.as_bytes().to_vec(),
		})
		.chain(extra.iter().map(|(k, v)| proto::HeaderValue {
			key: k.to_string(),
			raw_value: v.as_bytes().to_vec(),
		}))
		.collect_vec();
	Some(proto::HeaderMap { headers: h })
}

fn processing_request(data: Request) -> ProcessingRequest {
	ProcessingRequest {
		observability_mode: false,
		attributes: Default::default(),
		protocol_config: Default::default(),
		request: Some(data),
	}
}

#[derive(Clone, Debug)]
pub struct GrpcReferenceChannel {
	pub target: Arc<SimpleBackendReference>,
	pub client: PolicyClient,
}

impl tower::Service<::http::Request<tonic::body::Body>> for GrpcReferenceChannel {
	type Response = http::Response;
	type Error = anyhow::Error;
	type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

	fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
		Ok(()).into()
	}

	fn call(&mut self, req: ::http::Request<tonic::body::Body>) -> Self::Future {
		let client = self.client.clone();
		let target = self.target.clone();
		let req = req.map(http::Body::new);
		Box::pin(async move { Ok(client.call_reference(req, &target).await?) })
	}
}
