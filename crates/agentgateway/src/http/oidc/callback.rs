use aws_lc_rs::constant_time::verify_slices_are_equal;
use base64::Engine;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;

use super::session::{
	BrowserSession, TransactionState, generate_nonce, generate_pkce_verifier, generate_state,
	generate_transaction_id, normalize_original_uri,
};
use super::{Error, OidcPolicy, build_redirect_response, cap_session_expiry, now_unix, provider};
use crate::http::Request;
use crate::proxy::httpproxy::PolicyClient;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CallbackTransactionState {
	pub transaction_id: String,
	pub csrf_state: String,
}

struct CallbackRequestContext {
	code: String,
	callback_state: CallbackTransactionState,
	transaction_cookie_name: String,
	transaction_cookie: String,
}

impl CallbackTransactionState {
	pub fn encode(&self) -> String {
		let json =
			serde_json::to_vec(self).expect("serializing callback transaction state is infallible");
		base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)
	}

	pub fn decode(value: &str) -> Result<Self, Error> {
		let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
			.decode(value)
			.map_err(|_| Error::InvalidCallback)?;
		serde_json::from_slice(&raw).map_err(|_| Error::InvalidCallback)
	}
}

struct CallbackQuery {
	state: String,
	code: Option<String>,
	error: Option<String>,
}

impl CallbackQuery {
	/// Parse callback query parameters from the request in a single pass.
	/// Returns `None` if the query does not contain `state` + (`code` | `error`),
	/// meaning this request is not an OAuth2 callback.
	fn parse(req: &Request) -> Option<Self> {
		let mut state = None;
		let mut code = None;
		let mut error = None;
		for (key, value) in
			url::form_urlencoded::parse(req.uri().query().unwrap_or_default().as_bytes())
		{
			match key.as_ref() {
				"state" => state = Some(value.into_owned()),
				"code" => code = Some(value.into_owned()),
				"error" => error = Some(value.into_owned()),
				_ => {},
			}
		}
		let state = state?;
		if code.is_none() && error.is_none() {
			return None;
		}
		Some(CallbackQuery { state, code, error })
	}
}

/// Intercept the OAuth2 redirect callback. Returns `Ok(None)` when the
/// request is not a callback for this policy (wrong method/path, or the query
/// is not a callback query — see [`CallbackQuery::parse`]). A provider
/// `error` is honored only after the transaction cookie is found, so an
/// attacker cannot clear state with a forged error response.
pub(super) async fn maybe_handle(
	policy: &super::OidcPolicy,
	req: &mut Request,
	client: PolicyClient,
) -> Result<Option<crate::http::PolicyResponse>, Error> {
	if req.method() != ::http::Method::GET
		|| req.uri().path() != policy.redirect_uri.callback_path.path()
	{
		return Ok(None);
	}

	let Some(query) = CallbackQuery::parse(req) else {
		return Ok(None);
	};

	let callback_state = CallbackTransactionState::decode(&query.state)?;
	let transaction_cookie_name = policy
		.session
		.transaction_cookie_name(&callback_state.transaction_id);
	let transaction_cookie = crate::http::read_request_cookie(req, &transaction_cookie_name)
		.ok_or(Error::MissingTransaction)?
		.to_string();
	if let Some(error) = query.error {
		return Err(Error::ProviderCallback(error));
	}
	let code = query.code.ok_or(Error::InvalidCallback)?;
	let response = handle_callback(
		policy,
		CallbackRequestContext {
			code,
			callback_state,
			transaction_cookie_name,
			transaction_cookie,
		},
		client,
	)
	.await?;
	Ok(Some(response))
}

