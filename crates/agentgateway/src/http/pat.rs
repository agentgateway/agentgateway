#![cfg(feature = "pat")]
use std::net::IpAddr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::Result;
use argon2::{
	Algorithm, Argon2, Params, PasswordHash, PasswordVerifier, Version,
	password_hash::{PasswordHasher, SaltString, rand_core::OsRng},
};
use chrono::{DateTime, Utc};
use moka::future::Cache;
use secrecy::{ExposeSecret, SecretString};
use sqlx::postgres::PgListener;
// (De)serialization derives are intentionally minimized here; TokenRow is mapped into a public
// representation elsewhere so we don't need serde derives on it.
use rand::Rng as _;
use rand::distr::Alphanumeric;
use sqlx::{PgPool, types::Uuid};
use tracing::debug;

use crate::http::jwt::Claims;
use serde_json::{Map, Value};

const PREFIX_LEN: usize = 24;

// Explicit Argon2id parameters (default replaced) for consistent, auditable hashing.
const ARGON2_M_COST: u32 = 64 * 1024; // 64 MiB
const ARGON2_T_COST: u32 = 3; // iterations
const ARGON2_P_COST: u32 = 1; // parallelism (lanes)

fn argon2_instance() -> Argon2<'static> {
	let params =
		Params::new(ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST, None).expect("valid argon2 params");
	Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

// Revocation + cache tuning constants (parity with source branch expectations)
const POS_CACHE_TTL_SECS: u64 = 300;
const NEG_CACHE_TTL_SECS: u64 = 15;
const NEG_CACHE_CAP: u64 = 50_000;
const POS_CACHE_CAP: u64 = 100_000;
const REVOCATION_CACHE_CAP: u64 = 50_000;
const REVOCATION_TTL_SECS: u64 = 86_400; // 24h, conservatively > max token expiry scan interval

static REVOKED_PREFIXES: OnceLock<moka::future::Cache<String, ()>> = OnceLock::new();
fn revoked_cache() -> &'static moka::future::Cache<String, ()> {
	REVOKED_PREFIXES.get_or_init(|| {
		moka::future::Cache::builder()
			.time_to_live(Duration::from_secs(REVOCATION_TTL_SECS))
			.max_capacity(REVOCATION_CACHE_CAP)
			.build()
	})
}
pub fn mark_pat_revoked(prefix: &str) {
	// Insert asynchronously; we don't need to await this for correctness and
	// want to avoid unused Future warnings at call sites.
	let fut = revoked_cache().insert(prefix.to_string(), ());
	tokio::spawn(async move {
		let _ = fut.await;
	});
}

// Optional dedicated DB URL for revocation LISTEN to isolate from main pool saturation.
static PAT_DB_URL: OnceLock<String> = OnceLock::new();
#[cfg(feature = "pat")]
pub fn set_pat_db_url(url: &str) {
	let _ = PAT_DB_URL.set(url.to_string());
}
fn pat_db_url() -> Option<&'static str> {
	PAT_DB_URL.get().map(|s| s.as_str())
}

#[derive(thiserror::Error, Debug)]
pub enum PatError {
	#[error("missing bearer token")]
	Missing,
	#[error("not a PAT")]
	NotPat,
	#[error("not found")]
	NotFound,
	#[error("revoked/expired")]
	Disabled,
	#[error("invalid token")]
	Invalid,
	#[error("internal: {0}")]
	Internal(String),
}

#[derive(Clone, Debug)]
pub struct PatAuth {
	repo: TokenRepo,
	pos: Cache<String, Claims>,
	neg: Cache<String, ()>,
}

// Resolve the configured base prefix (default "agpk"). This is the static discriminator preceding
// any environment tag, allowing operators to change branding or namespace without touching code.
pub fn token_base_prefix() -> &'static str {
	static BASE: OnceLock<String> = OnceLock::new();
	BASE.get_or_init(|| {
		let raw = std::env::var("PAT_TOKEN_PREFIX").unwrap_or_else(|_| "agpk".to_string());
		// Basic sanitation: trim and fallback if empty
		let trimmed = raw.trim();
		if trimmed.is_empty() {
			"agpk".to_string()
		} else {
			trimmed.to_string()
		}
	})
}

