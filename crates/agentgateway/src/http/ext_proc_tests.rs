use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ::http::{Method, Request};
use hyper_util::client::legacy::Client;
use protos::envoy::service::ext_proc::v3::processing_response;
use serde_json::json;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tonic::Status;
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::cel::Expression;
use crate::http::ext_proc::proto::header_value_option::HeaderAppendAction;
use crate::http::ext_proc::proto::{
	BodyMutation, CommonResponse, HeaderMutation, HeaderValue, HeaderValueOption, HttpHeaders,
	ProcessingResponse, body_mutation,
};
use crate::http::ext_proc::{ExtProcDynamicMetadata, proto};
use crate::http::{Body, ext_proc};
use crate::test_helpers::MockInstance;
use crate::test_helpers::extprocmock::{
	ExtProcMock, Handler, immediate_response, request_body_response, request_header_response,
	response_body_response, response_header_response,
};
use crate::test_helpers::proxymock::*;
use crate::*;

#[tokio::test]
async fn nop_ext_proc() {
	let mock = body_mock(b"").await;
	let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock(
		mock,
		ext_proc::FailureMode::FailClosed,
		ExtProcMock::new(NopExtProc::default),
		"{}",
	)
	.await;
	let res = send_request(io, Method::POST, "http://lo").await;
	assert_eq!(res.status(), 200);
	let body = read_body_raw(res.into_body()).await;
	assert_eq!(body.as_ref(), b"");
}

#[tokio::test]
async fn nop_ext_proc_body() {
	let mock = body_mock(b"original").await;
	let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock(
		mock,
		ext_proc::FailureMode::FailClosed,
		ExtProcMock::new(NopExtProc::default),
		"{}",
	)
	.await;
	let res = send_request_body(io, Method::GET, "http://lo", b"request").await;
	assert_eq!(res.status(), 200);
	let body = read_body_raw(res.into_body()).await;
	// Server returns no body
	assert_eq!(body.as_ref(), b"");
}

#[tokio::test]
async fn body_based_router() {
	let mock = simple_mock().await;
	let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock(
		mock,
		ext_proc::FailureMode::FailClosed,
		ExtProcMock::new(|| BBRExtProc::new(false)),
		"{}",
	)
	.await;
	let res = send_request_body(io, Method::POST, "http://lo", b"request").await;
	assert_eq!(res.status(), 200);
	let body = read_body(res.into_body()).await;
	assert_eq!(
		body
			.headers
			.get("x-gateway-model-name")
			.unwrap()
			.to_str()
			.unwrap(),
		"my-model-name"
	);
}

#[tokio::test]
async fn body_based_router_buffer_body() {
	let mock = simple_mock().await;
	let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock(
		mock,
		ext_proc::FailureMode::FailClosed,
		ExtProcMock::new(|| BBRExtProc::new(true)),
		"{}",
	)
	.await;
	let res = send_request_body(io, Method::POST, "http://lo", b"request").await;
	assert_eq!(res.status(), 200);
	let body = read_body(res.into_body()).await;
	assert_eq!(
		body
			.headers
			.get("x-gateway-model-name")
			.unwrap()
			.to_str()
			.unwrap(),
		"my-model-name"
	);
}

#[tokio::test]
async fn immediate_response_request() {
	let mock = simple_mock().await;
	let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock(
		mock,
		ext_proc::FailureMode::FailClosed,
		ExtProcMock::new(ImmediateResponseExtProc::default),
		"{}",
	)
	.await;
	let res = send_request_body(io, Method::POST, "http://lo", b"request").await;
	assert_eq!(res.status(), 202);
	let body = read_body_raw(res.into_body()).await;
	assert_eq!(body.as_ref(), b"immediate");
}

#[tokio::test]
async fn immediate_response_request_body_is_deferred_to_response() {
	let mock = simple_mock().await;
	let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock(
		mock,
		ext_proc::FailureMode::FailClosed,
		ExtProcMock::new(ImmediateResponseRequestBodyExtProc::default),
		"{}",
	)
	.await;
	let res = send_request_body(io, Method::POST, "http://lo", b"request").await;
	assert_eq!(res.status(), 403);
	let body = read_body_raw(res.into_body()).await;
	assert_eq!(body.as_ref(), b"Access denied");
}

#[tokio::test]
async fn immediate_response_response() {
	let mock = simple_mock().await;
	let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock(
		mock,
		ext_proc::FailureMode::FailClosed,
		ExtProcMock::new(ImmediateResponseExtProcResponse::default),
		"{}",
	)
	.await;
	let res = send_request_body(io, Method::POST, "http://lo", b"request").await;
	assert_eq!(res.status(), 202);
	let body = read_body_raw(res.into_body()).await;
	assert_eq!(body.as_ref(), b"immediate");
}

