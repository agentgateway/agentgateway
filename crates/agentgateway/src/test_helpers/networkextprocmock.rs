//! Sample Envoy NetworkExternalProcessor server for TCP proxy integration tests.

use async_trait::async_trait;
use tokio::sync::mpsc;
use tonic::{Request, Response, Status, Streaming};

use crate::http::network_ext_proc::proto::network_external_processor_server::{
	NetworkExternalProcessor, NetworkExternalProcessorServer,
};
use crate::http::network_ext_proc::proto::{self};
use crate::test_helpers::common::MockInstance;

#[async_trait]
pub trait Handler: Send + Sync + 'static {
	async fn process(
		&mut self,
		request: proto::ProcessingRequest,
	) -> Result<proto::ProcessingResponse, Status> {
		let (read_data, write_data) = (request.read_data, request.write_data);
		Ok(proto::ProcessingResponse {
			read_data,
			write_data,
			data_processing_status: proto::processing_response::DataProcessedStatus::Unmodified as i32,
			connection_status: proto::processing_response::ConnectionStatus::Continue as i32,
			..Default::default()
		})
	}
}

pub struct NetworkExtProcMock<T> {
	handler: std::sync::Arc<dyn Fn() -> T + Send + Sync>,
}
impl<T> NetworkExtProcMock<T>
where
	T: Handler,
{
	pub fn new(handler: impl Fn() -> T + Send + Sync + 'static) -> Self {
		Self {
			handler: std::sync::Arc::new(handler),
		}
	}
	pub async fn spawn(&self) -> MockInstance {
		super::common::spawn_service(NetworkExternalProcessorServer::new(self.clone())).await
	}
}
impl<T> Clone for NetworkExtProcMock<T> {
	fn clone(&self) -> Self {
		Self {
			handler: self.handler.clone(),
		}
	}
}

#[async_trait]
impl<T: Handler> NetworkExternalProcessor for NetworkExtProcMock<T> {
	type ProcessStream =
		tokio_stream::wrappers::ReceiverStream<Result<proto::ProcessingResponse, Status>>;
	async fn process(
		&self,
		request: Request<Streaming<proto::ProcessingRequest>>,
	) -> Result<Response<Self::ProcessStream>, Status> {
		let (tx, rx) = mpsc::channel::<Result<proto::ProcessingResponse, Status>>(32);
		let mut handler = (self.handler)();
		tokio::spawn(async move {
			let mut stream = request.into_inner();
			while let Some(request) = stream.message().await? {
				tx.send(Ok(handler.process(request).await?))
					.await
					.map_err(|_| Status::cancelled("client disconnected"))?;
			}
			Ok::<(), Status>(())
		});
		Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
			rx,
		)))
	}
}
