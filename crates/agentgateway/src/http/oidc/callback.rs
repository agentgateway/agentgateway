use base64::Engine;
use secrecy::SecretString;
use serde_json::Value;

use crate::http::Request;
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::RequestLog;
use tracing::debug;

use super::provider;
use super::session::{
	BrowserSession, TransactionState, generate_nonce, generate_pkce_verifier, generate_state,
	normalize_original_uri,
};
use super::{Error, OidcPolicy, build_redirect_response, cap_session_expiry, now_unix};

pub(super) struct CallbackRequestContext {
	pub is_https: bool,
	pub code: String,
	pub state: String,
	pub transaction_cookie: String,
}

pub(super) fn start_login(
	policy: &OidcPolicy,
	_log: Option<&mut RequestLog>,
	req: &Request,
) -> Result<crate::http::PolicyResponse, Error> {
	let is_https = req.uri().scheme_str() == Some("https");
	let state = generate_state();
	let nonce = generate_nonce();
	let pkce_verifier = generate_pkce_verifier();
	let original_uri = normalize_original_uri(req.uri().path_and_query());
	let transaction = TransactionState {
		policy_id: policy.policy_id.clone(),
		csrf_state: state.clone(),
		nonce: nonce.clone(),
		pkce_verifier: SecretString::new(pkce_verifier.clone().into_boxed_str()),
		original_uri,
		expires_at_unix: now_unix().saturating_add(policy.session.transaction_ttl.as_secs()),
	};
	let encoded = policy.session.encode_transaction(&transaction)?;
	let cookie = policy.session.set_cookie(
		&policy.session.transaction_cookie_name,
		&encoded,
		is_https,
		policy.session.transaction_ttl,
	);
	let code_challenge = {
		let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, pkce_verifier.as_bytes());
		base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest.as_ref())
	};
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
	_log: Option<&mut RequestLog>,
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
	if transaction.csrf_state != context.state {
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
	if nonce != transaction.nonce {
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
		context.is_https,
		policy.session.ttl,
	);
	let clear_transaction = policy
		.session
		.clear_cookie(&policy.session.transaction_cookie_name, context.is_https);
	let location = validated_original_uri(&transaction.original_uri);
	let response = build_redirect_response(&location, &[session_cookie, clear_transaction])?;
	Ok(crate::http::PolicyResponse::default().with_response(response))
}

fn with_query(uri: &http::Uri, params: &[(&str, String)]) -> String {
	let mut url =
		url::Url::parse(&uri.to_string()).expect("authorization endpoint must be a valid URL");
	{
		let mut query = url.query_pairs_mut();
		for (key, value) in params {
			query.append_pair(key, value);
		}
	}
	url.to_string()
}

pub(super) fn validated_original_uri(original_uri: &str) -> String {
	const LIMIT: usize = 2048;
	if original_uri.len() > LIMIT {
		return "/".into();
	}

	let decoded = percent_encoding::percent_decode_str(original_uri)
		.decode_utf8_lossy()
		.into_owned();
	let valid = original_uri.starts_with('/')
		&& !original_uri.starts_with("//")
		&& !original_uri.contains('\\')
		&& decoded.starts_with('/')
		&& !decoded.starts_with("//")
		&& !decoded.contains('\\')
		&& http::uri::PathAndQuery::try_from(original_uri).is_ok();
	if valid {
		original_uri.to_string()
	} else {
		"/".into()
	}
}
