use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::types::agent::OAuth2AttachmentKey;

#[derive(Serialize, Deserialize)]
pub(super) struct HandshakeState {
	pub(super) attachment_key: OAuth2AttachmentKey,
	pub(super) original_url: String,
	pub(super) nonce: String,
	pub(super) pkce_verifier: Option<String>,
	pub(super) expires_at: SystemTime,
	#[serde(default)]
	pub(super) handshake_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct SessionState {
	pub(super) access_token: String,
	pub(super) refresh_token: Option<String>,
	pub(super) expires_at: SystemTime,
	#[serde(default)]
	pub(super) nonce: Option<String>,
	#[serde(default)]
	pub(super) id_token: Option<String>,
}

impl SessionState {
	pub(super) fn is_expired(&self) -> bool {
		SystemTime::now() > self.expires_at
	}

	pub(super) fn cookie_max_age(&self) -> cookie::time::Duration {
		if self.refresh_token.is_some() {
			let seconds =
				i64::try_from(super::DEFAULT_REFRESHABLE_COOKIE_MAX_AGE.as_secs()).unwrap_or(i64::MAX);
			return cookie::time::Duration::seconds(seconds);
		}
		let remaining = self
			.expires_at
			.duration_since(SystemTime::now())
			.unwrap_or_default();
		let seconds = remaining.as_secs().max(1);
		let seconds = i64::try_from(seconds).unwrap_or(i64::MAX);
		cookie::time::Duration::seconds(seconds)
	}
}
