use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_stream;
use tonic::{Request, Response as TonicResponse, Status, Streaming};

use crate::http::ext_proc::proto::external_processor_server::{
	ExternalProcessor, ExternalProcessorServer,
};
use crate::http::ext_proc::proto::{
	self, CommonResponse, HttpHeaders, HttpTrailers, ProcessingRequest, ProcessingResponse,
	processing_request, processing_response,
};
use crate::*;

pub fn request_header_response(cr: Option<CommonResponse>) -> Result<ProcessingResponse, Status> {
	Ok(ProcessingResponse {
		response: Some(processing_response::Response::RequestHeaders(
			proto::HeadersResponse { response: cr },
		)),
		..Default::default()
	})
}

pub fn request_body_response(cr: Option<CommonResponse>) -> Result<ProcessingResponse, Status> {
	Ok(ProcessingResponse {
		response: Some(processing_response::Response::RequestBody(
			proto::BodyResponse { response: cr },
		)),
		..Default::default()
	})
}

pub fn response_header_response(cr: Option<CommonResponse>) -> Result<ProcessingResponse, Status> {
	Ok(ProcessingResponse {
		response: Some(processing_response::Response::ResponseHeaders(
			proto::HeadersResponse { response: cr },
		)),
		..Default::default()
	})
}
pub fn response_body_response(cr: Option<CommonResponse>) -> Result<ProcessingResponse, Status> {
	Ok(ProcessingResponse {
		response: Some(processing_response::Response::ResponseBody(
			proto::BodyResponse { response: cr },
		)),
		..Default::default()
	})
}

pub fn immediate_response(cr: proto::ImmediateResponse) -> Result<ProcessingResponse, Status> {
	Ok(ProcessingResponse {
		response: Some(processing_response::Response::ImmediateResponse(cr)),
		..Default::default()
	})
}

#[async_trait]
pub trait Handler {
	async fn handle_request_headers(
		&mut self,
		_headers: &HttpHeaders,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender.send(request_header_response(None)).await;
		Ok(())
	}

	async fn handle_request_body(
		&mut self,
		_body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender.send(request_body_response(None)).await;
		Ok(())
	}

	async fn handle_response_headers(
		&mut self,
		_headers: &HttpHeaders,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender.send(response_header_response(None)).await;
		Ok(())
	}

	async fn handle_response_body(
		&mut self,
		_body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender.send(response_body_response(None)).await;
		Ok(())
	}

	async fn handle_request_trailers(
		&mut self,
		_trailers: &HttpTrailers,
		_sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		Ok(())
	}

	async fn handle_response_trailers(
		&mut self,
		_trailers: &HttpTrailers,
		_sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		Ok(())
	}
}

/// Mock ext_proc server for testing
pub struct ExtProcMock<T> {
	handler: Arc<dyn Fn() -> T + Send + Sync + 'static>,
}

pub struct ExtProcMockInstance {
	pub address: SocketAddr,
	handle: JoinHandle<()>,
}

impl Drop for ExtProcMockInstance {
	fn drop(&mut self) {
		self.handle.abort();
	}
}

impl<T> Clone for ExtProcMock<T> {
	fn clone(&self) -> Self {
		Self {
			handler: self.handler.clone(),
		}
	}
}

impl<T> ExtProcMock<T>
where
	T: Handler + Send + Sync + 'static,
{
	/// Create a new mock with default configuration
	pub fn new(handler: impl Fn() -> T + Send + Sync + 'static) -> Self {
		Self {
			handler: Arc::new(handler),
		}
	}

	pub async fn spawn(&self) -> ExtProcMockInstance {
		use hyper::server::conn::http2;
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();

		let addr = listener.local_addr().unwrap();
		let s: ExtProcMock<T> = self.clone();
		let srv = ExternalProcessorServer::new(s);
		let task = tokio::spawn(async move {
			while let Ok((socket, _)) = listener.accept().await {
				let srv = srv.clone();
				tokio::spawn(async move {
					if let Err(err) = http2::Builder::new(::hyper_util::rt::TokioExecutor::new())
						.serve_connection(
							hyper_util::rt::TokioIo::new(socket),
							super::hyper_tower::TowerToHyperService::new(srv),
						)
						.await
					{
						error!("Error serving connection: {:?}", err);
					}
				});
			}
		});
		ExtProcMockInstance {
			address: addr,
			handle: task,
		}
	}
}

#[tonic::async_trait]
impl<T> ExternalProcessor for ExtProcMock<T>
where
	T: Handler + Send + Sync + 'static,
{
	type ProcessStream = tokio_stream::wrappers::ReceiverStream<Result<ProcessingResponse, Status>>;

	async fn process(
		&self,
		request: Request<Streaming<ProcessingRequest>>,
	) -> Result<TonicResponse<Self::ProcessStream>, Status> {
		let (tx, rx) = mpsc::channel(32);

		let mut handler = (self.handler.clone())();

		tokio::spawn(async move {
			let mut request_stream = request.into_inner();

			while let Some(request_result) = request_stream.message().await? {
				trace!("Received request: {:?}", request_result.request);
				match request_result.request {
					Some(processing_request::Request::RequestHeaders(headers)) => {
						handler.handle_request_headers(&headers, &tx).await?;
					},
					Some(processing_request::Request::RequestBody(body)) => {
						handler.handle_request_body(&body, &tx).await?;
					},
					Some(processing_request::Request::ResponseHeaders(headers)) => {
						handler.handle_response_headers(&headers, &tx).await?;
					},
					Some(processing_request::Request::ResponseBody(body)) => {
						handler.handle_response_body(&body, &tx).await?;
					},
					Some(processing_request::Request::RequestTrailers(trailers)) => {
						handler.handle_request_trailers(&trailers, &tx).await?;
					},
					Some(processing_request::Request::ResponseTrailers(trailers)) => {
						handler.handle_response_trailers(&trailers, &tx).await?;
					},
					None => {
						// Invalid request
						continue;
					},
				}
			}
			Ok::<(), Status>(())
		});

		Ok(TonicResponse::new(
			tokio_stream::wrappers::ReceiverStream::new(rx),
		))
	}
}
