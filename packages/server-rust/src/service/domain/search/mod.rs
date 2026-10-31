//! Search domain service backed by tantivy full-text search engine.
//!
//! Manages per-map tantivy `Index` instances (RAM directory), handles one-shot
//! search and live search subscriptions with ENTER/UPDATE/LEAVE delta semantics.
//! `SearchMutationObserver` indexes records and re-scores standing subscriptions
//! on every data mutation.

pub mod hybrid;
pub mod registry;
pub mod rrf;

pub use hybrid::{HybridSearchEngine, HybridSearchError, HybridSearchResult, SearchMethod};

use hybrid::FullTextMode;
pub use registry::{RegistryEntry, SubscriptionRegistry};

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll};
use std::time::Duration;

use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::{Mutex, RwLock};
use tantivy::schema::Value as TantivyValue;
use tokio::sync::mpsc;
use tower::Service;

use topgun_core::messages::base::ChangeEventType;
use topgun_core::messages::hybrid::SearchMethod as WireSearchMethod;
use topgun_core::messages::hybrid::{
    HybridSearchPayload, HybridSearchRespPayload, HybridSearchResultEntry, HybridSearchSubPayload,
    HybridSearchUnsubPayload, HybridSearchUpdatePayload,
};
use topgun_core::messages::search::{
    SearchOptions, SearchRespPayload, SearchResultEntry, SearchUpdatePayload,
};
use topgun_core::messages::Message;

use tracing::Instrument;

use crate::network::connection::{ConnectionId, ConnectionRegistry, OutboundMessage};
use crate::service::domain::index::IndexObserverFactory;
use crate::service::domain::predicate::value_to_rmpv;
use crate::service::operation::{
    service_names, Operation, OperationContext, OperationError, OperationResponse,
};
use crate::service::registry::{ManagedService, ServiceContext};
use crate::storage::mutation_observer::MutationObserver;
use crate::storage::record::{Record, RecordValue};
use crate::storage::RecordStoreFactory;

// ---------------------------------------------------------------------------
// CachedHybridResult (used by HybridSearchSubscription)
// ---------------------------------------------------------------------------

/// Last-known fused score and per-method scores for a key in a hybrid subscription.
#[derive(Debug, Clone)]
pub struct CachedHybridResult {
    pub score: f64,
    pub method_scores: HashMap<SearchMethod, f64>,
    pub value: Option<rmpv::Value>,
}

// ---------------------------------------------------------------------------
// HybridSearchSubscription
// ---------------------------------------------------------------------------

/// A standing hybrid search subscription.
pub struct HybridSearchSubscription {
    pub subscription_id: String,
    pub connection_id: ConnectionId,
    pub map_name: String,
    pub query_text: String,
    pub methods: Vec<SearchMethod>,
    pub k: usize,
    pub query_vector: Option<Vec<f32>>,
    pub predicate: Option<topgun_core::messages::base::PredicateNode>,
    pub include_value: bool,
    pub min_score: Option<f64>,
    /// Current result cache: key -> `CachedHybridResult`.
    pub current_results: DashMap<String, CachedHybridResult>,
}

impl HybridSearchSubscription {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        subscription_id: String,
        connection_id: ConnectionId,
        map_name: String,
        query_text: String,
        methods: Vec<SearchMethod>,
        k: usize,
        query_vector: Option<Vec<f32>>,
        predicate: Option<topgun_core::messages::base::PredicateNode>,
        include_value: bool,
        min_score: Option<f64>,
    ) -> Self {
        Self {
            subscription_id,
            connection_id,
            map_name,
            query_text,
            methods,
            k,
            query_vector,
            predicate,
            include_value,
            min_score,
            current_results: DashMap::new(),
        }
    }
}

impl RegistryEntry for HybridSearchSubscription {
    fn subscription_id(&self) -> &str {
        &self.subscription_id
    }

    fn connection_id(&self) -> ConnectionId {
        self.connection_id
    }

    fn map_name(&self) -> &str {
        &self.map_name
    }
}

// ---------------------------------------------------------------------------
// HybridSearchRegistry type alias
// ---------------------------------------------------------------------------

/// Concurrent registry of standing hybrid search subscriptions.
///
/// Type alias for `SubscriptionRegistry<HybridSearchSubscription>` providing
/// the full API surface (`register`, `unregister`, `unregister_by_connection`,
/// `get_subscriptions_for_map`, `has_subscriptions_for_map`, `has_any_subscriptions`).
pub type HybridSearchRegistry = SubscriptionRegistry<HybridSearchSubscription>;

// ---------------------------------------------------------------------------
// SearchConfig
// ---------------------------------------------------------------------------

/// Configuration for the search indexing batch processor.
///
/// Production defaults use a 100ms interval and 500-event threshold to reduce
/// tantivy commit frequency from ~60/sec down to ~2-10/sec under sustained load.
/// Test overrides keep 16ms/100 to preserve integration test responsiveness.
#[derive(Debug, Clone, Copy)]
pub struct SearchConfig {
    /// Milliseconds between batch flushes (default: 100).
    pub batch_interval_ms: u64,
    /// Maximum events before forced flush (default: 500).
    pub batch_flush_threshold: usize,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            batch_interval_ms: 100,
            batch_flush_threshold: 500,
        }
    }
}

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

impl RegistryEntry for SearchSubscription {
    fn subscription_id(&self) -> &str {
        &self.subscription_id
    }

    fn connection_id(&self) -> ConnectionId {
        self.connection_id
    }

