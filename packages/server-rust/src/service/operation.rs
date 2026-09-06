//! Operation types for the service routing framework.
//!
//! Defines the typed `Operation` enum (one variant per client-to-server message),
//! `OperationContext` (per-operation metadata), and response/error types.

use topgun_core::messages;
use topgun_core::Timestamp;

use crate::network::connection::ConnectionId;

// ---------------------------------------------------------------------------
// OperationPipeline type alias
// ---------------------------------------------------------------------------

/// Concrete type for the composed Tower middleware pipeline that wraps
/// `OperationRouter`. Uses `BoxService` to erase the unnameable future type
/// produced by `build_operation_pipeline()`. Stored in `AppState` behind
/// `Arc<tokio::sync::Mutex<OperationPipeline>>` because `Service::call()`
/// requires `&mut self`.
pub type OperationPipeline = tower::util::BoxService<Operation, OperationResponse, OperationError>;

// ---------------------------------------------------------------------------
// Service name constants
// ---------------------------------------------------------------------------

/// Service name constants used for operation routing.
/// These must match the `ManagedService::name()` return values of domain services.
pub mod service_names {
    pub const CRDT: &str = "crdt";
    pub const SYNC: &str = "sync";
    pub const QUERY: &str = "query";
    pub const MESSAGING: &str = "messaging";
    pub const COORDINATION: &str = "coordination";
    pub const SEARCH: &str = "search";
    pub const PERSISTENCE: &str = "persistence";
}

// ---------------------------------------------------------------------------
// CallerOrigin
// ---------------------------------------------------------------------------

/// Origin of the operation caller, used for access control and routing decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerOrigin {
    /// Direct client connection.
    Client,
    /// HTTP client request authenticated via Bearer JWT.
    HttpClient,
    /// Unauthenticated HTTP request (no Bearer token). Subject to RBAC evaluation.
    Anonymous,
    /// Forwarded from another cluster node.
    Forwarded,
    /// Backup replication from primary node.
    Backup,
    /// Wide-area replication.
    Wan,
    /// System-internal operation (e.g., GC).
    System,
}

// ---------------------------------------------------------------------------
// OperationContext
// ---------------------------------------------------------------------------

/// Metadata carried with every operation through the pipeline.
///
/// Does not derive `Default` because `call_id`, `service_name`, and `timestamp`
/// are required fields with no sensible defaults. Use `OperationContext::new()`
/// to construct with required fields, then set optional fields
/// (`partition_id`, `client_id`, `caller_node_id`, `connection_id`) as needed.
#[derive(Debug, Clone)]
pub struct OperationContext {
    /// Unique identifier for this operation invocation.
    pub call_id: u64,
    /// Partition this operation targets, if applicable.
    pub partition_id: Option<u32>,
    /// Name of the domain service that handles this operation.
    pub service_name: &'static str,
    /// Where the operation originated.
    pub caller_origin: CallerOrigin,
    /// Client connection identifier, if from a client.
    pub client_id: Option<String>,
    /// Node identifier of the caller, if forwarded from another node.
    pub caller_node_id: Option<String>,
    /// Connection identifier for the caller, set by the WebSocket handler.
    /// Used by domain services for side-effects (heartbeat updates, subscription
    /// tracking, message routing). Transport metadata only — not used for
    /// identity resolution.
    pub connection_id: Option<ConnectionId>,
    /// Principal identity for this operation. Set eagerly by all transport
    /// handlers (WebSocket, HTTP) before pipeline dispatch. Used by authorization
    /// middleware for RBAC evaluation.
    pub principal: Option<topgun_core::Principal>,
    /// HLC timestamp for this operation.
    pub timestamp: Timestamp,
    /// Timeout for this operation in milliseconds.
    pub call_timeout_ms: u64,
}

impl OperationContext {
    /// Create a new `OperationContext` with required fields.
    /// Optional fields default to `None`; `caller_origin` defaults to `Client`.
    #[must_use]
    pub fn new(
        call_id: u64,
        service_name: &'static str,
        timestamp: Timestamp,
        default_timeout_ms: u64,
    ) -> Self {
        Self {
            call_id,
            partition_id: None,
            service_name,
            caller_origin: CallerOrigin::Client,
            client_id: None,
            caller_node_id: None,
            connection_id: None,
            principal: None,
            timestamp,
            call_timeout_ms: default_timeout_ms,
        }
    }
}

// ---------------------------------------------------------------------------
// Operation enum
// ---------------------------------------------------------------------------

