use std::sync::Arc;

use async_trait::async_trait;
use prost_wkt_types::Struct;
use tonic::{Request, Response as TonicResponse, Status};

use protos::ext_mcp::{
	AuthorizationError, McpRequest, McpRequestResult, McpResponse, McpResponseResult, Pass,
	authorization_error::Code as ErrCode,
	ext_mcp_server::{ExtMcp, ExtMcpServer},
	mcp_request_result, mcp_response_result,
};

pub fn pass_request() -> Result<McpRequestResult, Status> {
	Ok(McpRequestResult {
		result: Some(mcp_request_result::Result::Pass(Pass {})),
	})
}

pub fn pass_response() -> Result<McpResponseResult, Status> {
	Ok(McpResponseResult {
		result: Some(mcp_response_result::Result::Pass(Pass {})),
	})
}

pub fn reject_request(code: ErrCode, reason: impl Into<String>) -> Result<McpRequestResult, Status> {
	Ok(McpRequestResult {
		result: Some(mcp_request_result::Result::Error(AuthorizationError {
			code: code as i32,
			reason: reason.into(),
			mcp_error: None,
		})),
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

pub fn mutated_request(body: Struct) -> Result<McpRequestResult, Status> {
	Ok(McpRequestResult {
		result: Some(mcp_request_result::Result::Mutated(body)),
	})
}

pub fn mutated_response(body: Struct) -> Result<McpResponseResult, Status> {
	Ok(McpResponseResult {
		result: Some(mcp_response_result::Result::Mutated(body)),
	})
}

pub fn mutated_request_json(body: serde_json::Value) -> Result<McpRequestResult, Status> {
	mutated_request(serde_json::from_value(body).expect("body must be a JSON object"))
}

pub fn mutated_response_json(body: serde_json::Value) -> Result<McpResponseResult, Status> {
	mutated_response(serde_json::from_value(body).expect("body must be a JSON object"))
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
