use std::sync::Arc;

use async_trait::async_trait;
use protos::ateapi::control_server::{Control, ControlServer};
use protos::ateapi::{
	Actor, EgressPolicy, GetActorEgressPolicyRequest, GetActorRequest, ResumeActorRequest,
	ResumeActorResponse,
};
use tonic::{Request, Response, Status};

#[async_trait]
pub trait Handler {
	async fn get_actor(&mut self, _request: &GetActorRequest) -> Result<Actor, Status> {
		Err(Status::unimplemented("GetActor is not implemented"))
	}

	async fn resume_actor(
		&mut self,
		_request: &ResumeActorRequest,
	) -> Result<ResumeActorResponse, Status> {
		Err(Status::unimplemented("ResumeActor is not implemented"))
	}

	async fn get_actor_egress_policy(
		&mut self,
		_request: &GetActorEgressPolicyRequest,
	) -> Result<EgressPolicy, Status> {
		Err(Status::unimplemented(
			"GetActorEgressPolicy is not implemented",
		))
	}
}

/// Mock Substrate ate-api server for testing.
pub struct AteApiMock<T> {
	handler: Arc<dyn Fn() -> T + Send + Sync + 'static>,
}

impl<T> Clone for AteApiMock<T> {
	fn clone(&self) -> Self {
		Self {
			handler: self.handler.clone(),
		}
	}
}

impl<T> AteApiMock<T>
where
	T: Handler + Send + Sync + 'static,
{
	pub fn new(handler: impl Fn() -> T + Send + Sync + 'static) -> Self {
		Self {
			handler: Arc::new(handler),
		}
	}

	pub async fn spawn(&self) -> super::common::MockInstance {
		super::common::spawn_service(ControlServer::new(self.clone())).await
	}

	pub async fn spawn_on(&self, address: std::net::SocketAddr) -> super::common::MockInstance {
		super::common::spawn_service_on(ControlServer::new(self.clone()), address).await
	}
}

#[tonic::async_trait]
impl<T> Control for AteApiMock<T>
where
	T: Handler + Send + Sync + 'static,
{
	async fn get_actor(&self, request: Request<GetActorRequest>) -> Result<Response<Actor>, Status> {
		let mut handler = (self.handler)();
		Ok(Response::new(handler.get_actor(request.get_ref()).await?))
	}

	async fn resume_actor(
		&self,
		request: Request<ResumeActorRequest>,
	) -> Result<Response<ResumeActorResponse>, Status> {
		let mut handler = (self.handler)();
		Ok(Response::new(
			handler.resume_actor(request.get_ref()).await?,
		))
	}

	async fn get_actor_egress_policy(
		&self,
		request: Request<GetActorEgressPolicyRequest>,
	) -> Result<Response<EgressPolicy>, Status> {
		let mut handler = (self.handler)();
		Ok(Response::new(
			handler.get_actor_egress_policy(request.get_ref()).await?,
		))
	}
}