#[tokio::test]
async fn failure_fail_closed() {
	let mock = simple_mock().await;
	let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock(
		mock,
		ext_proc::FailureMode::FailClosed,
		ExtProcMock::new(FailureExtProcResponse::default),
		"{}",
	)
	.await;
	let res = send_request_body(io, Method::POST, "http://lo", b"request").await;
	assert_eq!(res.status(), 500);
	let body = read_body_raw(res.into_body()).await;
	assert!(body.as_ref().starts_with(b"ext_proc failed:"));
}

#[tokio::test]
async fn failure_fail_open_body() {
	let mock = simple_mock().await;
	let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock(
		mock,
		ext_proc::FailureMode::FailOpen,
		ExtProcMock::new(FailureExtProcResponse::default),
		"{}",
	)
	.await;

	// If we have a body, we should NOT fail open
	let res = send_request_body(io, Method::POST, "http://lo", b"request").await;
	assert_eq!(res.status(), 500);
}

#[tokio::test]
async fn failure_fail_open() {
	let mock = simple_mock().await;
	let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock(
		mock,
		ext_proc::FailureMode::FailOpen,
		ExtProcMock::new(FailureExtProcResponse::default),
		"{}",
	)
	.await;

	let res = send_request(io, Method::POST, "http://lo").await;
	assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn dynamic_metadata() {
	let mock = body_mock(b"").await;
	let (_mock, _ext_proc, mut bind, _io) = setup_ext_proc_mock(
		mock,
		ext_proc::FailureMode::FailClosed,
		ExtProcMock::new(DynamicMetadataExtProc::default),
		"{}",
	)
	.await;
	bind
		.attach_route_policy(json!({
			"transformations": {
				"response": {
					"set": {
						"x-extproc-metadata": "extproc.some[0]",
					},
				},
			},
		}))
		.await;
	let io = bind.serve_http(strng::new("bind"));
	let res = send_request(io, Method::POST, "http://lo").await;
	assert_eq!(res.status(), 200);
	assert_eq!(
		res
			.headers()
			.get("x-extproc-metadata")
			.unwrap()
			.to_str()
			.unwrap(),
		"a"
	);
	let body = read_body_raw(res.into_body()).await;
	assert_eq!(body.as_ref(), b"");
}

pub async fn setup_ext_proc_mock<T: Handler + Send + Sync + 'static>(
	mock: MockServer,
	failure_mode: ext_proc::FailureMode,
	mock_ext_proc: ExtProcMock<T>,
	config: &str,
) -> (
	MockServer,
	MockInstance,
	TestBind,
	Client<MemoryConnector, Body>,
) {
	setup_ext_proc_mock_with_meta(mock, failure_mode, mock_ext_proc, config, None, None, None).await
}

pub async fn setup_ext_proc_mock_with_meta<T: Handler + Send + Sync + 'static>(
	mock: MockServer,
	failure_mode: ext_proc::FailureMode,
	mock_ext_proc: ExtProcMock<T>,
	config: &str,
	metadata_context: Option<HashMap<String, HashMap<String, Arc<Expression>>>>,
	request_attributes: Option<HashMap<String, Arc<Expression>>>,
	response_attributes: Option<HashMap<String, Arc<Expression>>>,
) -> (
	MockServer,
	MockInstance,
	TestBind,
	Client<MemoryConnector, Body>,
) {
	let ext_proc = mock_ext_proc.spawn().await;

	let t = setup_proxy_test(config)
		.unwrap()
		.with_backend(*mock.address())
		.with_backend(ext_proc.address)
		.with_bind(simple_bind())
		.with_route(basic_route(*mock.address()))
		.attach_route_policy_builder(json!({
			"extProc": {
				"host": ext_proc.address,
				"failureMode": failure_mode,
				"metadataContext": metadata_context,
				"requestAttributes": request_attributes,
				"responseAttributes": response_attributes,
			}
		}))
		.await;
	let io = t.serve_http(strng::new("bind"));
	(mock, ext_proc, t, io)
}

const STANDALONE_SERVICE_NAME: &str = "model-service.default.svc.cluster.local";
const STANDALONE_SERVICE_REF: &str = "default/model-service.default.svc.cluster.local";
const STANDALONE_SERVICE_PORT: u16 = 8000;

#[derive(Clone)]
struct StandaloneInferenceRouter {
	target: SocketAddr,
	request_headers_seen: Arc<AtomicUsize>,
}

