use ::http::Method;
use hyper_util::client::legacy::Client;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tonic::Status;
use wiremock::MockServer;

use crate::http::ext_proc::proto;
use crate::http::ext_proc::proto::{
	BodyMutation, CommonResponse, HeaderMutation, HeaderValue, HeaderValueOption, HttpHeaders,
	ProcessingResponse, body_mutation,
};
use crate::http::{Body, ext_proc};
use crate::test_helpers::extprocmock::{
	ExtProcMock, ExtProcMockInstance, Handler, immediate_response, request_body_response,
	request_header_response, response_body_response,
};
use crate::test_helpers::proxymock::*;
use crate::types::agent::{Policy, PolicyTarget, SimpleBackendReference, TargetedPolicy};
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
	assert_eq!(body.as_ref(), b"ext_proc failed: no more responses");
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
	let res = send_request_body(io, Method::POST, "http://lo", b"request").await;
	assert_eq!(res.status(), 200);
	let body = read_body(res.into_body()).await;
	assert_eq!(body.body.as_ref(), b"request");
}

pub async fn setup_ext_proc_mock<T: Handler + Send + Sync + 'static>(
	mock: MockServer,
	failure_mode: ext_proc::FailureMode,
	mock_ext_proc: ExtProcMock<T>,
	config: &str,
) -> (
	MockServer,
	ExtProcMockInstance,
	TestBind,
	Client<MemoryConnector, Body>,
) {
	let ext_proc = mock_ext_proc.spawn().await;

	let t = setup_proxy_test(config)
		.unwrap()
		.with_backend(*mock.address())
		.with_backend(ext_proc.address)
		.with_bind(simple_bind(basic_route(*mock.address())))
		.with_policy(TargetedPolicy {
			name: strng::new("ext_proc"),
			target: PolicyTarget::Route("route".into()),
			policy: Policy::ExtProc(ext_proc::ExtProc {
				target: Arc::new(SimpleBackendReference::Backend(
					ext_proc.address.to_string().into(),
				)),
				failure_mode,
			}),
		});
	let io = t.serve_http(strng::new("bind"));
	(mock, ext_proc, t, io)
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
								raw_value: b"my-model-name".to_vec(),
							}),
							append: None,
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
