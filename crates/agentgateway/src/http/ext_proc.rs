use std::convert::Infallible;

use ::http::Uri;
use ::http::uri::Authority;
use anyhow::anyhow;
use bytes::Bytes;
use http_body::Frame;
use http_body_util::BodyStream;
use itertools::Itertools;
use minijinja::__context::build;
use proto::body_mutation::Mutation;
use proto::processing_request::Request;
use proto::processing_response::Response;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

use crate::client::{Client, Transport};
use crate::control::AuthSource;
use crate::http::backendtls::BackendTLS;
use crate::http::ext_proc::proto::{
	BodyMutation, BodyResponse, HeadersResponse, HttpBody, HttpHeaders, HttpTrailers,
	ProcessingRequest, ProcessingResponse,
};
use crate::http::{HeaderName, HeaderValue};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::types::agent;
use crate::types::agent::{Backend, SimpleBackendReference, Target};
use crate::types::discovery::NamespacedHostname;
use crate::*;

#[allow(warnings)]
#[allow(clippy::derive_partial_eq_without_eq)]
pub mod proto {
	tonic::include_proto!("envoy.service.ext_proc.v3");
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceRouting {
	pub target: Arc<SimpleBackendReference>,
}

#[derive(Debug, Default)]
pub struct InferencePoolRouter {
	ext_proc: Option<ExtProc>,
}

impl InferenceRouting {
	pub fn build(&self, client: PolicyClient) -> InferencePoolRouter {
		InferencePoolRouter {
			ext_proc: Some(ExtProc::new(client, self.target.clone())),
		}
	}
}

impl InferencePoolRouter {
	pub async fn mutate_request(
		&mut self,
		req: &mut http::Request,
	) -> Result<Option<SocketAddr>, ProxyError> {
		let Some(ext_proc) = &mut self.ext_proc else {
			return Ok(None);
		};
		let r = std::mem::take(req);
		*req = ext_proc
			.mutate_request(r)
			.await
			.context("ext_proc call")
			.map_err(ProxyError::Processing)?;
		let dest = req
			.headers()
			.get(HeaderName::from_static("x-gateway-destination-endpoint"))
			.and_then(|v| v.to_str().ok())
			.map(|v| v.parse::<SocketAddr>())
			.transpose()
			.map_err(|e| ProxyError::Processing(anyhow!("EPP returned invalid address: {e}")))?;
		Ok(dest)
	}

	pub async fn mutate_response(&mut self, resp: &mut http::Response) -> Result<(), ProxyError> {
		let Some(ext_proc) = &mut self.ext_proc else {
			return Ok(());
		};
		let r = std::mem::take(resp);
		*resp = ext_proc
			.mutate_response(r)
			.await
			.context("ext_proc call")
			.map_err(ProxyError::Processing)?;
		Ok(())
	}
}

// Very experimental support for ext_proc
#[derive(Debug)]
pub struct ExtProc {
	tx_req: Sender<ProcessingRequest>,
	rx_resp: Receiver<ProcessingResponse>,
}

impl ExtProc {
	pub fn new(client: PolicyClient, target: Arc<SimpleBackendReference>) -> ExtProc {
		trace!("connecting to {:?}", target);
		let chan = GrpcReferenceChannel { target, client };
		let mut c = proto::external_processor_client::ExternalProcessorClient::new(chan);
		let (tx_req, rx_req) = tokio::sync::mpsc::channel(10);
		let (tx_resp, rx_resp) = tokio::sync::mpsc::channel(10);
		let req_stream = tokio_stream::wrappers::ReceiverStream::new(rx_req);
		tokio::task::spawn(async move {
			// Spawn a task to handle processing requests.
			// Incoming requests get send to tx_req and will be piped through here.
			//
			let Ok(responses) = c.process(req_stream).await else {
				warn!("failed to initialize endpoint picker");
				return;
			};
			trace!("initial stream established");
			let mut responses = responses.into_inner();
			while let Ok(Some(item)) = responses.message().await {
				trace!("received response item");
				let _ = tx_resp.send(item).await;
			}
		});
		Self { tx_req, rx_resp }
	}