#[derive(Debug)]
struct NoDestinationInferenceRouter {
	request_headers_seen: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Handler for StandaloneInferenceRouter {
	async fn handle_request_headers(
		&mut self,
		_headers: &HttpHeaders,
		sender: &Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		self.request_headers_seen.fetch_add(1, Ordering::SeqCst);
		let _ = sender
			.send(request_header_response(Some(CommonResponse {
				header_mutation: Some(HeaderMutation {
					set_headers: vec![HeaderValueOption {
						header: Some(HeaderValue {
							key: "x-gateway-destination-endpoint".to_string(),
							value: self.target.to_string(),
							raw_value: Vec::new(),
						}),
						append: Some(false),
						..Default::default()
					}],
					remove_headers: vec![],
				}),
				..Default::default()
			})))
			.await;
		Ok(())
	}
}

#[async_trait::async_trait]
impl Handler for NoDestinationInferenceRouter {
	async fn handle_request_headers(
		&mut self,
		_headers: &HttpHeaders,
		sender: &Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		self.request_headers_seen.fetch_add(1, Ordering::SeqCst);
		let _ = sender.send(request_header_response(None)).await;
		Ok(())
	}
}

async fn named_backend(body: &'static str) -> MockServer {
	let mock = MockServer::start().await;
	Mock::given(wiremock::matchers::path_regex("/.*"))
		.respond_with(ResponseTemplate::new(200).set_body_string(body))
		.mount(&mock)
		.await;
	mock
}

fn configure_standalone_service(t: &TestBind) {
	use crate::types::discovery::{NetworkAddress, Service};

	let service = Service {
		name: "model-service".into(),
		namespace: "default".into(),
		hostname: STANDALONE_SERVICE_NAME.into(),
		vips: vec![NetworkAddress {
			network: strng::EMPTY,
			address: "127.0.0.1".parse().unwrap(),
		}],
		ports: HashMap::from([(STANDALONE_SERVICE_PORT, STANDALONE_SERVICE_PORT)]),
		..Default::default()
	};

	t.pi
		.stores
		.discovery
		.sync_local(vec![service], vec![], Default::default())
		.unwrap();
}

async fn setup_inference_routing_mock(
	target: SocketAddr,
	request_headers_seen: Arc<AtomicUsize>,
) -> (MockInstance, TestBind, Client<MemoryConnector, Body>) {
	let ext_proc = ExtProcMock::new(move || StandaloneInferenceRouter {
		target,
		request_headers_seen: request_headers_seen.clone(),
	})
	.spawn()
	.await;

	let mut t = setup_proxy_test("{}").unwrap().with_bind(simple_bind());
	configure_standalone_service(&t);
	t.attach_route(json!({
		"name": "standalone-epp",
		"backends": [
			{
				"service": {
					"name": STANDALONE_SERVICE_REF,
					"port": STANDALONE_SERVICE_PORT,
				},
				"policies": {
					"inferenceRouting": {
						"endpointPicker": {
							"host": ext_proc.address.to_string(),
						},
					},
				},
			}
		],
	}))
	.await;
	let io = t.serve_http(BIND_KEY);
	(ext_proc, t, io)
}

#[tokio::test]
async fn standalone_inference_routing_uses_epp_selected_destination_without_local_endpoints() {
	let backend_a = named_backend("backend-a").await;
	let backend_b = named_backend("backend-b").await;
	let request_headers_seen = Arc::new(AtomicUsize::new(0));
	let (_ext_proc, _bind, io) =
		setup_inference_routing_mock(*backend_b.address(), request_headers_seen.clone()).await;

	let res = send_request(io, Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);
	let body = read_body_raw(res.into_body()).await;
	assert_eq!(body.as_ref(), b"backend-b");
	assert_eq!(
		request_headers_seen.load(Ordering::SeqCst),
		1,
		"request should consult the local EPP",
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
		1,
		"EPP-selected endpoint should receive traffic",
	);
}

#[tokio::test]
async fn standalone_inference_routing_requires_epp_selected_destination() {
	let request_headers_seen = Arc::new(AtomicUsize::new(0));
	let request_headers_seen_for_handler = request_headers_seen.clone();
	let ext_proc = ExtProcMock::new(move || NoDestinationInferenceRouter {
		request_headers_seen: request_headers_seen_for_handler.clone(),
	})
	.spawn()
	.await;

	let mut t = setup_proxy_test("{}").unwrap().with_bind(simple_bind());
	configure_standalone_service(&t);
	t.attach_route(json!({
		"name": "standalone-epp",
		"backends": [
			{
				"service": {
					"name": STANDALONE_SERVICE_REF,
					"port": STANDALONE_SERVICE_PORT,
				},
				"policies": {
					"inferenceRouting": {
						"endpointPicker": {
							"host": ext_proc.address.to_string(),
						},
					},
				},
			}
		],
	}))
	.await;
	let io = t.serve_http(BIND_KEY);

	let res = send_request(io, Method::GET, "http://lo").await;
	assert_eq!(res.status(), 503);
	assert_eq!(
		request_headers_seen.load(Ordering::SeqCst),
		1,
		"gateway should consult EPP before rejecting the request",
	);
}

#[derive(Debug, Default)]
struct NopExtProc {
	sent_req_body: bool,
	sent_resp_body: bool,
}

#[async_trait::async_trait]
impl Handler for NopExtProc {
	async fn handle_request_body(
		&mut self,
		_body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		if !self.sent_req_body {
			let _ = sender.send(request_body_response(None)).await;
		}
		self.sent_req_body = true;
		Ok(())
	}

