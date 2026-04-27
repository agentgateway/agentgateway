use std::sync::Arc;

use async_trait::async_trait;
use opentelemetry_proto::tonic::collector::trace::v1::trace_service_server::{
	TraceService, TraceServiceServer,
};
use opentelemetry_proto::tonic::collector::trace::v1::{
	ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use tonic::{Request, Response as TonicResponse, Status};

pub fn ok_response() -> Result<ExportTraceServiceResponse, Status> {
	Ok(ExportTraceServiceResponse {
		partial_success: None,
	})
}

#[async_trait]
pub trait Handler {
	async fn export(
		&mut self,
		_request: &ExportTraceServiceRequest,
	) -> Result<ExportTraceServiceResponse, Status> {
		ok_response()
	}
}

/// Mock OTLP TraceService server for testing
pub struct OtelTraceMock<T> {
	handler: Arc<dyn Fn() -> T + Send + Sync + 'static>,
}

impl<T> Clone for OtelTraceMock<T> {
	fn clone(&self) -> Self {
		Self {
			handler: self.handler.clone(),
		}
	}
}

impl<T> OtelTraceMock<T>
where
	T: Handler + Send + Sync + 'static,
{
	pub fn new(handler: impl Fn() -> T + Send + Sync + 'static) -> Self {
		Self {
			handler: Arc::new(handler),
		}
	}

	pub async fn spawn(&self) -> super::common::MockInstance {
		let srv = TraceServiceServer::new(self.clone());
		super::common::spawn_service(srv).await
	}

	pub async fn spawn_on(&self, address: std::net::SocketAddr) -> super::common::MockInstance {
		let srv = TraceServiceServer::new(self.clone());
		super::common::spawn_service_on(srv, address).await
	}
}

#[tonic::async_trait]
impl<T> TraceService for OtelTraceMock<T>
where
	T: Handler + Send + Sync + 'static,
{
	async fn export(
		&self,
		request: Request<ExportTraceServiceRequest>,
	) -> Result<TonicResponse<ExportTraceServiceResponse>, Status> {
		let mut handler = (self.handler.clone())();
		let response = handler.export(request.get_ref()).await?;
		Ok(TonicResponse::new(response))
	}
}
