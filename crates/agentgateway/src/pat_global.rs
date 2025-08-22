#![cfg(feature = "pat")]
//! Global PAT authentication singleton for cache sharing across all routes

use crate::http::pat::PatAuth;
use sqlx::PgPool;
use std::sync::{Arc, OnceLock};

/// Global PAT authenticator instance shared across all routes
static GLOBAL_PAT_AUTH: OnceLock<Arc<PatAuth>> = OnceLock::new();

/// Initialize the global PAT authenticator.
/// This should be called once during application startup when PAT is enabled.
/// Returns the Arc to the global instance.
pub fn init_global_pat_auth(pool: PgPool) -> Arc<PatAuth> {
	GLOBAL_PAT_AUTH
		.get_or_init(|| {
			tracing::info!(
				target = "audit",
				action = "pat.global.init",
				"Initializing global PAT authenticator"
			);
			Arc::new(PatAuth::new(pool))
		})
		.clone()
}

/// Get the global PAT authenticator if it has been initialized
pub fn get_global_pat_auth() -> Option<Arc<PatAuth>> {
	GLOBAL_PAT_AUTH.get().cloned()
}
