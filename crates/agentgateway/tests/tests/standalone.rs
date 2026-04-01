use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use agentgateway::http::ext_proc::proto::external_processor_server::{
	ExternalProcessor, ExternalProcessorServer,
};
use agentgateway::http::ext_proc::proto::{
	self, BodyMutation, CommonResponse, HeaderMutation, HeaderValue, HeaderValueOption, HttpBody,
	ProcessingRequest, ProcessingResponse, body_mutation, processing_request, processing_response,
};
use http::{Method, StatusCode};
use http_body_util::BodyExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};
use tonic::{Request, Response as TonicResponse, Status, Streaming};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::common::gateway::AgentGateway;

const SERVICE_NAME: &str = "model-service.default.svc.cluster.local";
const SERVICE_REF: &str = "default/model-service.default.svc.cluster.local";
const SERVICE_PORT: u16 = 8000;

struct ExtProcServerHandle {
	address: SocketAddr,
	task: JoinHandle<()>,
}

impl Drop for ExtProcServerHandle {
	fn drop(&mut self) {
		self.task.abort();
	}
}

#[derive(Clone)]
struct StandaloneEppMock {
	target: Arc<str>,
	request_headers_seen: Arc<AtomicUsize>,
}

impl StandaloneEppMock {
	fn new(target: impl Into<Arc<str>>, request_headers_seen: Arc<AtomicUsize>) -> Self {
		Self {
			target: target.into(),
			request_headers_seen,
		}
	}
}

#[tonic::async_trait]
impl ExternalProcessor for StandaloneEppMock {
	type ProcessStream = ReceiverStream<Result<ProcessingResponse, Status>>;

	async fn process(
		&self,
		request: Request<Streaming<ProcessingRequest>>,
	) -> Result<TonicResponse<Self::ProcessStream>, Status> {
		let mut request_stream = request.into_inner();
		let (tx, rx) = mpsc::channel(16);
		let target = self.target.clone();
		let seen = self.request_headers_seen.clone();

		tokio::spawn(async move {
			while let Some(message) = request_stream.message().await? {
				match message.request {
					Some(processing_request::Request::RequestHeaders(_headers)) => {
						seen.fetch_add(1, Ordering::SeqCst);
						tx.send(inference_request_headers_response(target.as_ref()))
							.await
							.map_err(|_| Status::aborted("receiver dropped"))?;
					},
					Some(processing_request::Request::RequestBody(body)) => {
						tx.send(echo_request_body_response(&body))
							.await
							.map_err(|_| Status::aborted("receiver dropped"))?;
					},
					Some(processing_request::Request::ResponseHeaders(_headers)) => {
						tx.send(empty_response_headers_response())
							.await
							.map_err(|_| Status::aborted("receiver dropped"))?;
					},
					Some(processing_request::Request::ResponseBody(body)) => {
						tx.send(echo_response_body_response(&body))
							.await
							.map_err(|_| Status::aborted("receiver dropped"))?;
					},
					Some(processing_request::Request::RequestTrailers(_trailers)) => {},
					Some(processing_request::Request::ResponseTrailers(_trailers)) => {},
					None => {},
				}
			}
			Ok::<(), Status>(())
		});

		Ok(TonicResponse::new(ReceiverStream::new(rx)))
	}
}

fn inference_request_headers_response(target: &str) -> Result<ProcessingResponse, Status> {
	Ok(ProcessingResponse {
		response: Some(processing_response::Response::RequestHeaders(
			proto::HeadersResponse {
				response: Some(CommonResponse {
					header_mutation: Some(HeaderMutation {
						set_headers: vec![HeaderValueOption {
							header: Some(HeaderValue {
								key: "x-gateway-destination-endpoint".to_string(),
								value: target.to_string(),
								raw_value: Vec::new(),
							}),
							append: Some(false),
							..Default::default()
						}],
						remove_headers: vec![],
					}),
					..Default::default()
				}),
			},
		)),
		..Default::default()
	})
}

fn echo_request_body_response(body: &HttpBody) -> Result<ProcessingResponse, Status> {
	Ok(ProcessingResponse {
		response: Some(processing_response::Response::RequestBody(
			proto::BodyResponse {
				response: Some(CommonResponse {
					body_mutation: Some(BodyMutation {
						mutation: Some(body_mutation::Mutation::StreamedResponse(
							proto::StreamedBodyResponse {
								body: body.body.clone(),
								end_of_stream: body.end_of_stream,
							},
						)),
					}),
					..Default::default()
				}),
			},
		)),
		..Default::default()
	})
}

fn empty_response_headers_response() -> Result<ProcessingResponse, Status> {
	Ok(ProcessingResponse {
		response: Some(processing_response::Response::ResponseHeaders(
			proto::HeadersResponse {
				response: Some(CommonResponse::default()),
			},
		)),
		..Default::default()
	})
}

