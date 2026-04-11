//! LRU deduplication cache for `SharedVector` instances.
//!
//! `VectorCache` wraps `quick_cache::sync::Cache` with a custom weighter
//! that charges `vector.mem_size() + key_overhead_bytes` against a configurable
//! byte budget. Because `SharedVector` has `Arc` semantics, cache entries are
//! cheap to clone even for large vectors.

use topgun_core::vector::SharedVector;

/// Configuration for `VectorCache` capacity and per-entry cost.
///
/// Consumed from TS configuration; `Deserialize` lets it be loaded from JSON
/// or `MsgPack` config payloads using camelCase field names.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorCacheConfig {
    /// Maximum total weight (bytes) the cache is allowed to hold.
    pub capacity_bytes: u64,
    /// Estimated per-entry overhead (hashmap slot, key string, struct fields).
    pub key_overhead_bytes: u32,
}

impl Default for VectorCacheConfig {
    fn default() -> Self {
        Self {
            capacity_bytes: 64 * 1024 * 1024, // 64 MiB
            key_overhead_bytes: 64,
        }
    }
}

/// LRU cache for `SharedVector` instances, bounded by a configurable byte budget.
pub struct VectorCache {
    inner: quick_cache::sync::Cache<String, SharedVector, VectorWeighter>,
    _config: VectorCacheConfig,
}

/// Custom weighter that charges `vector.mem_size() + key_overhead_bytes` per entry.
#[derive(Clone)]
struct VectorWeighter {
    key_overhead_bytes: u32,
}

impl quick_cache::Weighter<String, SharedVector> for VectorWeighter {
    fn weight(&self, _key: &String, val: &SharedVector) -> u64 {
        let size = val.mem_size().saturating_add(self.key_overhead_bytes as usize);
        // Safe: usize fits in u64 on all supported platforms (64-bit targets).
        u64::try_from(size).unwrap_or(u64::MAX)
    }
}

impl VectorCache {
    /// Creates a new `VectorCache` with the given configuration.
    #[must_use]
    pub fn new(config: VectorCacheConfig) -> Self {
        let weighter = VectorWeighter {
            key_overhead_bytes: config.key_overhead_bytes,
        };
        // Estimate a reasonable item count from the byte budget.
        // quick_cache requires an estimated item count for internal sharding;
        // the actual capacity is enforced by the weighter's weight budget.
        // Clamp to usize::MAX on 32-bit targets; the cache will simply use
        // a very large (but correct) shard count.
        let estimated_items = usize::try_from(
            (config.capacity_bytes / (u64::from(config.key_overhead_bytes) + 128)).max(16),
        )
        .unwrap_or(usize::MAX);
        let inner = quick_cache::sync::Cache::with_weighter(
            estimated_items,
            config.capacity_bytes,
            weighter,
        );
        Self {
            inner,
            _config: config,
        }
    }

    /// Returns the cached vector for `key`, or `None` if not present.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<SharedVector> {
        self.inner.get(key)
    }

    /// Inserts or replaces the vector for `key`.
    pub fn insert(&self, key: String, vector: SharedVector) {
        self.inner.insert(key, vector);
    }

    /// Removes the entry for `key`.
    pub fn remove(&self, key: &str) {
        self.inner.remove(key);
    }

    /// Evicts all entries from the cache.
    pub fn clear(&self) {
        self.inner.clear();
    }

    /// Returns the number of entries currently in the cache.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.inner.len() as u64
    }

    /// Returns the total weight (bytes) currently occupied by all entries.
    #[must_use]
    pub fn weight(&self) -> u64 {
        self.inner.weight()
    }

    /// Returns `true` when the cache contains no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_vector(dim: usize) -> SharedVector {
        use topgun_core::vector::{SharedVector, Vector};
        let data = vec![1.0f32; dim];
        SharedVector::new(Vector::F32(data))
    }

    #[test]
    fn cache_insert_and_get_roundtrip() {
        let cache = VectorCache::new(VectorCacheConfig::default());
        let v = make_vector(4);
        cache.insert("k1".to_string(), v.clone());
        let result = cache.get("k1");
        assert!(result.is_some());
    }

    #[test]
    fn cache_miss_returns_none() {
        let cache = VectorCache::new(VectorCacheConfig::default());
        assert!(cache.get("nonexistent").is_none());
    }

    #[test]
    fn cache_remove_evicts_entry() {
        let cache = VectorCache::new(VectorCacheConfig::default());
        let v = make_vector(4);
        cache.insert("k1".to_string(), v);
        cache.remove("k1");
        assert!(cache.get("k1").is_none());
    }

    #[test]
    fn cache_clear_empties_cache() {
        let cache = VectorCache::new(VectorCacheConfig::default());
        cache.insert("k1".to_string(), make_vector(4));
        cache.insert("k2".to_string(), make_vector(4));
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn cache_weight_tracks_inserted_vectors() {
        use topgun_core::vector::{SharedVector, Vector};
        let config = VectorCacheConfig {
            capacity_bytes: 64 * 1024 * 1024,
            key_overhead_bytes: 0,
        };
        let cache = VectorCache::new(config);
        // Each F32(4) vector uses 4 * 4 = 16 bytes of data
        let v1 = SharedVector::new(Vector::F32(vec![0.1, 0.2, 0.3, 0.4]));
        let v2 = SharedVector::new(Vector::F32(vec![0.5, 0.6, 0.7, 0.8]));
        let v3 = SharedVector::new(Vector::F32(vec![0.9, 1.0, 1.1, 1.2]));
        cache.insert("k1".to_string(), v1);
        cache.insert("k2".to_string(), v2);
        cache.insert("k3".to_string(), v3);
        // Weight must be > 0 after inserting 3 vectors
        assert!(cache.weight() > 0);
        assert!(!cache.is_empty());
    }

    #[test]
    fn cache_evicts_when_capacity_exceeded() {
        use topgun_core::vector::{SharedVector, Vector};
        // Use a tiny capacity so any reasonably-sized vector overflows
        let config = VectorCacheConfig {
            capacity_bytes: 100,
            key_overhead_bytes: 0,
        };
        let cache = VectorCache::new(config);
        // Insert several 64-float vectors (each ~256 bytes)
        for i in 0..10u32 {
            // Intentionally lossy cast: small integers fit exactly in f32 mantissa.
            #[allow(clippy::cast_precision_loss)]
            let data = vec![i as f32; 64];
            let v = SharedVector::new(Vector::F32(data));
            cache.insert(format!("k{i}"), v);
        }
        // Weight must not exceed capacity
        assert!(
            cache.weight() <= 100,
            "weight {} exceeded capacity 100",
            cache.weight()
        );
    }
}
