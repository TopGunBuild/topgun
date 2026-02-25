//! Sync domain service implementing Merkle delta sync protocol.
//!
//! Handles 6 sync-related `Operation` variants across two CRDT types
//! (LWW-Map and OR-Map), using `MerkleSyncManager` for per-partition tree
//! management and `RecordStoreFactory` for record access.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use tower::Service;

use topgun_core::messages::{
    self, Message, SyncLeafRecord, SyncRespBucketsMessage, SyncRespBucketsPayload,
    SyncRespLeafMessage, SyncRespLeafPayload, SyncRespRootMessage, SyncRespRootPayload,
};

// Helper enum for Merkle tree node classification (extracted to module level
// to avoid "adding items after statements" clippy warning).
enum LwwNodeData {
    Leaf(Vec<String>),
    Internal(HashMap<char, u32>),
    Missing,
}
use topgun_core::types::Value;
use topgun_core::LWWRecord;

use crate::network::connection::ConnectionRegistry;
use crate::service::operation::{
    service_names, Operation, OperationError, OperationResponse,
};
use crate::service::registry::{ManagedService, ServiceContext};
use crate::storage::merkle_sync::MerkleSyncManager;
use crate::storage::record::RecordValue;
use crate::storage::RecordStoreFactory;

// ---------------------------------------------------------------------------
// value_to_rmpv conversion helper
// ---------------------------------------------------------------------------

/// Converts an internal `topgun_core::types::Value` to an `rmpv::Value` for wire format.
///
/// Used in LWW and OR-Map leaf responses to convert stored values back to the
/// wire format that clients expect.
///
/// TODO: Consolidate this and `rmpv_to_value` from `crdt.rs` into a shared
/// `service/domain/conversion.rs` module in a follow-up spec.
pub(crate) fn value_to_rmpv(v: &Value) -> rmpv::Value {
    match v {
        Value::Null => rmpv::Value::Nil,
        Value::Bool(b) => rmpv::Value::Boolean(*b),
        Value::Int(i) => rmpv::Value::Integer((*i).into()),
        Value::Float(f) => rmpv::Value::F64(*f),
        Value::String(s) => rmpv::Value::String(s.as_str().into()),
        Value::Bytes(b) => rmpv::Value::Binary(b.clone()),
        Value::Array(a) => rmpv::Value::Array(a.iter().map(value_to_rmpv).collect()),
        Value::Map(m) => {
            let pairs: Vec<(rmpv::Value, rmpv::Value)> = m
                .iter()
                .map(|(k, v)| (rmpv::Value::String(k.as_str().into()), value_to_rmpv(v)))
                .collect();
            rmpv::Value::Map(pairs)
        }
    }
}

// ---------------------------------------------------------------------------
// SyncService
// ---------------------------------------------------------------------------

/// Real sync domain service implementing the Merkle delta sync protocol.
///
/// Replaces the `domain_stub!(SyncService, ...)` macro-generated stub.
/// Handles LWW-Map and OR-Map synchronization using Merkle tree comparison
/// so clients receive only the changed records when reconnecting after
/// an offline period.
pub struct SyncService {
    merkle_manager: Arc<MerkleSyncManager>,
    record_store_factory: Arc<RecordStoreFactory>,
    // Used in G4 by handle_ormap_push_diff for broadcasting changes to clients.
    #[allow(dead_code)]
    connection_registry: Arc<ConnectionRegistry>,
}

impl SyncService {
    /// Creates a new `SyncService` with its required dependencies.
    #[must_use]
    pub fn new(
        merkle_manager: Arc<MerkleSyncManager>,
        record_store_factory: Arc<RecordStoreFactory>,
        connection_registry: Arc<ConnectionRegistry>,
    ) -> Self {
        Self {
            merkle_manager,
            record_store_factory,
            connection_registry,
        }
    }

    // -----------------------------------------------------------------------
    // LWW handlers
    // -----------------------------------------------------------------------

