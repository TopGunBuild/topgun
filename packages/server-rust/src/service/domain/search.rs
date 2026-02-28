//! Search domain service backed by tantivy full-text search engine.
//!
//! Manages per-map tantivy `Index` instances (RAM directory), handles one-shot
//! search and live search subscriptions with ENTER/UPDATE/LEAVE delta semantics.
//! `SearchMutationObserver` indexes records and re-scores standing subscriptions
//! on every data mutation.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::{Mutex, RwLock};
use tantivy::schema::Value as TantivyValue;
use tokio::sync::mpsc;
use tower::Service;

use topgun_core::messages::base::ChangeEventType;
use topgun_core::messages::search::{
    SearchOptions, SearchRespPayload, SearchResultEntry, SearchUpdatePayload,
};
use topgun_core::messages::Message;

use crate::network::connection::{ConnectionId, ConnectionRegistry, OutboundMessage};
use crate::service::domain::predicate::value_to_rmpv;
use crate::service::operation::{
    service_names, Operation, OperationError, OperationResponse,
};
use crate::service::registry::{ManagedService, ServiceContext};
use crate::storage::mutation_observer::MutationObserver;
use crate::storage::record::{Record, RecordValue};
use crate::storage::RecordStoreFactory;

// ---------------------------------------------------------------------------
// Internal scored-document result
// ---------------------------------------------------------------------------

/// A single document scored by tantivy, returned internally by `TantivyMapIndex`.
#[derive(Debug, Clone)]
pub struct ScoredDoc {
    /// The record key (from the `_key` field in the tantivy index).
    pub key: String,
    /// Relevance score from tantivy.
    pub score: f64,
    /// Query terms that matched this document.
    pub matched_terms: Vec<String>,
}

// ---------------------------------------------------------------------------
// Cached search result (per subscription per key)
// ---------------------------------------------------------------------------

/// Last-known score/terms for a key within a standing subscription.
/// Used to compute ENTER/UPDATE/LEAVE deltas on mutation.
#[derive(Debug, Clone)]
pub struct CachedSearchResult {
    pub score: f64,
    pub matched_terms: Vec<String>,
}

// ---------------------------------------------------------------------------
// SearchSubscription
// ---------------------------------------------------------------------------

/// A standing search subscription registered by a client.
pub struct SearchSubscription {
    /// Unique subscription identifier.
    pub subscription_id: String,
    /// Connection that owns this subscription.
    pub connection_id: ConnectionId,
    /// Map being searched.
    pub map_name: String,
    /// Original query string.
    pub query: String,
    /// Search options (limit, `min_score`, …).
    pub options: SearchOptions,
    /// Current result cache: key -> `CachedSearchResult`.
    ///
    /// Used to compute ENTER/UPDATE/LEAVE deltas when data changes.
    pub current_results: DashMap<String, CachedSearchResult>,
}

impl SearchSubscription {
    /// Creates a new subscription with an empty result cache.
    #[must_use]
    pub fn new(
        subscription_id: String,
        connection_id: ConnectionId,
        map_name: String,
        query: String,
        options: SearchOptions,
    ) -> Self {
        Self {
            subscription_id,
            connection_id,
            map_name,
            query,
            options,
            current_results: DashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// SearchRegistry
// ---------------------------------------------------------------------------

/// Concurrent registry of standing search subscriptions.
///
/// Keyed by `subscription_id` for O(1) lookup. Per-map iteration is
/// O(n) where n is the total number of subscriptions, acceptable for
/// the expected workload (few hundred concurrent subscriptions per server).
pub struct SearchRegistry {
    /// `subscription_id` -> `SearchSubscription`
    subscriptions: DashMap<String, Arc<SearchSubscription>>,
}

impl SearchRegistry {
    /// Creates a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            subscriptions: DashMap::new(),
        }
    }

    /// Registers a standing search subscription.
    pub fn register(&self, sub: SearchSubscription) {
        let id = sub.subscription_id.clone();
        self.subscriptions.insert(id, Arc::new(sub));
    }

    /// Removes a subscription by ID.
    ///
    /// Returns the removed subscription, or `None` if not found.
    #[must_use] 
    pub fn unregister(&self, subscription_id: &str) -> Option<Arc<SearchSubscription>> {
        self.subscriptions
            .remove(subscription_id)
            .map(|(_, sub)| sub)
    }

    /// Removes all subscriptions for a given connection ID.
    ///
    /// Returns the IDs of all removed subscriptions.
    #[allow(dead_code)] // Caller wiring is deferred — see spec Observable Truth 5.
    #[must_use] 
    pub fn unregister_by_connection(&self, connection_id: ConnectionId) -> Vec<String> {
        let mut removed = Vec::new();
        self.subscriptions.retain(|id, sub| {
            if sub.connection_id == connection_id {
                removed.push(id.clone());
                false
            } else {
                true
            }
        });
        removed
    }

    /// Returns all subscriptions targeting the given map.
    #[must_use] 
    pub fn get_subscriptions_for_map(&self, map_name: &str) -> Vec<Arc<SearchSubscription>> {
        self.subscriptions
            .iter()
            .filter(|entry| entry.value().map_name == map_name)
            .map(|entry| Arc::clone(entry.value()))
            .collect()
    }
}

impl Default for SearchRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// TantivyMapIndex
// ---------------------------------------------------------------------------

/// Per-map tantivy index backed by a RAM directory.
///
/// Fixed schema: `_key` (STRING | STORED, indexed but not tokenized) and
/// `_all_text` (TEXT, indexed with tokenization). All string values from a
/// record's `MsgPack` map are concatenated into `_all_text`. Per-field indexing
/// is deferred to a future spec.
pub struct TantivyMapIndex {
    index: tantivy::Index,
    reader: tantivy::IndexReader,
    writer: Mutex<tantivy::IndexWriter>,
    /// Cached schema fields to avoid repeated schema lookups.
    key_field: tantivy::schema::Field,
    all_text_field: tantivy::schema::Field,
}

impl TantivyMapIndex {
    /// Creates a new RAM-backed tantivy index with the fixed schema.
    ///
    /// # Panics
    ///
    /// Panics if tantivy fails to create the index (out-of-memory or internal
    /// tantivy error). Treated as unrecoverable at startup.
    #[must_use] 
    pub fn new() -> Self {
        use tantivy::schema::{SchemaBuilder, STORED, STRING, TEXT};

        let mut schema_builder = SchemaBuilder::new();
        // _key: STRING | STORED — indexed as a single token, stored for retrieval.
        let key_field = schema_builder.add_text_field("_key", STRING | STORED);
        // _all_text: TEXT — tokenized full-text field, not stored (content is in RecordStore).
        let all_text_field = schema_builder.add_text_field("_all_text", TEXT);
        let schema = schema_builder.build();

        let index = tantivy::Index::create_in_ram(schema);
        let reader = index
            .reader_builder()
            .reload_policy(tantivy::ReloadPolicy::Manual)
            .try_into()
            .expect("tantivy index reader creation failed");
        let writer = index
            .writer(50_000_000) // 50 MB heap budget
            .expect("tantivy index writer creation failed");

        Self {
            index,
            reader,
            writer: Mutex::new(writer),
            key_field,
            all_text_field,
        }
    }