impl PatAuth {
	pub fn new(pool: PgPool) -> Self {
		let me = Self {
			repo: TokenRepo::new(pool),
			pos: Cache::builder()
				.time_to_live(Duration::from_secs(POS_CACHE_TTL_SECS))
				.max_capacity(POS_CACHE_CAP)
				.build(),
			neg: Cache::builder()
				.time_to_live(Duration::from_secs(NEG_CACHE_TTL_SECS))
				.max_capacity(NEG_CACHE_CAP)
				.build(),
		};
		me.spawn_revocation_listener();
		me.spawn_index_check();
		me
	}

	fn spawn_revocation_listener(&self) {
		let pool = self.repo.pool.clone();
		let pos_cache = self.pos.clone();
		tokio::spawn(async move {
			let mut backoff = Duration::from_secs(1);
			loop {
				// Use dedicated connection if PAT_DB_URL set, else borrow from pool
				let dedicated = pat_db_url();
				let listener_res = if let Some(url) = dedicated {
					PgListener::connect(url).await
				} else {
					PgListener::connect_with(&pool).await
				};
				match listener_res {
					Ok(mut listener) => {
						if dedicated.is_some() {
							tracing::info!(
								target = "audit",
								action = "pat.revoke.listener",
								mode = "dedicated_conn",
								"revocation listener using dedicated connection"
							);
						}
						match listener.listen("pat_revoked").await {
							Ok(_) => {
								tracing::info!(
									target = "audit",
									action = "pat.revoke.listener",
									status = "started",
									"revocation listener active"
								);
								backoff = Duration::from_secs(1);
								while let Ok(notification) = listener.recv().await {
									let payload = notification.payload().to_string();
									mark_pat_revoked(&payload);
									// Invalidate the singleton's positive cache
									pos_cache.invalidate(&payload).await;
									tracing::debug!(target="audit", action="pat.revoke.propagate", token_prefix=%payload, "propagated token revocation");
								}
								tracing::warn!(
									target = "audit",
									action = "pat.revoke.listener",
									status = "disconnected",
									"revocation listener disconnected; will retry"
								);
							},
							Err(e) => {
								tracing::warn!(target="audit", action="pat.revoke.listener", status="listen_failed", error=%e, "failed to LISTEN; will retry");
							},
						}
					},
					Err(e) => {
						tracing::warn!(target="audit", action="pat.revoke.listener", status="connect_failed", error=%e, "failed to connect PgListener; will retry");
					},
				}
				// Add jitter to prevent thundering herd
				let jitter = Duration::from_millis(rand::rng().random_range(0..1000));
				tokio::time::sleep(backoff + jitter).await;
				backoff = (backoff * 2).min(Duration::from_secs(60));
			}
		});
	}

	pub async fn authenticate(
		&self,
		authz_value: &str,
		peer_ip: Option<IpAddr>,
	) -> Result<(Claims, ZeroToken), PatError> {
		let token = authz_value
			.strip_prefix("Bearer ")
			.ok_or(PatError::Missing)?;
		if !token.starts_with(&format!("{}_", token_base_prefix())) {
			tracing::warn!(target = "audit", action = "pat.auth", outcome = "not_pat");
			return Err(PatError::NotPat);
		}
		if token.len() < PREFIX_LEN {
			tracing::info!(target = "audit", action = "pat.auth", outcome = "too_short");
			return Err(PatError::Invalid);
		}
		let prefix: String = token.get(..PREFIX_LEN).unwrap_or(token).to_string();
		if self.neg.get(&prefix).await.is_some() {
			tracing::info!(
				target = "audit",
				action = "pat.auth",
				outcome = "cached_negative"
			);
			return Err(PatError::NotFound);
		}
		if let Some(p) = self.pos.get(&prefix).await {
			if revoked_cache().get(&prefix).await.is_some() {
				self.pos.invalidate(&prefix).await;
			} else {
				return Ok((p, ZeroToken::new(token)));
			}
		}
		let t0 = Instant::now();
		let rec_opt = self
			.repo
			.find_active_by_prefix(&prefix)
			.await
			.map_err(|e| PatError::Internal(e.to_string()))?;
		let rec = match rec_opt {
			Some(r) => r,
			None => {
				tracing::info!(target="audit", action="pat.auth", prefix=%prefix, outcome="not_found");
				return Err(PatError::NotFound);
			},
		};
		let parsed = PasswordHash::new(&rec.key_hash).map_err(|_| PatError::Invalid)?;
		if argon2_instance()
			.verify_password(token.as_bytes(), &parsed)
			.is_err()
		{
			self.neg.insert(prefix.clone(), ()).await;
			tracing::info!(target="audit", action="pat.auth", prefix=%prefix, outcome="invalid_hash");
			return Err(PatError::Invalid);
		}
		let mut claims_map = Map::with_capacity(6);
		claims_map.insert("sub".to_string(), Value::String(rec.user_id.clone()));
		claims_map.insert(
			"tenant_id".to_string(),
			Value::String(rec.tenant_id.clone()),
		);
		claims_map.insert("token_type".to_string(), Value::String("pat".to_string()));
		if !rec.scopes.is_empty() {
			claims_map.insert("scope".to_string(), Value::String(rec.scopes.join(" ")));
		}
		if let Some(email) = &rec.creator_email {
			claims_map.insert("email".to_string(), Value::String(email.clone()));
		}
		if !rec.creator_groups.is_empty() {
			claims_map.insert(
				"groups".to_string(),
				Value::Array(
					rec
						.creator_groups
						.iter()
						.map(|g| Value::String(g.clone()))
						.collect(),
				),
			);
		}
		let claims = Claims {
			inner: claims_map.clone(),
			jwt: SecretString::new(format!("pat:{}", rec.token_prefix).into()),
		};
		self
			.pos
			.insert(rec.token_prefix.clone(), claims.clone())
			.await;
		let repo = self.repo.clone();
		tokio::spawn(async move {
			let _ = repo.touch_last_used(rec.id, peer_ip).await;
		});
		debug!(took_ms=%t0.elapsed().as_millis(), prefix=%prefix, "pat validated");
		Ok((claims, ZeroToken::new(token)))
	}
	pub fn repo(&self) -> &TokenRepo {
		&self.repo
	}
}

