use pin_project_lite::pin_project;
use tonic::Status;
use tonic::body::Body;
use tower::{BoxError, ServiceExt};

// Copied from https://github.com/hyperium/tonic/blob/34b863b1d2a204ef3dd871ec86860fc92aafb451/examples/src/tls_rustls/server.rs

/// An adaptor which converts a [`tower::Service`] to a [`hyper::service::Service`].
///
/// The [`hyper::service::Service`] trait is used by hyper to handle incoming requests,
/// and does not support the `poll_ready` method that is used by tower services.
///
/// This is provided here because the equivalent adaptor in hyper-util does not support
/// tonic::body::Body bodies.
#[derive(Debug, Clone)]
pub struct TowerToHyperService<S> {
	service: S,
}

impl<S> TowerToHyperService<S> {
	/// Create a new `TowerToHyperService` from a tower service.
	pub fn new(service: S) -> Self {
		Self { service }
	}
}

impl<S> hyper::service::Service<hyper::Request<hyper::body::Incoming>> for TowerToHyperService<S>
where
	S: tower::Service<hyper::Request<Body>> + Clone,
	S::Error: Into<BoxError> + 'static,
{
	type Response = S::Response;
	type Error = BoxError;
	type Future = TowerToHyperServiceFuture<S, hyper::Request<Body>>;

	fn call(&self, req: hyper::Request<hyper::body::Incoming>) -> Self::Future {
		use http_body_util::BodyExt;
		let req = req.map(|incoming| Body::new(incoming.map_err(|err| Status::from_error(err.into()))));
		TowerToHyperServiceFuture {
			future: self.service.clone().oneshot(req),
		}
	}
}

pin_project! {
		/// Future returned by [`TowerToHyperService`].
		#[derive(Debug)]
		pub struct TowerToHyperServiceFuture<S, R>
		where
				S: tower::Service<R>,
		{
				#[pin]
				future: tower::util::Oneshot<S, R>,
		}
}

impl<S, R> std::future::Future for TowerToHyperServiceFuture<S, R>
where
	S: tower::Service<R>,
	S::Error: Into<BoxError> + 'static,
{
	type Output = Result<S::Response, BoxError>;

	#[inline]
	fn poll(
		self: std::pin::Pin<&mut Self>,
		cx: &mut std::task::Context<'_>,
	) -> std::task::Poll<Self::Output> {
		self.project().future.poll(cx).map_err(Into::into)
	}
}
