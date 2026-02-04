use std::collections::HashMap;
use std::sync::Arc;

use ::http::{HeaderMap, Method, Request};
use hyper_util::client::legacy::Client;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tonic::Status;
use wiremock::MockServer;

use crate::cel::Expression;
use crate::http::ext_proc::proto::header_value_option::HeaderAppendAction;
use crate::http::ext_proc::proto::{
	BodyMutation, CommonResponse, HeaderMutation, HeaderValue, HeaderValueOption, HttpHeaders,
	ProcessingResponse, body_mutation,
};
use crate::http::ext_proc::{ExtProcDynamicMetadata, proto};
use crate::http::{Body, ext_proc};
use crate::test_helpers::extprocmock::{
	ExtProcMock, ExtProcMockInstance, Handler, immediate_response, request_body_response,
	request_header_response, response_body_response, response_header_response,
};
use crate::test_helpers::proxymock::*;
use crate::types::agent::{
	PolicyTarget, RouteTarget, SimpleBackendReference, TargetedPolicy, TrafficPolicy,
};
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
	assert!(body.as_ref().starts_with(b"ext_proc failed:"));
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
			key: strng::new("ext_proc"),
			name: None,
			target: PolicyTarget::Route(RouteTarget {
				name: "route".into(),
				namespace: Default::default(),
				rule_name: None,
				kind: None,
			}),
			policy: TrafficPolicy::ExtProc(ext_proc::ExtProc {
				target: Arc::new(SimpleBackendReference::Backend(strng::format!(
					"/{}",
					ext_proc.address
				))),
				failure_mode,
				metadata_context,
				request_attributes,
				response_attributes,
			})
			.into(),
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

#[test]
fn test_default_append_action_overwrite() {
	let mut headers = HeaderMap::new();
	headers.insert("existing", "old".parse().unwrap());

	let mutation = Some(HeaderMutation {
		remove_headers: vec![],
		set_headers: vec![HeaderValueOption {
			header: Some(HeaderValue {
				key: "existing".to_string(),
				value: String::new(),
				raw_value: b"new".to_vec(),
			}),
			append: None,
			append_action: 0, // default
		}],
	});

	super::apply_header_mutations(&mut headers, mutation.as_ref()).unwrap();

	let values: Vec<_> = headers.get_all("existing").iter().collect();
	assert_eq!(values.len(), 1);
	assert_eq!(values[0], "new");
}

#[test]
fn test_append_if_exists_or_add() {
	let mut headers = HeaderMap::new();
	headers.insert("existing", "value1".parse().unwrap());

	let mutation = Some(HeaderMutation {
		remove_headers: vec![],
		set_headers: vec![
			HeaderValueOption {
				header: Some(HeaderValue {
					key: "existing".to_string(),
					value: String::new(),
					raw_value: b"value2".to_vec(),
				}),
				append: Some(true),
				append_action: HeaderAppendAction::AppendIfExistsOrAdd as i32,
			},
			HeaderValueOption {
				header: Some(HeaderValue {
					key: "new".to_string(),
					value: String::new(),
					raw_value: b"added".to_vec(),
				}),
				append: Some(true),
				append_action: HeaderAppendAction::AppendIfExistsOrAdd as i32,
			},
		],
	});

	super::apply_header_mutations(&mut headers, mutation.as_ref()).unwrap();

	let values: Vec<_> = headers.get_all("existing").iter().collect();
	assert_eq!(values.len(), 2);
	assert_eq!(values[0], "value1");
	assert_eq!(values[1], "value2");
	assert_eq!(headers.get("new").unwrap(), "added");
}

#[test]
fn test_add_if_absent() {
	let mut headers = HeaderMap::new();
	headers.insert("existing", "value1".parse().unwrap());

	let mutation = Some(HeaderMutation {
		remove_headers: vec![],
		set_headers: vec![
			HeaderValueOption {
				header: Some(HeaderValue {
					key: "existing".to_string(),
					value: String::new(),
					raw_value: b"should-not-add".to_vec(),
				}),
				append: None,
				append_action: HeaderAppendAction::AddIfAbsent as i32,
			},
			HeaderValueOption {
				header: Some(HeaderValue {
					key: "new".to_string(),
					value: String::new(),
					raw_value: b"added".to_vec(),
				}),
				append: None,
				append_action: HeaderAppendAction::AddIfAbsent as i32,
			},
		],
	});

	super::apply_header_mutations(&mut headers, mutation.as_ref()).unwrap();

	let values: Vec<_> = headers.get_all("existing").iter().collect();
	assert_eq!(values.len(), 1);
	assert_eq!(values[0], "value1");
	assert_eq!(headers.get("new").unwrap(), "added");
}