impl PatAuth {
	fn spawn_index_check(&self) {
		let pool = self.repo.pool.clone();
		tokio::spawn(async move {
			if let Ok(rows) = sqlx::query_scalar::<_, String>("SELECT indexrelid::regclass::text FROM pg_index i JOIN pg_class c ON c.oid=i.indrelid JOIN pg_class ic ON ic.oid=i.indexrelid WHERE c.relname='proxy_keys'").fetch_all(&pool).await {
				let have: std::collections::HashSet<_> = rows.iter().map(|s| s.as_str()).collect();
				if !have.iter().any(|n| n.contains("token_prefix")) {
					tracing::warn!(target="audit", action="pat.index.check", missing="token_prefix", suggestion = "CREATE UNIQUE INDEX IF NOT EXISTS proxy_keys_token_prefix_idx ON proxy_keys(token_prefix)");
				}
				if !have.iter().any(|n| n.contains("tenant_user_created_at")) {
					tracing::warn!(target="audit", action="pat.index.check", missing="tenant_user_created_at", suggestion = "CREATE INDEX IF NOT EXISTS proxy_keys_tenant_user_created_at_idx ON proxy_keys(tenant_id, user_id, created_at DESC)");
				}
			}
		});
	}
}

#[derive(Clone)]
pub struct ZeroToken(SecretString);
impl ZeroToken {
	pub fn new(raw: &str) -> Self {
		Self(SecretString::new(raw.to_string().into_boxed_str()))
	}
	pub fn expose(&self) -> &str {
		self.0.expose_secret()
	}
}
impl std::fmt::Debug for ZeroToken {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "<redacted>")
	}
}

#[derive(sqlx::FromRow, Clone)]
pub struct TokenRow {
	pub id: Uuid,
	pub tenant_id: String,
	pub user_id: String,
	pub name: Option<String>,
	pub token_prefix: String,
	pub hash_algo: String,
	pub hash_params: serde_json::Value,
	pub key_hash: String,
	pub scopes: Vec<String>,
	pub created_by: Option<String>,
	pub creator_email: Option<String>,
	pub creator_groups: Vec<String>,
	pub created_at: DateTime<Utc>,
	pub last_used_at: Option<DateTime<Utc>>,
	pub last_used_ip: Option<String>,
	pub expires_at: Option<DateTime<Utc>>,
	pub revoked_at: Option<DateTime<Utc>>,
}

