mod gateway;
pub mod httpproxy;
#[cfg(any(test, feature = "testing"))]
pub mod request_builder;
pub mod tcpproxy;

pub use gateway::Gateway;
use hyper_util_fork::client::legacy::Error as HyperError;

use crate::http::{HeaderValue, Response, StatusCode, ext_proc};
use crate::types::agent::{
	Backend, BackendPolicy, BackendReference, BackendWithPolicies, SimpleBackend,
	SimpleBackendReference,
};
use crate::*;

#[derive(thiserror::Error, Debug)]
pub enum ProxyResponse {
	#[error("{0}")]
	Error(#[from] ProxyError),
	#[error("direct response")]
	DirectResponse(Box<Response>),
}

impl ProxyResponse {
	pub fn as_reason(&self) -> ProxyResponseReason {
		let ProxyResponse::Error(e) = self else {
			return ProxyResponseReason::DirectResponse;
		};
		match e {
			ProxyError::BindNotFound
			| ProxyError::ListenerNotFound
			| ProxyError::RouteNotFound
			| ProxyError::ServiceNotFound => ProxyResponseReason::NotFound,
			ProxyError::NoHealthyEndpoints
			| ProxyError::InvalidBackendType
			| ProxyError::DnsResolution
			| ProxyError::NoValidBackends
			| ProxyError::BackendDoesNotExist => ProxyResponseReason::NoHealthyBackend,
			ProxyError::UpgradeFailed(_, _)
			| ProxyError::InvalidRequest
			| ProxyError::ProcessingString(_)
			| ProxyError::Processing(_)
			| ProxyError::BackendUnsupportedMirror
			| ProxyError::FilterError(_) => ProxyResponseReason::Internal,
			ProxyError::JwtAuthenticationFailure(_) => ProxyResponseReason::JwtAuth,
			ProxyError::BasicAuthenticationFailure(_) => ProxyResponseReason::BasicAuth,
			ProxyError::APIKeyAuthenticationFailure(_) => ProxyResponseReason::APIKeyAuth,
			ProxyError::ExternalAuthorizationFailed(_) => ProxyResponseReason::ExtAuth,
			ProxyError::AuthorizationFailed | ProxyError::CsrfValidationFailed => {
				ProxyResponseReason::Authorization
			},
			ProxyError::UpstreamCallFailed(_)
			| ProxyError::UpstreamTCPCallFailed(_)
			| ProxyError::BackendAuthenticationFailed(_)
			| ProxyError::UpstreamTCPProxy(_) => ProxyResponseReason::UpstreamFailure,
			ProxyError::RequestTimeout => ProxyResponseReason::Timeout,
			ProxyError::ExtProc(_) => ProxyResponseReason::ExtProc,
			ProxyError::RateLimitFailed | ProxyError::RateLimitExceeded { .. } => {
				ProxyResponseReason::RateLimit
			},
		}
	}
	pub fn downcast(self) -> ProxyError {
		match self {
			ProxyResponse::Error(e) => e,
			ProxyResponse::DirectResponse(_) => ProxyError::ProcessingString(
				"attempted to return a direct response in an invalid context".to_string(),
			),
		}
	}
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum ProxyResponseReason {
	/// A response from the upstream
	Upstream,
	/// A response was directly recorded
	DirectResponse,
	/// The requested resource couldn't be found
	NotFound,
	/// There was not an endpoint eligible to send traffic to
	NoHealthyBackend,
	/// Some internal error in processing occurred
	Internal,
	/// JWT authentication failed
	JwtAuth,
	/// Basic authentication failed
	BasicAuth,
	/// API Key authentication failed
	APIKeyAuth,
	/// External Authorization failed
	ExtAuth,
	/// Authorization failed
	Authorization,
	/// Request timed out
	Timeout,
	/// External processing failed
	ExtProc,
	/// Rate limit exceeded
	RateLimit,
	/// The upstream request failed
	UpstreamFailure,
}

impl Display for ProxyResponseReason {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{:?}", self)
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
	#[error("basic authentication failure: {0}")]
	BasicAuthenticationFailure(http::basicauth::Error),
	#[error("api key authentication failure: {0}")]
	APIKeyAuthenticationFailure(http::apikey::Error),
	#[error("CSRF validation failed")]
	CsrfValidationFailed,
	#[error("service not found")]
	ServiceNotFound,
	#[error("invalid backend type")]
	InvalidBackendType,
	#[error("no healthy backends")]
	NoHealthyEndpoints,
	#[error("external authorization failed")]
	ExternalAuthorizationFailed(Option<StatusCode>),
	#[error("authorization failed")]
	AuthorizationFailed,
	#[error("backend authentication failed: {0}")]
	BackendAuthenticationFailed(anyhow::Error),
	#[error("upstream call failed: {0}")]
	UpstreamCallFailed(HyperError),
	#[error("upstream tcp call failed: {0}")]
	UpstreamTCPCallFailed(http::Error),
	#[error("upstream tcp proxy failed: {0}")]
	UpstreamTCPProxy(agent_core::copy::CopyError),
	#[error("request timeout")]
	RequestTimeout,
	#[error("processing failed: {0}")]
	Processing(anyhow::Error),
	#[error("ext_proc failed: {0}")]
	ExtProc(#[from] ext_proc::Error),
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
			ProxyError::ExtProc(_) => StatusCode::INTERNAL_SERVER_ERROR,
			ProxyError::CsrfValidationFailed => StatusCode::FORBIDDEN,

			ProxyError::UpgradeFailed(_, _) => StatusCode::BAD_GATEWAY,

			// Should it be 4xx?
			ProxyError::FilterError(_) => StatusCode::INTERNAL_SERVER_ERROR,
			ProxyError::InvalidRequest => StatusCode::BAD_REQUEST,

			ProxyError::JwtAuthenticationFailure(_) => StatusCode::UNAUTHORIZED,
			ProxyError::BasicAuthenticationFailure(_) => StatusCode::UNAUTHORIZED,
			ProxyError::APIKeyAuthenticationFailure(_) => StatusCode::UNAUTHORIZED,
			ProxyError::AuthorizationFailed => StatusCode::FORBIDDEN,
			ProxyError::ExternalAuthorizationFailed(status) => status.unwrap_or(StatusCode::FORBIDDEN),

			ProxyError::DnsResolution => StatusCode::SERVICE_UNAVAILABLE,
			ProxyError::NoHealthyEndpoints => StatusCode::SERVICE_UNAVAILABLE,
			ProxyError::UpstreamCallFailed(_) => StatusCode::SERVICE_UNAVAILABLE,

			ProxyError::RequestTimeout => StatusCode::GATEWAY_TIMEOUT,
			ProxyError::Processing(_) => StatusCode::SERVICE_UNAVAILABLE,
			ProxyError::ProcessingString(_) => StatusCode::SERVICE_UNAVAILABLE,
			ProxyError::RateLimitExceeded { .. } => StatusCode::TOO_MANY_REQUESTS,
			ProxyError::RateLimitFailed => StatusCode::TOO_MANY_REQUESTS,

			// Shouldn't happen on this path
			ProxyError::UpstreamTCPCallFailed(_) => StatusCode::INTERNAL_SERVER_ERROR,
			ProxyError::UpstreamTCPProxy(_) => StatusCode::INTERNAL_SERVER_ERROR,
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

		// Add WWW-Authenticate header for basic auth failures
		if let ProxyError::BasicAuthenticationFailure(err) = &self {
			let realm = match err {
				http::basicauth::Error::Missing { realm } => realm,
				http::basicauth::Error::InvalidCredentials { realm } => realm,
			};
			let auth_header = format!("Basic realm=\"{}\"", realm);
			if let Ok(hv) = HeaderValue::try_from(auth_header) {
				rb = rb.header(hyper::header::WWW_AUTHENTICATE, hv);
			}
		}

		rb.body(http::Body::from(msg)).unwrap()
	}
}

pub fn resolve_backend(
	b: &BackendReference,
	pi: &ProxyInputs,
) -> Result<BackendWithPolicies, ProxyError> {
	let backend = match b {
		BackendReference::Service { name, port } => {
			let svc = pi
				.stores
				.read_discovery()
				.services
				.get_by_namespaced_host(name)
				.ok_or(ProxyError::ServiceNotFound)?;
			Backend::Service(svc, *port).into()
		},
		BackendReference::Backend(_) => {
			let be = pi
				.stores
				.read_binds()
				.backend(&b.name())
				.ok_or(ProxyError::ServiceNotFound)?;
			Arc::unwrap_or_clone(be)
		},
		BackendReference::Invalid => Backend::Invalid.into(),
	};
	Ok(backend)
}

pub fn resolve_simple_backend(
	b: &SimpleBackendReference,
	pi: &ProxyInputs,
) -> Result<SimpleBackend, ProxyError> {
	resolve_simple_backend_with_policies(b, pi).map(|b| b.0)
}

pub fn resolve_simple_backend_with_policies(
	b: &SimpleBackendReference,
	pi: &ProxyInputs,
) -> Result<(SimpleBackend, Vec<BackendPolicy>), ProxyError> {
	let backend = match b {
		SimpleBackendReference::Service { name, port } => {
			let svc = pi
				.stores
				.read_discovery()
				.services
				.get_by_namespaced_host(name)
				.ok_or(ProxyError::ServiceNotFound)?;
			(SimpleBackend::Service(svc, *port), Vec::default())
		},
		SimpleBackendReference::Backend(_) => {
			let be = pi
				.stores
				.read_binds()
				.backend(&b.name())
				.ok_or(ProxyError::ServiceNotFound)?;
			(
				SimpleBackend::try_from(be.backend.clone()).map_err(|_| ProxyError::InvalidBackendType)?,
				be.inline_policies.clone(),
			)
		},
		SimpleBackendReference::Invalid => (SimpleBackend::Invalid, Vec::default()),
	};
	Ok(backend)
}