#[test]
fn test_overwrite_if_exists_or_add() {
	let mut headers = HeaderMap::new();
	headers.insert("existing", "old-value".parse().unwrap());

	let mutation = Some(HeaderMutation {
		remove_headers: vec![],
		set_headers: vec![
			HeaderValueOption {
				header: Some(HeaderValue {
					key: "existing".to_string(),
					value: String::new(),
					raw_value: b"overwritten".to_vec(),
				}),
				append: None,
				append_action: HeaderAppendAction::OverwriteIfExistsOrAdd as i32,
			},
			HeaderValueOption {
				header: Some(HeaderValue {
					key: "new".to_string(),
					value: String::new(),
					raw_value: b"added".to_vec(),
				}),
				append: None,
				append_action: HeaderAppendAction::OverwriteIfExistsOrAdd as i32,
			},
		],
	});

	super::apply_header_mutations(&mut headers, mutation.as_ref()).unwrap();

	let values: Vec<_> = headers.get_all("existing").iter().collect();
	assert_eq!(values.len(), 1);
	assert_eq!(values[0], "overwritten");
	assert_eq!(headers.get("new").unwrap(), "added");
}

#[test]
fn test_overwrite_if_exists() {
	let mut headers = HeaderMap::new();
	headers.insert("existing", "old-value".parse().unwrap());

	let mutation = Some(HeaderMutation {
		remove_headers: vec![],
		set_headers: vec![
			HeaderValueOption {
				header: Some(HeaderValue {
					key: "existing".to_string(),
					value: String::new(),
					raw_value: b"overwritten".to_vec(),
				}),
				append: None,
				append_action: HeaderAppendAction::OverwriteIfExists as i32,
			},
			HeaderValueOption {
				header: Some(HeaderValue {
					key: "new".to_string(),
					value: String::new(),
					raw_value: b"should-not-add".to_vec(),
				}),
				append: None,
				append_action: HeaderAppendAction::OverwriteIfExists as i32,
			},
		],
	});

	super::apply_header_mutations(&mut headers, mutation.as_ref()).unwrap();

	let values: Vec<_> = headers.get_all("existing").iter().collect();
	assert_eq!(values.len(), 1);
	assert_eq!(values[0], "overwritten");
	assert!(headers.get("new").is_none());
}

#[test]
fn test_remove_headers() {
	let mut headers = HeaderMap::new();
	headers.insert("to-remove", "value".parse().unwrap());
	headers.insert("keep", "value".parse().unwrap());

	let mutation = Some(HeaderMutation {
		remove_headers: vec!["to-remove".to_string()],
		set_headers: vec![],
	});

	super::apply_header_mutations(&mut headers, mutation.as_ref()).unwrap();

	assert!(headers.get("to-remove").is_none());
	assert_eq!(headers.get("keep").unwrap(), "value");
}

#[test]
fn test_apply_header_mutations_request() {
	let mut req = ::http::Request::builder()
		.uri("http://example.com")
		.header("existing", "value1")
		.body(Body::empty())
		.unwrap();

	let mutation = Some(HeaderMutation {
		remove_headers: vec!["to-remove".to_string()],
		set_headers: vec![HeaderValueOption {
			header: Some(HeaderValue {
				key: "existing".to_string(),
				value: String::new(),
				raw_value: b"value2".to_vec(),
			}),
			append: Some(true),
			append_action: HeaderAppendAction::AppendIfExistsOrAdd as i32,
		}],
	});

	super::apply_header_mutations_request(&mut req, mutation.as_ref()).unwrap();

	let headers = req.headers();
	assert!(headers.get("to-remove").is_none());

	let values: Vec<_> = headers.get_all("existing").iter().collect();
	assert_eq!(values.len(), 2);
	assert_eq!(values[0], "value1");
	assert_eq!(values[1], "value2");
}