/// Typed operation variants dispatched through the pipeline.
///
/// Each variant carries an `OperationContext` plus domain-specific payload fields
/// extracted from the corresponding `Message` inner struct. The `service_name` in
/// the context determines which domain service handles the operation.
///
/// Grouped by domain:
/// - **CRDT** (2): `ClientOp`, `OpBatch`
/// - **Sync** (6): `SyncInit`, `MerkleReqBucket`, `ORMapSyncInit`, `ORMapMerkleReqBucket`, `ORMapDiffRequest`, `ORMapPushDiff`
/// - **Query** (4): `QuerySubscribe`, `QueryUnsubscribe`, `QuerySyncInit`, `SqlQuery`
/// - **Messaging** (3): `TopicSubscribe`, `TopicUnsubscribe`, `TopicPublish`
/// - **Coordination** (4): `LockRequest`, `LockRelease`, `PartitionMapRequest`, `Ping`
/// - **Search** (3): `Search`, `SearchSubscribe`, `SearchUnsubscribe`
/// - **Persistence** (10): `CounterRequest`, `CounterSync`, `EntryProcess`, `EntryProcessBatch`,
///   `RegisterResolver`, `UnregisterResolver`, `ListResolvers`, `JournalSubscribe`, `JournalUnsubscribe`, `JournalRead`
/// - **System** (1): `GarbageCollect` (internal, not from `classify()`)
#[derive(Debug)]
pub enum Operation {
    // --- CRDT domain (service_name = "crdt") ---
    /// Single client write operation.
    ClientOp {
        ctx: OperationContext,
        payload: messages::ClientOpMessage,
    },
    /// Batch of client write operations.
    OpBatch {
        ctx: OperationContext,
        payload: messages::OpBatchMessage,
    },

    // --- Sync domain (service_name = "sync") ---
    /// Client initiates LWW merkle sync.
    SyncInit {
        ctx: OperationContext,
        payload: messages::SyncInitMessage,
    },
    /// Client requests a specific merkle bucket.
    MerkleReqBucket {
        ctx: OperationContext,
        payload: messages::MerkleReqBucketMessage,
    },
    /// Client initiates `ORMap` sync.
    ORMapSyncInit {
        ctx: OperationContext,
        payload: messages::ORMapSyncInit,
    },
    /// Client requests a specific `ORMap` merkle bucket.
    ORMapMerkleReqBucket {
        ctx: OperationContext,
        payload: messages::ORMapMerkleReqBucket,
    },
    /// Client requests `ORMap` diff entries.
    ORMapDiffRequest {
        ctx: OperationContext,
        payload: messages::ORMapDiffRequest,
    },
    /// Client pushes `ORMap` diff (bidirectional message).
    ORMapPushDiff {
        ctx: OperationContext,
        payload: messages::ORMapPushDiff,
    },

    // --- Query domain (service_name = "query") ---
    /// Client subscribes to a live query.
    QuerySubscribe {
        ctx: OperationContext,
        payload: messages::QuerySubMessage,
    },
    /// Client unsubscribes from a live query.
    QueryUnsubscribe {
        ctx: OperationContext,
        payload: messages::QueryUnsubMessage,
    },
    /// Client initiates query Merkle delta sync reconnect.
    QuerySyncInit {
        ctx: OperationContext,
        payload: messages::QuerySyncInitMessage,
    },
    /// Client executes a SQL query.
    SqlQuery {
        ctx: OperationContext,
        payload: messages::query::SqlQueryPayload,
    },
    /// Client requests ANN search over a map's vector index.
    VectorSearch {
        ctx: OperationContext,
        payload: messages::vector::VectorSearchPayload,
    },

    // --- Messaging domain (service_name = "messaging") ---
    /// Client subscribes to a topic.
    TopicSubscribe {
        ctx: OperationContext,
        payload: messages::TopicSubPayload,
    },
    /// Client unsubscribes from a topic.
    TopicUnsubscribe {
        ctx: OperationContext,
        payload: messages::TopicUnsubPayload,
    },
    /// Client publishes to a topic.
    TopicPublish {
        ctx: OperationContext,
        payload: messages::TopicPubPayload,
    },

    // --- Coordination domain (service_name = "coordination") ---
    /// Client requests a distributed lock.
    LockRequest {
        ctx: OperationContext,
        payload: messages::LockRequestPayload,
    },
    /// Client releases a distributed lock.
    LockRelease {
        ctx: OperationContext,
        payload: messages::LockReleasePayload,
    },
    /// Client requests the partition map.
    PartitionMapRequest {
        ctx: OperationContext,
        payload: Option<messages::PartitionMapRequestPayload>,
    },
    /// Client heartbeat ping.
    Ping {
        ctx: OperationContext,
        payload: messages::PingData,
    },

