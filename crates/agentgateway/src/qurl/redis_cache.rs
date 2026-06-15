//! Redis-backed L2 cache implementation for qURL resolutions

use std::time::Duration;

use async_trait::async_trait;
use redis::{aio::MultiplexedConnection, Client, RedisResult};
use serde::{Deserialize, Serialize};
use tracing::{debug, error, warn};

use super::{L2Cache, CacheEntry, QurlError};

/// Redis cache configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisCacheConfig {
    /// Redis connection URL (e.g., redis://localhost:6379)
    pub url: String,
    /// Key prefix for all cache entries
    pub key_prefix: String,
    /// Connection pool size
    pub pool_size: u32,
    /// Connection timeout
    pub connect_timeout: Duration,
    /// Operation timeout
    pub operation_timeout: Duration,
    /// Enable TLS
    pub tls: bool,
}

impl Default for RedisCacheConfig {
    fn default() -> Self {
        Self {
            url: "redis://localhost:6379".to_string(),
            key_prefix: "qurl:cache:".to_string(),
            pool_size: 10,
            connect_timeout: Duration::from_secs(5),
            operation_timeout: Duration::from_secs(2),
        }
    }
}

/// Redis L2 cache implementation
pub struct RedisL2Cache<K, V> {
    client: Client,
    config: RedisCacheConfig,
    _phantom: std::marker::PhantomData<(K, V)>,
}

impl<K, V> RedisL2Cache<K, V>
where
    K: Clone + Eq + std::hash::Hash + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static + serde::Serialize + for<'de> serde::Deserialize<'de>,
{
    /// Create new Redis L2 cache
    pub fn new(config: RedisCacheConfig) -> Result<Self, QurlError> {
        let client = Client::open(config.url.as_str())
            .map_err(|e| QurlError::ConfigError(format!("Invalid Redis URL: {}", e)))?;

        Ok(Self {
            client,
            config,
            _phantom: std::marker::PhantomData,
        })
    }

    /// Get a multiplexed connection from the pool
    async fn get_connection(&self) -> Result<MultiplexedConnection, QurlError> {
        self.client
            .get_multiplexed_tokio_connection()
            .await
            .map_err(|e| QurlError::ConfigError(format!("Redis connection failed: {}", e)))
    }

    /// Build full key with prefix
    fn full_key(&self, key: &K) -> String
    where
        K: ToString,
    {
        format!("{}{}", self.config.key_prefix, key.to_string())
    }
}

#[async_trait]
impl<K, V> L2Cache<K, V> for RedisL2Cache<K, V>
where
    K: Clone + Eq + std::hash::Hash + Send + Sync + 'static + ToString,
    V: Clone + Send + Sync + 'static + serde::Serialize + for<'de> serde::Deserialize<'de>,
{
    async fn get(&self, key: &K) -> Result<Option<V>, QurlError> {
        let mut conn = self.get_connection().await?;
        let full_key = self.full_key(key);

        let result: RedisResult<Option<String>> = redis::cmd("GET")
            .arg(&full_key)
            .query_async(&mut conn)
            .await;

        match result {
            Ok(Some(data)) => {
                match serde_json::from_str::<CacheEntry<V>>(&data) {
                    Ok(entry) => {
                        debug!(key = %full_key, "Redis L2 cache hit");
                        Ok(Some(entry.value))
                    }
                    Err(e) => {
                        warn!(key = %full_key, "Failed to deserialize cache entry: {}", e);
                        Ok(None)
                    }
                }
            }
            Ok(None) => {
                debug!(key = %full_key, "Redis L2 cache miss");
                Ok(None)
            }
            Err(e) => {
                error!(key = %full_key, "Redis GET failed: {}", e);
                Err(QurlError::ConfigError(format!("Redis get failed: {}", e)))
            }
        }
    }

    async fn set(&self, key: K, value: V, ttl: Duration) -> Result<(), QurlError> {
        let mut conn = self.get_connection().await?;
        let full_key = self.full_key(&key);

        let entry = CacheEntry {
            value,
            created_at: std::time::Instant::now(),
            expires_at: std::time::Instant::now() + ttl,
            last_accessed: std::time::Instant::now(),
            access_count: 0,
            stale_while_revalidate: Some(Duration::from_secs(60)),
            tags: Vec::new(),
        };

        let data = serde_json::to_string(&entry)
            .map_err(|e| QurlError::ConfigError(format!("Serialization failed: {}", e)))?;

        let ttl_secs = ttl.as_secs();
        let result: RedisResult<()> = redis::cmd("SETEX")
            .arg(&full_key)
            .arg(ttl_secs)
            .arg(&data)
            .query_async(&mut conn)
            .await;

        match result {
            Ok(()) => {
                debug!(key = %full_key, ttl = ttl_secs, "Redis L2 cache set");
                Ok(())
            }
            Err(e) => {
                error!(key = %full_key, "Redis SETEX failed: {}", e);
                Err(QurlError::ConfigError(format!("Redis set failed: {}", e)))
            }
        }
    }

    async fn delete(&self, key: &K) -> Result<(), QurlError> {
        let mut conn = self.get_connection().await?;
        let full_key = self.full_key(key);

        let result: RedisResult<()> = redis::cmd("DEL")
            .arg(&full_key)
            .query_async(&mut conn)
            .await;

        match result {
            Ok(()) => {
                debug!(key = %full_key, "Redis L2 cache delete");
                Ok(())
            }
            Err(e) => {
                error!(key = %full_key, "Redis DEL failed: {}", e);
                Err(QurlError::ConfigError(format!("Redis delete failed: {}", e)))
            }
        }
    }

    async fn exists(&self, key: &K) -> Result<bool, QurlError> {
        let mut conn = self.get_connection().await?;
        let full_key = self.full_key(key);

        let result: RedisResult<i32> = redis::cmd("EXISTS")
            .arg(&full_key)
            .query_async(&mut conn)
            .await;

        match result {
            Ok(count) => Ok(count > 0),
            Err(e) => {
                error!(key = %full_key, "Redis EXISTS failed: {}", e);
                Err(QurlError::ConfigError(format!("Redis exists failed: {}", e)))
            }
        }
    }

    async fn clear(&self) -> Result<(), QurlError> {
        let mut conn = self.get_connection().await?;

        // Use SCAN to find and delete all keys with prefix
        let pattern = format!("{}*", self.config.key_prefix);
        let mut cursor: u64 = 0;
        let mut total_deleted = 0;

        loop {
            let (new_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(100)
                .query_async(&mut conn)
                .await
                .map_err(|e| QurlError::ConfigError(format!("Redis SCAN failed: {}", e)))?;

            if !keys.is_empty() {
                let deleted: i32 = redis::cmd("DEL")
                    .arg(&keys)
                    .query_async(&mut conn)
                    .await
                    .map_err(|e| QurlError::ConfigError(format!("Redis DEL failed: {}", e)))?;
                total_deleted += deleted;
            }

            cursor = new_cursor;
            if cursor == 0 {
                break;
            }
        }

        info!("Redis L2 cache cleared: {} keys deleted", total_deleted);
        Ok(())
    }
}

/// In-memory L2 cache for testing/single-instance deployments
pub struct InMemoryL2Cache<K, V> {
    inner: Arc<parking_lot::RwLock<std::collections::HashMap<K, CacheEntry<V>>>>,
}

impl<K, V> Default for InMemoryL2Cache<K, V>
where
    K: Clone + Eq + std::hash::Hash + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    fn default() -> Self {
        Self {
            inner: Arc::new(parking_lot::RwLock::new(std::collections::HashMap::new())),
        }
    }
}

