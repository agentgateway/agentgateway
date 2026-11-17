use std::sync::Arc;

use async_trait::async_trait;
use protos::ateapi::control_server::{Control, ControlServer};
use protos::ateapi::{
	Actor, EgressPolicy, GetActorEgressPolicyRequest, GetActorRequest, MintActorJwtRequest,
	MintActorJwtResponse, ResumeActorRequest, ResumeActorResponse,
};
use tonic::{Request, Response, Status};

#[async_trait]
pub trait Handler {
	/// Observes the request metadata of every call before it is handled.
	fn metadata(&mut self, _metadata: &tonic::metadata::MetadataMap) {}

	async fn get_actor(&mut self, _request: &GetActorRequest) -> Result<Actor, Status> {
		Err(Status::unimplemented("GetActor is not implemented"))
	}

	async fn resume_actor(
		&mut self,
		_request: &ResumeActorRequest,
	) -> Result<ResumeActorResponse, Status> {
		Err(Status::unimplemented("ResumeActor is not implemented"))
	}

	fn resume_actor_metadata(&mut self, _metadata: &tonic::metadata::MetadataMap) {}

	async fn get_actor_egress_policy(
		&mut self,
		_request: &GetActorEgressPolicyRequest,
	) -> Result<EgressPolicy, Status> {
		Err(Status::unimplemented(
			"GetActorEgressPolicy is not implemented",
		))
	}

	async fn mint_actor_jwt(
		&mut self,
		_request: &MintActorJwtRequest,
	) -> Result<MintActorJwtResponse, Status> {
		Err(Status::unimplemented("MintActorJwt is not implemented"))
	}
}

/// Mock Substrate ate-api server for testing.
#[derive(Clone)]
pub struct AteApiMock {
	handler: Arc<dyn Fn() -> Box<dyn Handler + Send + Sync> + Send + Sync>,
}

impl AteApiMock {
	pub fn new<T: Handler + Send + Sync + 'static>(
		handler: impl Fn() -> T + Send + Sync + 'static,
	) -> Self {
		Self {
			handler: Arc::new(move || Box::new(handler())),
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
impl Control for AteApiMock {
	async fn get_actor(&self, request: Request<GetActorRequest>) -> Result<Response<Actor>, Status> {
		let mut handler = (self.handler)();
		handler.metadata(request.metadata());
		Ok(Response::new(handler.get_actor(request.get_ref()).await?))
	}

	async fn resume_actor(
		&self,
		request: Request<ResumeActorRequest>,
	) -> Result<Response<ResumeActorResponse>, Status> {
		let mut handler = (self.handler)();
		handler.metadata(request.metadata());
		handler.resume_actor_metadata(request.metadata());
		Ok(Response::new(
			handler.resume_actor(request.get_ref()).await?,
		))
	}

	async fn get_actor_egress_policy(
		&self,
		request: Request<GetActorEgressPolicyRequest>,
	) -> Result<Response<EgressPolicy>, Status> {
		let mut handler = (self.handler)();
		handler.metadata(request.metadata());
		Ok(Response::new(
			handler.get_actor_egress_policy(request.get_ref()).await?,
		))
	}

	async fn mint_actor_jwt(
		&self,
		request: Request<MintActorJwtRequest>,
	) -> Result<Response<MintActorJwtResponse>, Status> {
		let mut handler = (self.handler)();
		handler.metadata(request.metadata());
		Ok(Response::new(
			handler.mint_actor_jwt(request.get_ref()).await?,
		))
	}
}
