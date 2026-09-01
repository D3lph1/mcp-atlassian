//! Optional in-memory TTL cache for *reference* data (D25).
//!
//! Only endpoints whose answer is a property of the instance rather than of
//! the work in it — projects, issue types, boards, spaces, field definitions —
//! go through this. Issues, searches, comments and sprints never do: a cache
//! that hides a change someone just made is worse than a slow read.
//!
//! Disabled unless a TTL is configured, because caching changes observable
//! behaviour: a project created out of band stays invisible for up to one TTL.

use std::any::Any;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::Result;

/// A tiny type-erased cache: one map, one lock, values kept as `Arc<dyn Any>`
/// so a single instance can serve every reference-data endpoint of a client.
pub struct TtlCache {
    ttl: Duration,
    entries: Mutex<HashMap<String, Entry>>,
}

struct Entry {
    expires_at: Instant,
    value: Arc<dyn Any + Send + Sync>,
}

impl std::fmt::Debug for TtlCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let entries = self.entries.lock().map(|e| e.len()).unwrap_or(0);
        f.debug_struct("TtlCache")
            .field("ttl", &self.ttl)
            .field("entries", &entries)
            .finish()
    }
}

impl TtlCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Returns the cached value for `key`, or awaits `fetch` and caches its
    /// result.
    ///
    /// Two concurrent misses on the same key both fetch — no in-flight
    /// deduplication. For a single-tenant server the extra request is cheaper
    /// than the machinery to prevent it, and the second write simply wins.
    pub async fn get_or_fetch<T, F, Fut>(&self, key: &str, fetch: F) -> Result<T>
    where
        T: Clone + Send + Sync + 'static,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        if let Some(hit) = self.get(key) {
            tracing::debug!(key, "reference data served from cache");
            return Ok(hit);
        }
        let value = fetch().await?;
        self.insert(key, value.clone());
        Ok(value)
    }

    /// A hit only counts when the entry is unexpired *and* holds the type the
    /// caller asked for — a key reused for another type is a miss, never a
    /// panic.
    fn get<T: Clone + 'static>(&self, key: &str) -> Option<T> {
        let entries = self.lock();
        let entry = entries.get(key)?;
        (entry.expires_at > Instant::now())
            .then(|| entry.value.downcast_ref::<T>().cloned())
            .flatten()
    }

    fn insert<T: Send + Sync + 'static>(&self, key: &str, value: T) {
        let now = Instant::now();
        let mut entries = self.lock();
        // The key set is small and fixed, but dropping expired entries here
        // keeps a long-running server from holding stale payloads forever.
        entries.retain(|_, entry| entry.expires_at > now);
        entries.insert(
            key.to_string(),
            Entry {
                expires_at: now + self.ttl,
                value: Arc::new(value),
            },
        );
    }

    /// A poisoned lock only means some caller panicked while holding it; the
    /// map itself is intact, and a cache is not worth failing a request over.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    async fn counted(cache: &TtlCache, key: &str, calls: &AtomicUsize) -> Vec<String> {
        cache
            .get_or_fetch(key, || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(vec!["PROJ".to_string()])
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_second_read_of_the_same_key_does_not_fetch() {
        let cache = TtlCache::new(Duration::from_secs(60));
        let calls = AtomicUsize::new(0);
        assert_eq!(counted(&cache, "projects", &calls).await, ["PROJ"]);
        assert_eq!(counted(&cache, "projects", &calls).await, ["PROJ"]);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn different_keys_are_independent() {
        let cache = TtlCache::new(Duration::from_secs(60));
        let calls = AtomicUsize::new(0);
        counted(&cache, "boards:PROJ", &calls).await;
        counted(&cache, "boards:OPS", &calls).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn an_expired_entry_is_refetched() {
        // A zero TTL expires every entry the moment it is written, which tests
        // expiry without sleeping.
        let cache = TtlCache::new(Duration::ZERO);
        let calls = AtomicUsize::new(0);
        counted(&cache, "projects", &calls).await;
        counted(&cache, "projects", &calls).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_key_holding_another_type_is_a_miss() {
        let cache = TtlCache::new(Duration::from_secs(60));
        let calls = AtomicUsize::new(0);
        counted(&cache, "projects", &calls).await;
        let number: u32 = cache
            .get_or_fetch("projects", || async { Ok(7u32) })
            .await
            .unwrap();
        assert_eq!(number, 7);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_failed_fetch_is_not_cached() {
        let cache = TtlCache::new(Duration::from_secs(60));
        let result: Result<u32> = cache
            .get_or_fetch("myself", || async {
                Err(crate::Error::Config("boom".into()))
            })
            .await;
        assert!(result.is_err());
        let value: u32 = cache
            .get_or_fetch("myself", || async { Ok(1u32) })
            .await
            .unwrap();
        assert_eq!(value, 1);
    }
}
