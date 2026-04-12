//! Embedding write-back hook for auto-vectorizing records.
//!
//! `EmbeddingMutationObserver` intercepts writes to maps configured in
//! `VectorConfig.maps`, extracts text from configured fields, and enqueues
//! embedding generation events. A background `tokio::spawn` task batches events,
//! calls `EmbeddingProvider::batch_embed`, and writes the resulting vector back
//! to the record's `_embedding` field via a read-modify-write on `RecordStoreFactory`.
//!
//! Follows the same unbounded-channel + background-batch-processor pattern as
//! `SearchMutationObserver` in `search.rs`, swapping tantivy indexing for
//! embedding generation and `RecordStore` write-back.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dashmap::DashSet;
use parking_lot::Mutex;
use std::sync::OnceLock;
use tokio::sync::mpsc;

use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;

use crate::service::domain::embedding::{EmbeddingProvider, VectorConfig};
use crate::service::domain::predicate::value_to_rmpv;
use crate::storage::factory::RecordStoreFactory;
use crate::storage::mutation_observer::MutationObserver;
use crate::storage::record::{Record, RecordValue};
use crate::storage::record_store::{CallerProvenance, ExpiryPolicy};
use crate::storage::factory::ObserverFactory;

// ---------------------------------------------------------------------------
// EmbeddingConfig
// ---------------------------------------------------------------------------

/// Configuration for the embedding batch processor.
///
/// Production defaults use a 100ms interval and 500-event threshold, matching
/// the search batch processor to keep system-wide write-back latency consistent.
#[derive(Debug, Clone, Copy)]
pub struct EmbeddingConfig {
    /// Milliseconds between batch flushes (default: 100).
    pub batch_interval_ms: u64,
    /// Maximum events before forced flush (default: 500).
    pub batch_flush_threshold: usize,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            batch_interval_ms: 100,
            batch_flush_threshold: 500,
        }
    }
}

// ---------------------------------------------------------------------------
// EmbeddingEvent (internal)
// ---------------------------------------------------------------------------

/// A single embedding generation event sent to the background batch processor.
struct EmbeddingEvent {
    map_name: String,
    key: String,
    partition_id: u32,
    /// Concatenated text from all configured fields for this record.
    text: String,
}

// ---------------------------------------------------------------------------
// EmbeddingMutationObserver
// ---------------------------------------------------------------------------

/// Intercepts record writes for maps configured in `VectorConfig.maps` and
/// enqueues embedding generation events for the background batch processor.
///
/// The observer itself is synchronous; all async work (embedding API calls,
/// `RecordStore` write-back) is performed by the background task.
pub struct EmbeddingMutationObserver {
    map_name: String,
    partition_id: u32,
    /// Field names to extract text from, as declared in `MapVectorConfig.fields`.
    fields: Vec<String>,
    event_tx: mpsc::UnboundedSender<EmbeddingEvent>,
    /// Shared with the batch processor to prevent re-entrancy on write-back.
    /// Keys are `(map_name, record_key)`.
    in_flight: Arc<DashSet<(String, String)>>,
}

impl EmbeddingMutationObserver {
    /// Creates a new observer. Does not spawn the background task — that is
    /// done by `EmbeddingObserverFactory::init()`.
    #[must_use]
    fn new(
        map_name: String,
        partition_id: u32,
        fields: Vec<String>,
        event_tx: mpsc::UnboundedSender<EmbeddingEvent>,
        in_flight: Arc<DashSet<(String, String)>>,
    ) -> Self {
        Self {
            map_name,
            partition_id,
            fields,
            event_tx,
            in_flight,
        }
    }

    /// Extracts text from configured fields in the rmpv value and enqueues
    /// an embedding event if the result is non-empty and not in-flight.
    fn enqueue_if_applicable(&self, key: &str, rmpv_val: &rmpv::Value) {
        // Re-entrancy guard: skip if the batch processor is currently writing back
        // an embedding for this record to avoid infinite observer loops.
        if self
            .in_flight
            .contains(&(self.map_name.clone(), key.to_string()))
        {
            return;
        }

        let text = extract_fields_text(rmpv_val, &self.fields);
        if text.is_empty() {
            return;
        }

        let _ = self.event_tx.send(EmbeddingEvent {
            map_name: self.map_name.clone(),
            key: key.to_owned(),
            partition_id: self.partition_id,
            text,
        });
    }
}