#[test]
fn test_apply_pseudo_headers_request_with_raw_value() {
	let mut req = ::http::Request::builder()
		.uri("http://example.com/old-path")
		.method("GET")
		.body(Body::empty())
		.unwrap();

	let mutation = Some(HeaderMutation {
		remove_headers: vec![],
		set_headers: vec![
			HeaderValueOption {
				header: Some(HeaderValue {
					key: ":method".to_string(),
					value: String::new(),
					raw_value: b"POST".to_vec(),
				}),
				append: None,
				append_action: 0,
			},
			HeaderValueOption {
				header: Some(HeaderValue {
					key: ":path".to_string(),
					value: String::new(),
					raw_value: b"/new-path".to_vec(),
				}),
				append: None,
				append_action: 0,
			},
			HeaderValueOption {
				header: Some(HeaderValue {
					key: ":authority".to_string(),
					value: String::new(),
					raw_value: b"new-host.com".to_vec(),
				}),
				append: None,
				append_action: 0,
			},
			HeaderValueOption {
				header: Some(HeaderValue {
					key: ":scheme".to_string(),
					value: String::new(),
					raw_value: b"https".to_vec(),
				}),
				append: None,
				append_action: 0,
			},
		],
	});

	super::apply_header_mutations_request(&mut req, mutation.as_ref()).unwrap();

	// Verify pseudo-headers were applied
	assert_eq!(req.method(), "POST");
	assert_eq!(req.uri().path(), "/new-path");
	assert_eq!(req.uri().scheme_str(), Some("https"));
	assert_eq!(req.uri().authority().unwrap().as_str(), "new-host.com");
}

#[test]
fn test_apply_pseudo_headers_request_with_value_field() {
	let mut req = ::http::Request::builder()
		.uri("http://example.com/old-path")
		.method("GET")
		.body(Body::empty())
		.unwrap();

	let mutation = Some(HeaderMutation {
		remove_headers: vec![],
		set_headers: vec![
			HeaderValueOption {
				header: Some(HeaderValue {
					key: ":method".to_string(),
					value: "PUT".to_string(),
					raw_value: vec![], // Empty, should use value field
				}),
				append: None,
				append_action: 0,
			},
			HeaderValueOption {
				header: Some(HeaderValue {
					key: ":path".to_string(),
					value: "/updated-path".to_string(),
					raw_value: vec![],
				}),
				append: None,
				append_action: 0,
			},
		],
	});

	super::apply_header_mutations_request(&mut req, mutation.as_ref()).unwrap();

	// Verify pseudo-headers from value field were applied
	assert_eq!(req.method(), "PUT");
	assert_eq!(req.uri().path(), "/updated-path");
}

#[test]
fn test_pseudo_headers_request_raw_value_precedence() {
	let mut req = ::http::Request::builder()
		.uri("http://example.com/path")
		.method("GET")
		.body(Body::empty())
		.unwrap();

	let mutation = Some(HeaderMutation {
		remove_headers: vec![],
		set_headers: vec![HeaderValueOption {
			header: Some(HeaderValue {
				key: ":method".to_string(),
				value: "PUT".to_string(),      // Should be ignored
				raw_value: b"DELETE".to_vec(), // Should be used
			}),
			append: None,
			append_action: 0,
		}],
	});

	super::apply_header_mutations_request(&mut req, mutation.as_ref()).unwrap();

	// raw_value should take precedence
	assert_eq!(req.method(), "DELETE");
}

#[test]
fn test_apply_header_mutations_response() {
	let mut resp = ::http::Response::builder()
		.status(200)
		.header("existing", "value1")
		.body(Body::empty())
		.unwrap();

	let mutation = Some(HeaderMutation {
		remove_headers: vec!["to-remove".to_string()],
		set_headers: vec![HeaderValueOption {
			header: Some(HeaderValue {
				key: "existing".to_string(),
				value: String::new(),
				raw_value: b"value2".to_vec(),
			}),
			append: Some(true),
			append_action: HeaderAppendAction::AppendIfExistsOrAdd as i32,
		}],
	});

	super::apply_header_mutations_response(&mut resp, mutation.as_ref()).unwrap();

	let headers = resp.headers();
	assert!(headers.get("to-remove").is_none());

	let values: Vec<_> = headers.get_all("existing").iter().collect();
	assert_eq!(values.len(), 2);
	assert_eq!(values[0], "value1");
	assert_eq!(values[1], "value2");
}