	async fn handle_response_body(
		&mut self,
		_body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		if !self.sent_resp_body {
			let _ = sender.send(response_body_response(None)).await;
		}
		self.sent_resp_body = true;
		Ok(())
	}
}

#[derive(Debug, Default)]
struct DynamicMetadataExtProc {
	sent_req_body: bool,
	sent_resp_body: bool,
}

#[async_trait::async_trait]
impl Handler for DynamicMetadataExtProc {
	async fn handle_request_headers(
		&mut self,
		_headers: &HttpHeaders,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		use prost_wkt_types::Value;
		use prost_wkt_types::value::Kind;

		let _ = sender
			.send(Ok(ProcessingResponse {
				response: Some(processing_response::Response::RequestHeaders(
					proto::HeadersResponse { response: None },
				)),
				dynamic_metadata: Some(prost_wkt_types::Struct {
					fields: HashMap::from([(
						"some".to_string(),
						Value {
							kind: Some(Kind::ListValue(prost_wkt_types::ListValue {
								values: vec![
									Value {
										kind: Some(Kind::StringValue("a".to_string())),
									},
									Value {
										kind: Some(Kind::StringValue("b".to_string())),
									},
								],
							})),
						},
					)]),
				}),
				..Default::default()
			}))
			.await;
		Ok(())
	}
	async fn handle_request_body(
		&mut self,
		_body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		if !self.sent_req_body {
			let _ = sender.send(request_body_response(None)).await;
		}
		self.sent_req_body = true;
		Ok(())
	}

	async fn handle_response_body(
		&mut self,
		_body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		if !self.sent_resp_body {
			let _ = sender.send(response_body_response(None)).await;
		}
		self.sent_resp_body = true;
		Ok(())
	}
}

/// Simulate GIE body based router
#[derive(Debug)]
struct BBRExtProc {
	req_body: Vec<u8>,
	buffer_body: bool,
	res_body: Vec<u8>,
}

impl BBRExtProc {
	pub fn new(buffer_body: bool) -> Self {
		Self {
			buffer_body,
			req_body: Default::default(),
			res_body: Default::default(),
		}
	}
}

// https://github.com/kubernetes-sigs/gateway-api-inference-extension/blob/2a187ea174ed2fafd22e6aff8cb13e532dc7604e/pkg/bbr/handlers/server.go#L74
#[async_trait::async_trait]
impl Handler for BBRExtProc {
	async fn handle_request_headers(
		&mut self,
		headers: &HttpHeaders,
		sender: &Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		if headers.end_of_stream {
			let _ = sender.send(request_header_response(None)).await;
		}
		Ok(())
	}

	async fn handle_request_body(
		&mut self,
		body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		self.req_body.extend_from_slice(&body.body);
		if body.end_of_stream {
			let _ = sender
				.send(request_header_response(Some(CommonResponse {
					header_mutation: Some(HeaderMutation {
						set_headers: vec![HeaderValueOption {
							header: Some(HeaderValue {
								key: "X-Gateway-Model-Name".to_string(),
								value: String::new(),
								raw_value: b"my-model-name".to_vec(),
							}),
							append: None,
							append_action: 0,
						}],
						remove_headers: vec![],
					}),
					..Default::default()
				})))
				.await;
			let _ = sender
				.send(request_body_response(Some(CommonResponse {
					body_mutation: Some(BodyMutation {
						mutation: Some(body_mutation::Mutation::StreamedResponse(
							proto::StreamedBodyResponse {
								body: self.req_body.clone(),
								end_of_stream: true,
							},
						)),
					}),
					..Default::default()
				})))
				.await;
		}
		Ok(())
	}