// Minimal projection for listing tokens (excludes hash fields)
#[derive(sqlx::FromRow, Clone, Debug)]
pub struct PublicTokenRow {
	pub id: Uuid,
	pub token_prefix: String,
	pub scopes: Vec<String>,
	pub creator_email: Option<String>,
	pub creator_groups: Vec<String>,
	pub created_at: DateTime<Utc>,
	pub last_used_at: Option<DateTime<Utc>>,
	pub expires_at: Option<DateTime<Utc>>,
	pub revoked_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for TokenRow {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("TokenRow")
			.field("id", &self.id)
			.field("tenant_id", &self.tenant_id)
			.field("user_id", &self.user_id)
			.field("name", &self.name)
			.field("token_prefix", &self.token_prefix)
			// intentionally omit hash_algo, hash_params, key_hash
			.field("scopes", &self.scopes)
			.field("created_by", &self.created_by)
			.field("creator_email", &self.creator_email)
			.field("creator_groups", &self.creator_groups)
			.field("created_at", &self.created_at)
			.field("last_used_at", &self.last_used_at)
			.field("last_used_ip", &self.last_used_ip)
			.field("expires_at", &self.expires_at)
			.field("revoked_at", &self.revoked_at)
			.finish()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn tokenrow_debug_redacts_hash() {
		let row = TokenRow {
			id: Uuid::nil(),
			tenant_id: "t".into(),
			user_id: "u".into(),
			name: Some("n".into()),
			token_prefix: "agpk_live_exampleprefixxx"
				.chars()
				.take(PREFIX_LEN)
				.collect(),
			hash_algo: "argon2id".into(),
			hash_params: serde_json::json!({"m": 65536, "t":3}),
			key_hash: "$argon2id$v=19$m=65536,t=3,p=1$SALT$HASH".into(),
			scopes: vec!["read".into()],
			created_by: Some("creator".into()),
			creator_email: Some("c@example.com".into()),
			creator_groups: vec!["g1".into()],
			created_at: Utc::now(),
			last_used_at: None,
			last_used_ip: None,
			expires_at: None,
			revoked_at: None,
		};
		let dbg = format!("{:?}", row);
		assert!(
			!dbg.contains("key_hash"),
			"debug output should not contain key_hash field name"
		);
		assert!(
			!dbg.contains("argon2id"),
			"debug output should not leak hash algorithm"
		);
		assert!(
			!dbg.contains("$argon2id$"),
			"debug output should not leak hash value"
		);
		assert!(dbg.contains("TokenRow"));
		assert!(dbg.contains("token_prefix")); // safe
	}
}

/// Parameters for creating a new token
pub struct CreateTokenParams<'a> {
	pub creator: &'a str,
	pub creator_email: Option<&'a str>,
	pub creator_groups: &'a [String],
	pub tenant_id: &'a str,
	pub user_id: &'a str,
	pub name: Option<&'a str>,
	pub scopes: &'a [String],
	pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct TokenRepo {
	pool: PgPool,
}
impl TokenRepo {
	pub fn new(pool: PgPool) -> Self {
		Self { pool }
	}
}

impl TokenRepo {
	pub async fn create(&self, params: CreateTokenParams<'_>) -> Result<(ZeroToken, TokenRow)> {
		let CreateTokenParams {
			creator,
			creator_email,
			creator_groups,
			tenant_id,
			user_id,
			name,
			scopes,
			expires_at,
		} = params;
		// Scope allow-list validation removed (was unused)
		// Generate random suffix (40 chars) using thread-local RNG (scoped so non-Send RNG is dropped before any await)
		let rand: String = {
			let mut rng = rand::rng();
			(0..40).map(|_| rng.sample(Alphanumeric) as char).collect()
		}; // rng dropped here
		// Optional environment tag (PAT_TOKEN_ENV). If unset or empty, no environment segment is inserted.
		// Resulting formats:
		//   Without env: <base>_<rand>
		//   With env:    <base>_<env>_<rand>
		fn token_env() -> Option<&'static str> {
			static ENV: OnceLock<Option<String>> = OnceLock::new();
			ENV
				.get_or_init(|| match std::env::var("PAT_TOKEN_ENV") {
					Ok(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
					_ => None,
				})
				.as_ref()
				.map(|s| s.as_str())
		}
		let token = if let Some(env) = token_env() {
			format!("{}_{}_{rand}", token_base_prefix(), env)
		} else {
			format!("{}_{}", token_base_prefix(), rand)
		};
		let prefix: String = token.chars().take(PREFIX_LEN).collect();
		// Use OsRng (from password-hash's rand_core) to avoid version mismatch with rand 0.9
		let salt = SaltString::generate(&mut OsRng);
		let hash = argon2_instance()
			.hash_password(token.as_bytes(), &salt)
			.map_err(|e| anyhow::anyhow!("argon2 hash error: {e}"))?
			.to_string();
		let row: TokenRow = sqlx::query_as(r#"INSERT INTO proxy_keys (tenant_id, user_id, name, token_prefix, hash_algo, hash_params, key_hash, scopes, expires_at, created_by, creator_email, creator_groups)
            VALUES ($1,$2,$3,$4,'argon2id','{}',$5,$6,$7,$8,$9,$10)
            RETURNING id, tenant_id, user_id, name, token_prefix, hash_algo, hash_params, key_hash, scopes, created_by, creator_email, creator_groups, created_at, last_used_at, last_used_ip, expires_at, revoked_at"#)
            .bind(tenant_id).bind(user_id).bind(name).bind(&prefix).bind(&hash).bind(scopes).bind(expires_at).bind(creator).bind(creator_email.map(|s| s.to_string())).bind(creator_groups)
            .fetch_one(&self.pool).await?;
		Ok((ZeroToken::new(&token), row))
	}
	pub async fn list_public(
		&self,
		tenant_id: &str,
		user_id: &str,
		limit: i64,
		offset: i64,
		search: Option<&str>,
	) -> Result<Vec<PublicTokenRow>> {
		let lim = limit.clamp(1, 200);
		let off = offset.max(0);
		if let Some(s) = search {
			let like = format!("%{}%", s.replace('%', "\\%"));
			Ok(sqlx::query_as::<_, PublicTokenRow>("SELECT id, token_prefix, scopes, creator_email, creator_groups, created_at, last_used_at, expires_at, revoked_at FROM proxy_keys WHERE tenant_id=$1 AND user_id=$2 AND (token_prefix ILIKE $3 OR $3 = '%%') ORDER BY created_at DESC LIMIT $4 OFFSET $5")
                .bind(tenant_id).bind(user_id).bind(&like).bind(lim).bind(off).fetch_all(&self.pool).await?)
		} else {
			Ok(sqlx::query_as::<_, PublicTokenRow>("SELECT id, token_prefix, scopes, creator_email, creator_groups, created_at, last_used_at, expires_at, revoked_at FROM proxy_keys WHERE tenant_id=$1 AND user_id=$2 ORDER BY created_at DESC LIMIT $3 OFFSET $4")
                .bind(tenant_id).bind(user_id).bind(lim).bind(off).fetch_all(&self.pool).await?)
		}
	}
	pub async fn revoke(&self, tenant_id: &str, id: Uuid) -> Result<Option<String>> {
		let rec = sqlx::query_as::<_, (String,)>("UPDATE proxy_keys SET revoked_at=now() WHERE id=$1 AND tenant_id=$2 AND revoked_at IS NULL RETURNING token_prefix")
            .bind(id).bind(tenant_id).fetch_optional(&self.pool).await?;
		if let Some((prefix,)) = rec.clone() {
			// send cluster notification
			let _ = sqlx::query("NOTIFY pat_revoked, $1")
				.bind(&prefix)
				.execute(&self.pool)
				.await; // best-effort
		}
		Ok(rec.map(|t| t.0))
	}
	pub async fn find_active_by_prefix(&self, prefix: &str) -> Result<Option<TokenRow>> {
		Ok(sqlx::query_as::<_, TokenRow>("SELECT id, tenant_id, user_id, name, token_prefix, hash_algo, hash_params, key_hash, scopes, created_by, creator_email, creator_groups, created_at, last_used_at, last_used_ip, expires_at, revoked_at FROM proxy_keys WHERE token_prefix=$1 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at>now()) LIMIT 1").bind(prefix).fetch_optional(&self.pool).await?)
	}
	pub async fn touch_last_used(&self, id: Uuid, ip: Option<IpAddr>) -> Result<()> {
		// Bind IP as text and cast to inet to avoid needing IpAddr Encode impl
		let ip_txt: Option<String> = ip.map(|i| i.to_string());
		sqlx::query("UPDATE proxy_keys SET last_used_at=now(), last_used_ip=COALESCE($2::inet,last_used_ip) WHERE id=$1")
            .bind(id)
            .bind(ip_txt)
            .execute(&self.pool)
            .await?;
		Ok(())
	}
}
