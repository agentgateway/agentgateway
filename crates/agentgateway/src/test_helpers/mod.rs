pub mod extauthmock;
pub mod extprocmock;
mod hyper_tower;
pub mod oteltracemock;
#[cfg(any(test, feature = "internal_benches"))]
pub mod proxymock;
pub mod ratelimitmock;
pub use common::MockInstance;

mod common {
	use hyper::server::conn::http2;
	use std::net::SocketAddr;
	use tokio::task::JoinHandle;
	use tonic::body::Body;
	use tower::BoxError;
	use tracing::error;

	pub struct MockInstance {
		pub address: SocketAddr,
		handle: JoinHandle<()>,
	}

	impl Drop for MockInstance {
		fn drop(&mut self) {
			self.handle.abort();
		}
	}

	pub async fn spawn_service<S>(srv: S) -> MockInstance
	where
		S: tower::Service<hyper::Request<Body>, Response = http::Response<Body>>
			+ Clone
			+ Send
			+ Sync
			+ 'static,
		S::Future: Send + 'static,
		S::Error: Into<BoxError> + 'static,
	{
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let addr = listener.local_addr().unwrap();
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
		MockInstance {
			address: addr,
			handle: task,
		}
	}

	pub async fn spawn_service_on<S>(srv: S, address: SocketAddr) -> MockInstance
	where
		S: tower::Service<hyper::Request<Body>, Response = http::Response<Body>>
			+ Clone
			+ Send
			+ Sync
			+ 'static,
		S::Future: Send + 'static,
		S::Error: Into<BoxError> + 'static,
	{
		let listener = tokio::net::TcpListener::bind(address).await.unwrap();
		let addr = listener.local_addr().unwrap();
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
		MockInstance {
			address: addr,
			handle: task,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{extauthmock, extprocmock, oteltracemock, ratelimitmock};
	use crate::test_helpers::extauthmock::allow_response;
	use crate::test_helpers::oteltracemock::ok_response;
	use opentelemetry_proto::tonic::collector::trace::v1::{
		ExportTraceServiceRequest, ExportTraceServiceResponse,
	};
	use protos::envoy::service::auth::v3::{CheckRequest, CheckResponse};
	use tonic::Status;

	struct DevExtProcHandler;

	#[async_trait::async_trait]
	impl extprocmock::Handler for DevExtProcHandler {}

	struct DevRateLimitHandler;

	#[async_trait::async_trait]
	impl ratelimitmock::Handler for DevRateLimitHandler {}

	struct DevExtAuthHandler;

	#[async_trait::async_trait]
	impl extauthmock::Handler for DevExtAuthHandler {
		async fn check(&mut self, request: &CheckRequest) -> Result<CheckResponse, Status> {
			tracing::info!("got extauth request {:#?}", request);
			allow_response(None)
		}
	}

	struct DevOtelTraceHandler;

	#[async_trait::async_trait]
	impl oteltracemock::Handler for DevOtelTraceHandler {
		async fn export(
			&mut self,
			_request: &ExportTraceServiceRequest,
		) -> Result<ExportTraceServiceResponse, Status> {
			tracing::info!("got trace request");
			ok_response()
		}
	}

	// Run with: cargo test --lib -p agentgateway -- --ignored start_dev_mocks_on_fixed_ports --nocapture
	#[tokio::test]
	#[ignore = "dev helper: starts mock services on fixed ports and hangs"]
	async fn start_dev_mocks_on_fixed_ports() {
		agent_core::telemetry::testing::setup_test_logging();
		let ext_proc = extprocmock::ExtProcMock::new(|| DevExtProcHandler)
			.spawn_on(([127, 0, 0, 1], 9995).into())
			.await;
		tracing::info!("ext_proc mock started on {}", ext_proc.address);

		let rate_limit = ratelimitmock::RateLimitMock::new(|| DevRateLimitHandler)
			.spawn_on(([127, 0, 0, 1], 9996).into())
			.await;
		tracing::info!("ratelimit mock started on {}", rate_limit.address);

		let ext_auth = extauthmock::ExtAuthMock::new(|| DevExtAuthHandler)
			.spawn_on(([127, 0, 0, 1], 9997).into())
			.await;
		tracing::info!("ext_auth mock started on {}", ext_auth.address);

		let otel_trace = oteltracemock::OtelTraceMock::new(|| DevOtelTraceHandler)
			.spawn_on(([127, 0, 0, 1], 9998).into())
			.await;
		tracing::info!("otel trace mock started on {}", otel_trace.address);

		let _instances = (ext_proc, rate_limit, ext_auth, otel_trace);
		std::future::pending::<()>().await;
	}
}
