use aws_lc_rs::constant_time::verify_slices_are_equal;
use base64::Engine;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::http::Request;
use crate::proxy::httpproxy::PolicyClient;
use tracing::debug;

use super::provider;
use super::session::{
	BrowserSession, TransactionState, generate_nonce, generate_pkce_verifier, generate_state,
	generate_transaction_id, normalize_original_uri,
};
use super::{Error, OidcPolicy, build_redirect_response, cap_session_expiry, now_unix};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CallbackTransactionState {
	pub transaction_id: String,
	pub csrf_state: String,
}

pub(super) struct CallbackRequestContext {
	pub code: String,
	pub callback_state: CallbackTransactionState,
	pub transaction_cookie_name: String,
	pub transaction_cookie: String,
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
	let location = with_query(
		&policy.provider.authorization_endpoint,
		&[
			("response_type", "code".into()),
			("client_id", policy.client.client_id.clone()),
			("redirect_uri", policy.redirect_uri.redirect_uri.clone()),
			("scope", policy.scopes.join(" ")),
			("state", state),
			("nonce", nonce),
			("code_challenge", code_challenge),
			("code_challenge_method", "S256".into()),
		],
	);
	let response = build_redirect_response(&location, &[cookie])?;
	Ok(crate::http::PolicyResponse::default().with_response(response))
}

pub(super) async fn handle_callback(
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

fn with_query(uri: &super::ProviderEndpoint, params: &[(&str, String)]) -> String {
	uri.with_query(params)
}

fn constant_time_str_eq(expected: &str, actual: &str) -> bool {
	verify_slices_are_equal(expected.as_bytes(), actual.as_bytes()).is_ok()
}