    fn map_name(&self) -> &str {
        &self.map_name
    }
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
// SearchRegistry type alias
// ---------------------------------------------------------------------------

/// Concurrent registry of standing text-search subscriptions.
///
/// Type alias for `SubscriptionRegistry<SearchSubscription>` providing
/// the full API surface (`register`, `unregister`, `unregister_by_connection`,
/// `get_subscriptions_for_map`, `has_subscriptions_for_map`, `has_any_subscriptions`).
pub type SearchRegistry = SubscriptionRegistry<SearchSubscription>;

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
        let key_query = Box::new(TermQuery::new(key_term, IndexRecordOption::Basic));

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
            writer.commit().expect("tantivy commit after clear failed");
        } // drop writer lock before reloading reader
        self.reader
            .reload()
            .expect("tantivy reader reload after clear failed");
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
        .map(|t| {
            t.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
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

/// Generic unsubscribe handler used by both text-search and hybrid-search unsubscribe arms.
///
/// Removes the subscription from the registry and answers with no frame: an
/// unsubscribe is fire-and-forget on the client, and an operation acknowledgement
/// here would be read as confirming writes it never carried (TG-SYNC-004).
/// This is a free function (not a method on `SearchService`) to avoid borrow complications
/// when both `self.registry` and `self.hybrid_registry` need to be called from within
/// the same match arm.
///
/// Only the calling connection's subscription of that id is removed; an id it
/// does not hold — another connection's, or none at all — is a no-op.
fn handle_unsubscribe<S: RegistryEntry>(
    registry: &SubscriptionRegistry<S>,
    connection_id: Option<ConnectionId>,
    subscription_id: &str,
) -> OperationResponse {
    if let Some(connection_id) = connection_id {
        let _ = registry.unregister(connection_id, subscription_id);
    }
    OperationResponse::Empty
}

// ---------------------------------------------------------------------------
// Batched mutation event
// ---------------------------------------------------------------------------

/// Describes the index operation to apply in the batch processor.
#[derive(Debug)]
enum IndexOp {
    /// Index (insert or update) a document with the given key/value.
    Index {
        key: String,
        value: rmpv::Value,
        change_type: ChangeEventType,
    },
    /// Remove a document by key.
    Remove { key: String },
    /// Clear all documents for the map.
    Clear,
}

/// A single batched mutation event sent to the background processor.
#[derive(Debug)]
struct MutationEvent {
    map_name: String,
    op: IndexOp,
}

// ---------------------------------------------------------------------------
// Population state, write generations and the hybrid dirty set
// ---------------------------------------------------------------------------

/// State shared by the `SearchService` and every `SearchMutationObserver`
/// attached to it.
///
/// - `generations`: one write generation per map. An attached observer bumps it
///   at callback time for every non-backup event, whether the event is then
///   enqueued or skipped, never in the batch processor. A population reads it
///   before its resident snapshot and records the map as populated only when it
///   is unchanged after the swap (`TG-READ-001`).
/// - `dirty`: the map names whose hybrid subscriptions need a re-evaluation.
///   The batch processor inserts a name instead of sending it on a channel; an
///   insert is idempotent, never blocks and wakes the one notifier task, which
///   takes the whole set at the start of a pass. The set holds at most one entry
///   per map, whatever the write rate.
#[derive(Default)]
struct SearchShared {
    generations: DashMap<String, Arc<AtomicU64>>,
    dirty: Mutex<HashSet<String>>,
    dirty_wake: tokio::sync::Notify,
}

impl SearchShared {
    /// The write generation of `map_name`, created at 0 on first use.
    fn generation(&self, map_name: &str) -> Arc<AtomicU64> {
        if let Some(cell) = self.generations.get(map_name) {
            return Arc::clone(cell.value());
        }
        Arc::clone(
            self.generations
                .entry(map_name.to_owned())
                .or_default()
                .value(),
        )
    }

    /// Marks `map_name` for a hybrid re-evaluation and wakes the notifier. A
    /// name already in the set is not added twice; a wake while a pass runs is
    /// kept as one permit, so the next pass serves it.
    fn mark_dirty(&self, map_name: &str) {
        {
            let mut dirty = self.dirty.lock();
            if !dirty.contains(map_name) {
                dirty.insert(map_name.to_owned());
            }
        }
        self.dirty_wake.notify_one();
    }

    fn take_dirty(&self) -> HashSet<String> {
        std::mem::take(&mut *self.dirty.lock())
    }
}

/// The link from one `SearchMutationObserver` (and its batch processor) to the
/// service it is attached to, returned by `SearchMutationObserver::new` and
/// consumed by `SearchService::spawn_hybrid_notifier`.
///
/// Until it is attached, the observer sets the map's `needs_population` flag on
/// every non-backup event instead of bumping a generation (searches stay
/// correct at the cost of a population per search) and its batch processor
/// marks nothing dirty (no hybrid deltas).
pub struct HybridLink {
    cell: Arc<OnceLock<Arc<SearchShared>>>,
}

/// The population state of one map's full-text index. An absent entry means
/// "needs population"; so does a set `needs_population` flag over `Populated`.
enum PopState {
    /// The last population's scan was overlapped by no write.
    Populated,
    /// A population task runs; a search that needs population joins it.
    InFlight {
        id: u64,
        outcome: tokio::sync::watch::Receiver<Option<bool>>,
    },
}

// ---------------------------------------------------------------------------
// SearchMutationObserver
// ---------------------------------------------------------------------------

/// Implements `MutationObserver` for the search domain.
///
/// On data mutations:
///   1. Enqueues an `IndexOp` event to the background batch processor (non-blocking).
///   2. The background task indexes documents in batch (WRITE lock + commit once per map).
///   3. After commit, re-scores changed keys against standing subscriptions
///      and sends `SearchUpdate` messages (ENTER/UPDATE/LEAVE) via `ConnectionRegistry`.
pub struct SearchMutationObserver {
    map_name: String,
    /// Sender for the background batch processor.
    /// `UnboundedSender::send()` is synchronous, safe to call from sync trait methods.
    event_tx: mpsc::Sender<MutationEvent>,
    /// Shutdown signal for the background task.
    shutdown_tx: Arc<tokio::sync::watch::Sender<bool>>,
    /// Retained for per-enqueue subscription check — avoids indexing cost when
    /// no text-search subscriptions are active for this map.
    registry: Arc<SearchRegistry>,
    /// Retained for per-enqueue hybrid subscription check — ensures the tantivy
    /// index is kept current even when only hybrid subscriptions are active.
    hybrid_registry: Arc<HybridSearchRegistry>,
    /// Shared flag map: set to true when a write is skipped due to no active
    /// subscriptions, so `SearchService` knows to populate the index on first query.
    needs_population: Arc<DashMap<String, AtomicBool>>,
    /// Set once the link returned by `new` is attached to a service.
    link: Arc<OnceLock<Arc<SearchShared>>>,
    /// This map's write generation, looked up once after the link is attached.
    generation: OnceLock<Arc<AtomicU64>>,
}

impl SearchMutationObserver {
    /// Creates a new observer and spawns the background batch processor task.
    ///
    /// Returns `(Self, HybridLink)`. The caller hands the link to
    /// `SearchService::spawn_hybrid_notifier`, which attaches this observer and
    /// its batch processor to the service's write generations and hybrid dirty
    /// set, before the store serves a write. The link avoids giving the
    /// observer a reference to `SearchService`, which would create a circular
    /// `Arc` dependency.
    pub fn new(
        map_name: String,
        registry: Arc<SearchRegistry>,
        hybrid_registry: Arc<HybridSearchRegistry>,
        indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>>,
        connection_registry: Arc<ConnectionRegistry>,
        config: SearchConfig,
        needs_population: Arc<DashMap<String, AtomicBool>>,
    ) -> (Self, HybridLink) {
        // Bounded so a stalled batch processor applies backpressure instead of
        // growing memory without limit. Capacity 10_000 is large enough that
        // healthy sustained write throughput never reaches it; if it does,
        // try_send returns Err(Full) and the call site logs a warn so the
        // condition becomes observable rather than a silent OOM.
        let (event_tx, event_rx) = mpsc::channel::<MutationEvent>(10_000);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let shutdown_tx = Arc::new(shutdown_tx);

        let link: Arc<OnceLock<Arc<SearchShared>>> = Arc::new(OnceLock::new());

        // Clone the registry Arc so the struct retains a reference for the
        // subscription check in enqueue_index/enqueue_remove, while the batch
        // processor task receives its own clone for subscription notification.
        let registry_for_task = Arc::clone(&registry);

        // Spawn background batch processor — registry_for_task is moved into
        // the batch task; the struct holds the original registry for checks.
        tokio::spawn(run_batch_processor(
            event_rx,
            shutdown_rx,
            registry_for_task,
            indexes,
            connection_registry,
            Duration::from_millis(config.batch_interval_ms),
            config.batch_flush_threshold,
            Arc::clone(&link),
        ));

        let observer = Self {
            map_name,
            event_tx,
            shutdown_tx,
            registry,
            hybrid_registry,
            needs_population,
            link: Arc::clone(&link),
            generation: OnceLock::new(),
        };
        (observer, HybridLink { cell: link })
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

    /// Records a non-backup event at callback time, before it is enqueued or
    /// skipped (`TG-READ-001`): a population that this write overlapped must
    /// not record the map as populated, also when the write is skipped and
    /// never reaches the batch processor. Unattached, the observer marks the
    /// map as needing population instead.
    fn note_write(&self) {
        if let Some(shared) = self.link.get() {
            self.generation
                .get_or_init(|| shared.generation(&self.map_name))
                .fetch_add(1, Ordering::SeqCst);
        } else {
            self.mark_needs_population();
        }
    }

    fn mark_needs_population(&self) {
        self.needs_population
            .entry(self.map_name.clone())
            .or_insert_with(|| AtomicBool::new(false))
            .store(true, Ordering::Release);
    }

    /// Enqueues an index operation for the batch processor (no synchronous indexing).
    ///
    /// Skips enqueue when no text-search *or* hybrid subscriptions are active for
    /// this map, avoiding tantivy indexing cost for pure write workloads. Sets the
    /// `needs_population` flag so the index is rebuilt lazily on first search query.
    /// Hybrid subscriptions also require the tantivy index to be current so they
    /// are included in the "should we index?" check.
    fn enqueue_index(&self, key: &str, value: rmpv::Value, change_type: ChangeEventType) {
        // Skip indexing when neither text-search nor hybrid subscriptions exist for this map.
        let has_text = self.registry.has_subscriptions_for_map(&self.map_name);
        let has_hybrid = self
            .hybrid_registry
            .has_subscriptions_for_map(&self.map_name);
        if !has_text && !has_hybrid {
            // Mark this map as needing population when a subscription arrives.
            self.mark_needs_population();
            return;
        }
        if let Err(err) = self.event_tx.try_send(MutationEvent {
            map_name: self.map_name.clone(),
            op: IndexOp::Index {
                key: key.to_owned(),
                value,
                change_type,
            },
        }) {
            tracing::warn!(
                map = %self.map_name,
                key,
                "search index queue saturated (10k capacity); event dropped — investigate stalled batch processor: {err}"
            );
        }
    }

    /// Enqueues a remove operation for the batch processor (no synchronous indexing).
    ///
    /// Skips enqueue when no text-search or hybrid subscriptions are active for
    /// this map. Sets the `needs_population` flag so the index is rebuilt lazily
    /// on first search query (ensuring removals are reflected when indexing resumes).
    fn enqueue_remove(&self, key: &str) {
        // Skip indexing when no subscriptions exist for this map.
        let has_text = self.registry.has_subscriptions_for_map(&self.map_name);
        let has_hybrid = self
            .hybrid_registry
            .has_subscriptions_for_map(&self.map_name);
        if !has_text && !has_hybrid {
            // Mark map as needing population so the first query triggers a full
            // rebuild that correctly excludes the removed key.
            self.mark_needs_population();
            return;
        }
        if let Err(err) = self.event_tx.try_send(MutationEvent {
            map_name: self.map_name.clone(),
            op: IndexOp::Remove {
                key: key.to_owned(),
            },
        }) {
            tracing::warn!(
                map = %self.map_name,
                key,
                "search index queue saturated (10k capacity); remove dropped — investigate stalled batch processor: {err}"
            );
        }
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
        self.note_write();
        let rmpv_val = record_to_rmpv(&record.value);
        self.enqueue_index(key, rmpv_val, ChangeEventType::ENTER);
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
        self.note_write();
        let rmpv_val = record_to_rmpv(&record.value);
        self.enqueue_index(key, rmpv_val, ChangeEventType::UPDATE);
    }

    fn on_remove(&self, key: &str, _record: &Record, is_backup: bool) {
        if is_backup {
            return;
        }
        self.note_write();
        self.enqueue_remove(key);
    }

    /// A delete of a key that was not resident. The index holds stored keys
    /// whatever their residency, so without this the deleted key would stay in
    /// it and search would return it.
    fn on_remove_key(&self, key: &str, is_backup: bool) {
        if is_backup {
            return;
        }
        self.note_write();
        self.enqueue_remove(key);
    }

    fn on_clear(&self) {
        self.note_write();
        if let Err(err) = self.event_tx.try_send(MutationEvent {
            map_name: self.map_name.clone(),
            op: IndexOp::Clear,
        }) {
            tracing::warn!(
                map = %self.map_name,
                "search index queue saturated (10k capacity); clear dropped — investigate stalled batch processor: {err}"
            );
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
        RecordValue::OrMap { records, .. } => {
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
/// Flushes when either `batch_interval` elapses or `batch_flush_threshold` events
/// have been accumulated, whichever comes first. On shutdown signal, drains
/// remaining events before exiting.
#[allow(clippy::too_many_arguments)]
async fn run_batch_processor(
    mut event_rx: mpsc::Receiver<MutationEvent>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    registry: Arc<SearchRegistry>,
    indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>>,
    connection_registry: Arc<ConnectionRegistry>,
    batch_interval: Duration,
    batch_flush_threshold: usize,
    link: Arc<OnceLock<Arc<SearchShared>>>,
) {
    let mut batch: Vec<MutationEvent> = Vec::new();

    loop {
        // Phase 1: Accumulate events (up to batch_interval or BATCH_FLUSH_THRESHOLD)
        if batch.is_empty() {
            // Nothing pending — wait for first event or shutdown.
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
                            process_batch(batch, &registry, &indexes, &connection_registry, &link);
                        }
                        return;
                    }
                }
            }
        }

        // Accumulate more events until timer or threshold.
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
                            None => break, // channel closed
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
                                process_batch(batch, &registry, &indexes, &connection_registry, &link);
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
            process_batch(
                current_batch,
                &registry,
                &indexes,
                &connection_registry,
                &link,
            );
        }
    }
}

/// Sends a LEAVE `SearchUpdate` for the given subscription and key.
fn send_leave(conn_reg: &ConnectionRegistry, sub: &SearchSubscription, key: &str) {
    let msg = Message::SearchUpdate {
        payload: SearchUpdatePayload {
            subscription_id: sub.subscription_id.clone(),
            key: key.to_owned(),
            value: rmpv::Value::Nil,
            score: 0.0,
            matched_terms: Vec::new(),
            change_type: ChangeEventType::LEAVE,
        },
    };
    send_to_connection(conn_reg, sub.connection_id, &msg);
}

/// Notifies subscriptions about a Clear: sends LEAVE for every cached key and
/// clears `current_results`.
fn notify_clear_subscriptions(
    registry: &SearchRegistry,
    connection_registry: &ConnectionRegistry,
    map_name: &str,
) {
    let subs = registry.get_subscriptions_for_map(map_name);
    for sub in &subs {
        let keys: Vec<String> = sub
            .current_results
            .iter()
            .map(|e| e.key().clone())
            .collect();
        for key in keys {
            sub.current_results.remove(&key);
            send_leave(connection_registry, sub, &key);
        }
    }
}

/// Re-scores a single key against all subscriptions for a map and sends
/// ENTER/UPDATE/LEAVE deltas as appropriate.
fn notify_key_subscriptions(
    index: &TantivyMapIndex,
    subs: &[Arc<SearchSubscription>],
    connection_registry: &ConnectionRegistry,
    key: &str,
    change_type: &ChangeEventType,
) {
    if *change_type == ChangeEventType::LEAVE {
        for sub in subs {
            if sub.current_results.remove(key).is_some() {
                send_leave(connection_registry, sub, key);
            }
        }
        return;
    }

    for sub in subs {
        let scored = index.score_single_document(key, &sub.query);
        match scored {
            Some(doc) => {
                let score = doc.score;
                let min_score = sub.options.min_score.unwrap_or(0.0);
                if score < min_score {
                    if sub.current_results.remove(key).is_some() {
                        send_leave(connection_registry, sub, key);
                    }
                    continue;
                }

                let delta_type = if sub.current_results.contains_key(key) {
                    ChangeEventType::UPDATE
                } else {
                    ChangeEventType::ENTER
                };

                sub.current_results.insert(
                    key.to_owned(),
                    CachedSearchResult {
                        score,
                        matched_terms: doc.matched_terms.clone(),
                    },
                );

                let msg = Message::SearchUpdate {
                    payload: SearchUpdatePayload {
                        subscription_id: sub.subscription_id.clone(),
                        key: key.to_owned(),
                        value: rmpv::Value::Nil,
                        score,
                        matched_terms: doc.matched_terms,
                        change_type: delta_type,
                    },
                };
                send_to_connection(connection_registry, sub.connection_id, &msg);
            }
            None => {
                if sub.current_results.remove(key).is_some() {
                    send_leave(connection_registry, sub, key);
                }
            }
        }
    }
}

/// Processes a batch of mutation events: indexes documents, then notifies subscribers.
///
/// Three phases:
///   1. Deduplicate per `(map_name, key)` keeping the last `IndexOp`. A `Clear` op
///      discards all prior per-key ops for that map.
///   2. Acquire WRITE lock per map, apply index/remove/clear ops, commit once.
///   3. Acquire READ lock, re-score subscriptions, send ENTER/UPDATE/LEAVE deltas.
///      For `Clear` ops, send LEAVE for all cached keys and clear `current_results`.
fn process_batch(
    batch: Vec<MutationEvent>,
    registry: &Arc<SearchRegistry>,
    indexes: &Arc<RwLock<HashMap<String, TantivyMapIndex>>>,
    connection_registry: &Arc<ConnectionRegistry>,
    link: &OnceLock<Arc<SearchShared>>,
) {
    // --- Phase 1: Deduplicate ---
    let mut per_key_ops: HashMap<(String, String), IndexOp> = HashMap::new();
    let mut cleared_maps: HashMap<String, bool> = HashMap::new();

    for evt in batch {
        match evt.op {
            IndexOp::Clear => {
                per_key_ops.retain(|(m, _), _| *m != evt.map_name);
                cleared_maps.insert(evt.map_name, true);
            }
            IndexOp::Index { ref key, .. } | IndexOp::Remove { ref key, .. } => {
                let map_key = (evt.map_name, key.clone());
                per_key_ops.insert(map_key, evt.op);
            }
        }
    }

    // Group per-key ops by map_name.
    let mut ops_by_map: HashMap<String, Vec<IndexOp>> = HashMap::new();
    for ((map_name, _), op) in per_key_ops {
        ops_by_map.entry(map_name).or_default().push(op);
    }
    for map_name in cleared_maps.keys() {
        ops_by_map.entry(map_name.clone()).or_default();
    }

    // --- Phase 2: Index documents (WRITE lock, one commit per map) ---
    {
        let mut indexes_w = indexes.write();
        for (map_name, ops) in &ops_by_map {
            let has_clear = cleared_maps.contains_key(map_name);
            let index = indexes_w.entry(map_name.clone()).or_default();

            if has_clear {
                index.clear();
            }

            let mut did_mutate = has_clear;
            for op in ops {
                match op {
                    IndexOp::Index { key, value, .. } => {
                        index.index_document(key, value);
                        did_mutate = true;
                    }
                    IndexOp::Remove { key } => {
                        index.remove_document(key);
                        did_mutate = true;
                    }
                    IndexOp::Clear => {}
                }
            }

            // TantivyMapIndex::clear() commits internally, so only commit
            // explicitly when there are per-key ops after a clear, or when
            // there was no clear at all.
            if did_mutate && (!has_clear || !ops.is_empty()) {
                index.commit();
            }
        }
    } // WRITE lock dropped

    // --- Phase 3: Notify subscribers ---
    for map_name in cleared_maps.keys() {
        notify_clear_subscriptions(registry, connection_registry, map_name);
    }

    for (map_name, ops) in &ops_by_map {
        let subs = registry.get_subscriptions_for_map(map_name);
        if subs.is_empty() {
            continue;
        }

        let indexes_read = indexes.read();
        let Some(index) = indexes_read.get(map_name) else {
            continue;
        };

        for op in ops {
            let (key, change_type) = match op {
                IndexOp::Index {
                    key, change_type, ..
                } => (key.as_str(), change_type),
                IndexOp::Remove { key } => (key.as_str(), &ChangeEventType::LEAVE),
                IndexOp::Clear => continue,
            };
            notify_key_subscriptions(index, &subs, connection_registry, key, change_type);
        }
    }

    // --- Phase 4: Notify hybrid subscribers ---
    // Mark each mutated map dirty (cleared maps are in `ops_by_map` too); the
    // one notifier task re-evaluates each dirty map once per pass.
    if let Some(shared) = link.get() {
        for map_name in ops_by_map.keys() {
            shared.mark_dirty(map_name);
        }
    }
}

// ---------------------------------------------------------------------------
// Full-text index population
// ---------------------------------------------------------------------------

/// The message a search answers when the population it waited for failed.
const STORE_SCAN_FAILED: &str = "store scan failed";

/// Populates per-map full-text indexes from every stored record
/// (`TG-READ-001`).
///
/// A population runs in its own spawned task, at most one per map; a search
/// that needs population while one runs joins it. It reads through
/// `RecordStoreFactory::scan_merged_batches`, builds a new index on a blocking
/// thread, and takes the indexes' write lock only to swap it in. It records
/// the map as populated only when the map's write generation did not move
/// between the read before its resident snapshot and the swap.
struct Populations {
    factory: Arc<RecordStoreFactory>,
    indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>>,
    needs_population: Arc<DashMap<String, AtomicBool>>,
    shared: Arc<SearchShared>,
    states: Mutex<HashMap<String, PopState>>,
    next_id: AtomicU64,
    /// Populations started so far.
    #[cfg(test)]
    started: AtomicU64,
    /// When set, the blocking indexer panics in its commit.
    #[cfg(test)]
    commit_panic: AtomicBool,
}

impl Populations {
    /// Returns once `map_name`'s index answers from a completed population:
    /// at once when the map is populated, else after the population this call
    /// starts or joins.
    ///
    /// # Errors
    ///
    /// The population failed or panicked: its store scan or its indexer.
    async fn ensure(self: &Arc<Self>, map_name: &str) -> anyhow::Result<()> {
        let mut outcome = {
            let mut states = self.states.lock();
            let flagged = self
                .needs_population
                .get(map_name)
                .is_some_and(|flag| flag.load(Ordering::Acquire));
            match states.get(map_name) {
                Some(PopState::Populated) if !flagged => return Ok(()),
                Some(PopState::InFlight { outcome, .. }) => outcome.clone(),
                _ => {
                    let id = self.next_id.fetch_add(1, Ordering::Relaxed);
                    let (tx, rx) = tokio::sync::watch::channel(None);
                    states.insert(
                        map_name.to_owned(),
                        PopState::InFlight {
                            id,
                            outcome: rx.clone(),
                        },
                    );
                    // Spawned, not awaited in the request future: a request
                    // timeout that drops the search cannot cut the population.
                    tokio::spawn(Arc::clone(self).run(map_name.to_owned(), id, tx));
                    rx
                }
            }
        };
        let succeeded = outcome
            .wait_for(Option::is_some)
            .await
            .is_ok_and(|done| *done == Some(true));
        if succeeded {
            Ok(())
        } else {
            Err(anyhow::anyhow!(STORE_SCAN_FAILED))
        }
    }

    async fn run(
        self: Arc<Self>,
        map_name: String,
        id: u64,
        outcome: tokio::sync::watch::Sender<Option<bool>>,
    ) {
        // Publishes "failed" and releases the in-flight marker on every exit
        // that does not finish, a panic included.
        let guard = PopGuard {
            pops: Arc::clone(&self),
            map_name: map_name.clone(),
            id,
            outcome,
            finished: false,
        };
        #[cfg(test)]
        self.started.fetch_add(1, Ordering::SeqCst);

        // Both before the resident snapshot: a write the snapshot misses bumps
        // the generation after this read (the observer runs after the engine
        // reflects the write), so this population cannot record "populated".
        if let Some(flag) = self.needs_population.get(&map_name) {
            flag.store(false, Ordering::Release);
        }
        let generation = self.shared.generation(&map_name);
        let start_generation = generation.load(Ordering::SeqCst);

        // Capacity 1: back-pressure from the indexer to the scan, at most one
        // batch queued.
        let (batch_tx, mut batch_rx) = mpsc::channel::<Vec<(String, RecordValue)>>(1);
        #[cfg(test)]
        let commit_panic = self.commit_panic.load(Ordering::SeqCst);
        let indexer = tokio::task::spawn_blocking(move || {
            let index = TantivyMapIndex::new();
            while let Some(batch) = batch_rx.blocking_recv() {
                for (key, value) in &batch {
                    index.index_document(key, &record_to_rmpv(value));
                }
            }
            #[cfg(test)]
            assert!(!commit_panic, "injected indexer commit panic");
            index.commit();
            index
        });

        let scanned = {
            let batch_tx = &batch_tx;
            self.factory
                .scan_merged_batches(&map_name, move |batch| async move {
                    batch_tx
                        .send(batch)
                        .await
                        .map_err(|_| anyhow::anyhow!("the full-text indexer ended during the scan"))
                })
                .await
        };
        // The indexer cannot tell a finished scan from a failed one by the
        // channel closing, so it only builds; the swap is decided here.
        drop(batch_tx);
        let built = indexer.await;
        let index = match (scanned, built) {
            (Ok(()), Ok(index)) => index,
            (Err(error), _) => {
                tracing::warn!(map = %map_name, error = %error, "full-text index population failed: store scan failed");
                return;
            }
            (Ok(()), Err(error)) => {
                tracing::warn!(map = %map_name, error = %error, "full-text index population failed: the indexer did not finish");
                return;
            }
        };

        let replaced = self.indexes.write().insert(map_name, index);
        // Dropped after the write lock is released: dropping an index writer
        // joins its threads.
        drop(replaced);
        let overlapped = generation.load(Ordering::SeqCst) != start_generation;
        guard.finish(!overlapped);
    }

    #[cfg(test)]
    fn is_populated(&self, map_name: &str) -> bool {
        matches!(self.states.lock().get(map_name), Some(PopState::Populated))
    }

    #[cfg(test)]
    fn in_flight(&self, map_name: &str) -> bool {
        matches!(
            self.states.lock().get(map_name),
            Some(PopState::InFlight { .. })
        )
    }
}

/// Ends one population: publishes its outcome and replaces its in-flight
/// marker, unless a later population or a reset already replaced it.
struct PopGuard {
    pops: Arc<Populations>,
    map_name: String,
    id: u64,
    outcome: tokio::sync::watch::Sender<Option<bool>>,
    finished: bool,
}

impl PopGuard {
    fn settle(&self, next: Option<PopState>) {
        let mut states = self.pops.states.lock();
        if matches!(states.get(&self.map_name), Some(PopState::InFlight { id, .. }) if *id == self.id)
        {
            match next {
                Some(state) => {
                    states.insert(self.map_name.clone(), state);
                }
                None => {
                    states.remove(&self.map_name);
                }
            }
        }
    }

    /// The swap happened. `populated`: no write overlapped the scan, so the map
    /// is recorded as populated; otherwise it stays "needs population" and the
    /// swapped-in index serves only the searches that waited.
    fn finish(mut self, populated: bool) {
        self.settle(populated.then_some(PopState::Populated));
        self.finished = true;
        self.outcome.send_replace(Some(true));
    }
}

impl Drop for PopGuard {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if let Some(flag) = self.pops.needs_population.get(&self.map_name) {
            flag.store(true, Ordering::Release);
        }
        self.settle(None);
        self.outcome.send_replace(Some(false));
    }
}

// ---------------------------------------------------------------------------
// SearchService
// ---------------------------------------------------------------------------

/// Full-text search domain service backed by per-map tantivy indexes.
///
/// Handles `Operation::Search`, `Operation::SearchSubscribe`,
/// `Operation::SearchUnsubscribe`, `Operation::HybridSearch`,
/// `Operation::HybridSearchSubscribe`, and `Operation::HybridSearchUnsubscribe`.
/// Returns `OperationError::WrongService` for all other operations.
pub struct SearchService {
    registry: Arc<SearchRegistry>,
    hybrid_registry: Arc<HybridSearchRegistry>,
    indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>>,
    /// Used by hybrid search handler to resolve `IndexRegistry` per map.
    index_observer_factory: Arc<IndexObserverFactory>,
    /// Connection registry used to push hybrid search update deltas.
    connection_registry: Arc<ConnectionRegistry>,
    /// Shutdown signals collected from registered `SearchMutationObserver`s.
    /// Sending `true` on each channel tells the background batch processor to
    /// drain pending events and exit.
    observer_shutdown_signals: RwLock<Vec<Arc<tokio::sync::watch::Sender<bool>>>>,
    /// Shared with `SearchMutationObserver`: set to true when `enqueue_index`/
    /// `enqueue_remove` skips a write due to no active subscriptions. Cleared
    /// by a population before its resident snapshot.
    needs_population: Arc<DashMap<String, AtomicBool>>,
    /// `HybridSearchEngine` wired after construction (two-phase `OnceLock` to break
    /// the circular Arc: `SearchService` -> `HybridSearchEngine` -> `SearchService`).
    hybrid_engine: OnceLock<Arc<HybridSearchEngine>>,
    /// Write generations and the hybrid dirty set, shared with every attached
    /// observer.
    shared: Arc<SearchShared>,
    populations: Arc<Populations>,
    /// Set when the one hybrid notifier task has been spawned.
    notifier_started: AtomicBool,
    /// Maps handled by notifier passes so far.
    #[cfg(test)]
    notifier_passes: AtomicU64,
}

/// The answer to a `SEARCH` / `SEARCH_SUB` whose population failed.
fn search_failed(request_id: String) -> SearchRespPayload {
    SearchRespPayload {
        request_id,
        results: Vec::new(),
        total_count: 0,
        error: Some(STORE_SCAN_FAILED.to_string()),
    }
}

/// Convert an `Instant` elapsed duration to milliseconds, saturating at `u64::MAX`.
fn elapsed_ms(start: std::time::Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

impl SearchService {
    /// Creates a new `SearchService`.
    ///
    /// `hybrid_registry` must be the same `Arc` shared with `SearchObserverFactory`
    /// so that mutations indexed by the observer are visible to hybrid subscribers.
    #[must_use]
    pub fn new(
        registry: Arc<SearchRegistry>,
        hybrid_registry: Arc<HybridSearchRegistry>,
        indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>>,
        record_store_factory: Arc<RecordStoreFactory>,
        connection_registry: Arc<ConnectionRegistry>,
        needs_population: Arc<DashMap<String, AtomicBool>>,
        index_observer_factory: Arc<IndexObserverFactory>,
    ) -> Self {
        let shared = Arc::new(SearchShared::default());
        let populations = Arc::new(Populations {
            factory: record_store_factory,
            indexes: Arc::clone(&indexes),
            needs_population: Arc::clone(&needs_population),
            shared: Arc::clone(&shared),
            states: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(0),
            #[cfg(test)]
            started: AtomicU64::new(0),
            #[cfg(test)]
            commit_panic: AtomicBool::new(false),
        });
        Self {
            registry,
            hybrid_registry,
            indexes,
            index_observer_factory,
            connection_registry,
            observer_shutdown_signals: RwLock::new(Vec::new()),
            needs_population,
            hybrid_engine: OnceLock::new(),
            shared,
            populations,
            notifier_started: AtomicBool::new(false),
            #[cfg(test)]
            notifier_passes: AtomicU64::new(0),
        }
    }

    /// Wires the `HybridSearchEngine` after construction (two-phase `OnceLock`
    /// to break the circular Arc: `SearchService` -> `HybridSearchEngine` -> `SearchService`).
    ///
    /// # Panics
    ///
    /// Panics if called more than once (prevents accidental double-wiring).
    pub fn set_hybrid_engine(&self, engine: Arc<HybridSearchEngine>) {
        assert!(
            self.hybrid_engine.set(engine).is_ok(),
            "hybrid_engine set twice"
        );
    }

    /// Registers an observer's shutdown signal so that `ManagedService::shutdown()`
    /// can flush all background batch processors.
    pub fn register_observer_shutdown(&self, signal: Arc<tokio::sync::watch::Sender<bool>>) {
        self.observer_shutdown_signals.write().push(signal);
    }

    /// Attaches the observer behind `link` to this service (its write
    /// generations and hybrid dirty set) and, on the first call, spawns the one
    /// hybrid notifier task.
    ///
    /// A pass of the notifier takes the whole dirty set and re-evaluates each
    /// map in it once; passes are sequential, and wakes that arrive while a pass
    /// runs are served by the next one. The task holds an `Arc<SearchService>`;
    /// it lives as long as the process.
    pub fn spawn_hybrid_notifier(self: &Arc<Self>, link: HybridLink) {
        let _ = link.cell.set(Arc::clone(&self.shared));
        if self.notifier_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let svc = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                svc.shared.dirty_wake.notified().await;
                for map_name in svc.shared.take_dirty() {
                    svc.notify_hybrid_subscriptions_for_map(&map_name).await;
                }
            }
        });
    }

    /// Executes a search against the index for a map and builds `SearchResultEntry` list.
    async fn execute_search(
        &self,
        map_name: &str,
        query_str: &str,
        options: &SearchOptions,
    ) -> anyhow::Result<Vec<SearchResultEntry>> {
        let scored_docs = self
            .search_map(map_name, query_str, options, FullTextMode::Populate)
            .await?;
        Ok(scored_docs
            .into_iter()
            .map(|doc| {
                SearchResultEntry {
                    key: doc.key,
                    value: rmpv::Value::Nil, // client fetches full value if needed
                    score: doc.score,
                    matched_terms: doc.matched_terms,
                }
            })
            .collect())
    }

    /// Executes a BM25 full-text search against the tantivy index for `map_name`.
    ///
    /// With `FullTextMode::Populate` a map that needs population is populated
    /// first and the call waits for it (`TG-READ-001`); with
    /// `FullTextMode::CurrentIndex` the current index is read as it is. Returns
    /// raw `ScoredDoc` results so callers (e.g. `HybridSearchEngine`) can handle
    /// value enrichment separately.
    ///
    /// # Errors
    ///
    /// The population this search waited for failed.
    pub(crate) async fn search_map(
        &self,
        map_name: &str,
        query_str: &str,
        options: &SearchOptions,
        mode: FullTextMode,
    ) -> anyhow::Result<Vec<ScoredDoc>> {
        if mode == FullTextMode::Populate {
            self.populations.ensure(map_name).await?;
        }
        let indexes = self.indexes.read();
        Ok(indexes
            .get(map_name)
            .map(|index| index.search(query_str, options))
            .unwrap_or_default())
    }

    async fn handle(&self, op: Operation) -> Result<OperationResponse, OperationError> {
        match op {
            Operation::Search { ctx: _, payload } => {
                let options = payload.options.unwrap_or_default();
                let resp_payload = match self
                    .execute_search(&payload.map_name, &payload.query, &options)
                    .await
                {
                    Ok(results) => SearchRespPayload {
                        request_id: payload.request_id,
                        total_count: u32::try_from(results.len()).unwrap_or(u32::MAX),
                        results,
                        error: None,
                    },
                    Err(_) => search_failed(payload.request_id),
                };
                Ok(OperationResponse::Message(Box::new(Message::SearchResp {
                    payload: resp_payload,
                })))
            }

            Operation::SearchSubscribe { ctx, payload } => {
                let connection_id = ctx.connection_id.ok_or_else(|| {
                    OperationError::Internal(anyhow::anyhow!(
                        "SearchSubscribe requires connection_id in OperationContext"
                    ))
                })?;
                let options = payload.options.unwrap_or_default();
                // Execute initial search. A failed population is answered as an
                // error and registers nothing: a subscription seeded from an
                // incomplete index would look complete to the client.
                let Ok(results) = self
                    .execute_search(&payload.map_name, &payload.query, &options)
                    .await
                else {
                    return Ok(OperationResponse::Message(Box::new(Message::SearchResp {
                        payload: search_failed(payload.subscription_id),
                    })));
                };
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
                Ok(OperationResponse::Message(Box::new(Message::SearchResp {
                    payload: resp_payload,
                })))
            }

            Operation::SearchUnsubscribe { ctx, payload } => Ok(handle_unsubscribe(
                &self.registry,
                ctx.connection_id,
                &payload.subscription_id,
            )),

            _ => Err(OperationError::WrongService),
        }
    }

    /// Handle a one-shot hybrid search request.
    ///
    /// Builds `HybridSearchParams` from the wire payload, delegates to
    /// `HybridSearchEngine::hybrid_search()`, and converts results to
    /// a `HybridSearchRespPayload`.
    async fn handle_hybrid_search(
        &self,
        _ctx: &OperationContext,
        payload: &HybridSearchPayload,
    ) -> Result<OperationResponse, OperationError> {
        let start = std::time::Instant::now();

        // Helper macro that returns an error HybridSearchRespPayload.
        macro_rules! error_resp {
            ($msg:expr) => {{
                return Ok(OperationResponse::Message(Box::new(
                    Message::HybridSearchResp {
                        payload: HybridSearchRespPayload {
                            request_id: payload.request_id.clone(),
                            results: vec![],
                            search_time_ms: elapsed_ms(start),
                            error: Some($msg.to_string()),
                        },
                    },
                )));
            }};
        }

        // 1. Resolve IndexRegistry for the target map.
        let Some(registry) = self.index_observer_factory.get_registry(&payload.map_name) else {
            error_resp!("index registry not found for map");
        };

        // 2. Decode query_vector from little-endian f32 bytes (if present).
        let query_vector_f32: Option<Vec<f32>> = if let Some(ref bytes) = payload.query_vector {
            if bytes.len() % 4 != 0 {
                error_resp!("query_vector length is not a multiple of 4");
            }
            let floats: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            Some(floats)
        } else {
            None
        };

        // 3. Convert wire SearchMethod to server-side SearchMethod.
        let methods: Vec<SearchMethod> = payload
            .methods
            .iter()
            .map(|m| match m {
                WireSearchMethod::Exact => SearchMethod::Exact,
                WireSearchMethod::FullText => SearchMethod::FullText,
                WireSearchMethod::Semantic => SearchMethod::Semantic,
            })
            .collect();

        // 4. Get the HybridSearchEngine (must be set before first hybrid search).
        let Some(engine) = self.hybrid_engine.get() else {
            error_resp!("hybrid search engine not configured");
        };

        let include_value = payload.include_value.unwrap_or(true);

        // 5. Build HybridSearchParams and run hybrid_search.
        let params = crate::service::domain::search::hybrid::HybridSearchParams {
            map_name: &payload.map_name,
            index_registry: &registry,
            query_text: &payload.query_text,
            query_vector: query_vector_f32.as_deref(),
            predicate: payload.predicate.as_ref(),
            methods: &methods,
            k: payload.k as usize,
            include_value,
        };

        let mut results = match engine.hybrid_search(params).await {
            Ok(r) => r,
            Err(e) => error_resp!(e),
        };

        // 9. Apply min_score post-filter AFTER hybrid_search() returns.
        if let Some(min_score) = payload.min_score {
            results.retain(|r| r.score >= min_score);
        }

        // 6. Convert Vec<HybridSearchResult> to Vec<HybridSearchResultEntry>.
        let entries: Vec<HybridSearchResultEntry> = results
            .into_iter()
            .map(|r| {
                // Convert server-side SearchMethod keys to wire SearchMethod keys.
                let method_scores = r
                    .method_scores
                    .into_iter()
                    .map(|(k, v)| {
                        let wire_k = match k {
                            SearchMethod::Exact => WireSearchMethod::Exact,
                            SearchMethod::FullText => WireSearchMethod::FullText,
                            SearchMethod::Semantic => WireSearchMethod::Semantic,
                        };
                        (wire_k, v)
                    })
                    .collect();
                HybridSearchResultEntry {
                    key: r.key,
                    score: r.score,
                    method_scores,
                    value: r.value,
                }
            })
            .collect();

        // 10. Return HybridSearchResp.
        Ok(OperationResponse::Message(Box::new(
            Message::HybridSearchResp {
                payload: HybridSearchRespPayload {
                    request_id: payload.request_id.clone(),
                    results: entries,
                    search_time_ms: elapsed_ms(start),
                    error: None,
                },
            },
        )))
    }

    /// Handle a hybrid search subscription request.
    ///
    /// Executes an initial hybrid search, registers the subscription, and
    /// returns the initial snapshot as a `HybridSearchRespPayload`.
    #[allow(clippy::too_many_lines)]
    async fn handle_hybrid_search_subscribe(
        &self,
        ctx: &OperationContext,
        payload: &HybridSearchSubPayload,
    ) -> Result<OperationResponse, OperationError> {
        let connection_id = ctx.connection_id.ok_or_else(|| {
            OperationError::Internal(anyhow::anyhow!(
                "HybridSearchSubscribe requires connection_id in OperationContext"
            ))
        })?;

        // Decode query vector.
        let query_vector_f32: Option<Vec<f32>> = if let Some(ref bytes) = payload.query_vector {
            if bytes.len() % 4 != 0 {
                return Ok(OperationResponse::Message(Box::new(
                    Message::HybridSearchResp {
                        payload: HybridSearchRespPayload {
                            request_id: payload.subscription_id.clone(),
                            results: vec![],
                            search_time_ms: 0,
                            error: Some("query_vector length is not a multiple of 4".to_string()),
                        },
                    },
                )));
            }
            Some(
                bytes
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect(),
            )
        } else {
            None
        };

        let methods: Vec<SearchMethod> = payload
            .methods
            .iter()
            .map(|m| match m {
                WireSearchMethod::Exact => SearchMethod::Exact,
                WireSearchMethod::FullText => SearchMethod::FullText,
                WireSearchMethod::Semantic => SearchMethod::Semantic,
            })
            .collect();

        let include_value = payload.include_value.unwrap_or(true);

        // Execute initial search snapshot.
        let start = std::time::Instant::now();
        let initial_resp = if let Some(engine) = self.hybrid_engine.get() {
            if let Some(registry) = self.index_observer_factory.get_registry(&payload.map_name) {
                let params = crate::service::domain::search::hybrid::HybridSearchParams {
                    map_name: &payload.map_name,
                    index_registry: &registry,
                    query_text: &payload.query_text,
                    query_vector: query_vector_f32.as_deref(),
                    predicate: payload.predicate.as_ref(),
                    methods: &methods,
                    k: payload.k as usize,
                    include_value,
                };
                match engine.hybrid_search(params).await {
                    Ok(mut results) => {
                        if let Some(min_score) = payload.min_score {
                            results.retain(|r| r.score >= min_score);
                        }
                        results
                    }
                    // A failed initial evaluation registers nothing: a
                    // subscription seeded empty would look complete.
                    Err(error) => {
                        return Ok(OperationResponse::Message(Box::new(
                            Message::HybridSearchResp {
                                payload: HybridSearchRespPayload {
                                    request_id: payload.subscription_id.clone(),
                                    results: vec![],
                                    search_time_ms: elapsed_ms(start),
                                    error: Some(error.to_string()),
                                },
                            },
                        )));
                    }
                }
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        // Build initial result entries and populate cache.
        let sub = HybridSearchSubscription::new(
            payload.subscription_id.clone(),
            connection_id,
            payload.map_name.clone(),
            payload.query_text.clone(),
            methods,
            payload.k as usize,
            query_vector_f32,
            payload.predicate.clone(),
            include_value,
            payload.min_score,
        );
        for r in &initial_resp {
            sub.current_results.insert(
                r.key.clone(),
                CachedHybridResult {
                    score: r.score,
                    method_scores: r.method_scores.clone(),
                    value: r.value.clone(),
                },
            );
        }
        self.hybrid_registry.register(sub);

        let entries: Vec<HybridSearchResultEntry> = initial_resp
            .into_iter()
            .map(|r| {
                let method_scores = r
                    .method_scores
                    .into_iter()
                    .map(|(k, v)| {
                        let wire_k = match k {
                            SearchMethod::Exact => WireSearchMethod::Exact,
                            SearchMethod::FullText => WireSearchMethod::FullText,
                            SearchMethod::Semantic => WireSearchMethod::Semantic,
                        };
                        (wire_k, v)
                    })
                    .collect();
                HybridSearchResultEntry {
                    key: r.key,
                    score: r.score,
                    method_scores,
                    value: r.value,
                }
            })
            .collect();

        Ok(OperationResponse::Message(Box::new(
            Message::HybridSearchResp {
                payload: HybridSearchRespPayload {
                    request_id: payload.subscription_id.clone(),
                    results: entries,
                    search_time_ms: elapsed_ms(start),
                    error: None,
                },
            },
        )))
    }

    /// Handle a hybrid search unsubscribe request.
    fn handle_hybrid_search_unsubscribe(
        &self,
        ctx: &OperationContext,
        payload: &HybridSearchUnsubPayload,
    ) -> OperationResponse {
        handle_unsubscribe(
            &self.hybrid_registry,
            ctx.connection_id,
            &payload.subscription_id,
        )
    }

    /// Re-evaluate all hybrid subscriptions for a map on data mutation.
    ///
    /// Compares new results against cached results and emits ENTER/UPDATE/LEAVE
    /// deltas to each subscriber's connection. Called by the hybrid notifier task
    /// spawned in `spawn_hybrid_notifier`.
    ///
    /// The `exact` legs of all the map's subscriptions share one store scan;
    /// when it fails, those subscriptions get no delta in this pass (the next
    /// write marks the map dirty again). The `fullText` legs read the current
    /// index and never start or join a population.
    #[allow(clippy::too_many_lines)]
    pub async fn notify_hybrid_subscriptions_for_map(&self, map_name: &str) {
        #[cfg(test)]
        self.notifier_passes.fetch_add(1, Ordering::SeqCst);
        let subs = self.hybrid_registry.get_subscriptions_for_map(map_name);
        if subs.is_empty() {
            return;
        }

        let Some(engine_ref) = self.hybrid_engine.get() else {
            return;
        };
        let engine = Arc::clone(engine_ref);

        let factory = Arc::clone(&self.index_observer_factory);
        let conn_reg = Arc::clone(&self.connection_registry);

        // One scan of the map for every `exact` subscription on it. Without a
        // registry such subscriptions are skipped below, so they add nothing.
        // Looked up once, so the scan below and the per-subscription choice
        // agree on which subscriptions take an exact list.
        let map_registry = factory.get_registry(map_name);
        let empty_registry =
            Arc::new(crate::service::domain::index::registry::IndexRegistry::new());
        let predicates: Vec<&topgun_core::messages::base::PredicateNode> = if map_registry.is_some()
        {
            subs.iter()
                .filter(|sub| sub.methods.contains(&SearchMethod::Exact))
                .filter_map(|sub| sub.predicate.as_ref())
                .collect()
        } else {
            Vec::new()
        };
        let mut exact_lists = if predicates.is_empty() {
            Some(Vec::new().into_iter())
        } else {
            match engine.exact_matches(map_name, &predicates).await {
                Ok(lists) => Some(lists.into_iter()),
                Err(error) => {
                    tracing::warn!(map = %map_name, error = %error, "hybrid live re-evaluation: store scan failed; no exact deltas in this pass");
                    None
                }
            }
        };

        for sub in &subs {
            // Use the registered IndexRegistry if available. For subscriptions that
            // only use FullText or Semantic methods, an empty registry suffices since
            // no Exact-match predicate evaluation is performed against the index.
            // Subscriptions that only use FullText or Semantic methods can use an
            // empty registry, since no Exact predicate is evaluated against it.
            let registry: &crate::service::domain::index::registry::IndexRegistry =
                if let Some(reg) = map_registry.as_ref() {
                    reg
                } else if sub.methods.contains(&SearchMethod::Exact) {
                    // Exact search without a registered index is skipped rather
                    // than answered with empty deltas.
                    continue;
                } else {
                    &empty_registry
                };

            let exact = if sub.methods.contains(&SearchMethod::Exact) {
                let Some(lists) = exact_lists.as_mut() else {
                    continue;
                };
                // Lists come in the order the predicates were collected above.
                Some(if sub.predicate.is_some() {
                    lists.next().unwrap_or_default()
                } else {
                    Vec::new()
                })
            } else {
                None
            };

            let params = crate::service::domain::search::hybrid::HybridSearchParams {
                map_name: &sub.map_name,
                index_registry: registry,
                query_text: &sub.query_text,
                query_vector: sub.query_vector.as_deref(),
                predicate: sub.predicate.as_ref(),
                methods: &sub.methods,
                k: sub.k,
                include_value: sub.include_value,
            };

            let Ok(mut new_results) = engine
                .hybrid_search_with_exact(params, exact, FullTextMode::CurrentIndex)
                .await
            else {
                continue;
            };

            if let Some(min_score) = sub.min_score {
                new_results.retain(|r| r.score >= min_score);
            }

            // Compute deltas: ENTER, UPDATE, LEAVE.
            let new_map: HashMap<String, &HybridSearchResult> =
                new_results.iter().map(|r| (r.key.clone(), r)).collect();

            // LEAVE: in cache but not in new results.
            let leaves: Vec<String> = sub
                .current_results
                .iter()
                .filter(|entry| !new_map.contains_key(entry.key()))
                .map(|entry| entry.key().clone())
                .collect();

            for key in &leaves {
                let method_scores = HashMap::new();
                let msg = Message::HybridSearchUpdate {
                    payload: HybridSearchUpdatePayload {
                        subscription_id: sub.subscription_id.clone(),
                        key: key.clone(),
                        score: 0.0,
                        method_scores,
                        value: None,
                        change_type: ChangeEventType::LEAVE,
                    },
                };
                send_to_connection(&conn_reg, sub.connection_id, &msg);
                sub.current_results.remove(key);
            }

            // ENTER and UPDATE.
            for result in &new_results {
                let wire_scores: HashMap<WireSearchMethod, f64> = result
                    .method_scores
                    .iter()
                    .map(|(k, v)| {
                        let wk = match k {
                            SearchMethod::Exact => WireSearchMethod::Exact,
                            SearchMethod::FullText => WireSearchMethod::FullText,
                            SearchMethod::Semantic => WireSearchMethod::Semantic,
                        };
                        (wk, *v)
                    })
                    .collect();

                let change_type = if sub.current_results.contains_key(&result.key) {
                    ChangeEventType::UPDATE
                } else {
                    ChangeEventType::ENTER
                };

                let msg = Message::HybridSearchUpdate {
                    payload: HybridSearchUpdatePayload {
                        subscription_id: sub.subscription_id.clone(),
                        key: result.key.clone(),
                        score: result.score,
                        method_scores: wire_scores,
                        value: result.value.clone(),
                        change_type,
                    },
                };
                send_to_connection(&conn_reg, sub.connection_id, &msg);

                // Update cache.
                sub.current_results.insert(
                    result.key.clone(),
                    CachedHybridResult {
                        score: result.score,
                        method_scores: result.method_scores.clone(),
                        value: result.value.clone(),
                    },
                );
            }
        }
    }

    /// Returns a clone of the inner `Arc<SearchRegistry>`.
    ///
    /// Used by production wiring in `build_app()` so `AppState` can hold the
    /// text-search registry directly and release standing subscriptions at
    /// disconnect time without going through the service (avoids racing with
    /// service shutdown).
    #[must_use]
    pub fn search_registry_arc(&self) -> Arc<SearchRegistry> {
        Arc::clone(&self.registry)
    }

    /// Returns a clone of the inner `Arc<HybridSearchRegistry>`.
    ///
    /// Used by production wiring in `build_app()` so `AppState` can hold the
    /// hybrid-search registry directly and release standing subscriptions at
    /// disconnect time without going through the service (avoids racing with
    /// service shutdown).
    #[must_use]
    pub fn hybrid_search_registry_arc(&self) -> Arc<HybridSearchRegistry> {
        Arc::clone(&self.hybrid_registry)
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
        // Clear all indexes, subscriptions, and population flags.
        let mut indexes = self.indexes.write();
        for index in indexes.values_mut() {
            index.clear();
        }
        indexes.clear();
        self.registry.subscriptions.clear();
        self.hybrid_registry.subscriptions.clear();
        self.needs_population.clear();
        self.populations.states.lock().clear();
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
    type Future = Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, op: Operation) -> Self::Future {
        let svc = Arc::clone(self);
        let service_name = op.ctx().service_name;
        let call_id = op.ctx().call_id;
        let caller_origin = format!("{:?}", op.ctx().caller_origin);

        let span = tracing::info_span!(
            "domain_op",
            service = service_name,
            call_id = call_id,
            caller_origin = %caller_origin,
        );

        Box::pin(
            async move {
                match op {
                    Operation::HybridSearch {
                        ref ctx,
                        ref payload,
                    } => svc.handle_hybrid_search(ctx, payload).await,
                    Operation::HybridSearchSubscribe {
                        ref ctx,
                        ref payload,
                    } => svc.handle_hybrid_search_subscribe(ctx, payload).await,
                    Operation::HybridSearchUnsubscribe {
                        ref ctx,
                        ref payload,
                    } => Ok(svc.handle_hybrid_search_unsubscribe(ctx, payload)),
                    _ => svc.handle(op).await,
                }
            }
            .instrument(span),
        )
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
    use topgun_core::messages::search::{
        SearchOptions, SearchPayload, SearchSubPayload, SearchUnsubPayload,
    };
    use tower::ServiceExt;

    use super::*;
    use crate::network::connection::ConnectionRegistry;
    use crate::service::operation::{service_names, Operation, OperationContext, OperationError};
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
        let hybrid_reg = Arc::new(HybridSearchRegistry::new());
        let indexes = Arc::new(RwLock::new(HashMap::new()));
        let conn_reg = Arc::new(ConnectionRegistry::new());
        let store_factory = make_factory();
        let needs_population = Arc::new(DashMap::new());
        let index_factory = Arc::new(crate::service::domain::index::IndexObserverFactory::new());
        Arc::new(SearchService::new(
            reg,
            hybrid_reg,
            indexes,
            store_factory,
            conn_reg,
            needs_population,
            index_factory,
        ))
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

    fn make_unsubscribe_op(connection: ConnectionId, subscription_id: &str) -> Operation {
        let mut ctx = OperationContext::new(
            44,
            service_names::SEARCH,
            Timestamp {
                millis: 0,
                counter: 0,
                node_id: "test".to_string(),
            },
            5000,
        );
        ctx.connection_id = Some(connection);
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

        let opts = SearchOptions {
            limit: Some(3),
            ..Default::default()
        };
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

        assert!(reg.unregister(ConnectionId(7), "sub-1").is_none());
        let removed = reg.unregister(ConnectionId(42), "sub-1");
        assert!(removed.is_some());
        assert!(reg.get_subscriptions_for_map("my-map").is_empty());
    }

    #[test]
    fn search_registry_unregister_by_connection() {
        let reg = SearchRegistry::new();
        for i in 0..3u64 {
            let sub = SearchSubscription::new(
                format!("sub-{i}"),
                if i < 2 {
                    ConnectionId(10)
                } else {
                    ConnectionId(20)
                },
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
        let hybrid_reg = Arc::new(HybridSearchRegistry::new());
        let indexes = Arc::new(RwLock::new(HashMap::new()));
        let conn_reg = Arc::new(ConnectionRegistry::new());
        let store_factory = make_factory();
        let needs_population = Arc::new(DashMap::new());
        let index_factory = Arc::new(crate::service::domain::index::IndexObserverFactory::new());
        let svc = Arc::new(SearchService::new(
            Arc::clone(&reg),
            hybrid_reg,
            indexes,
            store_factory,
            conn_reg,
            needs_population,
            index_factory,
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
        let hybrid_reg = Arc::new(HybridSearchRegistry::new());
        let indexes = Arc::new(RwLock::new(HashMap::new()));
        let conn_reg = Arc::new(ConnectionRegistry::new());
        let store_factory = make_factory();
        let needs_population = Arc::new(DashMap::new());
        let index_factory = Arc::new(crate::service::domain::index::IndexObserverFactory::new());
        let svc = Arc::new(SearchService::new(
            Arc::clone(&reg),
            hybrid_reg,
            indexes,
            store_factory,
            conn_reg,
            needs_population,
            index_factory,
        ));

        svc.clone()
            .oneshot(make_subscribe_op("my-map", "hello"))
            .await
            .unwrap();
        assert_eq!(reg.get_subscriptions_for_map("my-map").len(), 1);

        let resp = svc
            .clone()
            .oneshot(make_unsubscribe_op(ConnectionId(1), "sub-1"))
            .await
            .unwrap();
        assert!(reg.get_subscriptions_for_map("my-map").is_empty());
        // An unsubscribe is not an op batch: an acknowledgement of it would name
        // no operation the client sent, so it must produce no frame.
        assert!(
            matches!(resp, OperationResponse::Empty),
            "an unsubscribe must be answered with no frame, got {resp:?}"
        );
    }

    #[tokio::test]
    async fn search_service_returns_wrong_service_for_non_search_ops() {
        let svc = make_service();
        let err = svc
            .oneshot(make_op_gc(service_names::SEARCH))
            .await
            .unwrap_err();
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
        let hybrid_reg = Arc::new(HybridSearchRegistry::new());
        let conn_reg = Arc::new(ConnectionRegistry::new());
        let store_factory = make_factory();
        let needs_population = Arc::new(DashMap::new());
        let index_factory = Arc::new(crate::service::domain::index::IndexObserverFactory::new());
        let svc = Arc::new(SearchService::new(
            reg,
            hybrid_reg,
            Arc::clone(&indexes),
            store_factory,
            conn_reg,
            needs_population,
            index_factory,
        ));

        assert!(indexes.read().is_empty(), "no index before first search");
        svc.clone()
            .oneshot(make_search_op("lazy-map", "hello"))
            .await
            .unwrap();
        assert!(
            indexes.read().contains_key("lazy-map"),
            "index created after search"
        );
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

    // --- HybridSearchRegistry tests ---

    #[test]
    fn hybrid_registry_register_and_get_by_map() {
        let reg = HybridSearchRegistry::new();
        let sub = HybridSearchSubscription::new(
            "hsub-1".to_string(),
            ConnectionId(99),
            "my-map".to_string(),
            "hello".to_string(),
            vec![SearchMethod::FullText],
            10,
            None,
            None,
            true,
            None,
        );
        reg.register(sub);

        let subs = reg.get_subscriptions_for_map("my-map");
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].subscription_id, "hsub-1");
        assert!(reg.get_subscriptions_for_map("other-map").is_empty());
    }

    #[test]
    fn hybrid_registry_unregister_removes_subscription() {
        let reg = HybridSearchRegistry::new();
        let sub = HybridSearchSubscription::new(
            "hsub-1".to_string(),
            ConnectionId(99),
            "my-map".to_string(),
            "hello".to_string(),
            vec![SearchMethod::Exact],
            5,
            None,
            None,
            true,
            None,
        );
        reg.register(sub);
        assert_eq!(reg.get_subscriptions_for_map("my-map").len(), 1);

        let removed = reg.unregister(ConnectionId(99), "hsub-1");
        assert!(removed.is_some());
        assert!(reg.get_subscriptions_for_map("my-map").is_empty());

        // Second unregister of the same ID returns None.
        let removed2 = reg.unregister(ConnectionId(99), "hsub-1");
        assert!(removed2.is_none());
    }

    // --- Hybrid search handler tests ---

    fn make_hybrid_search_op(map_name: &str, query_text: &str) -> Operation {
        let mut ctx = OperationContext::new(
            50,
            service_names::SEARCH,
            topgun_core::hlc::Timestamp {
                millis: 0,
                counter: 0,
                node_id: "test".to_string(),
            },
            5000,
        );
        ctx.connection_id = Some(ConnectionId(1));
        Operation::HybridSearch {
            ctx,
            payload: HybridSearchPayload {
                request_id: "req-h1".to_string(),
                map_name: map_name.to_string(),
                query_text: query_text.to_string(),
                methods: vec![WireSearchMethod::FullText],
                k: 5,
                query_vector: None,
                predicate: None,
                include_value: None,
                min_score: None,
            },
        }
    }

    fn make_hybrid_sub_op(map_name: &str, query_text: &str, sub_id: &str) -> Operation {
        let mut ctx = OperationContext::new(
            51,
            service_names::SEARCH,
            topgun_core::hlc::Timestamp {
                millis: 0,
                counter: 0,
                node_id: "test".to_string(),
            },
            5000,
        );
        ctx.connection_id = Some(ConnectionId(1));
        Operation::HybridSearchSubscribe {
            ctx,
            payload: HybridSearchSubPayload {
                subscription_id: sub_id.to_string(),
                map_name: map_name.to_string(),
                query_text: query_text.to_string(),
                methods: vec![WireSearchMethod::FullText],
                k: 5,
                query_vector: None,
                predicate: None,
                include_value: None,
                min_score: None,
            },
        }
    }

    fn make_hybrid_unsub_op(connection: ConnectionId, sub_id: &str) -> Operation {
        let mut ctx = OperationContext::new(
            52,
            service_names::SEARCH,
            topgun_core::hlc::Timestamp {
                millis: 0,
                counter: 0,
                node_id: "test".to_string(),
            },
            5000,
        );
        ctx.connection_id = Some(connection);
        Operation::HybridSearchUnsubscribe {
            ctx,
            payload: HybridSearchUnsubPayload {
                subscription_id: sub_id.to_string(),
            },
        }
    }

    #[tokio::test]
    async fn hybrid_search_without_engine_returns_error_resp() {
        // When hybrid_engine is not configured, handler returns HybridSearchResp with error.
        let svc = make_service();
        let op = make_hybrid_search_op("my-map", "hello");
        let resp = svc.oneshot(op).await.unwrap();
        match resp {
            OperationResponse::Message(msg) => {
                if let Message::HybridSearchResp { payload } = *msg {
                    assert!(
                        payload.error.is_some(),
                        "expected error when engine not set"
                    );
                } else {
                    panic!("expected HybridSearchResp, got different message");
                }
            }
            other => panic!("expected OperationResponse::Message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn hybrid_search_subscribe_registers_subscription_returns_resp() {
        // Without engine, subscribe still registers the subscription and returns
        // an empty-results HybridSearchResp (initial snapshot with no results).
        let svc = make_service();
        let op = make_hybrid_sub_op("my-map", "hello", "sub-h1");
        let resp = svc.clone().oneshot(op).await.unwrap();

        // Expect HybridSearchResp (initial snapshot).
        match resp {
            OperationResponse::Message(msg) => {
                assert!(
                    matches!(*msg, Message::HybridSearchResp { .. }),
                    "expected HybridSearchResp initial snapshot"
                );
            }
            other => panic!("expected Message response, got {other:?}"),
        }

        // Subscription must be registered.
        let subs = svc.hybrid_registry.get_subscriptions_for_map("my-map");
        assert_eq!(subs.len(), 1, "subscription should be registered");
        assert_eq!(subs[0].subscription_id, "sub-h1");
    }

    #[tokio::test]
    async fn hybrid_search_unsubscribe_removes_subscription_and_answers_with_no_frame() {
        let svc = make_service();

        // Subscribe first.
        svc.clone()
            .oneshot(make_hybrid_sub_op("my-map", "hello", "sub-h1"))
            .await
            .unwrap();
        assert_eq!(
            svc.hybrid_registry
                .get_subscriptions_for_map("my-map")
                .len(),
            1
        );

        // Unsubscribe.
        let resp = svc
            .clone()
            .oneshot(make_hybrid_unsub_op(ConnectionId(1), "sub-h1"))
            .await
            .unwrap();
        assert!(
            matches!(resp, OperationResponse::Empty),
            "a hybrid unsubscribe must be answered with no frame, got {resp:?}"
        );

        // Subscription removed.
        assert!(svc
            .hybrid_registry
            .get_subscriptions_for_map("my-map")
            .is_empty());
    }

    /// A search or hybrid-search subscription belongs to the connection that
    /// made it: another connection choosing the same id gets a subscription of
    /// its own, and an unsubscribe removes only the caller's.
    #[tokio::test]
    async fn search_subscriptions_are_owned_by_the_connection_that_made_them() {
        let svc = make_service();
        let (a, b) = (ConnectionId(1), ConnectionId(2));
        let as_connection = |op: Operation, connection: ConnectionId| match op {
            Operation::SearchSubscribe { mut ctx, payload } => {
                ctx.connection_id = Some(connection);
                Operation::SearchSubscribe { ctx, payload }
            }
            Operation::HybridSearchSubscribe { mut ctx, payload } => {
                ctx.connection_id = Some(connection);
                Operation::HybridSearchSubscribe { ctx, payload }
            }
            other => panic!("unexpected op {other:?}"),
        };
        let owners = |svc: &SearchService| {
            let mut text: Vec<ConnectionId> = svc
                .registry
                .get_subscriptions_for_map("my-map")
                .iter()
                .map(|sub| sub.connection_id)
                .collect();
            let mut hybrid: Vec<ConnectionId> = svc
                .hybrid_registry
                .get_subscriptions_for_map("my-map")
                .iter()
                .map(|sub| sub.connection_id)
                .collect();
            text.sort_unstable_by_key(|c| c.0);
            hybrid.sort_unstable_by_key(|c| c.0);
            (text, hybrid)
        };

        for connection in [a, b] {
            svc.clone()
                .oneshot(as_connection(
                    make_subscribe_op("my-map", "hello"),
                    connection,
                ))
                .await
                .unwrap();
            svc.clone()
                .oneshot(as_connection(
                    make_hybrid_sub_op("my-map", "hello", "sub-h1"),
                    connection,
                ))
                .await
                .unwrap();
        }
        assert_eq!(owners(&svc), (vec![a, b], vec![a, b]));

        svc.clone()
            .oneshot(make_unsubscribe_op(b, "sub-1"))
            .await
            .unwrap();
        svc.clone()
            .oneshot(make_hybrid_unsub_op(b, "sub-h1"))
            .await
            .unwrap();
        assert_eq!(owners(&svc), (vec![a], vec![a]));

        // A second unsubscribe by B finds nothing of its own and leaves A's.
        svc.clone()
            .oneshot(make_unsubscribe_op(b, "sub-1"))
            .await
            .unwrap();
        svc.clone()
            .oneshot(make_hybrid_unsub_op(b, "sub-h1"))
            .await
            .unwrap();
        assert_eq!(owners(&svc), (vec![a], vec![a]));
    }

    #[tokio::test]
    async fn hybrid_search_unsubscribe_of_an_unknown_subscription_answers_with_no_frame() {
        // Unsubscribing a subscription the server does not hold is a no-op, and a
        // no-op has even less to acknowledge than a real removal.
        let svc = make_service();
        let resp = svc
            .oneshot(make_hybrid_unsub_op(ConnectionId(1), "no-such-sub"))
            .await
            .unwrap();
        assert!(
            matches!(resp, OperationResponse::Empty),
            "unsubscribing an unknown subscription must be answered with no frame, got {resp:?}"
        );
    }

    // --- Hybrid delta delivery tests (AC1, AC2, AC3) ---

    /// Builds a `SearchService` backed by a real `ConnectionRegistry` with one
    /// registered test connection. Returns the service, a receiver for that
    /// connection's outbound messages, and the `ConnectionId`.
    fn make_service_with_connection() -> (
        Arc<SearchService>,
        tokio::sync::mpsc::Receiver<OutboundMessage>,
        ConnectionId,
    ) {
        use crate::network::config::ConnectionConfig;
        use crate::network::connection::ConnectionKind;

        let reg = Arc::new(SearchRegistry::new());
        let hybrid_reg = Arc::new(HybridSearchRegistry::new());
        let indexes = Arc::new(RwLock::new(HashMap::new()));
        let conn_reg = Arc::new(ConnectionRegistry::new());
        let store_factory = make_factory();
        let needs_population = Arc::new(DashMap::new());
        let index_factory = Arc::new(crate::service::domain::index::IndexObserverFactory::new());

        let (handle, rx) = conn_reg.register(ConnectionKind::Client, &ConnectionConfig::default());
        let conn_id = handle.id;

        let svc = Arc::new(SearchService::new(
            reg,
            hybrid_reg,
            indexes,
            store_factory,
            Arc::clone(&conn_reg),
            needs_population,
            index_factory,
        ));
        (svc, rx, conn_id)
    }

    /// Wires a `HybridSearchEngine` (FullText-only) onto the service, then
    /// populates the tantivy index with the given documents.
    fn wire_engine_and_index(
        svc: &Arc<SearchService>,
        map_name: &str,
        docs: &[(&str, &str)], // key -> text
    ) {
        let store_factory = make_factory();
        let engine = HybridSearchEngine::new(Arc::clone(svc), store_factory, None);
        svc.set_hybrid_engine(Arc::new(engine));

        // Populate the tantivy index directly.
        let mut indexes = svc.indexes.write();
        let index = indexes.entry(map_name.to_owned()).or_default();
        for (key, text) in docs {
            let val = make_rmpv_map(&[("text", text)]);
            index.index_document(key, &val);
        }
        index.commit();
    }

    /// Decodes a `HybridSearchUpdate` payload from a received `OutboundMessage`.
    fn decode_hybrid_update(msg: OutboundMessage) -> Option<HybridSearchUpdatePayload> {
        if let OutboundMessage::Binary(bytes) = msg {
            if let Ok(Message::HybridSearchUpdate { payload }) =
                rmp_serde::from_slice::<Message>(&bytes)
            {
                return Some(payload);
            }
        }
        None
    }

    #[tokio::test]
    async fn hybrid_delta_enter_delivered_when_record_matches_new_subscription() {
        // AC1: A record that matches the query after indexing delivers ENTER.
        let (svc, mut rx, conn_id) = make_service_with_connection();
        wire_engine_and_index(&svc, "map1", &[("k1", "hello world")]);

        // Register subscription with empty initial cache (k1 not yet in cache).
        let sub = HybridSearchSubscription::new(
            "hsub-ac1".to_string(),
            conn_id,
            "map1".to_string(),
            "hello".to_string(),
            vec![SearchMethod::FullText],
            10,
            None,
            None,
            false,
            None,
        );
        svc.hybrid_registry.register(sub);

        // Trigger notification (simulate batch processor sending map name).
        svc.notify_hybrid_subscriptions_for_map("map1").await;

        // Expect ENTER for k1.
        let msg = rx.try_recv().expect("expected HybridSearchUpdate message");
        let payload = decode_hybrid_update(msg).expect("expected HybridSearchUpdate payload");
        assert_eq!(payload.key, "k1", "wrong key in ENTER");
        assert_eq!(
            payload.change_type,
            ChangeEventType::ENTER,
            "expected ENTER change_type"
        );
    }

    #[tokio::test]
    async fn hybrid_delta_leave_delivered_when_record_removed_from_index() {
        // AC2: A key in cache that no longer matches delivers LEAVE.
        let (svc, mut rx, conn_id) = make_service_with_connection();
        // Index is empty — k1 will not be found in search results.
        wire_engine_and_index(&svc, "map2", &[]);

        // Subscription with k1 already in cache.
        let sub = HybridSearchSubscription::new(
            "hsub-ac2".to_string(),
            conn_id,
            "map2".to_string(),
            "hello".to_string(),
            vec![SearchMethod::FullText],
            10,
            None,
            None,
            false,
            None,
        );
        sub.current_results.insert(
            "k1".to_string(),
            CachedHybridResult {
                score: 1.0,
                method_scores: HashMap::new(),
                value: None,
            },
        );
        svc.hybrid_registry.register(sub);

        // Trigger notification — k1 won't be found in empty index.
        svc.notify_hybrid_subscriptions_for_map("map2").await;

        // Expect LEAVE for k1.
        let msg = rx.try_recv().expect("expected HybridSearchUpdate message");
        let payload = decode_hybrid_update(msg).expect("expected HybridSearchUpdate payload");
        assert_eq!(payload.key, "k1", "wrong key in LEAVE");
        assert_eq!(
            payload.change_type,
            ChangeEventType::LEAVE,
            "expected LEAVE change_type"
        );
    }

    #[tokio::test]
    async fn hybrid_delta_update_delivered_when_record_still_matches_but_score_changes() {
        // AC3: A key already in cache that still matches delivers UPDATE.
        let (svc, mut rx, conn_id) = make_service_with_connection();
        wire_engine_and_index(&svc, "map3", &[("k1", "hello world rust")]);

        // Subscription with k1 already in cache (from a prior notification).
        let sub = HybridSearchSubscription::new(
            "hsub-ac3".to_string(),
            conn_id,
            "map3".to_string(),
            "hello".to_string(),
            vec![SearchMethod::FullText],
            10,
            None,
            None,
            false,
            None,
        );
        sub.current_results.insert(
            "k1".to_string(),
            CachedHybridResult {
                score: 0.5,
                method_scores: HashMap::new(),
                value: None,
            },
        );
        svc.hybrid_registry.register(sub);

        // Trigger notification — k1 is still in the index and matches "hello".
        svc.notify_hybrid_subscriptions_for_map("map3").await;

        // Expect UPDATE for k1 (already cached, still matches).
        let msg = rx.try_recv().expect("expected HybridSearchUpdate message");
        let payload = decode_hybrid_update(msg).expect("expected HybridSearchUpdate payload");
        assert_eq!(payload.key, "k1", "wrong key in UPDATE");
        assert_eq!(
            payload.change_type,
            ChangeEventType::UPDATE,
            "expected UPDATE change_type"
        );
    }

    // --- TG-READ-001: population, notifier passes and failures ---

    mod stored_read_tests {
        use std::collections::BTreeMap;
        use std::sync::OnceLock;
        use std::time::Duration;

        use topgun_core::hash_to_partition;
        use topgun_core::messages::base::{PredicateNode, PredicateOp};
        use topgun_core::types::Value;
        use tower::Service as _;

        use super::*;
        use crate::network::config::ConnectionConfig;
        use crate::network::connection::ConnectionKind;
        use crate::service::domain::index::IndexObserverFactory;
        use crate::storage::factory::test_store::TestStore;
        use crate::storage::factory::ObserverFactory;
        use crate::storage::key_writer::KeyWriteToken;
        use crate::storage::record_store::{CallerProvenance, ExpiryPolicy};

        const M: &str = "m";
        const TERM: &str = "alpha";

        /// Mirrors the binary's `SearchObserverFactory::create_observer`: the
        /// observer's link is attached to the service before the store serves
        /// a write.
        struct TestSearchObservers {
            registry: Arc<SearchRegistry>,
            hybrid_registry: Arc<HybridSearchRegistry>,
            indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>>,
            connection_registry: Arc<ConnectionRegistry>,
            needs_population: Arc<DashMap<String, AtomicBool>>,
            svc: OnceLock<Arc<SearchService>>,
        }

        impl ObserverFactory for TestSearchObservers {
            fn create_observer(
                &self,
                map_name: &str,
                _partition_id: u32,
            ) -> Option<Arc<dyn MutationObserver>> {
                let (observer, link) = SearchMutationObserver::new(
                    map_name.to_string(),
                    Arc::clone(&self.registry),
                    Arc::clone(&self.hybrid_registry),
                    Arc::clone(&self.indexes),
                    Arc::clone(&self.connection_registry),
                    SearchConfig {
                        batch_interval_ms: 16,
                        batch_flush_threshold: 100,
                    },
                    Arc::clone(&self.needs_population),
                );
                if let Some(svc) = self.svc.get() {
                    svc.spawn_hybrid_notifier(link);
                }
                Some(Arc::new(observer))
            }
        }

        struct Harness {
            svc: Arc<SearchService>,
            factory: Arc<RecordStoreFactory>,
            store: Arc<TestStore>,
            index_factory: Arc<IndexObserverFactory>,
            rx: tokio::sync::mpsc::Receiver<OutboundMessage>,
            conn: ConnectionId,
        }

        fn harness() -> Harness {
            let store = Arc::new(TestStore::new(1000));
            let registry = Arc::new(SearchRegistry::new());
            let hybrid_registry = Arc::new(HybridSearchRegistry::new());
            let indexes = Arc::new(RwLock::new(HashMap::new()));
            let connection_registry = Arc::new(ConnectionRegistry::new());
            let needs_population = Arc::new(DashMap::new());
            let observers = Arc::new(TestSearchObservers {
                registry: Arc::clone(&registry),
                hybrid_registry: Arc::clone(&hybrid_registry),
                indexes: Arc::clone(&indexes),
                connection_registry: Arc::clone(&connection_registry),
                needs_population: Arc::clone(&needs_population),
                svc: OnceLock::new(),
            });
            let factory = Arc::new(
                RecordStoreFactory::new(
                    StorageConfig::default(),
                    Arc::clone(&store) as Arc<dyn crate::storage::MapDataStore>,
                    Vec::new(),
                )
                .with_observer_factories(vec![Arc::clone(&observers) as Arc<dyn ObserverFactory>]),
            );
            let index_factory = Arc::new(IndexObserverFactory::new());
            let svc = Arc::new(SearchService::new(
                registry,
                hybrid_registry,
                indexes,
                Arc::clone(&factory),
                Arc::clone(&connection_registry),
                needs_population,
                Arc::clone(&index_factory),
            ));
            let _ = observers.svc.set(Arc::clone(&svc));
            svc.set_hybrid_engine(Arc::new(HybridSearchEngine::new(
                Arc::clone(&svc),
                Arc::clone(&factory),
                None,
            )));
            let (handle, rx) =
                connection_registry.register(ConnectionKind::Client, &ConnectionConfig::default());
            Harness {
                svc,
                factory,
                store,
                index_factory,
                rx,
                conn: handle.id,
            }
        }

        fn doc(text: &str) -> RecordValue {
            RecordValue::Lww {
                value: Value::Map(BTreeMap::from([
                    ("text".to_string(), Value::String(text.to_string())),
                    ("status".to_string(), Value::String("open".to_string())),
                ])),
                timestamp: Timestamp {
                    millis: 1_000,
                    counter: 0,
                    node_id: "n".to_string(),
                },
            }
        }

        /// Stores `n` rows holding `TERM` out of band: none is resident.
        fn store_rows(h: &Harness, n: usize) {
            for i in 0..n {
                h.store
                    .put_row(M, &format!("s{i}"), doc(&format!("{TERM} stored {i}")));
            }
        }

        /// A write through the record store: engine put, observer callback,
        /// data-store add.
        async fn write(h: &Harness, key: &str, text: &str) {
            let store = h.factory.get_or_create(M, hash_to_partition(key));
            store
                .put(
                    &KeyWriteToken::for_test(M, key),
                    key,
                    doc(text),
                    ExpiryPolicy::NONE,
                    CallerProvenance::Client,
                )
                .await
                .unwrap();
        }

        async fn call(svc: &Arc<SearchService>, op: Operation) -> Message {
            let mut svc = Arc::clone(svc);
            match svc.call(op).await.expect("the search service answers") {
                OperationResponse::Message(msg) => *msg,
                other => panic!("unexpected response: {other:?}"),
            }
        }

        async fn search(svc: &Arc<SearchService>) -> SearchRespPayload {
            match call(svc, make_search_op(M, TERM)).await {
                Message::SearchResp { payload } => payload,
                other => panic!("unexpected message: {other:?}"),
            }
        }

        fn keys(payload: &SearchRespPayload) -> Vec<String> {
            let mut keys: Vec<String> = payload.results.iter().map(|r| r.key.clone()).collect();
            keys.sort();
            keys
        }

        fn hybrid_resp(msg: Message) -> HybridSearchRespPayload {
            match msg {
                Message::HybridSearchResp { payload } => payload,
                other => panic!("unexpected message: {other:?}"),
            }
        }

        fn status_open() -> PredicateNode {
            PredicateNode {
                op: PredicateOp::Eq,
                attribute: Some("status".to_string()),
                value: Some(rmpv::Value::String("open".into())),
                ..PredicateNode::default()
            }
        }

        fn exact_sub_op(sub_id: &str) -> Operation {
            let mut op = make_hybrid_sub_op(M, "", sub_id);
            if let Operation::HybridSearchSubscribe { payload, .. } = &mut op {
                payload.methods = vec![WireSearchMethod::Exact];
                payload.predicate = Some(status_open());
            }
            op
        }

        async fn eventually(what: &str, mut cond: impl FnMut() -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            while !cond() {
                assert!(std::time::Instant::now() < deadline, "timed out: {what}");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }

        fn old_index_has(h: &Harness, key: &str) -> bool {
            h.svc
                .indexes
                .read()
                .get(M)
                .is_some_and(|index| index.score_single_document(key, TERM).is_some())
        }

        /// AC18(a): a population survives the search that started it being
        /// dropped, records the map as populated, and serves the next search
        /// without a further scan.
        #[tokio::test(flavor = "multi_thread")]
        async fn tg_read_001_population_survives_a_dropped_search() {
            let h = harness();
            store_rows(&h, 3);
            h.store.close_gate();
            let svc = Arc::clone(&h.svc);
            let dropped = tokio::spawn(async move { search(&svc).await });
            eventually("the population's scan parks", || h.store.parked() == 1).await;
            dropped.abort();
            let _ = dropped.await;
            h.store.open_gate();

            eventually("the population ends", || !h.svc.populations.in_flight(M)).await;
            assert!(
                h.svc.populations.is_populated(M),
                "the population must finish in its own task and record the map"
            );
            assert_eq!(h.store.scans(M), 1);

            let resp = search(&h.svc).await;
            assert_eq!(keys(&resp), vec!["s0", "s1", "s2"]);
            assert_eq!(
                h.store.scans(M),
                1,
                "a populated map is searched without a scan"
            );
        }

        /// AC18(b): a write skipped for lack of subscriptions while the scan is
        /// parked leaves the map "needs population"; the next search returns it.
        #[tokio::test(flavor = "multi_thread")]
        async fn tg_read_001_a_skipped_write_during_population_blocks_populated() {
            let h = harness();
            store_rows(&h, 2);
            h.store.close_gate();
            let svc = Arc::clone(&h.svc);
            let first = tokio::spawn(async move { search(&svc).await });
            eventually("the population's scan parks", || h.store.parked() == 1).await;
            write(&h, "x", &format!("{TERM} written while parked")).await;
            h.store.open_gate();
            let _ = first.await.unwrap();

            eventually("the population ends", || !h.svc.populations.in_flight(M)).await;
            assert!(
                !h.svc.populations.is_populated(M),
                "a population a write overlapped must not record the map as populated"
            );
            let resp = search(&h.svc).await;
            assert!(keys(&resp).contains(&"x".to_string()), "{resp:?}");
        }

        /// AC18(c): with a text subscription the overlapping write is enqueued
        /// and applied to the old index before the swap; the swap must not
        /// leave the map recorded as populated without it.
        #[tokio::test(flavor = "multi_thread")]
        async fn tg_read_001_an_enqueued_write_during_population_blocks_populated() {
            let h = harness();
            store_rows(&h, 2);
            match call(&h.svc, make_subscribe_op(M, TERM)).await {
                Message::SearchResp { payload } => assert!(payload.error.is_none()),
                other => panic!("unexpected message: {other:?}"),
            }
            assert!(h.svc.registry.has_subscriptions_for_map(M));
            h.svc
                .needs_population
                .entry(M.to_string())
                .or_insert_with(|| AtomicBool::new(false))
                .store(true, Ordering::Release);

            h.store.close_gate();
            let svc = Arc::clone(&h.svc);
            let first = tokio::spawn(async move { search(&svc).await });
            eventually("the population's scan parks", || h.store.parked() == 1).await;
            write(&h, "x", &format!("{TERM} written while parked")).await;
            eventually("the batch processor applies x to the old index", || {
                old_index_has(&h, "x")
            })
            .await;
            h.store.open_gate();
            let _ = first.await.unwrap();

            eventually("the population ends", || !h.svc.populations.in_flight(M)).await;
            assert!(!h.svc.populations.is_populated(M));
            let resp = search(&h.svc).await;
            assert!(keys(&resp).contains(&"x".to_string()), "{resp:?}");
        }

        /// AC19: while the notifier's scan is parked, five further dirty
        /// inserts for the map cause exactly one further scan, shared by both
        /// exact subscriptions.
        #[tokio::test(flavor = "multi_thread")]
        async fn tg_read_001_notifier_scans_each_dirty_map_once_per_pass() {
            let h = harness();
            store_rows(&h, 2);
            h.index_factory.register_map(M);
            for id in ["e1", "e2"] {
                h.svc
                    .hybrid_registry
                    .register(HybridSearchSubscription::new(
                        id.to_string(),
                        h.conn,
                        M.to_string(),
                        String::new(),
                        vec![SearchMethod::Exact],
                        10,
                        None,
                        Some(status_open()),
                        false,
                        None,
                    ));
            }

            h.store.close_gate();
            write(&h, "w", "trigger").await;
            eventually("the notifier's scan parks", || h.store.parked() == 1).await;
            for _ in 0..5 {
                h.svc.shared.mark_dirty(M);
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            h.store.open_gate();

            eventually("the further scan runs", || h.store.scans(M) >= 2).await;
            tokio::time::sleep(Duration::from_millis(300)).await;
            assert_eq!(
                h.store.scans(M),
                2,
                "one parked scan and exactly one further"
            );
        }

        /// AC21(a): a failed population is answered as an error by every
        /// request path, registers no subscription, leaves the map "needs
        /// population", and the healed store serves the next search.
        #[tokio::test(flavor = "multi_thread")]
        async fn tg_read_001_a_failed_population_answers_errors_and_registers_nothing() {
            let mut h = harness();
            store_rows(&h, 2);
            h.index_factory.register_map(M);
            h.store.fail_scans(M, true);

            let resp = search(&h.svc).await;
            assert!(resp.error.is_some() && resp.results.is_empty(), "{resp:?}");
            assert!(!h.svc.populations.is_populated(M) && !h.svc.populations.in_flight(M));

            let hybrid = hybrid_resp(call(&h.svc, make_hybrid_search_op(M, TERM)).await);
            assert!(
                hybrid.error.is_some() && hybrid.results.is_empty(),
                "{hybrid:?}"
            );

            let sub = hybrid_resp(call(&h.svc, make_hybrid_sub_op(M, TERM, "hft")).await);
            assert!(sub.error.is_some() && sub.results.is_empty(), "{sub:?}");
            let sub = hybrid_resp(call(&h.svc, exact_sub_op("hex")).await);
            assert!(sub.error.is_some() && sub.results.is_empty(), "{sub:?}");
            assert!(
                !h.svc.hybrid_registry.has_subscriptions_for_map(M),
                "a failed initial evaluation must not register the subscription"
            );

            match call(&h.svc, make_subscribe_op(M, TERM)).await {
                Message::SearchResp { payload } => {
                    assert!(payload.error.is_some() && payload.results.is_empty());
                }
                other => panic!("unexpected message: {other:?}"),
            }
            assert!(
                !h.svc.registry.has_subscriptions_for_map(M),
                "a failed population must not register the text subscription"
            );
            assert!(
                h.rx.try_recv().is_err(),
                "nothing is pushed for a refused subscription"
            );

            h.store.fail_scans(M, false);
            let resp = search(&h.svc).await;
            assert!(resp.error.is_none());
            assert_eq!(keys(&resp), vec!["s0", "s1"]);
        }

        /// AC21(b): a panicking scan answers the waiting search with an error
        /// and releases the in-flight marker; the healed store is searched.
        #[tokio::test(flavor = "multi_thread")]
        async fn tg_read_001_a_panicked_population_releases_the_map() {
            let h = harness();
            store_rows(&h, 2);
            h.store.panic_scans(true);

            let resp = tokio::time::timeout(Duration::from_secs(10), search(&h.svc))
                .await
                .expect("the waiting search must not hang");
            assert!(resp.error.is_some() && resp.results.is_empty(), "{resp:?}");
            assert!(!h.svc.populations.in_flight(M));

            h.store.panic_scans(false);
            let resp = tokio::time::timeout(Duration::from_secs(10), search(&h.svc))
                .await
                .expect("a new population must start");
            assert_eq!(keys(&resp), vec!["s0", "s1"]);
        }

        /// AC21(c): an indexer that panics in its commit after the last batch
        /// fails the population: no swap, nothing recorded, marker released.
        #[tokio::test(flavor = "multi_thread")]
        async fn tg_read_001_an_indexer_panic_after_the_last_batch_swaps_nothing() {
            let h = harness();
            store_rows(&h, 2);
            {
                let old = TantivyMapIndex::new();
                old.index_document("sentinel", &make_rmpv_map(&[("text", TERM)]));
                old.commit();
                h.svc.indexes.write().insert(M.to_string(), old);
            }
            h.svc.populations.commit_panic.store(true, Ordering::SeqCst);

            let resp = search(&h.svc).await;
            assert!(resp.error.is_some() && resp.results.is_empty(), "{resp:?}");
            assert!(old_index_has(&h, "sentinel"), "the old index must stay");
            assert!(!old_index_has(&h, "s0"));
            assert!(!h.svc.populations.is_populated(M) && !h.svc.populations.in_flight(M));

            h.svc
                .populations
                .commit_panic
                .store(false, Ordering::SeqCst);
            let resp = search(&h.svc).await;
            assert_eq!(keys(&resp), vec!["s0", "s1"]);
        }

        /// AC24: notifier passes read the current index; they start no
        /// population and no store scan, and still deliver fullText deltas.
        #[tokio::test(flavor = "multi_thread")]
        async fn tg_read_001_notifier_passes_run_no_population() {
            let mut h = harness();
            store_rows(&h, 2);
            h.index_factory.register_map(M);

            h.store.close_gate();
            let svc = Arc::clone(&h.svc);
            let registration =
                tokio::spawn(async move { call(&svc, make_hybrid_sub_op(M, TERM, "hsub")).await });
            eventually("the registration's scan parks", || h.store.parked() == 1).await;
            write(&h, "w0", &format!("{TERM} overlapping")).await;
            h.store.open_gate();
            let resp = hybrid_resp(registration.await.unwrap());
            assert!(resp.error.is_none(), "{resp:?}");
            assert!(
                !h.svc.populations.is_populated(M),
                "the population was overlapped"
            );

            let passes_before = h.svc.notifier_passes.load(Ordering::SeqCst);
            for i in 1..=5 {
                write(&h, &format!("w{i}"), &format!("{TERM} steady {i}")).await;
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
            let mut delivered = Vec::new();
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while std::time::Instant::now() < deadline && delivered.is_empty() {
                while let Ok(msg) = h.rx.try_recv() {
                    if let Some(update) = decode_hybrid_update(msg) {
                        if (1..=5).any(|i| update.key == format!("w{i}")) {
                            delivered.push(update.key);
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;

            assert!(
                h.svc.notifier_passes.load(Ordering::SeqCst) > passes_before,
                "a pass over the map ran after registration"
            );
            assert!(
                !delivered.is_empty(),
                "the passes delivered a fullText delta"
            );
            assert_eq!(
                h.svc.populations.started.load(Ordering::SeqCst),
                1,
                "only registration populated"
            );
            assert_eq!(h.store.scans(M), 1, "only registration scanned");
        }
    }
}