#[test]
fn test_apply_pseudo_headers_response_with_raw_value() {
	let mut resp = ::http::Response::builder()
		.status(200)
		.header("x-test", "value")
		.body(Body::empty())
		.unwrap();

	let mutation = Some(HeaderMutation {
		remove_headers: vec![],
		set_headers: vec![HeaderValueOption {
			header: Some(HeaderValue {
				key: ":status".to_string(),
				value: String::new(),
				raw_value: b"404".to_vec(),
			}),
			append: None,
			append_action: 0,
		}],
	});

	super::apply_header_mutations_response(&mut resp, mutation.as_ref()).unwrap();

	// Verify :status pseudo-header was applied
	assert_eq!(resp.status(), 404);
	// Regular headers should still be present
	assert_eq!(resp.headers().get("x-test").unwrap(), "value");
}

#[test]
fn test_apply_pseudo_headers_response_with_value_field() {
	let mut resp = ::http::Response::builder()
		.status(200)
		.body(Body::empty())
		.unwrap();

	let mutation = Some(HeaderMutation {
		remove_headers: vec![],
		set_headers: vec![HeaderValueOption {
			header: Some(HeaderValue {
				key: ":status".to_string(),
				value: "201".to_string(),
				raw_value: vec![], // Empty, should use value field
			}),
			append: None,
			append_action: 0,
		}],
	});

	super::apply_header_mutations_response(&mut resp, mutation.as_ref()).unwrap();

	// Verify :status pseudo-header from value field was applied
	assert_eq!(resp.status(), 201);
}

#[test]
fn test_pseudo_headers_response_raw_value_precedence() {
	let mut resp = ::http::Response::builder()
		.status(200)
		.body(Body::empty())
		.unwrap();

	let mutation = Some(HeaderMutation {
		remove_headers: vec![],
		set_headers: vec![HeaderValueOption {
			header: Some(HeaderValue {
				key: ":status".to_string(),
				value: "500".to_string(),   // Should be ignored
				raw_value: b"403".to_vec(), // Should be used
			}),
			append: None,
			append_action: 0,
		}],
	});

	super::apply_header_mutations_response(&mut resp, mutation.as_ref()).unwrap();

	// raw_value should take precedence
	assert_eq!(resp.status(), 403);
}

#[test]
fn test_apply_mixed_headers_and_pseudo_headers_request() {
	let mut req = ::http::Request::builder()
		.uri("http://example.com/path")
		.method("GET")
		.header("x-custom", "old-value")
		.body(Body::empty())
		.unwrap();

	let mutation = Some(HeaderMutation {
		remove_headers: vec![],
		set_headers: vec![
			HeaderValueOption {
				header: Some(HeaderValue {
					key: ":method".to_string(),
					value: String::new(),
					raw_value: b"POST".to_vec(),
				}),
				append: None,
				append_action: 0,
			},
			HeaderValueOption {
				header: Some(HeaderValue {
					key: "x-custom".to_string(),
					value: String::new(),
					raw_value: b"new-value".to_vec(),
				}),
				append: None,
				append_action: HeaderAppendAction::OverwriteIfExistsOrAdd as i32,
			},
			HeaderValueOption {
				header: Some(HeaderValue {
					key: "x-new-header".to_string(),
					value: "added".to_string(),
					raw_value: vec![],
				}),
				append: None,
				append_action: HeaderAppendAction::AppendIfExistsOrAdd as i32,
			},
		],
	});

	super::apply_header_mutations_request(&mut req, mutation.as_ref()).unwrap();

	// Verify pseudo-header was applied
	assert_eq!(req.method(), "POST");
	// Verify regular headers were applied correctly
	assert_eq!(req.headers().get("x-custom").unwrap(), "new-value");
	assert_eq!(req.headers().get("x-new-header").unwrap(), "added");
}

