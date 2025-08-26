mod gateway;
pub mod httpproxy;
#[cfg(any(test, feature = "testing"))]
pub mod request_builder;
pub mod tcpproxy;

pub use gateway::Gateway;
use hyper_util_fork::client::legacy::Error as HyperError;

use crate::http::{HeaderValue, Response, StatusCode};
use crate::types::agent::{Backend, BackendReference, SimpleBackend, SimpleBackendReference};
use crate::*;

#[derive(thiserror::Error, Debug)]
pub enum ProxyResponse {
	#[error("{0}")]
	Error(#[from] ProxyError),
	#[error("direct response")]
	DirectResponse(Box<Response>),
}

impl ProxyResponse {
	pub fn downcast(self) -> ProxyError {
		match self {
			ProxyResponse::Error(e) => e,
			ProxyResponse::DirectResponse(_) => ProxyError::ProcessingString(
				"attempted to return a direct response in an invalid context".to_string(),
			),
		}
	}
}

#[derive(thiserror::Error, Debug)]
pub enum ProxyError {
	#[error("bind not found")]
	BindNotFound,
	#[error("listener not found")]
	ListenerNotFound,
	#[error("route not found")]
	RouteNotFound,
	#[error("no valid backends")]
	NoValidBackends,
	#[error("backends does not exist")]
	BackendDoesNotExist,
	#[error("backends required DNS resolution which failed")]
	DnsResolution,
	#[error("failed to apply filters: {0}")]
	FilterError(#[from] http::filters::Error),
	#[error("backend type cannot be used in mirror")]
	BackendUnsupportedMirror,
	#[error("authentication failure: {0}")]
	JwtAuthenticationFailure(http::jwt::TokenError),
	#[error("transformation failed")]
	TransformationFailure,
	#[error("service not found")]
	ServiceNotFound,
	#[error("invalid backend type")]
	InvalidBackendType,
	#[error("no healthy backends")]
	NoHealthyEndpoints,
	#[error("authorization failed")]
	AuthorizationFailed(Option<StatusCode>),
	#[error("backend authentication failed: {0}")]
	BackendAuthenticationFailed(anyhow::Error),
	#[error("upstream call failed: {0:?}")]
	UpstreamCallFailed(HyperError),
	#[error("request timeout")]
	RequestTimeout,
	#[error("processing failed: {0}")]
	Processing(anyhow::Error),
	#[error("processing failed: {0}")]
	ProcessingString(String),
	#[error("rate limit exceeded")]
	RateLimitExceeded {
		limit: u64,
		remaining: u64,
		reset_seconds: u64,
	},
	#[error("rate limit failed")]
	RateLimitFailed,
	#[error("invalid request")]
	InvalidRequest,
	#[error("request upgrade failed, backend tried {1:?} but {0:?} was requested")]
	UpgradeFailed(Option<HeaderValue>, Option<HeaderValue>),
}

impl ProxyError {
	#[allow(clippy::match_like_matches_macro)]
	pub fn is_retryable(&self) -> bool {
		match self {
			ProxyError::UpstreamCallFailed(_) => true,
			ProxyError::RequestTimeout => true,
			ProxyError::DnsResolution => true,
			_ => false,
		}
	}
	pub fn into_response(self) -> Response {
		let code = match self {
			ProxyError::BindNotFound => StatusCode::NOT_FOUND,
			ProxyError::ListenerNotFound => StatusCode::NOT_FOUND,
			ProxyError::RouteNotFound => StatusCode::NOT_FOUND,
			ProxyError::NoValidBackends => StatusCode::INTERNAL_SERVER_ERROR,
			ProxyError::BackendDoesNotExist => StatusCode::INTERNAL_SERVER_ERROR,
			ProxyError::BackendUnsupportedMirror => StatusCode::INTERNAL_SERVER_ERROR,
			ProxyError::ServiceNotFound => StatusCode::INTERNAL_SERVER_ERROR,
			ProxyError::BackendAuthenticationFailed(_) => StatusCode::INTERNAL_SERVER_ERROR,
			ProxyError::InvalidBackendType => StatusCode::INTERNAL_SERVER_ERROR,
			ProxyError::TransformationFailure => StatusCode::INTERNAL_SERVER_ERROR,

			ProxyError::UpgradeFailed(_, _) => StatusCode::BAD_GATEWAY,

			// Should it be 4xx?
			ProxyError::FilterError(_) => StatusCode::INTERNAL_SERVER_ERROR,
			ProxyError::InvalidRequest => StatusCode::BAD_REQUEST,

			ProxyError::JwtAuthenticationFailure(_) => StatusCode::FORBIDDEN,
			ProxyError::AuthorizationFailed(status) => status.unwrap_or(StatusCode::FORBIDDEN),

			ProxyError::DnsResolution => StatusCode::SERVICE_UNAVAILABLE,
			ProxyError::NoHealthyEndpoints => StatusCode::SERVICE_UNAVAILABLE,
			ProxyError::UpstreamCallFailed(_) => StatusCode::SERVICE_UNAVAILABLE,

			ProxyError::RequestTimeout => StatusCode::GATEWAY_TIMEOUT,
			ProxyError::Processing(_) => StatusCode::SERVICE_UNAVAILABLE,
			ProxyError::ProcessingString(_) => StatusCode::SERVICE_UNAVAILABLE,
			ProxyError::RateLimitExceeded { .. } => StatusCode::TOO_MANY_REQUESTS,
			ProxyError::RateLimitFailed => StatusCode::TOO_MANY_REQUESTS,
		};
		let msg = self.to_string();
		let mut rb = ::http::Response::builder()
			.status(code)
			.header(hyper::header::CONTENT_TYPE, "text/plain");

		// Apply per-error headers
		if let ProxyError::RateLimitExceeded {
			limit,
			remaining,
			reset_seconds,
		} = self
		{
			if let Ok(hv) = HeaderValue::try_from(limit.to_string()) {
				rb = rb.header(http::x_headers::X_RATELIMIT_LIMIT, hv)
			}
			if let Ok(hv) = HeaderValue::try_from(remaining.to_string()) {
				rb = rb.header(http::x_headers::X_RATELIMIT_REMAINING, hv)
			}
			if let Ok(hv) = HeaderValue::try_from(reset_seconds.to_string()) {
				rb = rb.header(http::x_headers::X_RATELIMIT_RESET, hv)
			}
		}
		rb.body(http::Body::from(msg)).unwrap()
	}
}

pub fn resolve_backend(b: &BackendReference, pi: &ProxyInputs) -> Result<Backend, ProxyError> {
	let backend = match b {
		BackendReference::Service { name, port } => {
			let svc = pi
				.stores
				.read_discovery()
				.services
				.get_by_namespaced_host(name)
				.ok_or(ProxyError::ServiceNotFound)?;
			Backend::Service(svc, *port)
		},
		BackendReference::Backend(_) => {
			let be = pi
				.stores
				.read_binds()
				.backend(&b.name())
				.ok_or(ProxyError::ServiceNotFound)?;
			Arc::unwrap_or_clone(be)
		},
		BackendReference::Invalid => Backend::Invalid,
	};
	Ok(backend)
}

pub fn resolve_simple_backend(
	b: &SimpleBackendReference,
	pi: &ProxyInputs,
) -> Result<SimpleBackend, ProxyError> {
	let backend = match b {
		SimpleBackendReference::Service { name, port } => {
			let svc = pi
				.stores
				.read_discovery()
				.services
				.get_by_namespaced_host(name)
				.ok_or(ProxyError::ServiceNotFound)?;
			SimpleBackend::Service(svc, *port)
		},
		SimpleBackendReference::Backend(_) => {
			let be = pi
				.stores
				.read_binds()
				.backend(&b.name())
				.ok_or(ProxyError::ServiceNotFound)?;
			SimpleBackend::try_from(Arc::unwrap_or_clone(be))
				.map_err(|_| ProxyError::InvalidBackendType)?
		},
		SimpleBackendReference::Invalid => SimpleBackend::Invalid,
	};
	Ok(backend)
}