	async fn handle_response_body(
		&mut self,
		body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		if self.buffer_body {
			self.res_body.extend_from_slice(&body.body);
			if body.end_of_stream {
				let _ = sender
					.send(response_body_response(Some(CommonResponse {
						body_mutation: Some(BodyMutation {
							mutation: Some(body_mutation::Mutation::StreamedResponse(
								proto::StreamedBodyResponse {
									body: self.res_body.clone(),
									end_of_stream: true,
								},
							)),
						}),
						..Default::default()
					})))
					.await;
			}
		} else {
			let _ = sender
				.send(response_body_response(Some(CommonResponse {
					body_mutation: Some(BodyMutation {
						mutation: Some(body_mutation::Mutation::StreamedResponse(
							proto::StreamedBodyResponse {
								body: body.body.clone(),
								end_of_stream: body.end_of_stream,
							},
						)),
					}),
					..Default::default()
				})))
				.await;
		}
		Ok(())
	}
}

#[derive(Debug, Default)]
struct ImmediateResponseExtProc {}

#[async_trait::async_trait]
impl Handler for ImmediateResponseExtProc {
	async fn handle_request_headers(
		&mut self,
		_: &HttpHeaders,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender
			.send(immediate_response(proto::ImmediateResponse {
				status: Some(proto::HttpStatus { code: 202 }),
				body: "immediate".to_string(),
				headers: None,
				grpc_status: None,
				details: "".to_string(),
			}))
			.await;
		Ok(())
	}
}

#[derive(Debug, Default)]
struct ImmediateResponseRequestBodyExtProc {
	sent: bool,
}

#[async_trait::async_trait]
impl Handler for ImmediateResponseRequestBodyExtProc {
	async fn handle_request_headers(
		&mut self,
		_: &HttpHeaders,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender.send(request_header_response(None)).await;
		Ok(())
	}

	async fn handle_request_body(
		&mut self,
		_: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		if !self.sent {
			self.sent = true;
			let _ = sender
				.send(immediate_response(proto::ImmediateResponse {
					status: Some(proto::HttpStatus {
						code: proto::StatusCode::Forbidden as i32,
					}),
					body: "Access denied".to_string(),
					headers: None,
					grpc_status: None,
					details: "".to_string(),
				}))
				.await;
		}
		Ok(())
	}
}

#[derive(Debug, Default)]
struct ImmediateResponseExtProcResponse {
	sent_req_body: bool,
}

#[async_trait::async_trait]
impl Handler for ImmediateResponseExtProcResponse {
	async fn handle_request_body(
		&mut self,
		_body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		if !self.sent_req_body {
			let _ = sender.send(request_body_response(None)).await;
		}
		self.sent_req_body = true;
		Ok(())
	}

	async fn handle_response_headers(
		&mut self,
		_headers: &HttpHeaders,
		sender: &Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender
			.send(immediate_response(proto::ImmediateResponse {
				status: Some(proto::HttpStatus { code: 202 }),
				body: "immediate".to_string(),
				headers: None,
				grpc_status: None,
				details: "".to_string(),
			}))
			.await;
		Ok(())
	}
}

#[derive(Debug, Default)]
struct FailureExtProcResponse {}

#[async_trait::async_trait]
impl Handler for FailureExtProcResponse {
	async fn handle_request_headers(
		&mut self,
		_: &HttpHeaders,
		_: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		Err(Status::failed_precondition("injected test error"))
	}
}

#[test]
fn test_req_to_header_map() {
	let req = Request::builder()
		.header("host", "foo.com")
		.header("content-type", "application/json")
		.uri("/path?query=param")
		.method("GET")
		.body(http::Body::empty())
		.unwrap();
	let headers = super::req_to_header_map(&req).unwrap();
	// 2 regular headers, 4 pseudo headers (method, scheme, authority, path)
	assert_eq!(headers.headers.len(), 6);
}

#[tokio::test]
async fn header_append_action_mock() {
	let mock = mock_with_header("x-test", "existing").await;
	let handler = HeaderAppendActionExtProc::new(vec![
		(
			"x-test",
			b"new-value",
			HeaderAppendAction::AppendIfExistsOrAdd,
		),
		("x-new", b"added", HeaderAppendAction::AppendIfExistsOrAdd),
	]);
	let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock(
		mock,
		ext_proc::FailureMode::FailClosed,
		ExtProcMock::new(move || handler.clone()),
		"{}",
	)
	.await;
	let res = send_request(io, Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);

	let values: Vec<_> = res.headers().get_all("x-test").iter().collect();
	assert_eq!(values.len(), 2);
	assert_eq!(values[0], "existing");
	assert_eq!(values[1], "new-value");
	assert_eq!(res.headers().get("x-new").unwrap(), "added");
}

#[derive(Debug, Clone)]
struct HeaderAppendActionExtProc {
	headers: Vec<(String, Vec<u8>, HeaderAppendAction)>,
}

impl HeaderAppendActionExtProc {
	fn new(headers: Vec<(&str, &[u8], HeaderAppendAction)>) -> Self {
		Self {
			headers: headers
				.into_iter()
				.map(|(k, v, a)| (k.to_string(), v.to_vec(), a))
				.collect(),
		}
	}
}

#[async_trait::async_trait]
impl Handler for HeaderAppendActionExtProc {
	async fn handle_response_headers(
		&mut self,
		_: &HttpHeaders,
		sender: &Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let set_headers = self
			.headers
			.iter()
			.map(|(key, value, action)| HeaderValueOption {
				header: Some(HeaderValue {
					key: key.clone(),
					value: String::new(),
					raw_value: value.clone(),
				}),
				append: Some(true),
				append_action: (*action).into(),
			})
			.collect();

		let _ = sender
			.send(response_header_response(Some(CommonResponse {
				header_mutation: Some(HeaderMutation {
					set_headers,
					remove_headers: vec![],
				}),
				..Default::default()
			})))
			.await;
		Ok(())
	}
}

async fn mock_with_header(header_name: &str, header_value: &str) -> MockServer {
	let header_name = header_name.to_string();
	let header_value = header_value.to_string();
	let mock = wiremock::MockServer::start().await;
	wiremock::Mock::given(wiremock::matchers::path_regex("/.*"))
		.respond_with(move |_: &wiremock::Request| {
			wiremock::ResponseTemplate::new(200)
				.insert_header(header_name.as_str(), header_value.as_str())
		})
		.mount(&mock)
		.await;
	mock
}

#[test]
fn test_dynamic_metadata_extraction() {
	let mut metadata = ExtProcDynamicMetadata::default();

	metadata
		.0
		.insert("user_id".to_string(), serde_json::json!("12345"));
	metadata
		.0
		.insert("role".to_string(), serde_json::json!("admin"));
	assert_eq!(metadata.0.get("user_id").unwrap(), "12345");
	assert_eq!(metadata.0.get("role").unwrap(), "admin");
}

mod extract_dynamic_metadata_tests {
	use std::collections::HashMap;

