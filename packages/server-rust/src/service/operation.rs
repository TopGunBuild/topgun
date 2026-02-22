//! Operation types for the service routing framework.
//!
//! Defines the typed `Operation` enum (one variant per client-to-server message),
//! `OperationContext` (per-operation metadata), and response/error types.

use topgun_core::messages;
use topgun_core::Timestamp;

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
/// to construct with required fields, then set optional fields as needed.
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
/// - **Query** (2): `QuerySubscribe`, `QueryUnsubscribe`
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

    /// Verify that the Operation enum has all 31 variants by constructing each one.
    /// This ensures the enum definition is exhaustive and compiles correctly.
    #[test]
    fn operation_variant_count_covers_all_30_client_plus_1_system() {
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