    /// Indexes a document from a `rmpv::Value::Map`, extracting all string values
    /// and concatenating them into the `_all_text` field.
    ///
    /// Uses a delete-then-add pattern to handle updates correctly.
    /// Non-string values are skipped; per-field TEXT indexing is deferred.
    ///
    /// # Panics
    ///
    /// Panics if tantivy fails to add the document (internal tantivy error).
    pub fn index_document(&self, key: &str, value: &rmpv::Value) {
        use tantivy::doc;
        use tantivy::schema::Term;

        let all_text = extract_all_text(value);

        let key_term = Term::from_field_text(self.key_field, key);
        let writer = self.writer.lock();
        writer.delete_term(key_term);
        writer
            .add_document(doc!(
                self.key_field => key,
                self.all_text_field => all_text,
            ))
            .expect("tantivy add_document failed");
    }

    /// Removes a document by its `_key` field.
    pub fn remove_document(&self, key: &str) {
        use tantivy::schema::Term;

        let key_term = Term::from_field_text(self.key_field, key);
        let writer = self.writer.lock();
        writer.delete_term(key_term);
    }

    /// Executes a search query and returns scored results.
    ///
    /// Respects `options.limit` (default 10) and `options.min_score` (default 0.0).
    pub fn search(&self, query_str: &str, options: &SearchOptions) -> Vec<ScoredDoc> {
        use tantivy::collector::TopDocs;
        use tantivy::query::QueryParser;

        let limit = options.limit.unwrap_or(10) as usize;
        let min_score = options.min_score.unwrap_or(0.0);

        let searcher = self.reader.searcher();
        let query_parser = QueryParser::for_index(&self.index, vec![self.all_text_field]);

        let Ok(query) = query_parser.parse_query(query_str) else {
            return Vec::new();
        };

        let Ok(top_docs) = searcher.search(&query, &TopDocs::with_limit(limit)) else {
            return Vec::new();
        };

        let terms = extract_query_terms(query_str);
        top_docs
            .into_iter()
            .filter(|(score, _)| (f64::from(*score)) >= min_score)
            .filter_map(|(score, doc_addr)| {
                let doc: tantivy::TantivyDocument = searcher.doc(doc_addr).ok()?;
                let key = doc.get_first(self.key_field)?.as_str()?.to_owned();
                Some(ScoredDoc {
                    key,
                    score: f64::from(score),
                    matched_terms: terms.clone(),
                })
            })
            .collect()
    }

    /// Re-scores a single document against a query.
    ///
    /// Uses a `BooleanQuery` combining a `_key` term filter with the user query,
    /// ensuring only the specific document is matched. Returns `None` if the
    /// document does not match.
    pub fn score_single_document(&self, key: &str, query_str: &str) -> Option<ScoredDoc> {
        use tantivy::collector::TopDocs;
        use tantivy::query::{BooleanQuery, Occur, QueryParser, TermQuery};
        use tantivy::schema::IndexRecordOption;
        use tantivy::schema::Term;

        let searcher = self.reader.searcher();
        let query_parser = QueryParser::for_index(&self.index, vec![self.all_text_field]);

        let user_query = query_parser.parse_query(query_str).ok()?;

        let key_term = Term::from_field_text(self.key_field, key);
        let key_query =
            Box::new(TermQuery::new(key_term, IndexRecordOption::Basic));

        let combined = BooleanQuery::new(vec![
            (Occur::Must, key_query as Box<dyn tantivy::query::Query>),
            (Occur::Must, user_query),
        ]);

        let top_docs = searcher.search(&combined, &TopDocs::with_limit(1)).ok()?;
        let (score, doc_addr) = top_docs.into_iter().next()?;
        let doc: tantivy::TantivyDocument = searcher.doc(doc_addr).ok()?;
        let found_key = doc.get_first(self.key_field)?.as_str()?.to_owned();

        let terms = extract_query_terms(query_str);
        Some(ScoredDoc {
            key: found_key,
            score: f64::from(score),
            matched_terms: terms,
        })
    }

    /// Commits pending index writes, making them visible to subsequent searches.
    ///
    /// Uses `ReloadPolicy::Manual` — explicitly reloads the reader after commit
    /// so that searches see the latest data immediately.
    ///
    /// # Panics
    ///
    /// Panics if tantivy fails to commit or reload the reader.
    pub fn commit(&self) {
        let mut writer = self.writer.lock();
        writer.commit().expect("tantivy commit failed");
        drop(writer); // release lock before reloading reader
        self.reader.reload().expect("tantivy reader reload failed");
    }