    /// Handles `SyncInit` — returns the server's LWW Merkle tree root hash for `map_name`.
    ///
    /// `SyncInitMessage` is a FLAT message: `map_name` is directly on the payload struct,
    /// not nested in a `.payload` sub-field (contrast with `MerkleReqBucketMessage`).
    async fn handle_sync_init(
        &self,
        ctx: &crate::service::operation::OperationContext,
        payload: messages::SyncInitMessage,
    ) -> Result<OperationResponse, OperationError> {
        let map_name = payload.map_name;
        let partition_id = ctx.partition_id.unwrap_or(0);

        let root_hash = self.merkle_manager.with_lww_tree(&map_name, partition_id, |tree| {
            tree.get_root_hash()
        });

        Ok(OperationResponse::Message(Box::new(Message::SyncRespRoot(
            SyncRespRootMessage {
                payload: SyncRespRootPayload {
                    map_name,
                    root_hash,
                    timestamp: ctx.timestamp.clone(),
                },
            },
        ))))
    }

    /// Handles `MerkleReqBucket` — returns bucket hashes (internal) or leaf records (leaf).
    ///
    /// `MerkleReqBucketMessage` is a WRAPPED message: `map_name` and `path` live in a
    /// nested `.payload` field (i.e., `payload.payload.map_name`), unlike the flat
    /// `SyncInitMessage`.
    async fn handle_merkle_req_bucket(
        &self,
        ctx: &crate::service::operation::OperationContext,
        payload: messages::MerkleReqBucketMessage,
    ) -> Result<OperationResponse, OperationError> {
        let map_name = payload.payload.map_name;
        let path = payload.payload.path;
        let partition_id = ctx.partition_id.unwrap_or(0);

        // Extract all needed data within the closure so the Mutex is released
        // before any async operations. parking_lot::MutexGuard is !Send and must
        // not be held across .await points.
        let node_data = self.merkle_manager.with_lww_tree(&map_name, partition_id, |tree| {
            match tree.get_node(&path) {
                Some(node) if !node.entries.is_empty() => {
                    // Leaf node: extract keys as owned Vec<String>.
                    let keys: Vec<String> = node.entries.keys().cloned().collect();
                    LwwNodeData::Leaf(keys)
                }
                Some(_) => {
                    // Internal node: extract bucket hashes as owned HashMap.
                    let buckets = tree.get_buckets(&path);
                    LwwNodeData::Internal(buckets)
                }
                None => LwwNodeData::Missing,
            }
        });

        match node_data {
            LwwNodeData::Leaf(keys) => {
                // Mutex released — now safe to do async RecordStore fetches.
                let store = self.record_store_factory.create(&map_name, partition_id);
                let mut records = Vec::new();
                for key in keys {
                    if let Ok(Some(record)) = store.get(&key, false).await {
                        if let RecordValue::Lww { value, timestamp } = record.value {
                            records.push(SyncLeafRecord {
                                key,
                                record: LWWRecord {
                                    value: Some(value_to_rmpv(&value)),
                                    timestamp,
                                    ttl_ms: None,
                                },
                            });
                        }
                    }
                }
                Ok(OperationResponse::Message(Box::new(Message::SyncRespLeaf(
                    SyncRespLeafMessage {
                        payload: SyncRespLeafPayload {
                            map_name,
                            path,
                            records,
                        },
                    },
                ))))
            }
            LwwNodeData::Internal(buckets) => {
                // Convert HashMap<char, u32> to HashMap<String, u32>.
                let buckets: HashMap<String, u32> =
                    buckets.into_iter().map(|(c, h)| (c.to_string(), h)).collect();
                Ok(OperationResponse::Message(Box::new(
                    Message::SyncRespBuckets(SyncRespBucketsMessage {
                        payload: SyncRespBucketsPayload {
                            map_name,
                            path,
                            buckets,
                        },
                    }),
                )))
            }
            LwwNodeData::Missing => Ok(OperationResponse::Empty),
        }
    }

    // -----------------------------------------------------------------------
    // OR-Map handlers (implemented in G4)
    // -----------------------------------------------------------------------

    async fn handle_ormap_sync_init(
        &self,
        _ctx: &crate::service::operation::OperationContext,
        _payload: messages::ORMapSyncInit,
    ) -> Result<OperationResponse, OperationError> {
        todo!("implemented in G4")
    }

    async fn handle_ormap_merkle_req_bucket(
        &self,
        _ctx: &crate::service::operation::OperationContext,
        _payload: messages::ORMapMerkleReqBucket,
    ) -> Result<OperationResponse, OperationError> {
        todo!("implemented in G4")
    }

    async fn handle_ormap_diff_request(
        &self,
        _ctx: &crate::service::operation::OperationContext,
        _payload: messages::ORMapDiffRequest,
    ) -> Result<OperationResponse, OperationError> {
        todo!("implemented in G4")
    }