    // --- Search domain (service_name = "search") ---
    /// Client sends a search request.
    Search {
        ctx: OperationContext,
        payload: messages::SearchPayload,
    },
    /// Client subscribes to live search results.
    SearchSubscribe {
        ctx: OperationContext,
        payload: messages::SearchSubPayload,
    },
    /// Client unsubscribes from live search.
    SearchUnsubscribe {
        ctx: OperationContext,
        payload: messages::SearchUnsubPayload,
    },
    /// Client requests hybrid search (exact + full-text + semantic).
    HybridSearch {
        ctx: OperationContext,
        payload: messages::hybrid::HybridSearchPayload,
    },
    /// Client subscribes to live hybrid search results.
    HybridSearchSubscribe {
        ctx: OperationContext,
        payload: messages::hybrid::HybridSearchSubPayload,
    },
    /// Client unsubscribes from live hybrid search.
    HybridSearchUnsubscribe {
        ctx: OperationContext,
        payload: messages::hybrid::HybridSearchUnsubPayload,
    },

    // --- Persistence domain (service_name = "persistence") ---
    /// Client requests counter state.
    CounterRequest {
        ctx: OperationContext,
        payload: messages::CounterRequestPayload,
    },
    /// Client syncs counter state (bidirectional message).
    CounterSync {
        ctx: OperationContext,
        payload: messages::CounterStatePayload,
    },
    /// Client requests entry processing for a single key.
    EntryProcess {
        ctx: OperationContext,
        payload: messages::EntryProcessData,
    },
    /// Client requests batch entry processing.
    EntryProcessBatch {
        ctx: OperationContext,
        payload: messages::EntryProcessBatchData,
    },
    /// Client registers a conflict resolver.
    RegisterResolver {
        ctx: OperationContext,
        payload: messages::RegisterResolverData,
    },
    /// Client unregisters a conflict resolver.
    UnregisterResolver {
        ctx: OperationContext,
        payload: messages::UnregisterResolverData,
    },
    /// Client lists conflict resolvers.
    ListResolvers {
        ctx: OperationContext,
        payload: messages::ListResolversData,
    },
    /// Client subscribes to journal events.
    JournalSubscribe {
        ctx: OperationContext,
        payload: messages::JournalSubscribeData,
    },
    /// Client unsubscribes from journal events.
    JournalUnsubscribe {
        ctx: OperationContext,
        payload: messages::JournalUnsubscribeData,
    },
    /// Client reads journal entries.
    JournalRead {
        ctx: OperationContext,
        payload: messages::JournalReadData,
    },

    // --- System domain (internal, not from classify) ---
    /// System-internal garbage collection operation.
    /// Triggered by `BackgroundWorker`, not by message classification.
    GarbageCollect { ctx: OperationContext },
}

impl Operation {
    /// Returns the `OperationContext` for this operation.
    #[must_use]
    pub fn ctx(&self) -> &OperationContext {
        match self {
            // CRDT
            Self::ClientOp { ctx, .. }
            | Self::OpBatch { ctx, .. }
            // Sync
            | Self::SyncInit { ctx, .. }
            | Self::MerkleReqBucket { ctx, .. }
            | Self::ORMapSyncInit { ctx, .. }
            | Self::ORMapMerkleReqBucket { ctx, .. }
            | Self::ORMapDiffRequest { ctx, .. }
            | Self::ORMapPushDiff { ctx, .. }
            // Query
            | Self::QuerySubscribe { ctx, .. }
            | Self::QueryUnsubscribe { ctx, .. }
            | Self::QuerySyncInit { ctx, .. }
            | Self::SqlQuery { ctx, .. }
            | Self::VectorSearch { ctx, .. }
            // Messaging
            | Self::TopicSubscribe { ctx, .. }
            | Self::TopicUnsubscribe { ctx, .. }
            | Self::TopicPublish { ctx, .. }
            // Coordination
            | Self::LockRequest { ctx, .. }
            | Self::LockRelease { ctx, .. }
            | Self::PartitionMapRequest { ctx, .. }
            | Self::Ping { ctx, .. }
            // Search
            | Self::Search { ctx, .. }
            | Self::SearchSubscribe { ctx, .. }
            | Self::SearchUnsubscribe { ctx, .. }
            | Self::HybridSearch { ctx, .. }
            | Self::HybridSearchSubscribe { ctx, .. }
            | Self::HybridSearchUnsubscribe { ctx, .. }
            // Persistence
            | Self::CounterRequest { ctx, .. }
            | Self::CounterSync { ctx, .. }
            | Self::EntryProcess { ctx, .. }
            | Self::EntryProcessBatch { ctx, .. }
            | Self::RegisterResolver { ctx, .. }
            | Self::UnregisterResolver { ctx, .. }
            | Self::ListResolvers { ctx, .. }
            | Self::JournalSubscribe { ctx, .. }
            | Self::JournalUnsubscribe { ctx, .. }
            | Self::JournalRead { ctx, .. }
            // System
            | Self::GarbageCollect { ctx } => ctx,
        }
    }