#[async_trait]
impl<K, V> L2Cache<K, V> for InMemoryL2Cache<K, V>
where
    K: Clone + Eq + std::hash::Hash + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    async fn get(&self, key: &K) -> Result<Option<V>, QurlError> {
        let now = std::time::Instant::now();
        let guard = self.inner.read();
        if let Some(entry) = guard.get(key) {
            if entry.expires_at > now {
                return Ok(Some(entry.value.clone()));
            }
        }
        Ok(None)
    }

    async fn set(&self, key: K, value: V, ttl: Duration) -> Result<(), QurlError> {
        let now = std::time::Instant::now();
        let entry = CacheEntry {
            value,
            created_at: now,
            expires_at: now + ttl,
            last_accessed: now,
            access_count: 0,
            stale_while_revalidate: Some(Duration::from_secs(60)),
            tags: Vec::new(),
        };
        self.inner.write().insert(key, entry);
        Ok(())
    }

    async fn delete(&self, key: &K) -> Result<(), QurlError> {
        self.inner.write().remove(key);
        Ok(())
    }

    async fn exists(&self, key: &K) -> Result<bool, QurlError> {
        let now = std::time::Instant::now();
        let guard = self.inner.read();
        Ok(guard.get(key).map_or(false, |e| e.expires_at > now))
    }

    async fn clear(&self) -> Result<(), QurlError> {
        self.inner.write().clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qurl::QurlError;

    #[tokio::test]
    async fn test_in_memory_l2_cache() {
        let cache = InMemoryL2Cache::<String, String>::default();

        // Miss
        assert!(cache.get(&"key1".to_string()).await.unwrap().is_none());

        // Set and hit
        cache.set("key1".to_string(), "value1".to_string(), Duration::from_secs(60)).await.unwrap();
        assert_eq!(cache.get(&"key1".to_string()).await.unwrap(), Some("value1".to_string()));

        // Delete
        cache.delete(&"key1".to_string()).await.unwrap();
        assert!(cache.get(&"key1".to_string()).await.unwrap().is_none());

        // Exists
        assert!(!cache.exists(&"key1".to_string()).await.unwrap());
        cache.set("key2".to_string(), "value2".to_string(), Duration::from_secs(60)).await.unwrap();
        assert!(cache.exists(&"key2".to_string()).await.unwrap());

        // Clear
        cache.clear().await.unwrap();
        assert!(!cache.exists(&"key2".to_string()).await.unwrap());
    }
}