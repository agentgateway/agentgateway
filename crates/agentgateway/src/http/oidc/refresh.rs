use std::sync::Arc;
use std::time::Duration;

use ::http::{HeaderMap, HeaderValue, header};
use futures::future::{BoxFuture, FutureExt, Shared};
use quick_cache::sync::{Cache, EntryAction, EntryResult};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use tokio::time::Instant;
use tracing::debug;

use super::{
	BrowserSession, Error, OidcPolicy, RefreshSession, cap_session_expiry, now_unix, provider,
};
use crate::http::{PolicyResponse, Request, jwt};
use crate::proxy::httpproxy::PolicyClient;

#[derive(Clone, Debug)]
pub(super) struct RefreshCache(Arc<Cache<[u8; 32], Shared<BoxFuture<'static, CachedRefresh>>>>);

impl Default for RefreshCache {
	fn default() -> Self {
		Self(Arc::new(Cache::new(1024)))
	}
}

#[derive(Clone)]
struct CachedRefresh {
	result: Result<Arc<RefreshedSession>, Arc<Error>>,
	expires_at: Instant,
}

struct RefreshedSession {
	claims: jwt::Claims,
	response_headers: HeaderMap,
	expires_at_unix: u64,
}

impl OidcPolicy {
	pub(super) async fn refresh_browser_session(
		&self,
		req: &mut Request,
		browser_session: BrowserSession,
		client: PolicyClient,
	) -> Result<Option<(jwt::Claims, PolicyResponse)>, Arc<Error>> {
		let Some(cookie) = crate::http::read_request_cookie(req, &self.session.refresh_cookie_name)
		else {
			return Ok(None);
		};
		let refresh_session = self
			.session
			.decode_refresh_session(&cookie)
			.map_err(Arc::new)?;
		// Bind the independently encrypted cookies to the same policy and original subject.
		if refresh_session.policy_id != browser_session.policy_id
			|| refresh_session.subject != browser_session.subject
		{
			return Err(Arc::new(Error::InvalidSession));
		}

		let subject = browser_session.subject.clone();
		let key =
			crate::crypto::digest::sha256(refresh_session.refresh_token.expose_secret().as_bytes());
		let refresh = match self
			.refresh_cache
			.0
			.entry_async(&key, |_, refresh| {
				if refresh
					.peek()
					.is_none_or(|cached| cached.expires_at > Instant::now())
				{
					EntryAction::Retain(refresh.clone())
				} else {
					EntryAction::ReplaceWithGuard
				}
			})
			.await
		{
			EntryResult::Retained(refresh) => refresh,
			EntryResult::Vacant(guard) | EntryResult::Replaced(guard, _) => {
				let policy = self.clone();
				let refresh = async move {
					let result = policy
						.fetch_browser_session(browser_session, refresh_session, client)
						.await
						.map(Arc::new)
						.map_err(Arc::new);
					let ttl = if result
						.as_ref()
						.is_err_and(|err| !err.is_terminal_refresh_failure())
					{
						Duration::from_secs(1)
					} else {
						Duration::from_secs(30)
					};
					CachedRefresh {
						result,
						expires_at: Instant::now() + ttl,
					}
				}
				.boxed()
				.shared();
				// Finish and cache the exchange even if every waiting request disconnects after the
				// provider has consumed a rotating refresh token.
				tokio::spawn(refresh.clone());
				let _ = guard.insert(refresh.clone());
				refresh
			},
			EntryResult::Removed(_, _) | EntryResult::Timeout => unreachable!(),
		};
		let refreshed = refresh.await.result?;

		if refreshed.expires_at_unix <= now_unix()
			|| refreshed.claims.inner.get("sub").and_then(Value::as_str) != subject.as_deref()
		{
			return Err(Arc::new(Error::InvalidSession));
		}
		Ok(Some((
			refreshed.claims.clone(),
			PolicyResponse {
				direct_response: None,
				response_headers: Some(refreshed.response_headers.clone()),
			},
		)))
	}

	pub(super) fn refresh_cookie(
		&self,
		refresh_token: Option<SecretString>,
		subject: Option<String>,
	) -> Result<String, Error> {
		if let Some(refresh_token) = refresh_token {
			let refresh_session = RefreshSession {
				policy_id: self.policy_id.clone(),
				subject,
				refresh_token,
				expires_at_unix: now_unix().saturating_add(self.session.ttl.as_secs()),
			};
			match self.session.encode_refresh_session(&refresh_session) {
				Ok(encoded) => {
					return Ok(self.session.set_cookie(
						&self.session.refresh_cookie_name,
						&encoded,
						self.redirect_uri.https,
						self.session.ttl,
					));
				},
				Err(Error::SessionCookieTooLarge) => {
					debug!("oidc refresh token exceeds cookie size budget; refresh disabled for session");
				},
				Err(err) => return Err(err),
			}
		}
		Ok(
			self
				.session
				.clear_cookie(&self.session.refresh_cookie_name, self.redirect_uri.https),
		)
	}

	async fn fetch_browser_session(
		&self,
		mut browser_session: BrowserSession,
		refresh_session: RefreshSession,
		client: PolicyClient,
	) -> Result<RefreshedSession, Error> {
		let token = provider::refresh_token(
			client,
			&self.provider,
			&self.client,
			&refresh_session.refresh_token,
		)
		.await?;
		let id_token = token.id_token.ok_or(Error::MissingIdToken)?;
		let claims = self
			.provider
			.id_token_validator
			.validate_claims(&id_token)
			.map_err(Error::InvalidIdToken)?;
		// OIDC Core 1.0 section 12.2 requires a refreshed ID token to have the same subject as the
		// original ID token. Normal signature, issuer, and audience validation alone would still
		// accept a valid token issued for a different user.
		if claims.inner.get("sub").and_then(Value::as_str) != browser_session.subject.as_deref() {
			return Err(Error::InvalidSession);
		}

		let expires_at_unix = cap_session_expiry(now_unix(), self.session.ttl, &claims.inner);
		browser_session.raw_id_token = SecretString::new(id_token.into_boxed_str());
		browser_session.expires_at_unix = Some(expires_at_unix);
		let encoded_session = self.session.encode_browser_session(&browser_session)?;
		let session_cookie = self.session.set_cookie(
			&self.session.cookie_name,
			&encoded_session,
			self.redirect_uri.https,
			self.session.ttl,
		);
		let refresh_token = token
			.refresh_token
			.map(SecretString::from)
			.unwrap_or(refresh_session.refresh_token);
		let refresh_cookie = self.refresh_cookie(Some(refresh_token), browser_session.subject)?;
		let mut response_headers = HeaderMap::new();
		for cookie in [session_cookie, refresh_cookie] {
			response_headers.append(
				header::SET_COOKIE,
				HeaderValue::from_str(&cookie)
					.map_err(|e| Error::Config(format!("invalid set-cookie header: {e}")))?,
			);
		}
		Ok(RefreshedSession {
			claims,
			response_headers,
			expires_at_unix,
		})
	}
}
