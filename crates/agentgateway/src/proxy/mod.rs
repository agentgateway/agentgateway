mod gateway;
pub mod httpproxy;
pub mod proxy_protocol;
pub mod request_builder;
pub mod tcpproxy;

pub use gateway::Gateway;
use hyper_util_fork::client::legacy::Error as HyperError;
use rmcp::ErrorData;
use rmcp::model::{ErrorCode, JsonRpcError};

use crate::http::{HeaderValue, Response, StatusCode, ext_proc};
use crate::types::agent::{
	Backend, BackendReference, BackendWithPolicies, ResourceName, SimpleBackend,
	SimpleBackendReference, SimpleBackendWithPolicies,
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
			| ProxyError::MisdirectedRequest
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
			| ProxyError::Body(_)
			| ProxyError::Http(_)
			| ProxyError::BackendUnsupportedMirror
			| ProxyError::FilterError(_) => ProxyResponseReason::Internal,
			ProxyError::JwtAuthenticationFailure(_) => ProxyResponseReason::JwtAuth,
			ProxyError::McpJwtAuthenticationFailure(_, _) => ProxyResponseReason::JwtAuth,
			ProxyError::BasicAuthenticationFailure(_) => ProxyResponseReason::BasicAuth,
			ProxyError::APIKeyAuthenticationFailure(_) => ProxyResponseReason::APIKeyAuth,
			ProxyError::ExternalAuthorizationFailed(_) => ProxyResponseReason::ExtAuth,
			ProxyError::MCP(_) => ProxyResponseReason::MCP,
			ProxyError::AuthorizationFailed | ProxyError::CsrfValidationFailed => {
				ProxyResponseReason::Authorization
			},
			ProxyError::UpstreamCallFailed(_)
			| ProxyError::UpstreamTCPCallFailed(_)
			| ProxyError::BackendAuthenticationFailed(_)
			| ProxyError::UpstreamTCPProxy(_) => ProxyResponseReason::UpstreamFailure,
			ProxyError::RequestTimeout | ProxyError::UpstreamCallTimeout => ProxyResponseReason::Timeout,
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
	/// MCP
	MCP,
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
	#[error("misdirected request")]
	MisdirectedRequest,
	#[error("no valid backends")]
	NoValidBackends,
	#[error("backend does not exist")]
	BackendDoesNotExist,
	#[error("backends required DNS resolution which failed")]
	DnsResolution,
	#[error("failed to apply filters: {0}")]
	FilterError(#[from] http::filters::Error),
	#[error("backend type cannot be used in mirror")]
	BackendUnsupportedMirror,
	#[error("authentication failure: {0}")]
	JwtAuthenticationFailure(http::jwt::TokenError),
	#[error("mcp authentication failure: {0}")]
	McpJwtAuthenticationFailure(Box<ProxyError>, String),
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
	#[error("parsing body: {0}")]
	Body(http::Error),
	#[error("upstream call failed: {0}")]
	UpstreamCallFailed(HyperError),
	#[error("upstream call timeout")]
	UpstreamCallTimeout,
	#[error("upstream tcp call failed: {0}")]
	UpstreamTCPCallFailed(http::Error),
	#[error("upstream tcp proxy failed: {0}")]
	UpstreamTCPProxy(agent_core::copy::CopyError),
	#[error("request timeout")]
	RequestTimeout,
	#[error("processing failed: {0}")]
	Processing(anyhow::Error),
	#[error("invalid http: {0}")]
	Http(#[from] ::http::Error),
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
	#[error("mcp: {0}")]
	MCP(mcp::Error),
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
			ProxyError::MisdirectedRequest => StatusCode::MISDIRECTED_REQUEST,
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
			ProxyError::McpJwtAuthenticationFailure(_, _) => StatusCode::UNAUTHORIZED,
			ProxyError::AuthorizationFailed => StatusCode::FORBIDDEN,
			ProxyError::ExternalAuthorizationFailed(status) => status.unwrap_or(StatusCode::FORBIDDEN),

			ProxyError::DnsResolution => StatusCode::SERVICE_UNAVAILABLE,
			ProxyError::NoHealthyEndpoints => StatusCode::SERVICE_UNAVAILABLE,
			ProxyError::UpstreamCallFailed(_) => StatusCode::SERVICE_UNAVAILABLE,
			ProxyError::UpstreamCallTimeout => StatusCode::GATEWAY_TIMEOUT,

			ProxyError::RequestTimeout => StatusCode::GATEWAY_TIMEOUT,
			ProxyError::Processing(_) => StatusCode::SERVICE_UNAVAILABLE,
			ProxyError::Http(_) => StatusCode::SERVICE_UNAVAILABLE,
			ProxyError::Body(_) => StatusCode::SERVICE_UNAVAILABLE,
			ProxyError::ProcessingString(_) => StatusCode::SERVICE_UNAVAILABLE,
			ProxyError::RateLimitExceeded { .. } => StatusCode::TOO_MANY_REQUESTS,
			// Rate limit service communication failure is a server error (500), not a rate limit (429).
			// This matches Envoy's behavior (status_on_error defaults to 500).
			ProxyError::RateLimitFailed => StatusCode::INTERNAL_SERVER_ERROR,

			// Shouldn't happen on this path
			ProxyError::UpstreamTCPCallFailed(_) => StatusCode::INTERNAL_SERVER_ERROR,
			ProxyError::UpstreamTCPProxy(_) => StatusCode::INTERNAL_SERVER_ERROR,
			ProxyError::MCP(mcp::Error::MethodNotAllowed) => StatusCode::METHOD_NOT_ALLOWED,
			ProxyError::MCP(mcp::Error::InvalidAccept) => StatusCode::NOT_ACCEPTABLE,
			ProxyError::MCP(mcp::Error::InvalidContentType) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
			ProxyError::MCP(mcp::Error::Deserialize(_)) => StatusCode::BAD_REQUEST,
			ProxyError::MCP(mcp::Error::StartSession(_)) => StatusCode::INTERNAL_SERVER_ERROR,
			ProxyError::MCP(mcp::Error::UnknownSession) => StatusCode::NOT_FOUND,
			ProxyError::MCP(mcp::Error::MissingSessionHeader) => StatusCode::UNPROCESSABLE_ENTITY,
			ProxyError::MCP(mcp::Error::SessionIdRequired) => StatusCode::UNPROCESSABLE_ENTITY,
			ProxyError::MCP(mcp::Error::InvalidSessionIdQuery) => StatusCode::UNPROCESSABLE_ENTITY,
			ProxyError::MCP(mcp::Error::InvalidSessionIdHeader) => StatusCode::BAD_REQUEST,
			ProxyError::MCP(mcp::Error::CreateSseUrl(_)) => StatusCode::BAD_REQUEST,
			ProxyError::MCP(mcp::Error::EstablishGetStream(_)) => StatusCode::INTERNAL_SERVER_ERROR,
			ProxyError::MCP(mcp::Error::ForwardLegacySse(_)) => StatusCode::INTERNAL_SERVER_ERROR,
			ProxyError::MCP(mcp::Error::Stdio(_)) => StatusCode::INTERNAL_SERVER_ERROR,
			ProxyError::MCP(mcp::Error::OpenAPI(_)) => StatusCode::INTERNAL_SERVER_ERROR,
			ProxyError::MCP(mcp::Error::UpstreamError(e)) => return e.0.map(http::Body::from),
			ProxyError::MCP(mcp::Error::SendError(_, _)) => StatusCode::INTERNAL_SERVER_ERROR,
			// Note: we do not return a 401/403 here, as the obscure that it was rejected due to auth
			ProxyError::MCP(mcp::Error::Authorization(_, _, _)) => StatusCode::INTERNAL_SERVER_ERROR,
		};
		let msg = self.to_string();
		let mut rb = ::http::Response::builder().status(code);

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

		// Add WWW-Authenticate header for MCP failures
		if let ProxyError::McpJwtAuthenticationFailure(_, www) = &self {
			if let Ok(hv) = HeaderValue::try_from(www) {
				rb = rb.header(hyper::header::WWW_AUTHENTICATE, hv);
			}
			rb = rb.header("content-type", "application/json");
			return rb
				.body(http::Body::from(Bytes::from(
					r#"{"error":"unauthorized","error_description":"JWT token required"}"#,
				)))
				.unwrap();
		}
		if let ProxyError::MCP(ref e @ mcp::Error::SendError(ref id, _)) = self {
			let err = if let Some(req_id) = id {
				serde_json::to_string(&JsonRpcError {
					jsonrpc: Default::default(),
					id: req_id.clone(),
					error: ErrorData {
						code: ErrorCode::INTERNAL_ERROR,
						message: format!("failed to send message: {e}",).into(),
						data: None,
					},
				})
				.ok()
			} else {
				None
			};
			let msg = err.unwrap_or_else(|| format!("failed to send message: {e}"));
			return rb
				.header("content-type", "application/json")
				.body(http::Body::from(msg))
				.unwrap();
		}
		if let ProxyError::MCP(ref e @ mcp::Error::Authorization(ref req_id, _, _)) = self {
			let msg = serde_json::to_string(&JsonRpcError {
				jsonrpc: Default::default(),
				id: req_id.clone(),
				error: ErrorData {
					code: ErrorCode::INVALID_PARAMS,
					message: e.to_string().into(),
					data: None,
				},
			})
			.unwrap_or_default();
			return rb
				.header("content-type", "application/json")
				.body(http::Body::from(msg))
				.unwrap();
		}

		rb.header(hyper::header::CONTENT_TYPE, "text/plain")
			.body(http::Body::from(msg))
			.unwrap()
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
		BackendReference::Backend(name) => {
			let be = pi
				.stores
				.read_binds()
				.backend(name)
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
) -> Result<SimpleBackendWithPolicies, ProxyError> {
	resolve_simple_backend_with_policies(b, pi)
}

pub fn resolve_simple_backend_with_policies(
	b: &SimpleBackendReference,
	pi: &ProxyInputs,
) -> Result<SimpleBackendWithPolicies, ProxyError> {
	let (backend, inline_policies) = match b {
		SimpleBackendReference::Service { name, port } => {
			let svc = pi
				.stores
				.read_discovery()
				.services
				.get_by_namespaced_host(name)
				.ok_or(ProxyError::ServiceNotFound)?;
			(SimpleBackend::Service(svc, *port), Vec::default())
		},
		SimpleBackendReference::Backend(name) => {
			let be = pi
				.stores
				.read_binds()
				.backend(name)
				.ok_or(ProxyError::ServiceNotFound)?;
			(
				SimpleBackend::try_from(be.backend.clone()).map_err(|_| ProxyError::InvalidBackendType)?,
				be.inline_policies.clone(),
			)
		},
		SimpleBackendReference::InlineBackend(t) => (
			SimpleBackend::Opaque(
				ResourceName::new(strng::format!("{}", t), strng::EMPTY),
				t.clone(),
			),
			Vec::default(),
		),
		SimpleBackendReference::Invalid => (SimpleBackend::Invalid, Vec::default()),
	};
	Ok(SimpleBackendWithPolicies {
		backend,
		inline_policies,
	})
}
