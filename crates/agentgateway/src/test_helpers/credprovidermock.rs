use std::sync::Arc;

use async_trait::async_trait;
use protos::credprovider::credential_provider_server::{
	CredentialProvider, CredentialProviderServer,
};
use protos::credprovider::{RequestSecretRequest, RequestSecretResponse};
use tonic::{Request, Response, Status};

#[async_trait]
pub trait Handler {
	async fn request_secret(
		&mut self,
		_request: &RequestSecretRequest,
	) -> Result<RequestSecretResponse, Status> {
		Err(Status::unimplemented("RequestSecret is not implemented"))
	}
}

/// Mock Substrate credential-provider server for testing.
pub struct CredentialProviderMock<T> {
	handler: Arc<dyn Fn() -> T + Send + Sync + 'static>,
}

impl<T> Clone for CredentialProviderMock<T> {
	fn clone(&self) -> Self {
		Self {
			handler: self.handler.clone(),
		}
	}
}

impl<T> CredentialProviderMock<T>
where
	T: Handler + Send + Sync + 'static,
{
	pub fn new(handler: impl Fn() -> T + Send + Sync + 'static) -> Self {
		Self {
			handler: Arc::new(handler),
		}
	}

	pub async fn spawn(&self) -> super::common::MockInstance {
		super::common::spawn_service(CredentialProviderServer::new(self.clone())).await
	}
}

#[tonic::async_trait]
impl<T> CredentialProvider for CredentialProviderMock<T>
where
	T: Handler + Send + Sync + 'static,
{
	async fn request_secret(
		&self,
		request: Request<RequestSecretRequest>,
	) -> Result<Response<RequestSecretResponse>, Status> {
		let mut handler = (self.handler)();
		Ok(Response::new(
			handler.request_secret(request.get_ref()).await?,
		))
	}
}