    /// Sets the `connection_id` on this operation's `OperationContext`.
    ///
    /// Called by the WebSocket handler before dispatching through the pipeline
    /// so domain services can look up the `ConnectionHandle` for side-effects
    /// (e.g., heartbeat updates, subscription tracking).
    ///
    /// Mirrors the `ctx()` pattern but with `&mut self` and mutable bindings.
    pub fn set_connection_id(&mut self, id: ConnectionId) {
        match self {
            // CRDT
            Self::ClientOp { ctx, .. }
            | Self::OpBatch { ctx, .. }
            // Sync
            | Self::SyncInit { ctx, .. }
            | Self::MerkleReqBucket { ctx, .. }
            | Self::ORMapSyncInit { ctx, .. }
            | Self::ORMapMerkleReqBucket { ctx, .. }
            | Self::ORMapDiffRequest { ctx, .. }
            | Self::ORMapPushDiff { ctx, .. }
            // Query
            | Self::QuerySubscribe { ctx, .. }
            | Self::QueryUnsubscribe { ctx, .. }
            | Self::QuerySyncInit { ctx, .. }
            | Self::SqlQuery { ctx, .. }
            | Self::VectorSearch { ctx, .. }
            // Messaging
            | Self::TopicSubscribe { ctx, .. }
            | Self::TopicUnsubscribe { ctx, .. }
            | Self::TopicPublish { ctx, .. }
            // Coordination
            | Self::LockRequest { ctx, .. }
            | Self::LockRelease { ctx, .. }
            | Self::PartitionMapRequest { ctx, .. }
            | Self::Ping { ctx, .. }
            // Search
            | Self::Search { ctx, .. }
            | Self::SearchSubscribe { ctx, .. }
            | Self::SearchUnsubscribe { ctx, .. }
            | Self::HybridSearch { ctx, .. }
            | Self::HybridSearchSubscribe { ctx, .. }
            | Self::HybridSearchUnsubscribe { ctx, .. }
            // Persistence
            | Self::CounterRequest { ctx, .. }
            | Self::CounterSync { ctx, .. }
            | Self::EntryProcess { ctx, .. }
            | Self::EntryProcessBatch { ctx, .. }
            | Self::RegisterResolver { ctx, .. }
            | Self::UnregisterResolver { ctx, .. }
            | Self::ListResolvers { ctx, .. }
            | Self::JournalSubscribe { ctx, .. }
            | Self::JournalUnsubscribe { ctx, .. }
            | Self::JournalRead { ctx, .. }
            // System
            | Self::GarbageCollect { ctx } => {
                ctx.connection_id = Some(id);
            }
        }
    }