	use prost_wkt_types::value::Kind;
	use prost_wkt_types::{Struct, Value};

	use super::super::extract_dynamic_metadata;
	use super::*;

	#[test]
	fn test_extract_creates_extension() {
		let metadata = Struct {
			fields: [(
				"user_id".to_string(),
				Value {
					kind: Some(Kind::StringValue("12345".to_string())),
				},
			)]
			.into(),
		};
		let mut req = ::http::Request::builder()
			.uri("http://test.com")
			.body(Body::empty())
			.unwrap();

		extract_dynamic_metadata(&mut req, &metadata).unwrap();

		let extracted = req
			.extensions()
			.get::<ExtProcDynamicMetadata>()
			.expect("metadata should be in extensions");
		assert_eq!(
			extracted.0.get("user_id"),
			Some(&serde_json::json!("12345"))
		);
	}

	#[test]
	fn test_extract_merges_with_existing() {
		let mut req = ::http::Request::builder()
			.uri("http://test.com")
			.body(Body::empty())
			.unwrap();

		let existing = ExtProcDynamicMetadata(
			[("existing".to_string(), serde_json::json!("value"))]
				.into_iter()
				.collect(),
		);
		req.extensions_mut().insert(existing);

		let metadata = Struct {
			fields: [(
				"new_key".to_string(),
				Value {
					kind: Some(Kind::StringValue("new_value".to_string())),
				},
			)]
			.into(),
		};
		extract_dynamic_metadata(&mut req, &metadata).unwrap();

		let extracted = req.extensions().get::<ExtProcDynamicMetadata>().unwrap();
		assert_eq!(extracted.0.len(), 2);
		assert_eq!(
			extracted.0.get("existing"),
			Some(&serde_json::json!("value"))
		);
		assert_eq!(
			extracted.0.get("new_key"),
			Some(&serde_json::json!("new_value"))
		);
	}

	#[test]
	fn test_extract_overwrites_existing_keys() {
		let mut req = ::http::Request::builder()
			.uri("http://test.com")
			.body(Body::empty())
			.unwrap();

		let existing = ExtProcDynamicMetadata(
			[("key".to_string(), serde_json::json!("old_value"))]
				.into_iter()
				.collect(),
		);
		req.extensions_mut().insert(existing);

		let metadata = Struct {
			fields: [(
				"key".to_string(),
				Value {
					kind: Some(Kind::StringValue("new_value".to_string())),
				},
			)]
			.into(),
		};
		extract_dynamic_metadata(&mut req, &metadata).unwrap();

		let extracted = req.extensions().get::<ExtProcDynamicMetadata>().unwrap();
		assert_eq!(extracted.0.len(), 1);
		assert_eq!(
			extracted.0.get("key"),
			Some(&serde_json::json!("new_value"))
		);
	}