    /// Deletes all documents from the index and commits.
    ///
    /// # Panics
    ///
    /// Panics if tantivy fails to delete all documents, commit, or reload.
    pub fn clear(&self) {
        {
            let mut writer = self.writer.lock();
            writer
                .delete_all_documents()
                .expect("tantivy delete_all_documents failed");
            writer
                .commit()
                .expect("tantivy commit after clear failed");
        } // drop writer lock before reloading reader
        self.reader.reload().expect("tantivy reader reload after clear failed");
    }

    /// Returns the number of documents in the index (after the last commit).
    pub fn doc_count(&self) -> u64 {
        self.reader.searcher().num_docs()
    }
}

impl Default for TantivyMapIndex {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extracts all string values from an `rmpv::Value::Map`, concatenated with spaces.
///
/// Non-map values yield an empty string. Non-string leaf values are skipped.
fn extract_all_text(value: &rmpv::Value) -> String {
    let mut parts = Vec::new();
    collect_strings(value, &mut parts);
    parts.join(" ")
}

fn collect_strings(value: &rmpv::Value, out: &mut Vec<String>) {
    match value {
        rmpv::Value::String(s) => {
            if let Some(text) = s.as_str() {
                if !text.is_empty() {
                    out.push(text.to_owned());
                }
            }
        }
        rmpv::Value::Map(entries) => {
            for (_, v) in entries {
                collect_strings(v, out);
            }
        }
        rmpv::Value::Array(items) => {
            for item in items {
                collect_strings(item, out);
            }
        }
        _ => {} // skip numeric, bool, nil, binary
    }
}

/// Extracts individual tokens from the query string for `matched_terms`.
///
/// Simple whitespace split; tantivy's tokenizer applies more thorough analysis
/// during indexing/search, but extracting exact post-analysis terms from the
/// query AST is not needed at this level.
fn extract_query_terms(query_str: &str) -> Vec<String> {
    query_str
        .split_whitespace()
        .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Serializes a `Message` to `MsgPack` bytes for sending over the wire.
fn serialize_message(msg: &Message) -> Option<Vec<u8>> {
    rmp_serde::to_vec_named(msg).ok()
}

/// Sends a message to a connection via the registry.
fn send_to_connection(conn_reg: &ConnectionRegistry, conn_id: ConnectionId, msg: &Message) {
    if let Some(bytes) = serialize_message(msg) {
        if let Some(handle) = conn_reg.get(conn_id) {
            let _ = handle.try_send(OutboundMessage::Binary(bytes));
        }
    }
}

// ---------------------------------------------------------------------------
// Notification batching event
// ---------------------------------------------------------------------------

/// A single batched mutation event sent to the background processor.
#[derive(Debug)]
struct MutationEvent {
    map_name: String,
    key: String,
    change_type: ChangeEventType,
}

// ---------------------------------------------------------------------------
// SearchMutationObserver
// ---------------------------------------------------------------------------

/// Implements `MutationObserver` for the search domain.
///
/// On data mutations:
///   1. Updates the tantivy index for the affected map.
///   2. Sends a batched event to the background processor.
///   3. The background task re-scores changed keys against standing subscriptions
///      and sends `SearchUpdate` messages (ENTER/UPDATE/LEAVE) via `ConnectionRegistry`.
pub struct SearchMutationObserver {
    map_name: String,
    registry: Arc<SearchRegistry>,
    indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>>,
    connection_registry: Arc<ConnectionRegistry>,
    /// Sender for the background batch processor.
    /// `UnboundedSender::send()` is synchronous, safe to call from sync trait methods.
    event_tx: mpsc::UnboundedSender<MutationEvent>,
    /// Shutdown signal for the background task.
    shutdown_tx: Arc<tokio::sync::watch::Sender<bool>>,
}

impl SearchMutationObserver {
    /// Creates a new observer and spawns the background batch processor task.
    pub fn new(
        map_name: String,
        registry: Arc<SearchRegistry>,
        indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>>,
        connection_registry: Arc<ConnectionRegistry>,
        batch_interval_ms: u64,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel::<MutationEvent>();
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let shutdown_tx = Arc::new(shutdown_tx);

        // Spawn background batch processor.
        let registry_clone = Arc::clone(&registry);
        let indexes_clone = Arc::clone(&indexes);
        let conn_reg_clone = Arc::clone(&connection_registry);
        tokio::spawn(run_batch_processor(
            event_rx,
            shutdown_rx,
            registry_clone,
            indexes_clone,
            conn_reg_clone,
            Duration::from_millis(batch_interval_ms),
        ));

        Self {
            map_name,
            registry,
            indexes,
            connection_registry,
            event_tx,
            shutdown_tx,
        }
    }

    /// Returns a clone of the shutdown signal sender, so that `SearchService`
    /// can register it and flush all observer background tasks on shutdown.
    #[must_use]
    pub fn shutdown_signal(&self) -> Arc<tokio::sync::watch::Sender<bool>> {
        Arc::clone(&self.shutdown_tx)
    }

    /// Sends a shutdown signal to allow the background task to drain pending events.
    pub async fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
        // Brief yield to allow background task to drain pending events.
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    /// Indexes a record value and enqueues a mutation event.
    fn index_and_notify(&self, key: &str, value: &rmpv::Value, change_type: ChangeEventType) {
        {
            let mut indexes = self.indexes.write();
            let index = indexes
                .entry(self.map_name.clone())
                .or_default();
            index.index_document(key, value);
            index.commit();
        }
        let _ = self.event_tx.send(MutationEvent {
            map_name: self.map_name.clone(),
            key: key.to_owned(),
            change_type,
        });
    }

    /// Removes a document from the index and enqueues a LEAVE event.
    fn remove_and_notify(&self, key: &str) {
        {
            let mut indexes = self.indexes.write();
            if let Some(index) = indexes.get_mut(&self.map_name) {
                index.remove_document(key);
                index.commit();
            }
        }
        let _ = self.event_tx.send(MutationEvent {
            map_name: self.map_name.clone(),
            key: key.to_owned(),
            change_type: ChangeEventType::LEAVE,
        });
    }
}

impl MutationObserver for SearchMutationObserver {
    fn on_put(
        &self,
        key: &str,
        record: &Record,
        _old_value: Option<&RecordValue>,
        is_backup: bool,
    ) {
        if is_backup {
            return;
        }
        let rmpv_val = record_to_rmpv(&record.value);
        self.index_and_notify(key, &rmpv_val, ChangeEventType::ENTER);
    }

    fn on_update(
        &self,
        key: &str,
        record: &Record,
        _old_value: &RecordValue,
        _new_value: &RecordValue,
        is_backup: bool,
    ) {
        if is_backup {
            return;
        }
        let rmpv_val = record_to_rmpv(&record.value);
        self.index_and_notify(key, &rmpv_val, ChangeEventType::UPDATE);
    }

    fn on_remove(&self, key: &str, _record: &Record, is_backup: bool) {
        if is_backup {
            return;
        }
        self.remove_and_notify(key);
    }

    fn on_clear(&self) {
        {
            let mut indexes = self.indexes.write();
            if let Some(index) = indexes.get_mut(&self.map_name) {
                index.clear();
            }
        }
        // Notify all subscriptions for this map of LEAVE for their cached keys.
        let subs = self.registry.get_subscriptions_for_map(&self.map_name);
        for sub in &subs {
            let keys: Vec<String> =
                sub.current_results.iter().map(|e| e.key().clone()).collect();
            for key in keys {
                sub.current_results.remove(&key);
                let msg = Message::SearchUpdate {
                    payload: SearchUpdatePayload {
                        subscription_id: sub.subscription_id.clone(),
                        key: key.clone(),
                        value: rmpv::Value::Nil,
                        score: 0.0,
                        matched_terms: Vec::new(),
                        change_type: ChangeEventType::LEAVE,
                    },
                };
                send_to_connection(&self.connection_registry, sub.connection_id, &msg);
            }
        }
    }

    fn on_reset(&self) {
        self.on_clear();
    }

    fn on_evict(&self, _key: &str, _record: &Record, _is_backup: bool) {}
    fn on_load(&self, _key: &str, _record: &Record, _is_backup: bool) {}
    fn on_replication_put(&self, _key: &str, _record: &Record, _populate_index: bool) {}
    fn on_destroy(&self, _is_shutdown: bool) {}
}

/// Converts a `RecordValue` to `rmpv::Value` for tantivy indexing.
fn record_to_rmpv(record_value: &RecordValue) -> rmpv::Value {
    match record_value {
        RecordValue::Lww { value, .. } => value_to_rmpv(value),
        RecordValue::OrMap { records } => {
            // OR-Map entries carry individual Values indexed by tag.
            // Wrap as an array of values for text extraction.
            let items: Vec<rmpv::Value> = records.iter().map(|e| value_to_rmpv(&e.value)).collect();
            rmpv::Value::Array(items)
        }
        RecordValue::OrTombstones { .. } => rmpv::Value::Nil,
    }
}

// ---------------------------------------------------------------------------
// Background batch processor
// ---------------------------------------------------------------------------

/// Runs as a tokio task, collecting mutation events and processing them in batches.
///
/// Sleeps for `batch_interval` to accumulate events, then processes all pending
/// events as a batch. On shutdown signal, drains remaining events before exiting.
async fn run_batch_processor(
    mut event_rx: mpsc::UnboundedReceiver<MutationEvent>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    registry: Arc<SearchRegistry>,
    indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>>,
    connection_registry: Arc<ConnectionRegistry>,
    batch_interval: Duration,
) {
    loop {
        // Wait for the batch interval or shutdown.
        tokio::select! {
            () = tokio::time::sleep(batch_interval) => {}
            result = shutdown_rx.changed() => {
                if result.is_ok() && *shutdown_rx.borrow() {
                    // Drain remaining events before exiting.
                    let mut pending = Vec::new();
                    while let Ok(evt) = event_rx.try_recv() {
                        pending.push(evt);
                    }
                    process_batch(pending, &registry, &indexes, &connection_registry);
                    return;
                }
            }
        }

        // Drain all events accumulated during this interval.
        let mut batch = Vec::new();
        while let Ok(evt) = event_rx.try_recv() {
            batch.push(evt);
        }
        if !batch.is_empty() {
            process_batch(batch, &registry, &indexes, &connection_registry);
        }
    }
}

/// Processes a batch of mutation events, sending `SearchUpdate` messages to subscribers.
///
/// Deduplicates events per (`map_name`, key), taking the last change type.
fn process_batch(
    batch: Vec<MutationEvent>,
    registry: &Arc<SearchRegistry>,
    indexes: &Arc<RwLock<HashMap<String, TantivyMapIndex>>>,
    connection_registry: &Arc<ConnectionRegistry>,
) {
    // Deduplicate: keep last event per (map_name, key).
    let mut deduped: HashMap<(String, String), ChangeEventType> = HashMap::new();
    for evt in batch {
        deduped.insert((evt.map_name, evt.key), evt.change_type);
    }

    for ((map_name, key), change_type) in deduped {
        let subs = registry.get_subscriptions_for_map(&map_name);
        if subs.is_empty() {
            continue;
        }

        // If the change is LEAVE, send LEAVE to all subscriptions that had this key cached.
        if change_type == ChangeEventType::LEAVE {
            for sub in &subs {
                if sub.current_results.remove(&key).is_some() {
                    let msg = Message::SearchUpdate {
                        payload: SearchUpdatePayload {
                            subscription_id: sub.subscription_id.clone(),
                            key: key.clone(),
                            value: rmpv::Value::Nil,
                            score: 0.0,
                            matched_terms: Vec::new(),
                            change_type: ChangeEventType::LEAVE,
                        },
                    };
                    send_to_connection(connection_registry, sub.connection_id, &msg);
                }
            }
            continue;
        }

        // Re-score each subscription against the changed key.
        let indexes_read = indexes.read();
        let Some(index) = indexes_read.get(&map_name) else {
            continue;
        };

        for sub in &subs {
            let scored = index.score_single_document(&key, &sub.query);

            match scored {
                Some(doc) => {
                    let score = doc.score;
                    let min_score = sub.options.min_score.unwrap_or(0.0);
                    if score < min_score {
                        // Below threshold — treat as LEAVE if was in results.
                        if sub.current_results.remove(&key).is_some() {
                            let msg = Message::SearchUpdate {
                                payload: SearchUpdatePayload {
                                    subscription_id: sub.subscription_id.clone(),
                                    key: key.clone(),
                                    value: rmpv::Value::Nil,
                                    score: 0.0,
                                    matched_terms: Vec::new(),
                                    change_type: ChangeEventType::LEAVE,
                                },
                            };
                            send_to_connection(connection_registry, sub.connection_id, &msg);
                        }
                        continue;
                    }

                    let delta_type = if sub.current_results.contains_key(&key) {
                        ChangeEventType::UPDATE
                    } else {
                        ChangeEventType::ENTER
                    };

                    sub.current_results.insert(
                        key.clone(),
                        CachedSearchResult {
                            score,
                            matched_terms: doc.matched_terms.clone(),
                        },
                    );

                    let msg = Message::SearchUpdate {
                        payload: SearchUpdatePayload {
                            subscription_id: sub.subscription_id.clone(),
                            key: key.clone(),
                            value: rmpv::Value::Nil,
                            score,
                            matched_terms: doc.matched_terms,
                            change_type: delta_type,
                        },
                    };
                    send_to_connection(connection_registry, sub.connection_id, &msg);
                }
                None => {
                    // No match — send LEAVE if was in results.
                    if sub.current_results.remove(&key).is_some() {
                        let msg = Message::SearchUpdate {
                            payload: SearchUpdatePayload {
                                subscription_id: sub.subscription_id.clone(),
                                key: key.clone(),
                                value: rmpv::Value::Nil,
                                score: 0.0,
                                matched_terms: Vec::new(),
                                change_type: ChangeEventType::LEAVE,
                            },
                        };
                        send_to_connection(connection_registry, sub.connection_id, &msg);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SearchService
// ---------------------------------------------------------------------------

/// Full-text search domain service backed by per-map tantivy indexes.
///
/// Handles `Operation::Search`, `Operation::SearchSubscribe`, and
/// `Operation::SearchUnsubscribe`. Returns `OperationError::WrongService`
/// for all other operations.
pub struct SearchService {
    registry: Arc<SearchRegistry>,
    indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>>,
    /// Retained for future full-record-value hydration in search results.
    #[allow(dead_code)]
    record_store_factory: Arc<RecordStoreFactory>,
    /// Retained for future subscription-aware push (currently unused after
    /// switching to `OperationResponse::Message` for request-response ops).
    #[allow(dead_code)]
    connection_registry: Arc<ConnectionRegistry>,
    /// Shutdown signals collected from registered `SearchMutationObserver`s.
    /// Sending `true` on each channel tells the background batch processor to
    /// drain pending events and exit.
    observer_shutdown_signals: RwLock<Vec<Arc<tokio::sync::watch::Sender<bool>>>>,
}

impl SearchService {
    /// Creates a new `SearchService`.
    #[must_use]
    pub fn new(
        registry: Arc<SearchRegistry>,
        indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>>,
        record_store_factory: Arc<RecordStoreFactory>,
        connection_registry: Arc<ConnectionRegistry>,
    ) -> Self {
        Self {
            registry,
            indexes,
            record_store_factory,
            connection_registry,
            observer_shutdown_signals: RwLock::new(Vec::new()),
        }
    }

    /// Registers an observer's shutdown signal so that `ManagedService::shutdown()`
    /// can flush all background batch processors.
    pub fn register_observer_shutdown(
        &self,
        signal: Arc<tokio::sync::watch::Sender<bool>>,
    ) {
        self.observer_shutdown_signals.write().push(signal);
    }

    /// Creates or retrieves the tantivy index for a map (lazy creation).
    fn ensure_index(&self, map_name: &str) {
        let mut indexes = self.indexes.write();
        indexes.entry(map_name.to_owned()).or_default();
    }

    /// Executes a search against the index for a map and builds `SearchResultEntry` list.
    fn execute_search(
        &self,
        map_name: &str,
        query_str: &str,
        options: &SearchOptions,
    ) -> Vec<SearchResultEntry> {
        self.ensure_index(map_name);
        let indexes = self.indexes.read();
        let Some(index) = indexes.get(map_name) else {
            return Vec::new();
        };
        let scored_docs = index.search(query_str, options);

        // For each hit, retrieve the full record value from RecordStoreFactory.
        // RecordStoreFactory::create() returns an empty store (lazy data entry via observers).
        scored_docs
            .into_iter()
            .map(|doc| {
                SearchResultEntry {
                    key: doc.key,
                    value: rmpv::Value::Nil, // client fetches full value if needed
                    score: doc.score,
                    matched_terms: doc.matched_terms,
                }
            })
            .collect()
    }

    fn handle(&self, op: Operation) -> Result<OperationResponse, OperationError> {
        match op {
            Operation::Search { ctx: _, payload } => {
                let options = payload.options.unwrap_or_default();
                let results = self.execute_search(&payload.map_name, &payload.query, &options);
                let total_count = u32::try_from(results.len()).unwrap_or(u32::MAX);
                let resp_payload = SearchRespPayload {
                    request_id: payload.request_id,
                    results,
                    total_count,
                    error: None,
                };
                Ok(OperationResponse::Message(Box::new(
                    Message::SearchResp { payload: resp_payload },
                )))
            }

            Operation::SearchSubscribe { ctx, payload } => {
                let connection_id = ctx.connection_id.ok_or_else(|| {
                    OperationError::Internal(anyhow::anyhow!(
                        "SearchSubscribe requires connection_id in OperationContext"
                    ))
                })?;
                let options = payload.options.unwrap_or_default();
                // Execute initial search.
                let results =
                    self.execute_search(&payload.map_name, &payload.query, &options);
                let total_count = u32::try_from(results.len()).unwrap_or(u32::MAX);

                // Build subscription with initial result cache populated.
                let sub = SearchSubscription::new(
                    payload.subscription_id.clone(),
                    connection_id,
                    payload.map_name.clone(),
                    payload.query.clone(),
                    options.clone(),
                );
                for entry in &results {
                    sub.current_results.insert(
                        entry.key.clone(),
                        CachedSearchResult {
                            score: entry.score,
                            matched_terms: entry.matched_terms.clone(),
                        },
                    );
                }
                self.registry.register(sub);

                let resp_payload = SearchRespPayload {
                    request_id: payload.subscription_id,
                    results,
                    total_count,
                    error: None,
                };
                Ok(OperationResponse::Message(Box::new(
                    Message::SearchResp { payload: resp_payload },
                )))
            }

            Operation::SearchUnsubscribe { ctx, payload } => {
                let _ = self.registry.unregister(&payload.subscription_id);
                Ok(OperationResponse::Ack { call_id: ctx.call_id })
            }

            _ => Err(OperationError::WrongService),
        }
    }
}

#[async_trait]
impl ManagedService for SearchService {
    fn name(&self) -> &'static str {
        service_names::SEARCH
    }

    async fn init(&self, _ctx: &ServiceContext) -> anyhow::Result<()> {
        Ok(())
    }

    async fn reset(&self) -> anyhow::Result<()> {
        // Clear all indexes and subscriptions.
        let mut indexes = self.indexes.write();
        for index in indexes.values_mut() {
            index.clear();
        }
        indexes.clear();
        self.registry.subscriptions.clear();
        Ok(())
    }

    async fn shutdown(&self, _terminate: bool) -> anyhow::Result<()> {
        // Signal all registered observer background tasks to drain pending
        // events and exit. Indexes are in-memory only — no persistent flush.
        let has_signals = {
            let signals = self.observer_shutdown_signals.read();
            for signal in signals.iter() {
                let _ = signal.send(true);
            }
            !signals.is_empty()
        }; // guard dropped before await
        // Brief yield to allow background tasks to drain.
        if has_signals {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Ok(())
    }
}

impl Service<Operation> for Arc<SearchService> {
    type Response = OperationResponse;
    type Error = OperationError;
    type Future =
        Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, op: Operation) -> Self::Future {
        let svc = Arc::clone(self);
        Box::pin(async move { svc.handle(op) })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use parking_lot::RwLock;
    use topgun_core::hlc::Timestamp;
    use topgun_core::messages::search::{SearchOptions, SearchPayload, SearchSubPayload, SearchUnsubPayload};
    use tower::ServiceExt;

    use super::*;
    use crate::network::connection::ConnectionRegistry;
    use crate::service::operation::{service_names, OperationContext, OperationError, Operation};
    use crate::storage::datastores::NullDataStore;
    use crate::storage::impls::StorageConfig;
    use crate::storage::RecordStoreFactory;

    fn make_rmpv_map(pairs: &[(&str, &str)]) -> rmpv::Value {
        rmpv::Value::Map(
            pairs
                .iter()
                .map(|(k, v)| {
                    (
                        rmpv::Value::String((*k).into()),
                        rmpv::Value::String((*v).into()),
                    )
                })
                .collect(),
        )
    }

    fn make_op_gc(service_name: &'static str) -> Operation {
        let ctx = OperationContext::new(
            1,
            service_name,
            Timestamp {
                millis: 0,
                counter: 0,
                node_id: "test".to_string(),
            },
            5000,
        );
        Operation::GarbageCollect { ctx }
    }

    fn make_factory() -> Arc<RecordStoreFactory> {
        Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ))
    }

    fn make_service() -> Arc<SearchService> {
        let reg = Arc::new(SearchRegistry::new());
        let indexes = Arc::new(RwLock::new(HashMap::new()));
        let conn_reg = Arc::new(ConnectionRegistry::new());
        let store_factory = make_factory();
        Arc::new(SearchService::new(reg, indexes, store_factory, conn_reg))
    }

    fn make_search_op(map_name: &str, query: &str) -> Operation {
        let mut ctx = OperationContext::new(
            42,
            service_names::SEARCH,
            Timestamp {
                millis: 0,
                counter: 0,
                node_id: "test".to_string(),
            },
            5000,
        );
        ctx.connection_id = Some(ConnectionId(1));
        Operation::Search {
            ctx,
            payload: SearchPayload {
                request_id: "req-1".to_string(),
                map_name: map_name.to_string(),
                query: query.to_string(),
                options: None,
            },
        }
    }

    fn make_subscribe_op(map_name: &str, query: &str) -> Operation {
        let mut ctx = OperationContext::new(
            43,
            service_names::SEARCH,
            Timestamp {
                millis: 0,
                counter: 0,
                node_id: "test".to_string(),
            },
            5000,
        );
        ctx.connection_id = Some(ConnectionId(1));
        Operation::SearchSubscribe {
            ctx,
            payload: SearchSubPayload {
                subscription_id: "sub-1".to_string(),
                map_name: map_name.to_string(),
                query: query.to_string(),
                options: None,
            },
        }
    }

    fn make_unsubscribe_op(subscription_id: &str) -> Operation {
        let ctx = OperationContext::new(
            44,
            service_names::SEARCH,
            Timestamp {
                millis: 0,
                counter: 0,
                node_id: "test".to_string(),
            },
            5000,
        );
        Operation::SearchUnsubscribe {
            ctx,
            payload: SearchUnsubPayload {
                subscription_id: subscription_id.to_string(),
            },
        }
    }

    // --- TantivyMapIndex tests ---

    #[test]
    fn tantivy_index_new_creates_empty_index() {
        let idx = TantivyMapIndex::new();
        assert_eq!(idx.doc_count(), 0);
    }

    #[test]
    fn tantivy_index_document_adds_and_makes_searchable() {
        let idx = TantivyMapIndex::new();
        let doc = make_rmpv_map(&[("title", "hello world"), ("body", "foo bar")]);
        idx.index_document("key-1", &doc);
        idx.commit();

        let opts = SearchOptions::default();
        let results = idx.search("hello", &opts);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].key, "key-1");
    }

    #[test]
    fn tantivy_index_update_replaces_document() {
        let idx = TantivyMapIndex::new();
        let doc1 = make_rmpv_map(&[("text", "alpha beta")]);
        let doc2 = make_rmpv_map(&[("text", "gamma delta")]);
        idx.index_document("key-1", &doc1);
        idx.commit();
        idx.index_document("key-1", &doc2);
        idx.commit();

        // Should not find original content.
        let opts = SearchOptions::default();
        let results_alpha = idx.search("alpha", &opts);
        assert!(results_alpha.is_empty(), "old content should be replaced");

        // Should find updated content.
        let results_gamma = idx.search("gamma", &opts);
        assert_eq!(results_gamma.len(), 1);
        assert_eq!(results_gamma[0].key, "key-1");
    }

    #[test]
    fn tantivy_index_remove_document() {
        let idx = TantivyMapIndex::new();
        let doc = make_rmpv_map(&[("text", "remove me")]);
        idx.index_document("key-1", &doc);
        idx.commit();
        assert_eq!(idx.doc_count(), 1);

        idx.remove_document("key-1");
        idx.commit();
        assert_eq!(idx.doc_count(), 0);

        let opts = SearchOptions::default();
        let results = idx.search("remove", &opts);
        assert!(results.is_empty());
    }

    #[test]
    fn tantivy_index_clear() {
        let idx = TantivyMapIndex::new();
        for i in 0..5 {
            let doc = make_rmpv_map(&[("text", &format!("document {i}"))]);
            idx.index_document(&format!("key-{i}"), &doc);
        }
        idx.commit();
        assert_eq!(idx.doc_count(), 5);

        idx.clear();
        assert_eq!(idx.doc_count(), 0);
    }

    #[test]
    fn tantivy_index_search_respects_limit() {
        let idx = TantivyMapIndex::new();
        for i in 0..10 {
            let doc = make_rmpv_map(&[("text", "common word here")]);
            idx.index_document(&format!("key-{i}"), &doc);
        }
        idx.commit();

        let opts = SearchOptions { limit: Some(3), ..Default::default() };
        let results = idx.search("common", &opts);
        assert!(results.len() <= 3);
    }

    #[test]
    fn tantivy_index_score_single_document_returns_none_for_no_match() {
        let idx = TantivyMapIndex::new();
        let doc = make_rmpv_map(&[("text", "something unrelated")]);
        idx.index_document("key-1", &doc);
        idx.commit();

        let result = idx.score_single_document("key-1", "totally different query zzz");
        assert!(result.is_none());
    }

    #[test]
    fn tantivy_index_score_single_document_finds_match() {
        let idx = TantivyMapIndex::new();
        let doc = make_rmpv_map(&[("text", "tantivy search engine")]);
        idx.index_document("key-1", &doc);
        idx.commit();

        let result = idx.score_single_document("key-1", "tantivy");
        assert!(result.is_some());
        let scored = result.unwrap();
        assert_eq!(scored.key, "key-1");
        assert!(scored.score > 0.0);
    }

    // --- SearchRegistry tests ---

    #[test]
    fn search_registry_register_and_get_by_map() {
        let reg = SearchRegistry::new();
        let sub = SearchSubscription::new(
            "sub-1".to_string(),
            ConnectionId(42),
            "my-map".to_string(),
            "hello".to_string(),
            SearchOptions::default(),
        );
        reg.register(sub);

        let subs = reg.get_subscriptions_for_map("my-map");
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].subscription_id, "sub-1");
    }

    #[test]
    fn search_registry_unregister_removes_subscription() {
        let reg = SearchRegistry::new();
        let sub = SearchSubscription::new(
            "sub-1".to_string(),
            ConnectionId(42),
            "my-map".to_string(),
            "hello".to_string(),
            SearchOptions::default(),
        );
        reg.register(sub);
        assert_eq!(reg.get_subscriptions_for_map("my-map").len(), 1);

        let removed = reg.unregister("sub-1");
        assert!(removed.is_some());
        assert!(reg.get_subscriptions_for_map("my-map").is_empty());
    }

    #[test]
    fn search_registry_unregister_by_connection() {
        let reg = SearchRegistry::new();
        for i in 0..3u64 {
            let sub = SearchSubscription::new(
                format!("sub-{i}"),
                if i < 2 { ConnectionId(10) } else { ConnectionId(20) },
                "my-map".to_string(),
                "hello".to_string(),
                SearchOptions::default(),
            );
            reg.register(sub);
        }

        let removed = reg.unregister_by_connection(ConnectionId(10));
        assert_eq!(removed.len(), 2);
        // Only subscription from connection 20 remains.
        assert_eq!(reg.get_subscriptions_for_map("my-map").len(), 1);
    }

    #[test]
    fn search_registry_get_subscriptions_for_map_filters_correctly() {
        let reg = SearchRegistry::new();
        let sub_a = SearchSubscription::new(
            "a".to_string(),
            ConnectionId(1),
            "map-a".to_string(),
            "q".to_string(),
            SearchOptions::default(),
        );
        let sub_b = SearchSubscription::new(
            "b".to_string(),
            ConnectionId(2),
            "map-b".to_string(),
            "q".to_string(),
            SearchOptions::default(),
        );
        reg.register(sub_a);
        reg.register(sub_b);

        assert_eq!(reg.get_subscriptions_for_map("map-a").len(), 1);
        assert_eq!(reg.get_subscriptions_for_map("map-b").len(), 1);
        assert!(reg.get_subscriptions_for_map("map-c").is_empty());
    }

    // --- SearchService tests ---

    #[tokio::test]
    async fn search_service_returns_message_for_search_op() {
        let svc = make_service();
        let op = make_search_op("my-map", "hello");
        let resp = svc.oneshot(op).await.unwrap();
        assert!(
            matches!(resp, OperationResponse::Message(ref msg) if matches!(**msg, Message::SearchResp { .. })),
            "expected OperationResponse::Message(SearchResp), got {resp:?}"
        );
    }

    #[tokio::test]
    async fn search_service_returns_message_for_subscribe_op() {
        let svc = make_service();
        let op = make_subscribe_op("my-map", "hello");
        let resp = svc.oneshot(op).await.unwrap();
        assert!(
            matches!(resp, OperationResponse::Message(ref msg) if matches!(**msg, Message::SearchResp { .. })),
            "expected OperationResponse::Message(SearchResp), got {resp:?}"
        );
    }

    #[tokio::test]
    async fn search_service_subscribe_registers_subscription() {
        let reg = Arc::new(SearchRegistry::new());
        let indexes = Arc::new(RwLock::new(HashMap::new()));
        let conn_reg = Arc::new(ConnectionRegistry::new());
        let store_factory = make_factory();
        let svc = Arc::new(SearchService::new(
            Arc::clone(&reg),
            indexes,
            store_factory,
            conn_reg,
        ));

        let op = make_subscribe_op("my-map", "hello");
        svc.clone().oneshot(op).await.unwrap();

        let subs = reg.get_subscriptions_for_map("my-map");
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].subscription_id, "sub-1");
    }

