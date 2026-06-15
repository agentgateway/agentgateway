//! Advanced caching for qURL resolutions with multi-tier support
//!
//! Provides L1 (in-memory) and L2 (Redis/distributed) caching with
//! stale-while-revalidate, cache warming, and cache-aside patterns.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::qurl::QurlError;

/// Cache entry with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry<V> {
    pub value: V,
    pub created_at: Instant,
    pub expires_at: Instant,
    pub last_accessed: Instant,
    pub access_count: u64,
    pub stale_while_revalidate: Option<Duration>,
    pub tags: Vec<String>,
}

/// Cache statistics
#[derive(Debug, Clone, Default, Serialize)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub expired: u64,
    pub revalidations: u64,
    pub size: usize,
    pub capacity: usize,
}

/// Cache configuration
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Maximum number of entries
    pub max_entries: usize,
    /// Default TTL for entries
    pub default_ttl: Duration,
    /// Stale-while-revalidate window
    pub stale_while_revalidate: Duration,
    /// Enable cache warming on startup
    pub warm_on_start: bool,
    /// Interval for background cleanup
    pub cleanup_interval: Duration,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            default_ttl: Duration::from_secs(300), // 5 minutes
            stale_while_revalidate: Duration::from_secs(60), // 1 minute
            warm_on_start: false,
            cleanup_interval: Duration::from_secs(60),
        }
    }
}

/// Multi-tier cache with L1 (memory) and optional L2 (distributed)
pub struct TieredCache<K, V> {
    config: CacheConfig,
    l1: Arc<RwLock<L1Cache<K, V>>>,
    l2: Option<Arc<dyn L2Cache<K, V>>>,
    stats: Arc<CacheStatsAtomic>,
}

/// L1 in-memory cache
type L1Cache<K, V> = HashMap<K, CacheEntry<V>>;

/// Atomic cache statistics
#[derive(Default)]
struct CacheStatsAtomic {
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
    expired: AtomicU64,
    revalidations: AtomicU64,
}

impl CacheStatsAtomic {
    fn record_hit(&self) { self.hits.fetch_add(1, Ordering::Relaxed); }
    fn record_miss(&self) { self.misses.fetch_add(1, Ordering::Relaxed); }
    fn record_eviction(&self) { self.evictions.fetch_add(1, Ordering::Relaxed); }
    fn record_expired(&self) { self.expired.fetch_add(1, Ordering::Relaxed); }
    fn record_revalidation(&self) { self.revalidations.fetch_add(1, Ordering::Relaxed); }

    fn snapshot(&self) -> CacheStats {
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            expired: self.expired.load(Ordering::Relaxed),
            revalidations: self.revalidations.load(Ordering::Relaxed),
            size: 0, // Set by caller
            capacity: 0, // Set by caller
        }
    }
}

/// Trait for L2 cache implementations (Redis, etc.)
#[async_trait::async_trait]
pub trait L2Cache<K, V>: Send + Sync {
    async fn get(&self, key: &K) -> Result<Option<V>, QurlError>;
    async fn set(&self, key: K, value: V, ttl: Duration) -> Result<(), QurlError>;
    async fn delete(&self, key: &K) -> Result<(), QurlError>;
    async fn exists(&self, key: &K) -> Result<bool, QurlError>;
    async fn clear(&self) -> Result<(), QurlError>;
}

