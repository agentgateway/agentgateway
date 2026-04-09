pub mod extauthmock;
pub mod extprocmock;
mod hyper_tower;
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
	use super::{extauthmock, extprocmock, ratelimitmock};

	struct DevExtProcHandler;

	#[async_trait::async_trait]
	impl extprocmock::Handler for DevExtProcHandler {}

	struct DevRateLimitHandler;

	#[async_trait::async_trait]
	impl ratelimitmock::Handler for DevRateLimitHandler {}

	struct DevExtAuthHandler;

	#[async_trait::async_trait]
	impl extauthmock::Handler for DevExtAuthHandler {}

	// Run with: cargo test --lib -p agentgateway -- --ignored start_dev_mocks_on_fixed_ports --nocapture
	#[tokio::test]
	#[ignore = "dev helper: starts mock services on fixed ports and hangs"]
	async fn start_dev_mocks_on_fixed_ports() {
		let ext_proc = extprocmock::ExtProcMock::new(|| DevExtProcHandler)
			.spawn_on(([127, 0, 0, 1], 9995).into())
			.await;
		println!("ext_proc mock started on {}", ext_proc.address);

		let rate_limit = ratelimitmock::RateLimitMock::new(|| DevRateLimitHandler)
			.spawn_on(([127, 0, 0, 1], 9996).into())
			.await;
		println!("ratelimit mock started on {}", rate_limit.address);

		let ext_auth = extauthmock::ExtAuthMock::new(|| DevExtAuthHandler)
			.spawn_on(([127, 0, 0, 1], 9997).into())
			.await;
		println!("ext_auth mock started on {}", ext_auth.address);

		let _instances = (ext_proc, rate_limit, ext_auth);
		std::future::pending::<()>().await;
	}
}
