use std::sync::Arc;

use async_trait::async_trait;
use tonic::{Request, Response as TonicResponse, Status};

use crate::http::ext_authz::proto::authorization_server::{Authorization, AuthorizationServer};
use crate::http::ext_authz::proto::check_response::HttpResponse;
use crate::http::ext_authz::proto::{self, CheckRequest, CheckResponse, DeniedHttpResponse};

pub fn allow_response(http_response: Option<HttpResponse>) -> Result<CheckResponse, Status> {
	Ok(CheckResponse {
		status: Some(proto::Status {
			code: 0,
			message: String::new(),
			details: vec![],
		}),
		http_response,
		dynamic_metadata: None,
	})
}

pub fn deny_response(
	status_code: proto::StatusCode,
	body: impl Into<String>,
) -> Result<CheckResponse, Status> {
	Ok(CheckResponse {
		status: Some(proto::Status {
			code: 7,
			message: "denied".to_string(),
			details: vec![],
		}),
		http_response: Some(HttpResponse::DeniedResponse(DeniedHttpResponse {
			status: Some(proto::HttpStatus {
				code: status_code as i32,
			}),
			headers: vec![],
			body: body.into(),
		})),
		dynamic_metadata: None,
	})
}

#[async_trait]
pub trait Handler {
	async fn check(&mut self, _request: &CheckRequest) -> Result<CheckResponse, Status> {
		allow_response(None)
	}
}

/// Mock ext_authz server for testing
#[derive(Clone)]
pub struct ExtAuthMock {
	handler: Arc<dyn Fn() -> Box<dyn Handler + Send + Sync> + Send + Sync>,
}

impl ExtAuthMock {
	pub fn new<T: Handler + Send + Sync + 'static>(
		handler: impl Fn() -> T + Send + Sync + 'static,
	) -> Self {
		Self {
			handler: Arc::new(move || Box::new(handler())),
		}
	}

	pub async fn spawn(&self) -> super::common::MockInstance {
		let srv = AuthorizationServer::new(self.clone());
		super::common::spawn_service(srv).await
	}

	pub async fn spawn_on(&self, address: std::net::SocketAddr) -> super::common::MockInstance {
		let srv = AuthorizationServer::new(self.clone());
		super::common::spawn_service_on(srv, address).await
	}
}

#[tonic::async_trait]
impl Authorization for ExtAuthMock {
	async fn check(
		&self,
		request: Request<CheckRequest>,
	) -> Result<TonicResponse<CheckResponse>, Status> {
		let mut handler = (self.handler.clone())();
		let response = handler.check(request.get_ref()).await?;
		Ok(TonicResponse::new(response))
	}
}