impl<K, V> TieredCache<K, V>
where
    K: Clone + Eq + std::hash::Hash + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// Create new tiered cache
    pub fn new(config: CacheConfig) -> Self {
        Self {
            config,
            l1: Arc::new(RwLock::new(HashMap::with_capacity(config.max_entries))),
            l2: None,
            stats: Arc::new(CacheStatsAtomic::default()),
        }
    }

    /// Create with L2 cache backend
    pub fn with_l2(config: CacheConfig, l2: Arc<dyn L2Cache<K, V>>) -> Self {
        let mut cache = Self::new(config);
        cache.l2 = Some(l2);
        cache
    }

    /// Get value from cache (L1 -> L2)
    pub async fn get(&self, key: &K) -> Option<V> {
        let now = Instant::now();

        // Try L1 first
        {
            let l1 = self.l1.read();
            if let Some(entry) = l1.get(key) {
                if entry.expires_at > now {
                    // Fresh hit
                    self.stats.record_hit();
                    debug!(?key, "L1 cache hit (fresh)");
                    return Some(entry.value.clone());
                } else if entry.stale_while_revalidate.map_or(false, |swr| {
                    entry.expires_at + swr > now
                }) {
                    // Stale but within SWR window - return stale, trigger revalidation
                    self.stats.record_hit();
                    self.stats.record_revalidation();
                    debug!(?key, "L1 cache hit (stale, triggering revalidation)");
                    return Some(entry.value.clone());
                } else {
                    // Expired
                    self.stats.record_expired();
                }
            }
        }

        // Try L2 if available
        if let Some(l2) = &self.l2 {
            if let Ok(Some(value)) = l2.get(key).await {
                // Promote to L1
                self.set(key.clone(), value.clone(), self.config.default_ttl).await;
                self.stats.record_hit();
                debug!(?key, "L2 cache hit, promoted to L1");
                return Some(value);
            }
        }

        self.stats.record_miss();
        debug!(?key, "Cache miss");
        None
    }

    /// Set value in cache (L1 + L2)
    pub async fn set(&self, key: K, value: V, ttl: Duration) {
        let now = Instant::now();
        let expires_at = now + ttl;
        let swr = self.config.stale_while_revalidate;

        let entry = CacheEntry {
            value,
            created_at: now,
            expires_at,
            last_accessed: now,
            access_count: 0,
            stale_while_revalidate: Some(swr),
            tags: Vec::new(),
        };

        // Set in L1
        {
            let mut l1 = self.l1.write();
            // Evict if at capacity
            if l1.len() >= self.config.max_entries {
                self.evict_lru(&mut l1);
            }
            l1.insert(key.clone(), entry);
        }

        // Set in L2 if available
        if let Some(l2) = &self.l2 {
            if let Err(e) = l2.set(key, entry.value.clone(), ttl).await {
                warn!("L2 cache set failed: {}", e);
            }
        }
    }

    /// Delete from cache
    pub async fn delete(&self, key: &K) {
        self.l1.write().remove(key);
        if let Some(l2) = &self.l2 {
            let _ = l2.delete(key).await;
        }
    }

    /// Invalidate by tag
    pub async fn invalidate_tag(&self, tag: &str) {
        let mut l1 = self.l1.write();
        l1.retain(|_, entry| !entry.tags.contains(&tag.to_string()));
    }

    /// Get or compute with cache-aside pattern
    pub async fn get_or_set<F, E>(&self, key: K, ttl: Duration, compute: F) -> Result<V, E>
    where
        F: FnOnce() -> Result<V, E>,
        E: std::error::Error + Send + Sync + 'static,
    {
        if let Some(value) = self.get(&key).await {
            return Ok(value);
        }

        let value = compute()?;
        self.set(key, value.clone(), ttl).await;
        Ok(value)
    }

    /// Get stats
    pub fn stats(&self) -> CacheStats {
        let l1_size = self.l1.read().len();
        let mut stats = self.stats.snapshot();
        stats.size = l1_size;
        stats.capacity = self.config.max_entries;
        stats
    }

    /// Start background cleanup task
    pub fn start_cleanup_task(&self) -> tokio::task::JoinHandle<()> {
        let l1 = self.l1.clone();
        let stats = self.stats.clone();
        let interval = self.config.cleanup_interval;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(interval);
            loop {
                interval.tick().await;
                let now = Instant::now();
                let mut l1 = l1.write();
                let initial_len = l1.len();
                l1.retain(|_, entry| entry.expires_at > now);
                let evicted = initial_len - l1.len();
                if evicted > 0 {
                    stats.record_eviction();
                    debug!("Cache cleanup: evicted {} expired entries", evicted);
                }
            }
        })
    }

    /// Evict least recently used entry
    fn evict_lru(&self, l1: &mut L1Cache<K, V>) {
        if let Some((key, _)) = l1.iter().min_by_key(|(_, e)| e.last_accessed) {
            let key = key.clone();
            l1.remove(&key);
            self.stats.record_eviction();
            debug!("Evicted LRU entry");
        }
    }

    /// Warm cache with precomputed values
    pub async fn warm(&self, entries: Vec<(K, V, Duration)>) {
        for (key, value, ttl) in entries {
            self.set(key, value, ttl).await;
        }
        info!("Cache warmed with {} entries", entries.len());
    }
}

/// Specialized cache for qURL resolutions
pub type QurlResolutionCache = TieredCache<String, crate::qurl::ResolveResponse>;

impl QurlResolutionCache {
    /// Create cache optimized for qURL resolutions
    pub fn for_qurl(config: CacheConfig) -> Self {
        // qURL resolutions have their own TTL from access_grant
        Self::new(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qurl::{ResolveResponse, AccessGrant};
    use chrono::Utc;
    use std::net::IpAddr;

    #[tokio::test]
    async fn test_basic_cache_operations() {
        let cache = TieredCache::<String, String>::new(CacheConfig::default());

        // Miss
        assert!(cache.get(&"key1".to_string()).await.is_none());

        // Set and hit
        cache.set("key1".to_string(), "value1".to_string(), Duration::from_secs(60)).await;
        assert_eq!(cache.get(&"key1".to_string()).await, Some("value1".to_string()));

        // Stats
        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
    }

    #[tokio::test]
    async fn test_cache_expiration() {
        let config = CacheConfig {
            default_ttl: Duration::from_millis(50),
            ..Default::default()
        };
        let cache = TieredCache::<String, String>::new(config);

        cache.set("key1".to_string(), "value1".to_string(), Duration::from_millis(50)).await;
        assert_eq!(cache.get(&"key1".to_string()).await, Some("value1".to_string()));

        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(cache.get(&"key1".to_string()).await.is_none());

        let stats = cache.stats();
        assert_eq!(stats.expired, 1);
    }

    #[tokio::test]
    async fn test_get_or_set() {
        let cache = TieredCache::<String, String>::new(CacheConfig::default());
        let call_count = Arc::new(AtomicUsize::new(0));
        let count_clone = call_count.clone();

        let value = cache.get_or_set("key1".to_string(), Duration::from_secs(60), move || {
            count_clone.fetch_add(1, Ordering::Relaxed);
            Ok("computed".to_string())
        }).await.unwrap();

        assert_eq!(value, "computed");
        assert_eq!(call_count.load(Ordering::Relaxed), 1);

        // Second call should use cache
        let value = cache.get_or_set("key1".to_string(), Duration::from_secs(60), move || {
            count_clone.fetch_add(1, Ordering::Relaxed);
            Ok("computed2".to_string())
        }).await.unwrap();

        assert_eq!(value, "computed");
        assert_eq!(call_count.load(Ordering::Relaxed), 1);
    }
}