    /// Sets the `principal` on this operation's `OperationContext`.
    ///
    /// Called by HTTP handlers after JWT authentication to carry the caller's
    /// identity through the pipeline for RBAC authorization checks.
    /// Mirrors the `set_connection_id` pattern.
    pub fn set_principal(&mut self, principal: topgun_core::Principal) {
        match self {
            // CRDT
            Self::ClientOp { ctx, .. }
            | Self::OpBatch { ctx, .. }
            // Sync
            | Self::SyncInit { ctx, .. }
            | Self::MerkleReqBucket { ctx, .. }
            | Self::ORMapSyncInit { ctx, .. }
            | Self::ORMapMerkleReqBucket { ctx, .. }
            | Self::ORMapDiffRequest { ctx, .. }
            | Self::ORMapPushDiff { ctx, .. }
            // Query
            | Self::QuerySubscribe { ctx, .. }
            | Self::QueryUnsubscribe { ctx, .. }
            | Self::QuerySyncInit { ctx, .. }
            | Self::SqlQuery { ctx, .. }
            | Self::VectorSearch { ctx, .. }
            // Messaging
            | Self::TopicSubscribe { ctx, .. }
            | Self::TopicUnsubscribe { ctx, .. }
            | Self::TopicPublish { ctx, .. }
            // Coordination
            | Self::LockRequest { ctx, .. }
            | Self::LockRelease { ctx, .. }
            | Self::PartitionMapRequest { ctx, .. }
            | Self::Ping { ctx, .. }
            // Search
            | Self::Search { ctx, .. }
            | Self::SearchSubscribe { ctx, .. }
            | Self::SearchUnsubscribe { ctx, .. }
            | Self::HybridSearch { ctx, .. }
            | Self::HybridSearchSubscribe { ctx, .. }
            | Self::HybridSearchUnsubscribe { ctx, .. }
            // Persistence
            | Self::CounterRequest { ctx, .. }
            | Self::CounterSync { ctx, .. }
            | Self::EntryProcess { ctx, .. }
            | Self::EntryProcessBatch { ctx, .. }
            | Self::RegisterResolver { ctx, .. }
            | Self::UnregisterResolver { ctx, .. }
            | Self::ListResolvers { ctx, .. }
            | Self::JournalSubscribe { ctx, .. }
            | Self::JournalUnsubscribe { ctx, .. }
            | Self::JournalRead { ctx, .. }
            // System
            | Self::GarbageCollect { ctx } => {
                ctx.principal = Some(principal);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// OperationResponse
// ---------------------------------------------------------------------------

/// Successful response from an operation handler.
#[derive(Debug)]
pub enum OperationResponse {
    /// Simple acknowledgement.
    Ack { call_id: u64 },
    /// Single message response.
    Message(Box<messages::Message>),
    /// Multiple message responses.
    Messages(Vec<messages::Message>),
    /// Operation not yet implemented by the domain service stub.
    NotImplemented {
        service_name: &'static str,
        call_id: u64,
    },
    /// No response needed.
    Empty,
}

// ---------------------------------------------------------------------------
// OperationError
// ---------------------------------------------------------------------------

/// Errors returned by operation handlers.
#[derive(Debug, thiserror::Error)]
pub enum OperationError {
    #[error("unknown service: {name}")]
    UnknownService { name: String },
    #[error("operation timed out after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },
    #[error("server overloaded, try again later")]
    Overloaded,
    #[error("wrong service for operation")]
    WrongService,
    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
    #[error("authentication required")]
    Unauthorized,
    #[error("write access denied for map: {map_name}")]
    Forbidden { map_name: String },
    #[error("value size {size} bytes exceeds maximum {max} bytes")]
    ValueTooLarge { size: u64, max: u64 },
    #[error("schema validation failed for map '{map_name}': {}", errors.join("; "))]
    SchemaInvalid {
        map_name: String,
        errors: Vec<String>,
    },
}

// ---------------------------------------------------------------------------
// ErrorDisposition
// ---------------------------------------------------------------------------

/// Whether retrying the identical operation can ever succeed.
///
/// This is a property of the error, decided once at the source, never
/// re-derived from a formatted message at a call site: `format!("{e}")` produces
/// the human-readable `reason` and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorDisposition {
    /// The condition can clear on its own — the same operation may succeed on a
    /// later attempt, so the client must keep it queued and retry it.
    Transient,
    /// The server refused the operation itself. No retry of the identical
    /// operation can succeed, so the client must retire it instead of
    /// re-sending it forever.
    Permanent,
}

/// Every label `OperationError::error_kind()` can return, in variant order.
///
/// The enumerable form of the same vocabulary: it exists so metric series can be
/// registered for the whole label space up front (a counter that has never been
/// incremented is invisible to a scrape, which is indistinguishable from
/// "nothing was refused"). Kept beside `error_kind()` so the two cannot drift
/// without the drift being visible in one screen.
pub(crate) const ALL_ERROR_KINDS: [&str; 9] = [
    "unknown_service",
    "timeout",
    "overloaded",
    "wrong_service",
    "internal",
    "unauthorized",
    "forbidden",
    "value_too_large",
    "schema_invalid",
];

impl OperationError {
    /// Classifies whether retrying the identical operation can ever succeed.
    ///
    /// Exhaustive with no `_` arm on purpose: a new `OperationError` variant must
    /// fail to compile until someone decides what a client should do about it.
    /// Defaulting an unclassified variant to either side is the failure this
    /// method exists to prevent — `Transient` would retry a doomed write forever,
    /// `Permanent` would silently discard a recoverable one.
    ///
    /// `Unauthorized` is **Transient**: it is a connection-level condition (the
    /// connection's identity is gone or was never established), not a property of
    /// the operation, so a logged-out user's queued writes must survive to the
    /// next authenticated connection rather than be retired.
    #[must_use]
    pub fn disposition(&self) -> ErrorDisposition {
        match self {
            Self::Forbidden { .. } | Self::ValueTooLarge { .. } | Self::SchemaInvalid { .. } => {
                ErrorDisposition::Permanent
            }
            Self::UnknownService { .. } | Self::WrongService => ErrorDisposition::Permanent,
            Self::Unauthorized | Self::Overloaded | Self::Timeout { .. } | Self::Internal(_) => {
                ErrorDisposition::Transient
            }
        }
    }

    /// The machine-readable code carried to the client on the wire.
    ///
    /// HTTP-style codes, so the same number means the same thing on both
    /// transports. Exhaustive with no `_` arm for the same reason as
    /// [`Self::disposition`].
    #[must_use]
    pub fn wire_code(&self) -> u32 {
        match self {
            Self::Unauthorized => 401,
            Self::Forbidden { .. } => 403,
            Self::ValueTooLarge { .. } => 413,
            Self::SchemaInvalid { .. } => 422,
            Self::Overloaded => 429,
            Self::Internal(_) => 500,
            Self::UnknownService { .. } | Self::WrongService => 501,
            Self::Timeout { .. } => 504,
        }
    }

    /// The stable, low-cardinality label used for metrics and for the client's
    /// closed set of refusal causes.
    ///
    /// These nine strings are one vocabulary shared with the operation-error
    /// metric emitted by the pipeline middleware; the strings are byte-identical
    /// on both sides and [`ALL_ERROR_KINDS`] is their enumerable form. Never
    /// derive a label from `format!("{e}")` — variant messages interpolate
    /// unbounded values (map names, sizes) and would blow up label cardinality.
    ///
    /// Exhaustive with no `_` arm for the same reason as [`Self::disposition`].
    #[must_use]
    pub fn error_kind(&self) -> &'static str {
        match self {
            Self::UnknownService { .. } => "unknown_service",
            Self::Timeout { .. } => "timeout",
            Self::Overloaded => "overloaded",
            Self::WrongService => "wrong_service",
            Self::Internal(_) => "internal",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden { .. } => "forbidden",
            Self::ValueTooLarge { .. } => "value_too_large",
            Self::SchemaInvalid { .. } => "schema_invalid",
        }
    }
}

// ---------------------------------------------------------------------------
// Per-operation verdicts (internal fold types)
// ---------------------------------------------------------------------------

/// The verdict reached for one client operation in one exchange.
///
/// Internal to the server: these types are folded at the transport and then
/// shaped into wire frames, so they carry no serde derives and never appear on
/// the wire themselves.
///
/// Only refusals get a per-operation verdict. Acceptance is carried by
/// [`OpOutcome::accepted`] as wire-ready `OpResult`s instead, so there is
/// deliberately no `Accepted` variant here: two representations of "this op was
/// applied" would be two things to keep in step, and the fold already has to
/// build the `OpResult` form to shape the ack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OpVerdict {
    /// The operation was refused permanently and is attributed to a named
    /// operation. A refusal that cannot be attributed to an id is NOT this
    /// variant — it stays a batch-level error, because guessing which write was
    /// refused is worse than reporting that the batch failed.
    Refused {
        /// Id of the refused operation. Attribution is the whole point of the
        /// variant, so this is not optional.
        op_id: String,
        /// Wire code from [`OperationError::wire_code`].
        code: u32,
        /// Human-readable reason, from the error's `Display`.
        reason: String,
        /// Metric label from [`OperationError::error_kind`].
        kind: &'static str,
    },
}

/// The folded result of dispatching one client batch.
///
/// **Contract: every operation of the batch reaches at most one terminal sink,
/// and exactly one when the exchange produced any verdict at all.** An operation
/// that is in `accepted` is never in `refused`, and the reverse; that is what
/// lets a client treat each frame it receives as final (TG-SYNC-003).
///
/// The two error sinks are deliberately separate rather than one field:
/// `transient` is retryable and `batch_error` is a permanent failure that could
/// NOT be attributed to a single operation. Folding the second into the first
/// would lie about its disposition; folding the first into the second would
/// retire operations that a retry would have accepted.
#[derive(Debug, Default)]
pub(crate) struct OpOutcome {
    /// Operations the server accepted, in the wire shape `OpAckPayload.results`
    /// takes.
    ///
    /// **Populated only when `refused` is non-empty**, which is the only case in
    /// which an acknowledgement has to name operations one by one: a batch
    /// nobody refused anything in is acknowledged by its last id with no
    /// `results` field at all, so materializing this vector there would clone an
    /// id per operation on the hot path for something no caller reads. Read it
    /// only after finding `refused` non-empty; an empty `accepted` alongside an
    /// empty `refused` means "not computed", NOT "nothing was accepted".
    ///
    /// Id-less accepted operations are absent even when it is populated: they
    /// cannot be named back to the client. That residue is TG-SYNC-001's stated
    /// exclusion.
    pub accepted: Vec<messages::OpResult>,
    /// Permanent refusals attributed to a named operation — one `OP_REJECTED`
    /// frame each, emitted before any acknowledgement (TG-SYNC-002).
    pub refused: Vec<OpVerdict>,
    /// The single transient error to report for the whole exchange, chosen
    /// deterministically when several sub-batches failed differently.
    pub transient: Option<OperationError>,
    /// A permanent error that was NOT attributed per operation — an unroutable
    /// operation, or a batch whose operations carry no ids. Reported as the
    /// batch-level error frame, never as a per-operation refusal.
    pub batch_error: Option<OperationError>,
}

// ---------------------------------------------------------------------------
// ClassifyError
// ---------------------------------------------------------------------------

/// Errors from classifying a `Message` into an `Operation`.
#[derive(Debug, thiserror::Error)]
pub enum ClassifyError {
    /// Server-to-client response messages cannot be classified as operations.
    #[error("server-to-client response cannot be classified as operation: {variant}")]
    ServerToClient { variant: &'static str },
    /// Transport envelopes must be unpacked by the network layer before classification.
    #[error("transport envelope must be unpacked before classification: {variant}")]
    TransportEnvelope { variant: &'static str },
    /// Authentication messages are handled at the transport layer.
    #[error("authentication message handled at transport layer: {variant}")]
    AuthMessage { variant: &'static str },
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_timestamp() -> Timestamp {
        Timestamp {
            millis: 1_700_000_000_000,
            counter: 1,
            node_id: "test-node".to_string(),
        }
    }

    #[test]
    fn operation_context_new_sets_defaults() {
        let ctx = OperationContext::new(42, service_names::CRDT, make_timestamp(), 30_000);
        assert_eq!(ctx.call_id, 42);
        assert_eq!(ctx.service_name, "crdt");
        assert_eq!(ctx.caller_origin, CallerOrigin::Client);
        assert!(ctx.partition_id.is_none());
        assert!(ctx.client_id.is_none());
        assert!(ctx.caller_node_id.is_none());
        assert!(ctx.connection_id.is_none());
        assert_eq!(ctx.call_timeout_ms, 30_000);
    }

    #[test]
    fn operation_ctx_accessor_returns_context() {
        let ctx = OperationContext::new(99, service_names::COORDINATION, make_timestamp(), 5000);
        let op = Operation::GarbageCollect { ctx };
        assert_eq!(op.ctx().call_id, 99);
        assert_eq!(op.ctx().service_name, "coordination");
    }

    #[test]
    fn caller_origin_equality() {
        assert_eq!(CallerOrigin::Client, CallerOrigin::Client);
        assert_ne!(CallerOrigin::Client, CallerOrigin::System);
        assert_ne!(CallerOrigin::Client, CallerOrigin::HttpClient);
        assert_ne!(CallerOrigin::HttpClient, CallerOrigin::System);
        assert_ne!(CallerOrigin::Anonymous, CallerOrigin::Client);
        assert_ne!(CallerOrigin::Anonymous, CallerOrigin::HttpClient);
        assert_ne!(CallerOrigin::Anonymous, CallerOrigin::System);
    }

    #[test]
    fn operation_error_display() {
        let err = OperationError::UnknownService {
            name: "bad-service".to_string(),
        };
        assert_eq!(format!("{err}"), "unknown service: bad-service");

        let err = OperationError::Timeout { timeout_ms: 5000 };
        assert_eq!(format!("{err}"), "operation timed out after 5000ms");

        let err = OperationError::Overloaded;
        assert_eq!(format!("{err}"), "server overloaded, try again later");
    }

    /// One value of every `OperationError` variant. Constructed by hand rather
    /// than generated so that adding a variant leaves this list short by one and
    /// the exhaustive `match` below fails to compile.
    fn one_of_every_error_variant() -> Vec<OperationError> {
        let all = vec![
            OperationError::UnknownService {
                name: "nope".to_string(),
            },
            OperationError::Timeout { timeout_ms: 1 },
            OperationError::Overloaded,
            OperationError::WrongService,
            OperationError::Internal(anyhow::anyhow!("boom")),
            OperationError::Unauthorized,
            OperationError::Forbidden {
                map_name: "m".to_string(),
            },
            OperationError::ValueTooLarge { size: 2, max: 1 },
            OperationError::SchemaInvalid {
                map_name: "m".to_string(),
                errors: vec!["bad".to_string()],
            },
        ];
        // Exhaustiveness guard: a new variant breaks this match, which is what
        // forces the list above to be extended too.
        for e in &all {
            match e {
                OperationError::UnknownService { .. }
                | OperationError::Timeout { .. }
                | OperationError::Overloaded
                | OperationError::WrongService
                | OperationError::Internal(_)
                | OperationError::Unauthorized
                | OperationError::Forbidden { .. }
                | OperationError::ValueTooLarge { .. }
                | OperationError::SchemaInvalid { .. } => {}
            }
        }
        all
    }

    /// The label vocabulary is one vocabulary, not two that can drift: the same
    /// nine strings are emitted by the pipeline middleware's own exhaustive
    /// match, and `ALL_ERROR_KINDS` is their enumerable form.
    #[test]
    fn error_kind_matches_metrics_middleware_vocabulary() {
        let mut from_fn: Vec<&str> = one_of_every_error_variant()
            .iter()
            .map(OperationError::error_kind)
            .collect();
        from_fn.sort_unstable();
        assert_eq!(from_fn.len(), 9, "one label per variant, no duplicates");

        let mut from_const: Vec<&str> = ALL_ERROR_KINDS.to_vec();
        from_const.sort_unstable();
        assert_eq!(from_fn, from_const, "error_kind() vs ALL_ERROR_KINDS drift");

        let mut expected = vec![
            "forbidden",
            "internal",
            "overloaded",
            "schema_invalid",
            "timeout",
            "unauthorized",
            "unknown_service",
            "value_too_large",
            "wrong_service",
        ];
        expected.sort_unstable();
        assert_eq!(
            from_fn, expected,
            "labels must stay byte-identical to the ones the operation-error metric already emits"
        );
    }

    /// Pins the classification table. `Unauthorized` being Transient is the
    /// load-bearing entry: it is a connection-level condition, so retiring the
    /// operation would throw away a write that succeeds after re-authentication.
    #[test]
    fn disposition_and_wire_code_follow_the_classification_table() {
        let expected: Vec<(&str, ErrorDisposition, u32)> = vec![
            ("unknown_service", ErrorDisposition::Permanent, 501),
            ("timeout", ErrorDisposition::Transient, 504),
            ("overloaded", ErrorDisposition::Transient, 429),
            ("wrong_service", ErrorDisposition::Permanent, 501),
            ("internal", ErrorDisposition::Transient, 500),
            ("unauthorized", ErrorDisposition::Transient, 401),
            ("forbidden", ErrorDisposition::Permanent, 403),
            ("value_too_large", ErrorDisposition::Permanent, 413),
            ("schema_invalid", ErrorDisposition::Permanent, 422),
        ];
        let actual: Vec<(&str, ErrorDisposition, u32)> = one_of_every_error_variant()
            .iter()
            .map(|e| (e.error_kind(), e.disposition(), e.wire_code()))
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn classify_error_display() {
        let err = ClassifyError::ServerToClient { variant: "OpAck" };
        assert!(format!("{err}").contains("OpAck"));

        let err = ClassifyError::TransportEnvelope { variant: "Batch" };
        assert!(format!("{err}").contains("Batch"));

        let err = ClassifyError::AuthMessage { variant: "Auth" };
        assert!(format!("{err}").contains("Auth"));
    }

    #[test]
    fn operation_response_not_implemented() {
        let resp = OperationResponse::NotImplemented {
            service_name: "crdt",
            call_id: 1,
        };
        assert!(matches!(resp, OperationResponse::NotImplemented { .. }));
    }

    #[test]
    fn service_name_constants() {
        assert_eq!(service_names::CRDT, "crdt");
        assert_eq!(service_names::SYNC, "sync");
        assert_eq!(service_names::QUERY, "query");
        assert_eq!(service_names::MESSAGING, "messaging");
        assert_eq!(service_names::COORDINATION, "coordination");
        assert_eq!(service_names::SEARCH, "search");
        assert_eq!(service_names::PERSISTENCE, "persistence");
    }

    /// Verify that the Operation enum has all variants by constructing each one.
    /// This ensures the enum definition is exhaustive and compiles correctly.
    #[test]
    fn operation_variant_count_covers_all_client_plus_system() {
        // We simply verify all variant paths exist by naming them.
        // The actual construction requires real payload types, so we just check
        // that the match arms compile with all expected variants.
        let ctx = OperationContext::new(1, service_names::CRDT, make_timestamp(), 1000);
        let op = Operation::GarbageCollect { ctx };

        // Exhaustive match ensures all variants are present at compile time.
        match op {
            Operation::ClientOp { .. }
            | Operation::OpBatch { .. }
            | Operation::SyncInit { .. }
            | Operation::MerkleReqBucket { .. }
            | Operation::ORMapSyncInit { .. }
            | Operation::ORMapMerkleReqBucket { .. }
            | Operation::ORMapDiffRequest { .. }
            | Operation::ORMapPushDiff { .. }
            | Operation::QuerySubscribe { .. }
            | Operation::QueryUnsubscribe { .. }
            | Operation::QuerySyncInit { .. }
            | Operation::SqlQuery { .. }
            | Operation::VectorSearch { .. }
            | Operation::TopicSubscribe { .. }
            | Operation::TopicUnsubscribe { .. }
            | Operation::TopicPublish { .. }
            | Operation::LockRequest { .. }
            | Operation::LockRelease { .. }
            | Operation::PartitionMapRequest { .. }
            | Operation::Ping { .. }
            | Operation::Search { .. }
            | Operation::SearchSubscribe { .. }
            | Operation::SearchUnsubscribe { .. }
            | Operation::HybridSearch { .. }
            | Operation::HybridSearchSubscribe { .. }
            | Operation::HybridSearchUnsubscribe { .. }
            | Operation::CounterRequest { .. }
            | Operation::CounterSync { .. }
            | Operation::EntryProcess { .. }
            | Operation::EntryProcessBatch { .. }
            | Operation::RegisterResolver { .. }
            | Operation::UnregisterResolver { .. }
            | Operation::ListResolvers { .. }
            | Operation::JournalSubscribe { .. }
            | Operation::JournalUnsubscribe { .. }
            | Operation::JournalRead { .. }
            | Operation::GarbageCollect { .. } => {}
        }
    }
}
