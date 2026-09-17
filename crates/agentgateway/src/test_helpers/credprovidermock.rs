use std::sync::Arc;

use async_trait::async_trait;
use protos::credprovider::credential_provider_server::{
	CredentialProvider, CredentialProviderServer,
};
use protos::credprovider::{FetchSecretRequest, FetchSecretResponse};
use tonic::{Request, Response, Status};

#[async_trait]
pub trait Handler {
	async fn fetch_secret(
		&mut self,
		_request: &FetchSecretRequest,
	) -> Result<FetchSecretResponse, Status> {
		Err(Status::unimplemented("FetchSecret is not implemented"))
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
	async fn fetch_secret(
		&self,
		request: Request<FetchSecretRequest>,
	) -> Result<Response<FetchSecretResponse>, Status> {
		let mut handler = (self.handler)();
		Ok(Response::new(
			handler.fetch_secret(request.get_ref()).await?,
		))
	}
}
