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
}