#[test]
fn test_apply_mixed_headers_and_pseudo_headers_response() {
	let mut resp = ::http::Response::builder()
		.status(200)
		.header("x-custom", "old-value")
		.body(Body::empty())
		.unwrap();

	let mutation = Some(HeaderMutation {
		remove_headers: vec![],
		set_headers: vec![
			HeaderValueOption {
				header: Some(HeaderValue {
					key: ":status".to_string(),
					value: String::new(),
					raw_value: b"201".to_vec(),
				}),
				append: None,
				append_action: 0,
			},
			HeaderValueOption {
				header: Some(HeaderValue {
					key: "x-custom".to_string(),
					value: String::new(),
					raw_value: b"new-value".to_vec(),
				}),
				append: None,
				append_action: HeaderAppendAction::OverwriteIfExistsOrAdd as i32,
			},
			HeaderValueOption {
				header: Some(HeaderValue {
					key: "x-new-header".to_string(),
					value: "added".to_string(),
					raw_value: vec![],
				}),
				append: None,
				append_action: HeaderAppendAction::AppendIfExistsOrAdd as i32,
			},
		],
	});

	super::apply_header_mutations_response(&mut resp, mutation.as_ref()).unwrap();

	// Verify pseudo-header was applied
	assert_eq!(resp.status(), 201);
	// Verify regular headers were applied correctly
	assert_eq!(resp.headers().get("x-custom").unwrap(), "new-value");
	assert_eq!(resp.headers().get("x-new-header").unwrap(), "added");
}

#[test]
fn test_deprecated_append_true() {
	let mut headers = HeaderMap::new();
	headers.insert("existing", "value1".parse().unwrap());

	let mutation = Some(HeaderMutation {
		remove_headers: vec![],
		set_headers: vec![
			HeaderValueOption {
				header: Some(HeaderValue {
					key: "existing".to_string(),
					value: String::new(),
					raw_value: b"value2".to_vec(),
				}),
				append: Some(true),
				append_action: 0, // Not set, should fall back to append field
			},
			HeaderValueOption {
				header: Some(HeaderValue {
					key: "new".to_string(),
					value: String::new(),
					raw_value: b"added".to_vec(),
				}),
				append: Some(true),
				append_action: 0,
			},
		],
	});

	super::apply_header_mutations(&mut headers, mutation.as_ref()).unwrap();

	let values: Vec<_> = headers.get_all("existing").iter().collect();
	assert_eq!(values.len(), 2);
	assert_eq!(values[0], "value1");
	assert_eq!(values[1], "value2");
	assert_eq!(headers.get("new").unwrap(), "added");
}

#[test]
fn test_deprecated_append_false() {
	let mut headers = HeaderMap::new();
	headers.insert("existing", "old-value".parse().unwrap());

	let mutation = Some(HeaderMutation {
		remove_headers: vec![],
		set_headers: vec![HeaderValueOption {
			header: Some(HeaderValue {
				key: "existing".to_string(),
				value: String::new(),
				raw_value: b"overwritten".to_vec(),
			}),
			append: Some(false),
			append_action: 0, // Not set, should fall back to append field
		}],
	});

	super::apply_header_mutations(&mut headers, mutation.as_ref()).unwrap();

	let values: Vec<_> = headers.get_all("existing").iter().collect();
	assert_eq!(values.len(), 1);
	assert_eq!(values[0], "overwritten");
}

#[test]
fn test_value_field_instead_of_raw_value() {
	let mut headers = HeaderMap::new();
	headers.insert("existing", "value1".parse().unwrap());

	let mutation = Some(HeaderMutation {
		remove_headers: vec![],
		set_headers: vec![
			HeaderValueOption {
				header: Some(HeaderValue {
					key: "existing".to_string(),
					value: "value2".to_string(),
					raw_value: vec![], // Empty raw_value, should use value field
				}),
				append: Some(true),
				append_action: HeaderAppendAction::AppendIfExistsOrAdd as i32,
			},
			HeaderValueOption {
				header: Some(HeaderValue {
					key: "new".to_string(),
					value: "added".to_string(),
					raw_value: vec![],
				}),
				append: Some(true),
				append_action: HeaderAppendAction::AppendIfExistsOrAdd as i32,
			},
		],
	});

	super::apply_header_mutations(&mut headers, mutation.as_ref()).unwrap();

	let values: Vec<_> = headers.get_all("existing").iter().collect();
	assert_eq!(values.len(), 2);
	assert_eq!(values[0], "value1");
	assert_eq!(values[1], "value2");
	assert_eq!(headers.get("new").unwrap(), "added");
}