	#[test]
	fn test_extract_empty_metadata_no_extension() {
		let metadata = Struct {
			fields: HashMap::new(),
		};
		let mut req = ::http::Request::builder()
			.uri("http://test.com")
			.body(Body::empty())
			.unwrap();

		extract_dynamic_metadata(&mut req, &metadata).unwrap();

		assert!(req.extensions().get::<ExtProcDynamicMetadata>().is_none());
	}

	#[test]
	fn test_extract_string_and_bool_values() {
		let metadata = Struct {
			fields: [
				(
					"string_val".to_string(),
					Value {
						kind: Some(Kind::StringValue("hello".to_string())),
					},
				),
				(
					"bool_true".to_string(),
					Value {
						kind: Some(Kind::BoolValue(true)),
					},
				),
				(
					"bool_false".to_string(),
					Value {
						kind: Some(Kind::BoolValue(false)),
					},
				),
			]
			.into(),
		};

		let mut req = ::http::Request::builder()
			.uri("http://test.com")
			.body(Body::empty())
			.unwrap();

		extract_dynamic_metadata(&mut req, &metadata).unwrap();

		let extracted = req.extensions().get::<ExtProcDynamicMetadata>().unwrap();

		assert_eq!(extracted.0.len(), 3);
		assert_eq!(
			extracted.0.get("string_val"),
			Some(&serde_json::json!("hello"))
		);
		assert_eq!(extracted.0.get("bool_true"), Some(&serde_json::json!(true)));
		assert_eq!(
			extracted.0.get("bool_false"),
			Some(&serde_json::json!(false))
		);
	}