fn echo_response_body_response(body: &HttpBody) -> Result<ProcessingResponse, Status> {
	Ok(ProcessingResponse {
		response: Some(processing_response::Response::ResponseBody(
			proto::BodyResponse {
				response: Some(CommonResponse {
					body_mutation: Some(BodyMutation {
						mutation: Some(body_mutation::Mutation::StreamedResponse(
							proto::StreamedBodyResponse {
								body: body.body.clone(),
								end_of_stream: body.end_of_stream,
							},
						)),
					}),
					..Default::default()
				}),
			},
		)),
		..Default::default()
	})
}

async fn start_ext_proc_server(
	target: SocketAddr,
	request_headers_seen: Arc<AtomicUsize>,
) -> anyhow::Result<ExtProcServerHandle> {
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
	let address = listener.local_addr()?;
	let incoming = TcpListenerStream::new(listener);
	let service = StandaloneEppMock::new(target.to_string(), request_headers_seen);
	let task = tokio::spawn(async move {
		tonic::transport::Server::builder()
			.add_service(ExternalProcessorServer::new(service))
			.serve_with_incoming(incoming)
			.await
			.expect("ext-proc server should run");
	});
	Ok(ExtProcServerHandle { address, task })
}

async fn start_backend(name: &'static str) -> anyhow::Result<MockServer> {
	let mock = MockServer::start().await;
	Mock::given(wiremock::matchers::path_regex("/.*"))
		.respond_with(ResponseTemplate::new(200).set_body_string(name))
		.mount(&mock)
		.await;
	Ok(mock)
}

fn standalone_config(ext_proc_addr: SocketAddr, backend_ports: &[u16]) -> String {
	let workloads = backend_ports
		.iter()
		.enumerate()
		.map(|(idx, port)| {
			format!(
				r#"  - uid: "backend-{idx}"
    name: "backend-{idx}"
    namespace: "default"
    workloadIps: ["127.0.0.1"]
    services:
      {SERVICE_REF}:
        "{SERVICE_PORT}": {port}
"#,
			)
		})
		.collect::<Vec<_>>()
		.join("");

	format!(
		r#"config: {{}}
workloads:
{workloads}services:
  - name: "model-service"
    namespace: "default"
    hostname: "{SERVICE_NAME}"
    vips:
      - "/127.0.0.1"
    ports:
      "{SERVICE_PORT}": 0
binds:
- port: $PORT
  listeners:
  - name: default
    protocol: HTTP
    routes:
    - name: default
      backends:
      - service:
          name: {SERVICE_REF}
          port: {SERVICE_PORT}
        policies:
          inferenceRouting:
            endpointPicker:
              host: {ext_proc_addr}
"#
	)
}

async fn read_body(resp: agentgateway::http::Response) -> anyhow::Result<String> {
	let body = resp.into_body().collect().await?.to_bytes();
	Ok(String::from_utf8(body.to_vec())?)
}

#[tokio::test]
async fn standalone_inference_routing_uses_epp_selected_service_endpoint() -> anyhow::Result<()> {
	let backend_a = start_backend("backend-a").await?;
	let backend_b = start_backend("backend-b").await?;
	let request_headers_seen = Arc::new(AtomicUsize::new(0));
	let ext_proc = start_ext_proc_server(*backend_b.address(), request_headers_seen.clone()).await?;
	let gw = AgentGateway::new(standalone_config(
		ext_proc.address,
		&[backend_a.address().port(), backend_b.address().port()],
	))
	.await?;

	for _ in 0..12 {
		let resp = gw.send_request(Method::GET, "http://localhost/infer").await;
		assert_eq!(resp.status(), StatusCode::OK);
		assert_eq!(read_body(resp).await?, "backend-b");
	}

	assert_eq!(
		request_headers_seen.load(Ordering::SeqCst),
		12,
		"each request should consult the local EPP",
	);
	assert_eq!(
		backend_a
			.received_requests()
			.await
			.expect("backend-a recording should be enabled")
			.len(),
		0,
		"non-selected service endpoints should not receive traffic",
	);
	assert_eq!(
		backend_b
			.received_requests()
			.await
			.expect("backend-b recording should be enabled")
			.len(),
		12,
		"EPP-selected endpoint should receive all traffic",
	);

	gw.shutdown().await;
	Ok(())
}

#[tokio::test]
async fn standalone_inference_routing_rejects_endpoint_outside_service() -> anyhow::Result<()> {
	let backend = start_backend("backend-a").await?;
	let request_headers_seen = Arc::new(AtomicUsize::new(0));
	let ext_proc =
		start_ext_proc_server("127.0.0.1:65535".parse()?, request_headers_seen.clone()).await?;
	let gw = AgentGateway::new(standalone_config(
		ext_proc.address,
		&[backend.address().port()],
	))
	.await?;

	let resp = gw.send_request(Method::GET, "http://localhost/infer").await;
	assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
	assert_eq!(
		request_headers_seen.load(Ordering::SeqCst),
		1,
		"gateway should consult EPP before rejecting the request",
	);
	assert_eq!(
		backend
			.received_requests()
			.await
			.expect("backend recording should be enabled")
			.len(),
		0,
		"gateway should not forward traffic to a non-selected endpoint",
	);

	gw.shutdown().await;
	Ok(())
}