    #[tokio::test]
    async fn search_service_unsubscribe_removes_subscription() {
        let reg = Arc::new(SearchRegistry::new());
        let indexes = Arc::new(RwLock::new(HashMap::new()));
        let conn_reg = Arc::new(ConnectionRegistry::new());
        let store_factory = make_factory();
        let svc = Arc::new(SearchService::new(
            Arc::clone(&reg),
            indexes,
            store_factory,
            conn_reg,
        ));

        svc.clone().oneshot(make_subscribe_op("my-map", "hello")).await.unwrap();
        assert_eq!(reg.get_subscriptions_for_map("my-map").len(), 1);

        svc.clone().oneshot(make_unsubscribe_op("sub-1")).await.unwrap();
        assert!(reg.get_subscriptions_for_map("my-map").is_empty());
    }

    #[tokio::test]
    async fn search_service_returns_wrong_service_for_non_search_ops() {
        let svc = make_service();
        let err = svc.oneshot(make_op_gc(service_names::SEARCH)).await.unwrap_err();
        assert!(matches!(err, OperationError::WrongService));
    }

    #[tokio::test]
    async fn search_service_name_is_search() {
        let svc = make_service();
        assert_eq!(svc.name(), "search");
    }

    #[tokio::test]
    async fn search_service_creates_index_lazily_on_first_search() {
        let indexes = Arc::new(RwLock::new(HashMap::<String, TantivyMapIndex>::new()));
        let reg = Arc::new(SearchRegistry::new());
        let conn_reg = Arc::new(ConnectionRegistry::new());
        let store_factory = make_factory();
        let svc = Arc::new(SearchService::new(
            reg,
            Arc::clone(&indexes),
            store_factory,
            conn_reg,
        ));

        assert!(indexes.read().is_empty(), "no index before first search");
        svc.clone().oneshot(make_search_op("lazy-map", "hello")).await.unwrap();
        assert!(indexes.read().contains_key("lazy-map"), "index created after search");
    }

    // --- Helper tests ---

    #[test]
    fn extract_all_text_collects_string_values() {
        let val = make_rmpv_map(&[("name", "Alice"), ("bio", "loves rust")]);
        let text = extract_all_text(&val);
        assert!(text.contains("Alice"));
        assert!(text.contains("loves rust"));
    }

    #[test]
    fn extract_all_text_skips_non_string_values() {
        let val = rmpv::Value::Map(vec![
            (
                rmpv::Value::String("name".into()),
                rmpv::Value::String("Bob".into()),
            ),
            (
                rmpv::Value::String("age".into()),
                rmpv::Value::Integer(30.into()),
            ),
            (
                rmpv::Value::String("active".into()),
                rmpv::Value::Boolean(true),
            ),
        ]);
        let text = extract_all_text(&val);
        assert!(text.contains("Bob"));
        assert!(!text.contains("30"));
    }

    #[test]
    fn extract_query_terms_splits_and_lowercases() {
        let terms = extract_query_terms("Hello World FOO");
        assert_eq!(terms, vec!["hello", "world", "foo"]);
    }
}