impl MutationObserver for EmbeddingMutationObserver {
    fn on_put(&self, key: &str, record: &Record, _old_value: Option<&RecordValue>, _is_backup: bool) {
        let rmpv_val = match &record.value {
            RecordValue::Lww { value, .. } => value_to_rmpv(value),
            _ => return,
        };
        self.enqueue_if_applicable(key, &rmpv_val);
    }

    fn on_update(
        &self,
        key: &str,
        _record: &Record,
        _old_value: &RecordValue,
        new_value: &RecordValue,
        _is_backup: bool,
    ) {
        let rmpv_val = match new_value {
            RecordValue::Lww { value, .. } => value_to_rmpv(value),
            _ => return,
        };
        self.enqueue_if_applicable(key, &rmpv_val);
    }

    fn on_remove(&self, _key: &str, _record: &Record, _is_backup: bool) {}
    fn on_evict(&self, _key: &str, _record: &Record, _is_backup: bool) {}
    fn on_load(&self, _key: &str, _record: &Record, _is_backup: bool) {}
    fn on_replication_put(&self, _key: &str, _record: &Record, _populate_index: bool) {}
    fn on_clear(&self) {}
    fn on_reset(&self) {}
    fn on_destroy(&self, _is_shutdown: bool) {}
}

// ---------------------------------------------------------------------------
// Text extraction
// ---------------------------------------------------------------------------

/// Extracts text from the listed field names in an rmpv Map value.
///
/// Navigates the top-level Map entries for each configured field name,
/// collecting string values and joining them with spaces.
fn extract_fields_text(value: &rmpv::Value, fields: &[String]) -> String {
    let rmpv::Value::Map(map_entries) = value else {
        return String::new();
    };

    let mut parts = Vec::new();
    for field_name in fields {
        for (k, v) in map_entries {
            let key_str = match k {
                rmpv::Value::String(s) => s.as_str().unwrap_or(""),
                _ => continue,
            };
            if key_str == field_name {
                if let rmpv::Value::String(s) = v {
                    if let Some(text) = s.as_str() {
                        if !text.is_empty() {
                            parts.push(text.to_owned());
                        }
                    }
                }
                break;
            }
        }
    }
    parts.join(" ")
}

// ---------------------------------------------------------------------------
// Background batch processor
// ---------------------------------------------------------------------------