#[test]
fn test_raw_value_takes_precedence_over_value() {
	let mut headers = HeaderMap::new();

	let mutation = Some(HeaderMutation {
		remove_headers: vec![],
		set_headers: vec![HeaderValueOption {
			header: Some(HeaderValue {
				key: "test".to_string(),
				value: "should-not-use".to_string(),
				raw_value: b"raw-value-wins".to_vec(),
			}),
			append: None,
			append_action: HeaderAppendAction::AppendIfExistsOrAdd as i32,
		}],
	});

	super::apply_header_mutations(&mut headers, mutation.as_ref()).unwrap();

	assert_eq!(headers.get("test").unwrap(), "raw-value-wins");
}

#[test]
fn test_append_action_priority_over_deprecated_append() {
	let mut headers = HeaderMap::new();
	headers.insert("existing", "value1".parse().unwrap());

	let mutation = Some(HeaderMutation {
		remove_headers: vec![],
		set_headers: vec![HeaderValueOption {
			header: Some(HeaderValue {
				key: "existing".to_string(),
				value: String::new(),
				raw_value: b"overwritten".to_vec(),
			}),
			append: Some(true),
			append_action: HeaderAppendAction::OverwriteIfExistsOrAdd as i32,
		}],
	});

	super::apply_header_mutations(&mut headers, mutation.as_ref()).unwrap();

	let values: Vec<_> = headers.get_all("existing").iter().collect();
	assert_eq!(values.len(), 1);
	assert_eq!(values[0], "overwritten");
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
	use std::sync::Arc;

	use prost_wkt_types::value::Kind;
	use prost_wkt_types::{Struct, Value};

	use super::*;
	use crate::http::ext_proc::ExtProcInstance;

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

		ExtProcInstance::extract_dynamic_metadata(Some(&mut req), &metadata).unwrap();

		let extracted = req
			.extensions()
			.get::<Arc<ExtProcDynamicMetadata>>()
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
		req.extensions_mut().insert(Arc::new(existing));

		let metadata = Struct {
			fields: [(
				"new_key".to_string(),
				Value {
					kind: Some(Kind::StringValue("new_value".to_string())),
				},
			)]
			.into(),
		};
		ExtProcInstance::extract_dynamic_metadata(Some(&mut req), &metadata).unwrap();

		let extracted = req
			.extensions()
			.get::<Arc<ExtProcDynamicMetadata>>()
			.unwrap();
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
		req.extensions_mut().insert(Arc::new(existing));

		let metadata = Struct {
			fields: [(
				"key".to_string(),
				Value {
					kind: Some(Kind::StringValue("new_value".to_string())),
				},
			)]
			.into(),
		};
		ExtProcInstance::extract_dynamic_metadata(Some(&mut req), &metadata).unwrap();

		let extracted = req
			.extensions()
			.get::<Arc<ExtProcDynamicMetadata>>()
			.unwrap();
		assert_eq!(extracted.0.len(), 1);
		assert_eq!(
			extracted.0.get("key"),
			Some(&serde_json::json!("new_value"))
		);
	}

	#[test]
	fn test_extract_none_request_ok() {
		let metadata = Struct {
			fields: [(
				"key".to_string(),
				Value {
					kind: Some(Kind::StringValue("value".to_string())),
				},
			)]
			.into(),
		};
		let result = ExtProcInstance::extract_dynamic_metadata(None, &metadata);
		assert!(result.is_ok());
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

		ExtProcInstance::extract_dynamic_metadata(Some(&mut req), &metadata).unwrap();

		assert!(
			req
				.extensions()
				.get::<Arc<ExtProcDynamicMetadata>>()
				.is_none()
		);
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

		ExtProcInstance::extract_dynamic_metadata(Some(&mut req), &metadata).unwrap();

		let extracted = req
			.extensions()
			.get::<Arc<ExtProcDynamicMetadata>>()
			.unwrap();

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
		ExtProcInstance::extract_dynamic_metadata(Some(&mut req), &metadata1).unwrap();

		let metadata2 = Struct {
			fields: [(
				"key2".to_string(),
				Value {
					kind: Some(Kind::BoolValue(true)),
				},
			)]
			.into(),
		};
		ExtProcInstance::extract_dynamic_metadata(Some(&mut req), &metadata2).unwrap();

		let extracted = req
			.extensions()
			.get::<Arc<ExtProcDynamicMetadata>>()
			.unwrap();
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