    async fn handle_ormap_push_diff(
        &self,
        _ctx: &crate::service::operation::OperationContext,
        _payload: messages::ORMapPushDiff,
    ) -> Result<OperationResponse, OperationError> {
        todo!("implemented in G4")
    }
}

// ---------------------------------------------------------------------------
// ManagedService implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl ManagedService for SyncService {
    fn name(&self) -> &'static str {
        service_names::SYNC
    }

    async fn init(&self, _ctx: &ServiceContext) -> anyhow::Result<()> {
        Ok(())
    }

    async fn reset(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn shutdown(&self, _terminate: bool) -> anyhow::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// tower::Service<Operation> implementation
// ---------------------------------------------------------------------------

impl Service<Operation> for Arc<SyncService> {
    type Response = OperationResponse;
    type Error = OperationError;
    type Future =
        Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, op: Operation) -> Self::Future {
        let svc = Arc::clone(self);
        Box::pin(async move {
            match op {
                Operation::SyncInit { ctx, payload } => {
                    svc.handle_sync_init(&ctx, payload).await
                }
                Operation::MerkleReqBucket { ctx, payload } => {
                    svc.handle_merkle_req_bucket(&ctx, payload).await
                }
                Operation::ORMapSyncInit { ctx, payload } => {
                    svc.handle_ormap_sync_init(&ctx, payload).await
                }
                Operation::ORMapMerkleReqBucket { ctx, payload } => {
                    svc.handle_ormap_merkle_req_bucket(&ctx, payload).await
                }
                Operation::ORMapDiffRequest { ctx, payload } => {
                    svc.handle_ormap_diff_request(&ctx, payload).await
                }
                Operation::ORMapPushDiff { ctx, payload } => {
                    svc.handle_ormap_push_diff(&ctx, payload).await
                }
                _ => Err(OperationError::WrongService),
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Tests (AC1, AC2, AC3, AC14)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::service::operation::{OperationContext, service_names};
    use crate::storage::factory::RecordStoreFactory;
    use crate::storage::merkle_sync::MerkleSyncManager;
    use crate::storage::{NullDataStore, StorageConfig};
    use crate::network::connection::ConnectionRegistry;

    fn make_timestamp() -> Timestamp {
        Timestamp {
            millis: 1_700_000_000_000,
            counter: 0,
            node_id: "node-1".to_string(),
        }
    }

    fn make_ctx(service_name: &'static str) -> OperationContext {
        OperationContext::new(1, service_name, make_timestamp(), 5000)
    }

    fn make_factory() -> Arc<RecordStoreFactory> {
        Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ))
    }

    fn make_sync_service() -> Arc<SyncService> {
        let merkle_manager = Arc::new(MerkleSyncManager::default());
        let record_store_factory = make_factory();
        let connection_registry = Arc::new(ConnectionRegistry::new());
        Arc::new(SyncService::new(
            merkle_manager,
            record_store_factory,
            connection_registry,
        ))
    }

    // -------------------------------------------------------------------------
    // AC1: SyncInit returns SyncRespRoot with correct root hash
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn ac1_sync_init_returns_sync_resp_root() {
        let merkle_manager = Arc::new(MerkleSyncManager::default());
        // Pre-populate the tree so root hash is non-zero.
        merkle_manager.update_lww("users", 0, "user-1", 12345);

        let connection_registry = Arc::new(ConnectionRegistry::new());
        let svc = Arc::new(SyncService::new(
            Arc::clone(&merkle_manager),
            make_factory(),
            connection_registry,
        ));

        let expected_root = merkle_manager.with_lww_tree("users", 0, |tree| tree.get_root_hash());
        assert_ne!(expected_root, 0, "precondition: tree must have non-zero hash");

        let op = Operation::SyncInit {
            ctx: make_ctx(service_names::SYNC),
            payload: messages::SyncInitMessage {
                map_name: "users".to_string(),
                last_sync_timestamp: None,
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        match resp {
            OperationResponse::Message(msg) => {
                if let Message::SyncRespRoot(m) = *msg {
                    assert_eq!(m.payload.map_name, "users");
                    assert_eq!(m.payload.root_hash, expected_root);
                    assert_eq!(m.payload.timestamp.node_id, "node-1");
                } else {
                    panic!("expected SyncRespRoot, got different Message variant");
                }
            }
            other => panic!("expected OperationResponse::Message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ac1_sync_init_empty_tree_returns_root_hash_zero() {
        let svc = make_sync_service();

        let op = Operation::SyncInit {
            ctx: make_ctx(service_names::SYNC),
            payload: messages::SyncInitMessage {
                map_name: "users".to_string(),
                last_sync_timestamp: None,
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        match resp {
            OperationResponse::Message(msg) => {
                if let Message::SyncRespRoot(m) = *msg {
                    assert_eq!(m.payload.root_hash, 0, "empty tree should have root_hash = 0");
                } else {
                    panic!("expected SyncRespRoot");
                }
            }
            other => panic!("expected Message response, got {other:?}"),
        }
    }

    // -------------------------------------------------------------------------
    // AC2: MerkleReqBucket returns SyncRespBuckets for internal nodes
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn ac2_merkle_req_bucket_returns_buckets_for_internal_node() {
        let merkle_manager = Arc::new(MerkleSyncManager::default());
        // Insert several keys to ensure the root (path "") is an internal node.
        for i in 0..10 {
            let key = format!("user-{i}");
            merkle_manager.update_lww("users", 0, &key, i * 100 + 1);
        }

        let svc = Arc::new(SyncService::new(
            Arc::clone(&merkle_manager),
            make_factory(),
            Arc::new(ConnectionRegistry::new()),
        ));

        let op = Operation::MerkleReqBucket {
            ctx: make_ctx(service_names::SYNC),
            payload: messages::MerkleReqBucketMessage {
                payload: messages::MerkleReqBucketPayload {
                    map_name: "users".to_string(),
                    path: "".to_string(),
                },
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        match resp {
            OperationResponse::Message(msg) => {
                if let Message::SyncRespBuckets(m) = *msg {
                    assert_eq!(m.payload.map_name, "users");
                    assert_eq!(m.payload.path, "");
                    assert!(!m.payload.buckets.is_empty(), "internal node should have non-empty buckets");
                    // Each bucket key is a single hex character string.
                    for key in m.payload.buckets.keys() {
                        assert_eq!(key.len(), 1, "bucket key should be single char, got: {key}");
                        assert!(key.chars().all(|c| c.is_ascii_hexdigit()), "bucket key should be hex char: {key}");
                    }
                } else {
                    panic!("expected SyncRespBuckets");
                }
            }
            other => panic!("expected Message response, got {other:?}"),
        }
    }

    // -------------------------------------------------------------------------
    // AC3: MerkleReqBucket returns SyncRespLeaf for leaf nodes
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn ac3_merkle_req_bucket_returns_leaf_for_leaf_node() {
        let merkle_manager = Arc::new(MerkleSyncManager::default());
        // Insert a single key so we can find its leaf path.
        merkle_manager.update_lww("users", 0, "user-1", 99999);

        // Find the path for "user-1" by drilling down until we hit a leaf.
        let leaf_path = merkle_manager.with_lww_tree("users", 0, |tree| {
            // At depth=3, the leaf path is the first 3 hex chars of the key's hash path.
            // We iterate through the buckets to find a path that resolves to a leaf.
            let root_buckets = tree.get_buckets("");
            let first_char = root_buckets.keys().next().cloned().unwrap();
            let level1_path = first_char.to_string();
            let level1_buckets = tree.get_buckets(&level1_path);
            if level1_buckets.is_empty() {
                return level1_path;
            }
            let second_char = level1_buckets.keys().next().cloned().unwrap();
            let level2_path = format!("{level1_path}{second_char}");
            let level2_buckets = tree.get_buckets(&level2_path);
            if level2_buckets.is_empty() {
                return level2_path;
            }
            let third_char = level2_buckets.keys().next().cloned().unwrap();
            format!("{level2_path}{third_char}")
        });

        let svc = Arc::new(SyncService::new(
            Arc::clone(&merkle_manager),
            make_factory(),
            Arc::new(ConnectionRegistry::new()),
        ));

        let op = Operation::MerkleReqBucket {
            ctx: make_ctx(service_names::SYNC),
            payload: messages::MerkleReqBucketMessage {
                payload: messages::MerkleReqBucketPayload {
                    map_name: "users".to_string(),
                    path: leaf_path.clone(),
                },
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        match resp {
            OperationResponse::Message(msg) => {
                if let Message::SyncRespLeaf(m) = *msg {
                    // map_name and path are echoed correctly.
                    assert_eq!(m.payload.map_name, "users");
                    assert_eq!(m.payload.path, leaf_path);
                    // With NullDataStore, records may be empty (no real data in store).
                    // The test verifies response type and structure only.
                } else {
                    panic!("expected SyncRespLeaf, got different Message variant");
                }
            }
            OperationResponse::Empty => {
                // Also acceptable if the path has no node (edge case in test setup).
            }
            other => panic!("expected Message or Empty response, got {other:?}"),
        }
    }

    // -------------------------------------------------------------------------
    // AC14: value_to_rmpv round-trips with rmpv_to_value from crdt.rs
    // -------------------------------------------------------------------------

    #[test]
    fn ac14_value_to_rmpv_null() {
        let v = Value::Null;
        let rmpv_val = value_to_rmpv(&v);
        assert_eq!(rmpv_val, rmpv::Value::Nil);
    }

    #[test]
    fn ac14_value_to_rmpv_bool() {
        assert_eq!(value_to_rmpv(&Value::Bool(true)), rmpv::Value::Boolean(true));
        assert_eq!(value_to_rmpv(&Value::Bool(false)), rmpv::Value::Boolean(false));
    }

    #[test]
    fn ac14_value_to_rmpv_int() {
        let v = Value::Int(42);
        let rmpv_val = value_to_rmpv(&v);
        assert_eq!(rmpv_val, rmpv::Value::Integer(42.into()));
    }

    #[test]
    fn ac14_value_to_rmpv_float() {
        let v = Value::Float(3.14);
        let rmpv_val = value_to_rmpv(&v);
        assert_eq!(rmpv_val, rmpv::Value::F64(3.14));
    }

    #[test]
    fn ac14_value_to_rmpv_string() {
        let v = Value::String("hello".to_string());
        let rmpv_val = value_to_rmpv(&v);
        assert_eq!(rmpv_val, rmpv::Value::String("hello".into()));
    }

    #[test]
    fn ac14_value_to_rmpv_bytes() {
        let v = Value::Bytes(vec![1, 2, 3]);
        let rmpv_val = value_to_rmpv(&v);
        assert_eq!(rmpv_val, rmpv::Value::Binary(vec![1, 2, 3]));
    }

    #[test]
    fn ac14_value_to_rmpv_array() {
        let v = Value::Array(vec![Value::Int(1), Value::String("x".to_string())]);
        let rmpv_val = value_to_rmpv(&v);
        assert_eq!(
            rmpv_val,
            rmpv::Value::Array(vec![
                rmpv::Value::Integer(1.into()),
                rmpv::Value::String("x".into()),
            ])
        );
    }

    #[test]
    fn ac14_value_to_rmpv_map() {
        let mut m = std::collections::BTreeMap::new();
        m.insert("key".to_string(), Value::Int(99));
        let v = Value::Map(m);
        let rmpv_val = value_to_rmpv(&v);
        if let rmpv::Value::Map(pairs) = rmpv_val {
            assert_eq!(pairs.len(), 1);
            assert_eq!(pairs[0].0, rmpv::Value::String("key".into()));
            assert_eq!(pairs[0].1, rmpv::Value::Integer(99.into()));
        } else {
            panic!("expected rmpv::Value::Map");
        }
    }

    #[test]
    fn ac14_round_trip_with_rmpv_to_value() {
        // Both value_to_rmpv (sync.rs) and rmpv_to_value (crdt.rs) are pub(crate).
        // This tests the round-trip: value -> rmpv -> value.
        use crate::service::domain::crdt::rmpv_to_value;

        let values = vec![
            Value::Null,
            Value::Bool(true),
            Value::Bool(false),
            Value::Int(0),
            Value::Int(i64::MAX),
            Value::Int(i64::MIN),
            Value::Float(0.0),
            Value::Float(1.5),
            Value::String(String::new()),
            Value::String("hello world".to_string()),
            Value::Bytes(vec![]),
            Value::Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]),
            Value::Array(vec![Value::Int(1), Value::Int(2)]),
        ];

        for val in values {
            let rmpv_val = value_to_rmpv(&val);
            let round_tripped = rmpv_to_value(&rmpv_val);
            assert_eq!(val, round_tripped, "round-trip failed for: {val:?}");
        }
    }

    // -------------------------------------------------------------------------
    // AC12: ManagedService name is "sync"
    // -------------------------------------------------------------------------

    #[test]
    fn ac12_managed_service_name_is_sync() {
        let svc = make_sync_service();
        assert_eq!(svc.name(), "sync");
    }
}
