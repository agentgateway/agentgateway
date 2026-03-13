use axum::response::Response;
use http::StatusCode;

use super::{CallbackValidation, Error, OAuth2, OAuth2CallContext, ProtocolEndpointKind};
use crate::client::Client;
use crate::http::{PolicyResponse, Request};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;

impl OAuth2 {
	#[tracing::instrument(
		skip_all,
		fields(provider_id = %self.config.provider_id, client_id = %self.config.client_id)
	)]
	pub async fn handle_protocol_endpoint(
		&self,
		client: &Client,
		policy_client: &PolicyClient,
		oidc: &crate::http::oauth2::OAuth2TokenService,
		req: &mut Request,
	) -> Result<Response, ProxyError> {
		tracing::debug!(path = req.uri().path(), "handling oauth2 protocol endpoint");
		let runtime = OAuth2CallContext {
			client,
			policy_client,
			oidc,
		};
		let response = match self.protocol_endpoint_kind(req.uri().path())? {
			Some(ProtocolEndpointKind::Callback) => {
				let redirect_uri = self.resolve_redirect_uri()?;
				let session_redirect = self.session_redirect_for_callback(req)?;
				if let Some(response) = session_redirect {
					Ok(response)
				} else {
					let (metadata, jwt_validator) = self.resolved_oidc_info()?;
					self
						.handle_callback(
							runtime,
							req.headers(),
							req.uri(),
							CallbackValidation {
								metadata: &metadata,
								jwt_validator: jwt_validator.as_deref(),
							},
							&redirect_uri,
						)
						.await
				}
			},
			None => Err(ProxyError::AuthPolicyConflict(
				"oauth2 protocol endpoint dispatch requires a callback path",
			)),
		}?;
		Self::into_direct_response(response)
	}

	fn session_redirect_for_callback(
		&self,
		req: &Request,
	) -> Result<Option<PolicyResponse>, ProxyError> {
		let redirect_uri = self.resolve_redirect_uri()?;
		if req.uri().path() != redirect_uri.path() {
			return Ok(None);
		}
		let Some(session) = self.get_session(req.headers()) else {
			return Ok(None);
		};
		if session.is_expired() {
			return Ok(None);
		}
		let target = self
			.extract_original_url(req.uri())
			.unwrap_or_else(|| "/".into());
		let target = if Self::is_safe_redirect_target(&target) {
			target
		} else {
			"/".to_string()
		};
		let resp = Response::builder()
			.status(StatusCode::FOUND)
			.header(http::header::LOCATION, target)
			.body(Default::default())
			.map_err(|e| {
				ProxyError::from(Error::Internal(format!(
					"failed to build redirect response: {e}"
				)))
			})?;
		Ok(Some(PolicyResponse::default().with_response(resp)))
	}

	fn protocol_endpoint_kind(&self, path: &str) -> Result<Option<ProtocolEndpointKind>, ProxyError> {
		if self.resolve_redirect_uri()?.path() == path {
			return Ok(Some(ProtocolEndpointKind::Callback));
		}
		Ok(None)
	}

	fn extract_original_url(&self, uri: &http::Uri) -> Option<String> {
		let state_str = Self::query_param(uri, "state")?;
		let state = self.decrypt_handshake_state(&state_str).ok()?;
		Some(state.original_url)
	}
}
