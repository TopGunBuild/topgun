//! No-op [`MapDataStore`] implementation.
//!
//! [`NullDataStore`] discards all writes and returns empty results for reads.
//! Useful for testing, benchmarks, and ephemeral data that does not require
//! persistence beyond the in-memory [`RecordStore`](crate::storage::RecordStore).

use async_trait::async_trait;

use crate::storage::map_data_store::MapDataStore;
use crate::storage::record::RecordValue;

/// No-op `MapDataStore` for testing and ephemeral data.
///
/// All write operations succeed immediately without side effects.
/// All read operations return empty results. This is the default
/// data store when no external persistence backend is configured.
pub struct NullDataStore;

#[async_trait]
impl MapDataStore for NullDataStore {
    async fn add(
        &self,
        _map: &str,
        _key: &str,
        _value: &RecordValue,
        _expiration_time: i64,
        _now: i64,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn add_backup(
        &self,
        _map: &str,
        _key: &str,
        _value: &RecordValue,
        _expiration_time: i64,
        _now: i64,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn remove(&self, _map: &str, _key: &str, _now: i64) -> anyhow::Result<()> {
        Ok(())
    }

    async fn remove_backup(&self, _map: &str, _key: &str, _now: i64) -> anyhow::Result<()> {
        Ok(())
    }

    async fn load(&self, _map: &str, _key: &str) -> anyhow::Result<Option<RecordValue>> {
        Ok(None)
    }

    async fn load_all(
        &self,
        _map: &str,
        _keys: &[String],
    ) -> anyhow::Result<Vec<(String, RecordValue)>> {
        Ok(Vec::new())
    }

    async fn remove_all(&self, _map: &str, _keys: &[String]) -> anyhow::Result<()> {
        Ok(())
    }

    fn is_loadable(&self, _key: &str) -> bool {
        true
    }

    fn pending_operation_count(&self) -> u64 {
        0
    }

    async fn soft_flush(&self) -> anyhow::Result<u64> {
        Ok(0)
    }

    async fn hard_flush(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn flush_key(
        &self,
        _map: &str,
        _key: &str,
        _value: &RecordValue,
        _is_backup: bool,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn reset(&self) {
        // Nothing to reset — NullDataStore holds no state.
    }

    fn is_null(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn add_returns_ok() {
        let store = NullDataStore;
        let val = dummy_value();
        assert!(store.add("map", "key", &val, 0, 1000).await.is_ok());
    }

    #[tokio::test]
    async fn add_backup_returns_ok() {
        let store = NullDataStore;
        let val = dummy_value();
        assert!(store.add_backup("map", "key", &val, 0, 1000).await.is_ok());
    }

    #[tokio::test]
    async fn remove_returns_ok() {
        let store = NullDataStore;
        assert!(store.remove("map", "key", 1000).await.is_ok());
    }

    #[tokio::test]
    async fn remove_backup_returns_ok() {
        let store = NullDataStore;
        assert!(store.remove_backup("map", "key", 1000).await.is_ok());
    }

    #[tokio::test]
    async fn load_returns_none() {
        let store = NullDataStore;
        let result = store.load("map", "key").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn load_all_returns_empty_vec() {
        let store = NullDataStore;
        let keys = vec!["a".to_string(), "b".to_string()];
        let result = store.load_all("map", &keys).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn remove_all_returns_ok() {
        let store = NullDataStore;
        let keys = vec!["a".to_string()];
        assert!(store.remove_all("map", &keys).await.is_ok());
    }

    #[test]
    fn is_null_returns_true() {
        let store = NullDataStore;
        assert!(store.is_null());
    }

    #[test]
    fn is_loadable_returns_true() {
        let store = NullDataStore;
        assert!(store.is_loadable("any-key"));
    }

    #[test]
    fn pending_operation_count_returns_zero() {
        let store = NullDataStore;
        assert_eq!(store.pending_operation_count(), 0);
    }

    #[tokio::test]
    async fn soft_flush_returns_zero() {
        let store = NullDataStore;
        let count = store.soft_flush().await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn hard_flush_returns_ok() {
        let store = NullDataStore;
        assert!(store.hard_flush().await.is_ok());
    }

    #[tokio::test]
    async fn flush_key_returns_ok() {
        let store = NullDataStore;
        let val = dummy_value();
        assert!(store.flush_key("map", "key", &val, false).await.is_ok());
    }

    /// Constructs a minimal `RecordValue` for test method signatures.
    fn dummy_value() -> RecordValue {
        use topgun_core::hlc::Timestamp;
        use topgun_core::types::Value;

        RecordValue::Lww {
            value: Value::Null,
            timestamp: Timestamp {
                millis: 0,
                counter: 0,
                node_id: String::new(),
            },
        }
    }
}