pub(super) fn start_login(
	policy: &OidcPolicy,
	req: &Request,
) -> Result<crate::http::PolicyResponse, Error> {
	let transaction_id = generate_transaction_id();
	let csrf_state = generate_state();
	let nonce = generate_nonce();
	let pkce_verifier = generate_pkce_verifier();
	let code_challenge = {
		let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, pkce_verifier.as_bytes());
		base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest.as_ref())
	};
	let original_uri = normalize_original_uri(req.uri().path_and_query());
	let transaction = TransactionState {
		policy_id: policy.policy_id.clone(),
		transaction_id: transaction_id.clone(),
		csrf_state: csrf_state.clone(),
		nonce: nonce.clone(),
		pkce_verifier: SecretString::new(pkce_verifier.into_boxed_str()),
		original_uri,
		expires_at_unix: now_unix().saturating_add(policy.session.transaction_ttl.as_secs()),
	};
	let callback_state = CallbackTransactionState {
		transaction_id,
		csrf_state,
	};
	let state = callback_state.encode();
	let encoded = policy.session.encode_transaction(&transaction)?;
	let transaction_cookie_name = policy
		.session
		.transaction_cookie_name(&callback_state.transaction_id);
	let cookie = policy.session.set_cookie(
		&transaction_cookie_name,
		&encoded,
		policy.redirect_uri.https,
		policy.session.transaction_ttl,
	);
	let location = policy.provider.authorization_endpoint.with_query(&[
		("response_type", "code".into()),
		("client_id", policy.client.client_id.clone()),
		("redirect_uri", policy.redirect_uri.redirect_uri.clone()),
		("scope", policy.scopes.join(" ")),
		("state", state),
		("nonce", nonce),
		("code_challenge", code_challenge),
		("code_challenge_method", "S256".into()),
	]);
	let response = build_redirect_response(&location, &[cookie])?;
	Ok(crate::http::PolicyResponse::default().with_response(response))
}

async fn handle_callback(
	policy: &OidcPolicy,
	context: CallbackRequestContext,
	client: PolicyClient,
) -> Result<crate::http::PolicyResponse, Error> {
	let transaction = policy
		.session
		.decode_transaction(&context.transaction_cookie)?;
	if transaction.policy_id != policy.policy_id {
		debug!("oidc callback rejected due to policy mismatch");
		return Err(Error::PolicyMismatch);
	}
	if !constant_time_str_eq(
		&transaction.transaction_id,
		&context.callback_state.transaction_id,
	) {
		debug!("oidc callback rejected due to transaction mismatch");
		return Err(Error::InvalidTransaction);
	}
	if !constant_time_str_eq(&transaction.csrf_state, &context.callback_state.csrf_state) {
		debug!("oidc callback rejected due to csrf mismatch");
		return Err(Error::CsrfMismatch);
	}

	let token = provider::exchange_code(
		client,
		&policy.provider,
		&policy.client,
		&policy.redirect_uri.redirect_uri,
		&context.code,
		&transaction.pkce_verifier,
		policy.provider_backend.as_ref(),
	)
	.await?;
	let id_token = token.id_token.ok_or(Error::MissingIdToken)?;
	let claims = policy
		.provider
		.id_token_validator
		.validate_claims(&id_token)
		.map_err(Error::InvalidIdToken)?;
	let nonce = claims
		.inner
		.get("nonce")
		.and_then(Value::as_str)
		.ok_or(Error::NonceMismatch)?;
	if !constant_time_str_eq(nonce, &transaction.nonce) {
		debug!("oidc callback rejected due to nonce mismatch");
		return Err(Error::NonceMismatch);
	}

	// TODO: Revisit whether browser sessions should persist access_token / refresh_token.
	// The current stateless cookie only stores the validated id_token because that is what
	// the runtime uses today, and larger token payloads can exceed browser cookie limits.
	let session = BrowserSession {
		policy_id: policy.policy_id.clone(),
		raw_id_token: SecretString::new(id_token.into_boxed_str()),
		expires_at_unix: Some(cap_session_expiry(
			now_unix(),
			policy.session.ttl,
			&claims.inner,
		)),
	};
	let encoded = policy.session.encode_browser_session(&session)?;
	let session_cookie = policy.session.set_cookie(
		&policy.session.cookie_name,
		&encoded,
		policy.redirect_uri.https,
		policy.session.ttl,
	);
	let clear_transaction = policy
		.session
		.clear_cookie(&context.transaction_cookie_name, policy.redirect_uri.https);
	let location = transaction.original_uri;
	let response = build_redirect_response(&location, &[session_cookie, clear_transaction])?;
	Ok(crate::http::PolicyResponse::default().with_response(response))
}

fn constant_time_str_eq(expected: &str, actual: &str) -> bool {
	verify_slices_are_equal(expected.as_bytes(), actual.as_bytes()).is_ok()
}
