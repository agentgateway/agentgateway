use std::sync::Arc;

use async_trait::async_trait;
use prost_wkt_types::Struct;
use protos::ext_mcp::authorization_error::Code as ErrCode;
use protos::ext_mcp::ext_mcp_server::{ExtMcp, ExtMcpServer};
use protos::ext_mcp::{
	AuthorizationError, HeaderMutation, McpHeader, McpRequest, McpRequestResult, McpResponse,
	McpResponseResult, Pass, mcp_request_result, mcp_response_result,
};
use tonic::{Request, Response as TonicResponse, Status};

pub fn pass_request() -> Result<McpRequestResult, Status> {
	Ok(McpRequestResult {
		result: Some(mcp_request_result::Result::Pass(Pass {})),
		header_mutation: None,
		metadata: None,
	})
}

pub fn pass_response() -> Result<McpResponseResult, Status> {
	Ok(McpResponseResult {
		result: Some(mcp_response_result::Result::Pass(Pass {})),
	})
}

pub fn reject_request(
	code: ErrCode,
	reason: impl Into<String>,
) -> Result<McpRequestResult, Status> {
	Ok(McpRequestResult {
		result: Some(mcp_request_result::Result::Error(AuthorizationError {
			code: code as i32,
			reason: reason.into(),
			mcp_error: None,
		})),
		header_mutation: None,
		metadata: None,
	})
}

pub fn reject_response(
	code: ErrCode,
	reason: impl Into<String>,
) -> Result<McpResponseResult, Status> {
	Ok(McpResponseResult {
		result: Some(mcp_response_result::Result::Error(AuthorizationError {
			code: code as i32,
			reason: reason.into(),
			mcp_error: None,
		})),
	})
}

pub fn mutated_request(body: bytes::Bytes) -> Result<McpRequestResult, Status> {
	Ok(McpRequestResult {
		result: Some(mcp_request_result::Result::Mutated(body)),
		header_mutation: None,
		metadata: None,
	})
}

pub fn mutated_response(body: bytes::Bytes) -> Result<McpResponseResult, Status> {
	Ok(McpResponseResult {
		result: Some(mcp_response_result::Result::Mutated(body)),
	})
}

/// Pass with a side effect: optional header mutation + metadata, applied to
/// the upstream request that carries this MCP call.
pub fn pass_request_with(
	set_headers: impl IntoIterator<Item = (impl Into<String>, impl Into<Vec<u8>>)>,
	remove_headers: impl IntoIterator<Item = impl Into<String>>,
	metadata: Option<Struct>,
) -> Result<McpRequestResult, Status> {
	Ok(McpRequestResult {
		result: Some(mcp_request_result::Result::Pass(Pass {})),
		header_mutation: Some(build_header_mutation(set_headers, remove_headers)),
		metadata,
	})
}

fn build_header_mutation(
	set_headers: impl IntoIterator<Item = (impl Into<String>, impl Into<Vec<u8>>)>,
	remove_headers: impl IntoIterator<Item = impl Into<String>>,
) -> HeaderMutation {
	HeaderMutation {
		set: set_headers
			.into_iter()
			.map(|(k, v)| McpHeader {
				key: k.into(),
				value: v.into(),
			})
			.collect(),
		remove: remove_headers.into_iter().map(Into::into).collect(),
	}
}

pub fn mutated_request_json(body: serde_json::Value) -> Result<McpRequestResult, Status> {
	mutated_request(serde_json::to_vec(&body).expect("serialize body").into())
}

pub fn mutated_response_json(body: serde_json::Value) -> Result<McpResponseResult, Status> {
	mutated_response(serde_json::to_vec(&body).expect("serialize body").into())
}

#[async_trait]
pub trait Handler {
	async fn check_request(&mut self, _req: &McpRequest) -> Result<McpRequestResult, Status> {
		pass_request()
	}
	async fn check_response(&mut self, _req: &McpResponse) -> Result<McpResponseResult, Status> {
		pass_response()
	}
}

type RequestFn = Arc<dyn Fn(&McpRequest) -> Result<McpRequestResult, Status> + Send + Sync>;
type ResponseFn = Arc<dyn Fn(&McpResponse) -> Result<McpResponseResult, Status> + Send + Sync>;

pub struct ClosureHandler {
	on_request: RequestFn,
	on_response: ResponseFn,
}

#[async_trait]
impl Handler for ClosureHandler {
	async fn check_request(&mut self, req: &McpRequest) -> Result<McpRequestResult, Status> {
		(self.on_request)(req)
	}
	async fn check_response(&mut self, req: &McpResponse) -> Result<McpResponseResult, Status> {
		(self.on_response)(req)
	}
}

/// Build an `ExtMcpMock` from two closures, skipping the per-test `struct + impl Handler` boilerplate.
/// State that varies per call (counters, capture buffers) is captured via `Arc` inside the closure.
pub fn closure_mock<R, P>(on_request: R, on_response: P) -> ExtMcpMock<ClosureHandler>
where
	R: Fn(&McpRequest) -> Result<McpRequestResult, Status> + Send + Sync + 'static,
	P: Fn(&McpResponse) -> Result<McpResponseResult, Status> + Send + Sync + 'static,
{
	let on_request: RequestFn = Arc::new(on_request);
	let on_response: ResponseFn = Arc::new(on_response);
	ExtMcpMock::new(move || ClosureHandler {
		on_request: on_request.clone(),
		on_response: on_response.clone(),
	})
}

/// Mock extMcp gRPC server for tests. Wraps a `Handler` factory; a fresh
/// handler instance is produced per RPC, so per-call state lives in the
/// caller's closure (typically an Arc<Mutex<…>>).
pub struct ExtMcpMock<T> {
	handler: Arc<dyn Fn() -> T + Send + Sync + 'static>,
}

impl<T> Clone for ExtMcpMock<T> {
	fn clone(&self) -> Self {
		Self {
			handler: self.handler.clone(),
		}
	}
}

impl<T> ExtMcpMock<T>
where
	T: Handler + Send + Sync + 'static,
{
	pub fn new(handler: impl Fn() -> T + Send + Sync + 'static) -> Self {
		Self {
			handler: Arc::new(handler),
		}
	}

	pub async fn spawn(&self) -> super::common::MockInstance {
		let srv = ExtMcpServer::new(self.clone());
		super::common::spawn_service(srv).await
	}

	pub async fn spawn_on(&self, address: std::net::SocketAddr) -> super::common::MockInstance {
		let srv = ExtMcpServer::new(self.clone());
		super::common::spawn_service_on(srv, address).await
	}
}

#[tonic::async_trait]
impl<T> ExtMcp for ExtMcpMock<T>
where
	T: Handler + Send + Sync + 'static,
{
	async fn check_request(
		&self,
		request: Request<McpRequest>,
	) -> Result<TonicResponse<McpRequestResult>, Status> {
		let mut handler = (self.handler.clone())();
		let response = handler.check_request(request.get_ref()).await?;
		Ok(TonicResponse::new(response))
	}

	async fn check_response(
		&self,
		request: Request<McpResponse>,
	) -> Result<TonicResponse<McpResponseResult>, Status> {
		let mut handler = (self.handler.clone())();
		let response = handler.check_response(request.get_ref()).await?;
		Ok(TonicResponse::new(response))
	}
}