	async fn recv(&mut self) -> anyhow::Result<ProcessingResponse> {
		self
			.rx_resp
			.recv()
			.await
			.ok_or(anyhow::anyhow!("no more response messages"))
	}

	async fn send_request(&mut self, req: ProcessingRequest) -> anyhow::Result<()> {
		if let Err(e) = self.tx_req.send(req).await {
			anyhow::bail!("failed to send ext_proc request: {e}");
		};
		Ok(())
	}

	pub async fn mutate_request(&mut self, mut req: http::Request) -> anyhow::Result<http::Request> {
		let headers = to_header_map(req.headers());
		let (parts, body) = req.into_parts();

		let preq = processing_request(Request::RequestHeaders(HttpHeaders {
			headers,
			attributes: Default::default(),
			end_of_stream: false,
		}));

		// Send the request headers to ext_proc.
		self.send_request(preq).await?;
		// The EPP will await for our headers and body. The body is going to be streaming in.
		// We will spin off a task that is going to pipe the body to the ext_proc server as we read it.
		let tx = self.tx_req.clone();

		tokio::task::spawn(async move {
			let mut stream = BodyStream::new(body);
			while let Some(Ok(frame)) = stream.next().await {
				let preq = if frame.is_data() {
					let frame = frame.into_data().expect("already checked");
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
		// Now we need to build the new body. This is going to be streamed in from the ext_proc server.
		let (mut tx_chunk, rx_chunk) = tokio::sync::mpsc::channel(1);
		let body = http_body_util::StreamBody::new(ReceiverStream::new(rx_chunk));
		let mut req = http::Request::from_parts(parts, http::Body::new(body));
		loop {
			// Loop through all the ext_proc responses and process them
			let resp = self.recv().await?;
			if handle_response_for_request_mutation(&mut req, &mut tx_chunk, resp).await? {
				trace!("request complete!");
				return Ok(req);
			}
		}
	}

	pub async fn mutate_response(
		&mut self,
		mut req: http::Response,
	) -> anyhow::Result<http::Response> {
		let headers = to_header_map(req.headers());
		let (parts, body) = req.into_parts();

		let preq = processing_request(Request::ResponseHeaders(HttpHeaders {
			headers,
			attributes: Default::default(),
			end_of_stream: false,
		}));

		// Send the request headers to ext_proc.
		self.send_request(preq).await?;
		// The EPP will await for our headers and body. The body is going to be streaming in.
		// We will spin off a task that is going to pipe the body to the ext_proc server as we read it.
		let tx = self.tx_req.clone();

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
		// Now we need to build the new body. This is going to be streamed in from the ext_proc server.
		let (mut tx_chunk, rx_chunk) = tokio::sync::mpsc::channel(1);
		let body = http_body_util::StreamBody::new(ReceiverStream::new(rx_chunk));
		let mut req = http::Response::from_parts(parts, http::Body::new(body));
		loop {
			// Loop through all the ext_proc responses and process them
			let resp = self.recv().await?;
			if handle_response_for_response_mutation(&mut req, &mut tx_chunk, resp).await? {
				trace!("response complete!");
				return Ok(req);
			}
		}
	}
}

// handle_response_for_request_mutation handles a single ext_proc response. If it returns 'true' we are done processing.
async fn handle_response_for_request_mutation(
	req: &mut http::Request,
	body_tx: &mut Sender<Result<Frame<Bytes>, Infallible>>,
	presp: ProcessingResponse,
) -> anyhow::Result<bool> {
	let cr = match presp.response {
		Some(Response::RequestHeaders(HeadersResponse { response: Some(cr) })) => cr,
		Some(Response::RequestBody(BodyResponse { response: Some(cr) })) => cr,
		msg => {
			// In theory, there can trailers too. EPP never sends them
			warn!("ignoring {msg:?}");
			return Ok(false);
		},
	};
	if let Some(h) = cr.header_mutation {
		for rm in &h.remove_headers {
			req.headers_mut().remove(rm);
		}
		for set in h.set_headers {
			let Some(h) = set.header else {
				continue;
			};
			let hk = HeaderName::try_from(h.key)?;
			if hk == http::header::CONTENT_LENGTH {
				debug!("skipping invalid content-length");
				// The EPP actually sets content-length to an invalid value, so don't respect it.
				// https://github.com/kubernetes-sigs/gateway-api-inference-extension/issues/943
				continue;
			}
			req
				.headers_mut()
				.insert(hk, HeaderValue::from_bytes(h.raw_value.as_slice())?);
		}
	}
	req.headers_mut().remove(http::header::CONTENT_LENGTH);
	if let Some(BodyMutation { mutation: Some(b) }) = cr.body_mutation {
		match b {
			Mutation::StreamedResponse(bb) => {
				let eos = bb.end_of_stream;
				let by = bytes::Bytes::from(bb.body);
				let _ = body_tx.send(Ok(Frame::data(by.clone()))).await;
				return Ok(eos);
			},
			Mutation::Body(_) => {
				warn!("Body() not valid for streaming mode, skipping...");
			},
			Mutation::ClearBody(_) => {
				warn!("ClearBody() not valid for streaming mode, skipping...");
			},
		}
	}
	trace!("still waiting for response...");
	Ok(false)
}

// handle_response_for_response_mutation handles a single ext_proc response. If it returns 'true' we are done processing.
async fn handle_response_for_response_mutation(
	req: &mut http::Response,
	body_tx: &mut Sender<Result<Frame<Bytes>, Infallible>>,
	presp: ProcessingResponse,
) -> anyhow::Result<bool> {
	let cr = match presp.response {
		Some(Response::ResponseHeaders(HeadersResponse { response: Some(cr) })) => cr,
		Some(Response::ResponseBody(BodyResponse { response: Some(cr) })) => cr,
		msg => {
			// In theory, there can trailers too. EPP never sends them
			warn!("ignoring {msg:?}");
			return Ok(false);
		},
	};
	if let Some(h) = cr.header_mutation {
		for rm in &h.remove_headers {
			req.headers_mut().remove(rm);
		}
		for set in h.set_headers {
			let Some(h) = set.header else {
				continue;
			};
			let hk = HeaderName::try_from(h.key)?;
			if hk == http::header::CONTENT_LENGTH {
				debug!("skipping invalid content-length");
				// The EPP actually sets content-length to an invalid value, so don't respect it.
				// https://github.com/kubernetes-sigs/gateway-api-inference-extension/issues/943
				continue;
			}
			req
				.headers_mut()
				.insert(hk, HeaderValue::from_bytes(h.raw_value.as_slice())?);
		}
	}
	req.headers_mut().remove(http::header::CONTENT_LENGTH);
	if let Some(BodyMutation { mutation: Some(b) }) = cr.body_mutation {
		match b {
			Mutation::StreamedResponse(bb) => {
				let eos = bb.end_of_stream;
				let by = bytes::Bytes::from(bb.body);
				let _ = body_tx.send(Ok(Frame::data(by.clone()))).await;
				return Ok(eos);
			},
			Mutation::Body(_) => {
				warn!("Body() not valid for streaming mode, skipping...");
			},
			Mutation::ClearBody(_) => {
				warn!("ClearBody() not valid for streaming mode, skipping...");
			},
		}
	}
	trace!("still waiting for response...");
	Ok(false)
}

fn to_header_map(headers: &http::HeaderMap) -> Option<proto::HeaderMap> {
	let h = headers
		.iter()
		.map(|(k, v)| proto::HeaderValue {
			key: k.to_string(),
			raw_value: v.as_bytes().to_vec(),
		})
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

	fn call(&mut self, mut req: ::http::Request<tonic::body::Body>) -> Self::Future {
		let client = self.client.clone();
		let target = self.target.clone();
		let mut req = req.map(http::Body::new);
		Box::pin(async move { Ok(client.call_reference(req, &target).await?) })
	}
}