	#[test]
	fn test_extract_multiple_calls_accumulate() {
		let mut req = ::http::Request::builder()
			.uri("http://test.com")
			.body(Body::empty())
			.unwrap();

		let metadata1 = Struct {
			fields: [(
				"key1".to_string(),
				Value {
					kind: Some(Kind::StringValue("value1".to_string())),
				},
			)]
			.into(),
		};
		extract_dynamic_metadata(&mut req, &metadata1).unwrap();

		let metadata2 = Struct {
			fields: [(
				"key2".to_string(),
				Value {
					kind: Some(Kind::BoolValue(true)),
				},
			)]
			.into(),
		};
		extract_dynamic_metadata(&mut req, &metadata2).unwrap();

		let extracted = req.extensions().get::<ExtProcDynamicMetadata>().unwrap();
		assert_eq!(extracted.0.len(), 2);
		assert_eq!(extracted.0.get("key1"), Some(&serde_json::json!("value1")));
		assert_eq!(extracted.0.get("key2"), Some(&serde_json::json!(true)));
	}
}

#[derive(Clone)]
struct MetadataTracker {
	requests: Arc<std::sync::Mutex<Vec<proto::ProcessingRequest>>>,
}

impl MetadataTracker {
	fn new() -> Self {
		Self {
			requests: Arc::new(std::sync::Mutex::new(Vec::new())),
		}
	}
}

#[async_trait::async_trait]
impl Handler for MetadataTracker {
	async fn on_request(&mut self, request: &proto::ProcessingRequest) {
		self.requests.lock().unwrap().push(request.clone());
	}
}

#[tokio::test]
async fn test_attributes_empty_without_config() {
	let mock = simple_mock().await;
	let tracker = MetadataTracker::new();
	let requests = tracker.requests.clone();

	let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock(
		mock,
		ext_proc::FailureMode::FailClosed,
		ExtProcMock::new(move || tracker.clone()),
		"{}",
	)
	.await;
	let res = send_request_body(io, Method::POST, "http://lo", b"request body").await;
	assert_eq!(res.status(), 200);

	let captured = requests.lock().unwrap();
	assert!(captured.len() >= 2);

	for (i, req) in captured.iter().enumerate() {
		assert!(
			req.attributes.is_empty(),
			"Message {} should have empty attributes when no config",
			i
		);
	}
}

struct DynamicMetadataResponder;

#[async_trait::async_trait]
impl Handler for DynamicMetadataResponder {
	async fn handle_request_headers(
		&mut self,
		_headers: &HttpHeaders,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		use prost_wkt_types::value::Kind;
		use prost_wkt_types::{Struct, Value};

		use crate::test_helpers::extprocmock::request_header_response_with_dynamic_metadata;

		let metadata = Struct {
			fields: [
				(
					"auth_user".to_string(),
					Value {
						kind: Some(Kind::StringValue("test-user".to_string())),
					},
				),
				(
					"is_admin".to_string(),
					Value {
						kind: Some(Kind::BoolValue(true)),
					},
				),
			]
			.into(),
		};
		let _ = sender
			.send(request_header_response_with_dynamic_metadata(
				None, metadata,
			))
			.await;
		Ok(())
	}
}

#[tokio::test]
async fn test_dynamic_metadata_response() {
	let mock = simple_mock().await;
	let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock(
		mock,
		ext_proc::FailureMode::FailClosed,
		ExtProcMock::new(|| DynamicMetadataResponder),
		"{}",
	)
	.await;
	let res = send_request(io, Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn test_cel_metadata_context_evaluation() {
	let mock = simple_mock().await;
	let tracker = MetadataTracker::new();
	let requests = tracker.requests.clone();

	let meta = HashMap::from([(
		"envoy.filters.http.ext_proc".to_string(),
		[
			(
				"path".to_string(),
				Arc::new(Expression::new_strict("request.path").unwrap()),
			),
			(
				"static".to_string(),
				Arc::new(Expression::new_strict("'value'").unwrap()),
			),
		]
		.into(),
	)]);

	let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_meta(
		mock,
		ext_proc::FailureMode::FailClosed,
		ExtProcMock::new(move || tracker.clone()),
		"{}",
		Some(meta),
		None,
		None,
	)
	.await;

	let res = send_request(io, Method::GET, "http://lo/test-path").await;
	assert_eq!(res.status(), 200);

	let reqs = requests.lock().unwrap();
	assert!(!reqs.is_empty());

	let req = &reqs[0];
	let meta_ctx = req
		.metadata_context
		.as_ref()
		.expect("should have metadata_context");
	let filter_meta = meta_ctx
		.filter_metadata
		.get("envoy.filters.http.ext_proc")
		.expect("should have namespace");

	let fields = &filter_meta.fields;
	match &fields.get("path").unwrap().kind {
		Some(prost_wkt_types::value::Kind::StringValue(s)) => assert_eq!(s, "/test-path"),
		invalid => panic!("exepected a string 'path' got {:?}", invalid),
	}
	match &fields.get("static").unwrap().kind {
		Some(prost_wkt_types::value::Kind::StringValue(s)) => assert_eq!(s, "value"),
		invalid => panic!("exepected a string 'static' field got {:?}", invalid),
	}
}

#[tokio::test]
async fn test_cel_req_attributes() {
	let mock = simple_mock().await;
	let tracker = MetadataTracker::new();
	let requests = tracker.requests.clone();

	let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_meta(
		mock,
		ext_proc::FailureMode::FailClosed,
		ExtProcMock::new(move || tracker.clone()),
		"{}",
		None,
		Some(
			[(
				"method".to_string(),
				Arc::new(Expression::new_strict("request.method").unwrap()),
			)]
			.into(),
		),
		None,
	)
	.await;

	let res = send_request(io, Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);

	let req = requests.lock().unwrap();
	let headers = req
		.iter()
		.find(|r| {
			matches!(
				r.request,
				Some(proto::processing_request::Request::RequestHeaders(_))
			)
		})
		.unwrap();

	let ns_attrs = &headers
		.attributes
		.get("envoy.filters.http.ext_proc")
		.expect("envoy ext_proc namespace");

	match &ns_attrs.fields.get("method").unwrap().kind {
		Some(prost_wkt_types::value::Kind::StringValue(s)) => assert_eq!(s, "GET"),
		invalid => panic!("exepected a string got {:?}", invalid),
	}
}

#[tokio::test]
async fn test_cel_resp_attributes() {
	let mock = simple_mock().await;
	let tracker = MetadataTracker::new();
	let requests = tracker.requests.clone();

	let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_meta(
		mock,
		ext_proc::FailureMode::FailClosed,
		ExtProcMock::new(move || tracker.clone()),
		"{}",
		None,
		None,
		Some(
			[(
				"status".to_string(),
				Arc::new(Expression::new_strict("response.code").unwrap()),
			)]
			.into(),
		),
	)
	.await;

	let res = send_request(io, Method::GET, "http://lo").await;
	assert_eq!(res.status(), 200);

	let resp = requests.lock().unwrap();
	let headers = resp
		.iter()
		.find(|r| {
			matches!(
				r.request,
				Some(proto::processing_request::Request::ResponseHeaders(_))
			)
		})
		.unwrap();

	let ns_attrs = &headers
		.attributes
		.get("envoy.filters.http.ext_proc")
		.expect("envoy ext_proc namespace");

	match &ns_attrs.fields.get("status").unwrap().kind {
		Some(prost_wkt_types::value::Kind::NumberValue(n)) => assert_eq!(*n, 200.0),
		invalid => panic!("exepected a number got {:?}", invalid),
	}
}
