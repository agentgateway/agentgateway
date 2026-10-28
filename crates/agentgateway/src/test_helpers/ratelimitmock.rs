use std::sync::Arc;

use async_trait::async_trait;
use tonic::{Request, Response as TonicResponse, Status};

use crate::http::remoteratelimit::proto::rate_limit_response::Code;
use crate::http::remoteratelimit::proto::rate_limit_service_server::{
	RateLimitService, RateLimitServiceServer,
};
use crate::http::remoteratelimit::proto::{RateLimitRequest, RateLimitResponse};

pub fn ok_response() -> Result<RateLimitResponse, Status> {
	Ok(RateLimitResponse {
		overall_code: Code::Ok as i32,
		statuses: vec![],
		response_headers_to_add: vec![],
		request_headers_to_add: vec![],
		raw_body: vec![],
		dynamic_metadata: None,
		quota: None,
	})
}

pub fn over_limit_response(raw_body: impl Into<Vec<u8>>) -> Result<RateLimitResponse, Status> {
	Ok(RateLimitResponse {
		overall_code: Code::OverLimit as i32,
		statuses: vec![],
		response_headers_to_add: vec![],
		request_headers_to_add: vec![],
		raw_body: raw_body.into(),
		dynamic_metadata: None,
		quota: None,
	})
}

#[async_trait]
pub trait Handler {
	async fn should_rate_limit(
		&mut self,
		_request: &RateLimitRequest,
	) -> Result<RateLimitResponse, Status> {
		ok_response()
	}
}

/// Mock remote ratelimit server for testing
#[derive(Clone)]
pub struct RateLimitMock {
	handler: Arc<dyn Fn() -> Box<dyn Handler + Send + Sync> + Send + Sync>,
}

impl RateLimitMock {
	pub fn new<T: Handler + Send + Sync + 'static>(
		handler: impl Fn() -> T + Send + Sync + 'static,
	) -> Self {
		Self {
			handler: Arc::new(move || Box::new(handler())),
		}
	}

	pub async fn spawn(&self) -> super::common::MockInstance {
		let srv = RateLimitServiceServer::new(self.clone());
		super::common::spawn_service(srv).await
	}

	pub async fn spawn_on(&self, address: std::net::SocketAddr) -> super::common::MockInstance {
		let srv = RateLimitServiceServer::new(self.clone());
		super::common::spawn_service_on(srv, address).await
	}
}

#[tonic::async_trait]
impl RateLimitService for RateLimitMock {
	async fn should_rate_limit(
		&self,
		request: Request<RateLimitRequest>,
	) -> Result<TonicResponse<RateLimitResponse>, Status> {
		let mut handler = (self.handler.clone())();
		let response = handler.should_rate_limit(request.get_ref()).await?;
		Ok(TonicResponse::new(response))
	}
}