/// Runs as a tokio task, collecting embedding events and processing them in batches.
///
/// Mirrors the structure of `run_batch_processor` in `search.rs`:
/// flushes when `batch_interval` elapses or `batch_flush_threshold` events
/// have been accumulated, whichever comes first. On shutdown signal, drains
/// remaining events and processes them before exiting.
async fn run_embedding_batch_processor(
    mut event_rx: mpsc::UnboundedReceiver<EmbeddingEvent>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    provider: Arc<dyn EmbeddingProvider>,
    record_store_factory: Arc<RecordStoreFactory>,
    batch_interval: Duration,
    batch_flush_threshold: usize,
    in_flight: Arc<DashSet<(String, String)>>,
) {
    let mut batch: Vec<EmbeddingEvent> = Vec::new();

    loop {
        // Phase 1: Wait for first event or shutdown.
        if batch.is_empty() {
            tokio::select! {
                Some(evt) = event_rx.recv() => {
                    batch.push(evt);
                }
                result = shutdown_rx.changed() => {
                    if result.is_ok() && *shutdown_rx.borrow() {
                        while let Ok(evt) = event_rx.try_recv() {
                            batch.push(evt);
                        }
                        if !batch.is_empty() {
                            flush_batch(batch, &provider, &record_store_factory, &in_flight).await;
                        }
                        return;
                    }
                }
            }
        }

        // Phase 2: Accumulate more events until timer or threshold.
        if !batch.is_empty() && batch.len() < batch_flush_threshold {
            let sleep = tokio::time::sleep(batch_interval);
            tokio::pin!(sleep);

            loop {
                tokio::select! {
                    biased;

                    result = event_rx.recv() => {
                        match result {
                            Some(evt) => {
                                batch.push(evt);
                                if batch.len() >= batch_flush_threshold {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                    () = &mut sleep => {
                        break;
                    }
                    result = shutdown_rx.changed() => {
                        if result.is_ok() && *shutdown_rx.borrow() {
                            while let Ok(evt) = event_rx.try_recv() {
                                batch.push(evt);
                            }
                            if !batch.is_empty() {
                                flush_batch(batch, &provider, &record_store_factory, &in_flight).await;
                            }
                            return;
                        }
                    }
                }
            }
        }

        // Drain any remaining buffered events without blocking.
        while let Ok(evt) = event_rx.try_recv() {
            batch.push(evt);
        }

        if !batch.is_empty() {
            let current_batch = std::mem::take(&mut batch);
            flush_batch(current_batch, &provider, &record_store_factory, &in_flight).await;
        }
    }
}

/// Processes one batch: calls `batch_embed` and writes results back to `RecordStore`.
///
/// Uses read-modify-write to merge `_embedding` into existing record fields without
/// clobbering user data. Inserts `(map_name, key)` into `in_flight` before each
/// write-back and removes it after, so the observer's re-entrancy guard fires
/// when the batch processor's own write arrives at the observer.
async fn flush_batch(
    batch: Vec<EmbeddingEvent>,
    provider: &Arc<dyn EmbeddingProvider>,
    record_store_factory: &Arc<RecordStoreFactory>,
    in_flight: &Arc<DashSet<(String, String)>>,
) {
    let texts: Vec<String> = batch.iter().map(|e| e.text.clone()).collect();

    let embeddings = match provider.batch_embed(&texts).await {
        Ok(v) => v,
        Err(err) => {
            tracing::warn!("embedding batch failed: {err}");
            return;
        }
    };

    for (evt, embedding) in batch.into_iter().zip(embeddings.into_iter()) {
        let store = record_store_factory.get_or_create(&evt.map_name, evt.partition_id);

        // Read existing record to perform a complete read-modify-write.
        // Write-back must preserve all existing user fields.
        let existing = match store.get(&evt.key, false).await {
            Ok(Some(record)) => record,
            Ok(None) => {
                tracing::warn!(
                    "embedding write-back skipped for {}/{}: record not found",
                    evt.map_name,
                    evt.key
                );
                continue;
            }
            Err(err) => {
                tracing::warn!(
                    "embedding write-back failed for {}/{}: read error: {err}",
                    evt.map_name,
                    evt.key
                );
                continue;
            }
        };

        // Serialize the embedding as a MsgPack binary blob (Vec<f32> via rmp_serde).
        let embedding_bytes = match rmp_serde::to_vec_named(&embedding) {
            Ok(b) => b,
            Err(err) => {
                tracing::warn!(
                    "embedding write-back failed for {}/{}: serialize error: {err}",
                    evt.map_name,
                    evt.key
                );
                continue;
            }
        };

        // Merge `_embedding` into the existing Value::Map.
        let merged_value = if let RecordValue::Lww { value, .. } = existing.value {
            merge_embedding_into_value(value, embedding_bytes)
        } else {
            tracing::warn!(
                "embedding write-back skipped for {}/{}: non-LWW record",
                evt.map_name,
                evt.key
            );
            continue;
        };

        // Construct a synthetic timestamp for the write-back.
        // Truncation from u128 to u64 is acceptable: ms since epoch fits in u64 until year 584,542,046.
        #[allow(clippy::cast_possible_truncation)]
        let now_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let record_value = RecordValue::Lww {
            value: merged_value,
            timestamp: Timestamp {
                millis: now_millis,
                counter: 0,
                node_id: "_embedding_hook".to_string(),
            },
        };

        // Mark in-flight before write so the observer's re-entrancy guard fires.
        in_flight.insert((evt.map_name.clone(), evt.key.clone()));

        if let Err(err) = store
            .put(&evt.key, record_value, ExpiryPolicy::NONE, CallerProvenance::CrdtMerge)
            .await
        {
            tracing::warn!(
                "embedding write-back failed for {}/{}: put error: {err}",
                evt.map_name,
                evt.key
            );
        }

        // Always remove from in-flight even on error to prevent permanent block.
        in_flight.remove(&(evt.map_name.clone(), evt.key.clone()));
    }
}

/// Merges the `_embedding` bytes into an existing `Value::Map`.
///
/// Inserts or replaces the `"_embedding"` key. If the value is not a Map,
/// wraps it in a new Map containing only `"_embedding"`.
fn merge_embedding_into_value(existing: Value, embedding_bytes: Vec<u8>) -> Value {
    use std::collections::BTreeMap;

    let embedding_val = Value::Bytes(embedding_bytes);

    match existing {
        Value::Map(mut entries) => {
            entries.insert("_embedding".to_string(), embedding_val);
            Value::Map(entries)
        }
        other => {
            // Non-map value: wrap in a Map. Preserves data under `"_value"` key.
            let mut m = BTreeMap::new();
            m.insert("_value".to_string(), other);
            m.insert("_embedding".to_string(), embedding_val);
            Value::Map(m)
        }
    }
}

// ---------------------------------------------------------------------------
// EmbeddingObserverFactory
// ---------------------------------------------------------------------------

/// Factory that creates `EmbeddingMutationObserver` instances for maps
/// listed in `VectorConfig.maps`.
///
/// Two-phase construction resolves the chicken-and-egg dependency between
/// `EmbeddingObserverFactory` and `RecordStoreFactory`:
///
/// 1. `new()` — called before `RecordStoreFactory` exists. Creates the channel
///    and stores all state, but does NOT spawn the background task yet.
/// 2. `init()` — called after `Arc<RecordStoreFactory>` is available. Injects
///    the factory reference and spawns the background task.
pub struct EmbeddingObserverFactory {
    config: EmbeddingConfig,
    vector_config: Arc<VectorConfig>,
    provider: Arc<dyn EmbeddingProvider>,
    event_tx: mpsc::UnboundedSender<EmbeddingEvent>,
    /// Holds the receiver until `init()` is called and the background task is spawned.
    event_rx: Mutex<Option<mpsc::UnboundedReceiver<EmbeddingEvent>>>,
    shutdown_tx: Arc<tokio::sync::watch::Sender<bool>>,
    in_flight: Arc<DashSet<(String, String)>>,
    /// Set during `init()`. Panics on double-init.
    record_store_factory: OnceLock<Arc<RecordStoreFactory>>,
}

impl EmbeddingObserverFactory {
    /// Phase 1: pre-wiring constructor.
    ///
    /// Creates the event channel and all shared state. Does not spawn the
    /// background task — call `init()` after `Arc<RecordStoreFactory>` is available.
    #[must_use]
    pub fn new(
        config: EmbeddingConfig,
        vector_config: Arc<VectorConfig>,
        provider: Arc<dyn EmbeddingProvider>,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel::<EmbeddingEvent>();
        let (shutdown_tx, _shutdown_rx) = tokio::sync::watch::channel(false);

        Self {
            config,
            vector_config,
            provider,
            event_tx,
            event_rx: Mutex::new(Some(event_rx)),
            shutdown_tx: Arc::new(shutdown_tx),
            in_flight: Arc::new(DashSet::new()),
            record_store_factory: OnceLock::new(),
        }
    }

    /// Phase 2: post-wiring init.
    ///
    /// Injects the `RecordStoreFactory` reference and spawns the background
    /// embedding batch processor task. Must be called exactly once after the
    /// `Arc<RecordStoreFactory>` has been created.
    ///
    /// # Panics
    ///
    /// Panics if called more than once.
    pub fn init(&self, record_store_factory: Arc<RecordStoreFactory>) {
        self.record_store_factory
            .set(record_store_factory.clone())
            .expect("EmbeddingObserverFactory::init called twice");

        let event_rx = self
            .event_rx
            .lock()
            .take()
            .expect("EmbeddingObserverFactory::init called twice");

        let shutdown_rx = self.shutdown_tx.subscribe();

        tokio::spawn(run_embedding_batch_processor(
            event_rx,
            shutdown_rx,
            Arc::clone(&self.provider),
            record_store_factory,
            Duration::from_millis(self.config.batch_interval_ms),
            self.config.batch_flush_threshold,
            Arc::clone(&self.in_flight),
        ));
    }

    /// Returns the shutdown signal sender for external shutdown orchestration.
    #[must_use]
    pub fn shutdown_signal(&self) -> Arc<tokio::sync::watch::Sender<bool>> {
        Arc::clone(&self.shutdown_tx)
    }
}

impl ObserverFactory for EmbeddingObserverFactory {
    fn create_observer(
        &self,
        map_name: &str,
        partition_id: u32,
    ) -> Option<Arc<dyn MutationObserver>> {
        let map_cfg = self.vector_config.maps.get(map_name)?;

        let observer = EmbeddingMutationObserver::new(
            map_name.to_string(),
            partition_id,
            map_cfg.fields.clone(),
            self.event_tx.clone(),
            Arc::clone(&self.in_flight),
        );
        Some(Arc::new(observer))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};
    use std::sync::Arc;
    use std::time::Duration;

    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    use crate::service::domain::embedding::noop::NoopEmbeddingProvider;
    use crate::service::domain::embedding::{
        EmbeddingProvider, MapVectorConfig, NoopConfig, VectorConfig,
    };
    use crate::storage::datastores::NullDataStore;
    use crate::storage::factory::{ObserverFactory, RecordStoreFactory};
    use crate::storage::impls::StorageConfig;
    use crate::storage::mutation_observer::MutationObserver;
    use crate::storage::record::{Record, RecordMetadata, RecordValue};
    use crate::storage::record_store::{CallerProvenance, ExpiryPolicy};

    use super::{EmbeddingConfig, EmbeddingObserverFactory};

    fn make_noop_provider(dim: u16) -> Arc<dyn EmbeddingProvider> {
        Arc::new(NoopEmbeddingProvider::new(&NoopConfig { dimension: dim }))
    }

    fn make_vector_config(map_name: &str, fields: Vec<String>) -> Arc<VectorConfig> {
        let mut maps = HashMap::new();
        maps.insert(
            map_name.to_string(),
            MapVectorConfig {
                fields,
                index_name: None,
                dimension: 4,
            },
        );
        Arc::new(VectorConfig {
            provider: crate::service::domain::embedding::EmbeddingProviderConfig::Noop(
                NoopConfig { dimension: 4 },
            ),
            maps,
        })
    }

    fn make_lww_record(fields: Vec<(&str, &str)>) -> Record {
        let mut map = BTreeMap::new();
        for (k, v) in fields {
            map.insert(k.to_string(), Value::String(v.to_string()));
        }
        Record {
            value: RecordValue::Lww {
                value: Value::Map(map),
                timestamp: Timestamp {
                    millis: 1_000_000,
                    counter: 0,
                    node_id: "node-1".to_string(),
                },
            },
            metadata: RecordMetadata::new(1_000_000, 64),
        }
    }

    // --- Unit test: factory routing ---

    #[test]
    fn factory_returns_observer_for_configured_map() {
        let vector_config = make_vector_config("docs", vec!["title".to_string()]);
        let factory = EmbeddingObserverFactory::new(
            EmbeddingConfig::default(),
            vector_config,
            make_noop_provider(4),
        );

        let obs = factory.create_observer("docs", 0);
        assert!(obs.is_some(), "should return an observer for configured map");
    }

    #[test]
    fn factory_returns_none_for_unconfigured_map() {
        let vector_config = make_vector_config("docs", vec!["title".to_string()]);
        let factory = EmbeddingObserverFactory::new(
            EmbeddingConfig::default(),
            vector_config,
            make_noop_provider(4),
        );

        let obs = factory.create_observer("other_map", 0);
        assert!(obs.is_none(), "should return None for unconfigured map");
    }

    // --- Unit test: re-entrancy guard ---

    #[test]
    fn observer_skips_enqueue_when_in_flight() {
        use dashmap::DashSet;
        use tokio::sync::mpsc;

        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let in_flight = Arc::new(DashSet::new());

        let observer = super::EmbeddingMutationObserver::new(
            "docs".to_string(),
            0,
            vec!["title".to_string()],
            event_tx,
            Arc::clone(&in_flight),
        );

        let record = make_lww_record(vec![("title", "hello world")]);

        // Without in-flight: event should be enqueued.
        observer.on_put("key1", &record, None, false);
        assert!(
            event_rx.try_recv().is_ok(),
            "event should be enqueued when not in-flight"
        );

        // With in-flight: event should be skipped.
        in_flight.insert(("docs".to_string(), "key1".to_string()));
        observer.on_put("key1", &record, None, false);
        assert!(
            event_rx.try_recv().is_err(),
            "event should be skipped when in-flight"
        );
    }

    #[test]
    fn observer_skips_enqueue_for_empty_text() {
        use dashmap::DashSet;
        use tokio::sync::mpsc;

        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let in_flight = Arc::new(DashSet::new());

        let observer = super::EmbeddingMutationObserver::new(
            "docs".to_string(),
            0,
            vec!["missing_field".to_string()], // field not in record
            event_tx,
            in_flight,
        );

        let record = make_lww_record(vec![("title", "hello world")]); // has "title" but not "missing_field"
        observer.on_put("key1", &record, None, false);
        assert!(
            event_rx.try_recv().is_err(),
            "event should be skipped when text is empty"
        );
    }

    // --- Integration test: write-back with NoopProvider ---

    #[tokio::test]
    async fn embedding_written_back_within_batch_window() {
        let vector_config = make_vector_config("docs", vec!["title".to_string()]);
        let provider = make_noop_provider(4);

        // Phase 1: build factory without RecordStoreFactory.
        let embedding_factory = Arc::new(EmbeddingObserverFactory::new(
            EmbeddingConfig {
                batch_interval_ms: 50, // faster for tests
                batch_flush_threshold: 500,
            },
            Arc::clone(&vector_config),
            Arc::clone(&provider),
        ));

        // Build RecordStoreFactory with the embedding factory wired in.
        let record_store_factory = Arc::new(
            RecordStoreFactory::new(
                StorageConfig::default(),
                Arc::new(NullDataStore),
                Vec::new(),
            )
            .with_observer_factories(vec![
                embedding_factory.clone() as Arc<dyn ObserverFactory>,
            ]),
        );

        // Phase 2: inject RecordStoreFactory and spawn background task.
        embedding_factory.init(Arc::clone(&record_store_factory));

        // Write a record to a configured map.
        let store = record_store_factory.get_or_create("docs", 0);
        let mut fields = BTreeMap::new();
        fields.insert("title".to_string(), Value::String("hello world".to_string()));
        let record_value = RecordValue::Lww {
            value: Value::Map(fields),
            timestamp: Timestamp {
                millis: 1_000_000,
                counter: 0,
                node_id: "client".to_string(),
            },
        };
        store
            .put("doc1", record_value, ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();

        // Wait for the batch window (50ms) + processing time.
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Read back and assert `_embedding` field is present.
        let record = store.get("doc1", false).await.unwrap();
        assert!(record.is_some(), "record should still exist after write-back");

        let record = record.unwrap();
        if let RecordValue::Lww { value: Value::Map(ref m), .. } = record.value {
            assert!(
                m.contains_key("_embedding"),
                "record should have _embedding field after write-back; got keys: {:?}",
                m.keys().collect::<Vec<_>>()
            );
            // Verify the embedding field contains a binary blob (MsgPack-encoded Vec<f32>).
            if let Some(Value::Bytes(ref bytes)) = m.get("_embedding") {
                let decoded: Vec<f32> = rmp_serde::from_slice(bytes).unwrap();
                assert_eq!(
                    decoded.len(),
                    4,
                    "embedding should have 4 dimensions (noop provider)"
                );
                assert!(
                    decoded.iter().all(|&v| v == 0.0f32),
                    "noop provider should return zero vectors"
                );
            } else {
                panic!("_embedding field should be a Bytes value");
            }
        } else {
            panic!("record value should be a LWW Map");
        }
    }

    // --- Integration test: shutdown drain ---

    #[tokio::test]
    async fn shutdown_drains_pending_events() {
        let vector_config = make_vector_config("docs", vec!["title".to_string()]);
        let provider = make_noop_provider(4);

        let embedding_factory = Arc::new(EmbeddingObserverFactory::new(
            EmbeddingConfig {
                batch_interval_ms: 10_000, // very long window so we control flush via shutdown
                batch_flush_threshold: 500,
            },
            Arc::clone(&vector_config),
            Arc::clone(&provider),
        ));

        let record_store_factory = Arc::new(
            RecordStoreFactory::new(
                StorageConfig::default(),
                Arc::new(NullDataStore),
                Vec::new(),
            )
            .with_observer_factories(vec![
                embedding_factory.clone() as Arc<dyn ObserverFactory>,
            ]),
        );

        embedding_factory.init(Arc::clone(&record_store_factory));

        // Write a record so an embedding event is enqueued.
        let store = record_store_factory.get_or_create("docs", 0);
        let mut fields = BTreeMap::new();
        fields.insert("title".to_string(), Value::String("drain test".to_string()));
        let record_value = RecordValue::Lww {
            value: Value::Map(fields),
            timestamp: Timestamp {
                millis: 2_000_000,
                counter: 0,
                node_id: "client".to_string(),
            },
        };
        store
            .put("doc2", record_value, ExpiryPolicy::NONE, CallerProvenance::Client)
            .await
            .unwrap();

        // Signal shutdown — the batch processor should drain and process remaining events.
        let shutdown_tx = embedding_factory.shutdown_signal();
        let _ = shutdown_tx.send(true);

        // Give the task time to drain and write-back.
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Record should have _embedding field after drain.
        let record = store.get("doc2", false).await.unwrap();
        assert!(record.is_some());
        if let Some(r) = record {
            if let RecordValue::Lww { value: Value::Map(ref m), .. } = r.value {
                assert!(
                    m.contains_key("_embedding"),
                    "shutdown drain should have written back _embedding"
                );
            } else {
                panic!("record value should be a LWW Map");
            }
        }
    }
}
