//! Shape registry and service for partial replication subscriptions.
//!
//! `ShapeRegistry` is a `DashMap`-based concurrent data structure that tracks
//! which shapes are active on which connections, following the same pattern
//! as the existing `QueryRegistry`.
//!
//! `ShapeService` is a Tower `Service<Operation>` that handles shape
//! subscribe/unsubscribe lifecycle: scanning the map, evaluating records,
//! and sending the initial `ShapeResp` snapshot.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use dashmap::DashMap;
use tower::Service;

use topgun_core::hash::fnv1a_hash;
use topgun_core::messages::shape::{ShapeRecord, ShapeRespMessage, ShapeRespPayload};
use topgun_core::messages::{Message, SyncRespRootMessage, SyncRespRootPayload};
use topgun_core::schema::SyncShape;

use tracing::Instrument;

use crate::network::connection::{ConnectionRegistry, OutboundMessage};
use crate::service::domain::predicate::value_to_rmpv;
use crate::service::domain::shape_evaluator;
use crate::service::operation::{
    service_names, Operation, OperationError, OperationResponse,
};
use crate::service::registry::{ManagedService, ServiceContext};
use crate::storage::record::RecordValue;
use crate::storage::shape_merkle::ShapeMerkleSyncManager;
use crate::storage::RecordStoreFactory;

// ---------------------------------------------------------------------------
// ShapeService
// ---------------------------------------------------------------------------

/// Tower `Service<Operation>` that handles shape subscribe/unsubscribe lifecycle.
///
/// On `ShapeSubscribe`: reads all records for the target map, evaluates them
/// against the shape filter, registers the shape, and sends the initial snapshot.
/// On `ShapeUnsubscribe`: removes the shape from the registry.
pub struct ShapeService {
    shape_registry: Arc<ShapeRegistry>,
    record_store_factory: Arc<RecordStoreFactory>,
    connection_registry: Arc<ConnectionRegistry>,
    /// Per-shape Merkle manager used by `handle_shape_sync_init`.
    /// `None` when shape Merkle sync is not wired (passes `None` at unchanged call sites).
    shape_merkle_manager: Option<Arc<ShapeMerkleSyncManager>>,
}

impl ShapeService {
    /// Creates a new `ShapeService` with its required dependencies.
    ///
    /// Pass `Some(shape_merkle_manager)` to enable shape-aware Merkle sync init.
    /// Pass `None` to keep all existing call sites working unchanged.
    #[must_use]
    pub fn new(
        shape_registry: Arc<ShapeRegistry>,
        record_store_factory: Arc<RecordStoreFactory>,
        connection_registry: Arc<ConnectionRegistry>,
        shape_merkle_manager: Option<Arc<ShapeMerkleSyncManager>>,
    ) -> Self {
        Self {
            shape_registry,
            record_store_factory,
            connection_registry,
            shape_merkle_manager,
        }
    }

    /// Returns a reference to the underlying `ShapeRegistry`.
    #[must_use]
    pub fn registry(&self) -> &Arc<ShapeRegistry> {
        &self.shape_registry
    }
}

// ---------------------------------------------------------------------------
// ManagedService implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl ManagedService for ShapeService {
    fn name(&self) -> &'static str {
        service_names::SHAPE
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

impl Service<Operation> for Arc<ShapeService> {
    type Response = OperationResponse;
    type Error = OperationError;
    type Future =
        Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

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
                    Operation::ShapeSubscribe { ctx, payload } => {
                        svc.handle_shape_subscribe(&ctx, &payload)
                    }
                    Operation::ShapeUnsubscribe { ctx, payload } => {
                        svc.handle_shape_unsubscribe(&ctx, &payload)
                    }
                    Operation::ShapeSyncInit { ctx, payload } => {
                        svc.handle_shape_sync_init(&ctx, payload).await
                    }
                    _ => Err(OperationError::WrongService),
                }
            }
            .instrument(span),
        )
    }
}

// ---------------------------------------------------------------------------
// Handler implementations
// ---------------------------------------------------------------------------

impl ShapeService {
    /// Handles a `ShapeSubscribe` operation.
    ///
    /// Steps (ordered to avoid the race window where updates arrive before snapshot):
    /// 1. Extract `connection_id` from context.
    /// 2. Access the `SyncShape` from the payload.
    /// 3. Read + evaluate all records for the target map.
    /// 4. Apply limit if set.
    /// 5. Register the shape in `ShapeRegistry`.
    /// 6. Send `ShapeRespMessage` with matching records.
    fn handle_shape_subscribe(
        &self,
        ctx: &crate::service::operation::OperationContext,
        payload: &topgun_core::messages::shape::ShapeSubscribeMessage,
    ) -> Result<OperationResponse, OperationError> {
        let connection_id = ctx.connection_id.ok_or_else(|| {
            OperationError::Internal(anyhow::anyhow!(
                "ShapeSubscribe requires connection_id in OperationContext"
            ))
        })?;

        let shape = payload.payload.shape.clone();
        let shape_id = shape.shape_id.clone();
        let map_name = shape.map_name.clone();

        // Step 3: Read all records for the target map across all partitions.
        // Scan ALL partitions to aggregate the full key space.
        let stores = self.record_store_factory.get_all_for_map(&map_name);

        let mut matching_records: Vec<ShapeRecord> = Vec::new();
        let mut total_matches: usize = 0;

        for store in &stores {
            let partition_id = store.partition_id();
            // Collect (key, hash) pairs for Merkle tree init.
            let mut key_hash_pairs: Vec<(String, u32)> = Vec::new();

            store.for_each_boxed(
                &mut |key, record| {
                    if let RecordValue::Lww {
                        ref value,
                        ref timestamp,
                    } = record.value
                    {
                        // Convert storage Value to rmpv::Value for shape evaluation.
                        let rmpv_value = value_to_rmpv(value);
                        // Step 4: Evaluate against the shape (filter + project).
                        if let Some(projected) = shape_evaluator::apply_shape(&shape, &rmpv_value) {
                            total_matches += 1;

                            // Compute hash matching MerkleSyncManager's compute_lww_hash pattern.
                            let item_hash = fnv1a_hash(&format!(
                                "{}:{}:{}:{}",
                                key, timestamp.millis, timestamp.counter, timestamp.node_id
                            ));
                            key_hash_pairs.push((key.to_string(), item_hash));

                            // Step 5 (part 1): only collect up to limit.
                            let limit = shape.limit.map(|l| l as usize);
                            if limit.is_none_or(|l| matching_records.len() < l) {
                                matching_records.push(ShapeRecord {
                                    key: key.to_string(),
                                    value: projected,
                                });
                            }
                        }
                    }
                    // OR-Map records are skipped per Assumption 1 (shapes are LWW-only).
                },
                false, // not backup
            );

            // Populate per-shape Merkle tree for this partition.
            if let Some(ref merkle) = self.shape_merkle_manager {
                if !key_hash_pairs.is_empty() {
                    merkle.init_tree(&shape_id, &map_name, partition_id, &key_hash_pairs);
                }
            }
        }

        // Step 5 (part 2): determine has_more.
        let limit = shape.limit.map(|l| l as usize);
        let has_more = limit.and_then(|l| {
            if total_matches > l {
                Some(true)
            } else {
                None
            }
        });

        // Step 6: Register shape in registry.
        // Registration before send is the lesser evil: sending before registration
        // risks the client missing updates entirely. Clients must buffer shape
        // updates until the initial response arrives.
        self.shape_registry
            .register(shape_id.clone(), connection_id.0, shape)
            .map_err(|e| {
                OperationError::Internal(anyhow::anyhow!("Shape registration failed: {e}"))
            })?;

        // Step 7: Send ShapeRespMessage with matching records and aggregate Merkle hash.
        let merkle_root_hash = self
            .shape_merkle_manager
            .as_ref()
            .map_or(0, |m| m.aggregate_shape_root_hash(&shape_id, &map_name));

        let resp = Message::ShapeResp(ShapeRespMessage {
            payload: ShapeRespPayload {
                shape_id,
                records: matching_records,
                merkle_root_hash,
                has_more,
            },
        });

        if let Ok(bytes) = rmp_serde::to_vec_named(&resp) {
            if let Some(handle) = self.connection_registry.get(connection_id) {
                let _ = handle.try_send(OutboundMessage::Binary(bytes));
            }
        }

        Ok(OperationResponse::Empty)
    }

    /// Handles a `ShapeUnsubscribe` operation.
    #[allow(clippy::unnecessary_wraps)]
    fn handle_shape_unsubscribe(
        &self,
        _ctx: &crate::service::operation::OperationContext,
        payload: &topgun_core::messages::shape::ShapeUnsubscribeMessage,
    ) -> Result<OperationResponse, OperationError> {
        let _ = self.shape_registry.unregister(&payload.payload.shape_id);
        Ok(OperationResponse::Empty)
    }

    /// Handles a `ShapeSyncInit` operation.
    ///
    /// Client sends its stored shape Merkle root hash. The server computes the aggregate
    /// shape root hash across all partitions and responds with `SyncRespRootMessage`.
    ///
    /// If the hashes differ, the client drives traversal via `MerkleReqBucket` messages
    /// with shape-prefixed paths (e.g. `"<shape_id>/<partition_id>/<sub_path>"`), which
    /// are handled by `SyncService::handle_merkle_req_bucket`.
    ///
    /// Returns `OperationResponse::Empty` (no error) when shape Merkle sync is not wired
    /// (i.e. `shape_merkle_manager` is `None`) so unchanged call sites continue working.
    #[allow(clippy::unused_async)]
    async fn handle_shape_sync_init(
        &self,
        ctx: &crate::service::operation::OperationContext,
        payload: topgun_core::messages::shape::ShapeSyncInitMessage,
    ) -> Result<OperationResponse, OperationError> {
        let Some(shape_merkle) = self.shape_merkle_manager.as_ref() else {
            return Ok(OperationResponse::Empty);
        };

        let shape_id = payload.payload.shape_id;

        // Resolve map_name for the shape from the registry.
        let Some(active_shape) = self.shape_registry.get(&shape_id) else {
            return Ok(OperationResponse::Empty);
        };
        let map_name = active_shape.shape.map_name.clone();

        // Compute the server's aggregate shape root hash across all partitions.
        let root_hash = shape_merkle.aggregate_shape_root_hash(&shape_id, &map_name);

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
}

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// An active shape subscription associated with a connection.
#[derive(Debug, Clone)]
pub struct ActiveShape {
    /// The shape definition (includes `map_name`, `filter`, `fields`, `limit`).
    pub shape: SyncShape,
    /// The connection that registered this shape.
    pub connection_id: u64,
}

/// Errors that can occur when interacting with the shape registry.
#[derive(Debug, thiserror::Error)]
pub enum ShapeRegistryError {
    /// The `shape_id` is already registered.
    #[error("Shape ID already registered: {0}")]
    DuplicateShapeId(String),
}

// ---------------------------------------------------------------------------
// ShapeRegistry
// ---------------------------------------------------------------------------

/// Concurrent registry tracking active shapes keyed by `shape_id`.
///
/// Uses `DashMap` for lock-free concurrent access. Shapes are registered
/// per-connection and can be queried by `shape_id`, `map_name`, or `connection_id`.
pub struct ShapeRegistry {
    shapes: DashMap<String, ActiveShape>,
}

impl ShapeRegistry {
    /// Creates a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            shapes: DashMap::new(),
        }
    }

    /// Registers a shape for a connection.
    ///
    /// Returns an error if a shape with the same `shape_id` is already registered.
    ///
    /// # Note
    ///
    /// `shape_id` must not be a 3-digit numeric string (e.g. `"042"`). Such IDs
    /// collide with partition path prefixes in Merkle sync bucket traversal, causing
    /// `parse_partition_prefix` to misidentify shape-prefixed paths as regular
    /// partition paths. In practice, `shape_id` values are client-generated UUIDs or
    /// prefixed strings (e.g. `"s-1"`), which are not affected.
    ///
    /// # Errors
    ///
    /// Returns `ShapeRegistryError::DuplicateShapeId` if the `shape_id` already exists.
    pub fn register(
        &self,
        shape_id: String,
        connection_id: u64,
        shape: SyncShape,
    ) -> Result<(), ShapeRegistryError> {
        use dashmap::mapref::entry::Entry;

        match self.shapes.entry(shape_id.clone()) {
            Entry::Occupied(_) => Err(ShapeRegistryError::DuplicateShapeId(shape_id)),
            Entry::Vacant(entry) => {
                entry.insert(ActiveShape {
                    shape,
                    connection_id,
                });
                Ok(())
            }
        }
    }

    /// Removes and returns a shape by its `shape_id`.
    #[must_use]
    pub fn unregister(&self, shape_id: &str) -> Option<ActiveShape> {
        self.shapes.remove(shape_id).map(|(_, v)| v)
    }

    /// Removes all shapes for a given connection, returning the removed `shape_id`s.
    #[must_use]
    pub fn unregister_all_for_connection(&self, connection_id: u64) -> Vec<String> {
        let shape_ids: Vec<String> = self
            .shapes
            .iter()
            .filter(|entry| entry.value().connection_id == connection_id)
            .map(|entry| entry.key().clone())
            .collect();

        for id in &shape_ids {
            self.shapes.remove(id);
        }

        shape_ids
    }

    /// Returns all active shapes targeting a specific map.
    ///
    /// Performs a linear scan of all registered shapes. Acceptable for
    /// small shape counts (tens to low hundreds per server).
    #[must_use]
    pub fn shapes_for_map(&self, map_name: &str) -> Vec<(String, ActiveShape)> {
        self.shapes
            .iter()
            .filter(|entry| entry.value().shape.map_name == map_name)
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    /// Returns all shapes for a specific connection.
    #[must_use]
    pub fn shapes_for_connection(&self, connection_id: u64) -> Vec<(String, ActiveShape)> {
        self.shapes
            .iter()
            .filter(|entry| entry.value().connection_id == connection_id)
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    /// Looks up a shape by its `shape_id`.
    #[must_use]
    pub fn get(&self, shape_id: &str) -> Option<ActiveShape> {
        self.shapes.get(shape_id).map(|entry| entry.value().clone())
    }
}

impl Default for ShapeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a SyncShape with the given map_name.
    fn make_shape(map_name: &str) -> SyncShape {
        SyncShape {
            shape_id: String::new(), // shape_id is tracked by registry key, not struct field
            map_name: map_name.to_string(),
            ..SyncShape::default()
        }
    }

    #[test]
    fn register_and_retrieve() {
        let reg = ShapeRegistry::new();
        let shape = make_shape("users");
        reg.register("s1".into(), 100, shape.clone()).unwrap();

        let active = reg.get("s1").unwrap();
        assert_eq!(active.shape.map_name, "users");
        assert_eq!(active.connection_id, 100);
    }

    #[test]
    fn register_duplicate_returns_error() {
        let reg = ShapeRegistry::new();
        reg.register("s1".into(), 100, make_shape("users")).unwrap();

        let err = reg.register("s1".into(), 200, make_shape("posts")).unwrap_err();
        assert!(
            matches!(err, ShapeRegistryError::DuplicateShapeId(id) if id == "s1")
        );
    }

    #[test]
    fn unregister_returns_removed_shape() {
        let reg = ShapeRegistry::new();
        reg.register("s1".into(), 100, make_shape("users")).unwrap();

        let removed = reg.unregister("s1").unwrap();
        assert_eq!(removed.shape.map_name, "users");
        assert_eq!(removed.connection_id, 100);

        // Should be gone now
        assert!(reg.get("s1").is_none());
    }

    #[test]
    fn unregister_nonexistent_returns_none() {
        let reg = ShapeRegistry::new();
        assert!(reg.unregister("nonexistent").is_none());
    }

    #[test]
    fn unregister_all_for_connection() {
        let reg = ShapeRegistry::new();
        reg.register("s1".into(), 100, make_shape("users")).unwrap();
        reg.register("s2".into(), 100, make_shape("posts")).unwrap();
        reg.register("s3".into(), 200, make_shape("users")).unwrap();

        let mut removed = reg.unregister_all_for_connection(100);
        removed.sort(); // DashMap iteration order is non-deterministic
        assert_eq!(removed, vec!["s1", "s2"]);

        // Connection 100 shapes should be gone
        assert!(reg.get("s1").is_none());
        assert!(reg.get("s2").is_none());

        // Connection 200 shape should remain
        assert!(reg.get("s3").is_some());
    }

    #[test]
    fn shapes_for_map_filters_correctly() {
        let reg = ShapeRegistry::new();
        reg.register("s1".into(), 100, make_shape("users")).unwrap();
        reg.register("s2".into(), 200, make_shape("posts")).unwrap();
        reg.register("s3".into(), 300, make_shape("users")).unwrap();

        let user_shapes = reg.shapes_for_map("users");
        assert_eq!(user_shapes.len(), 2);

        let ids: Vec<&str> = user_shapes.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.contains(&"s1"));
        assert!(ids.contains(&"s3"));
    }

    #[test]
    fn shapes_for_connection_filters_correctly() {
        let reg = ShapeRegistry::new();
        reg.register("s1".into(), 100, make_shape("users")).unwrap();
        reg.register("s2".into(), 100, make_shape("posts")).unwrap();
        reg.register("s3".into(), 200, make_shape("users")).unwrap();

        let conn_shapes = reg.shapes_for_connection(100);
        assert_eq!(conn_shapes.len(), 2);

        let ids: Vec<&str> = conn_shapes.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.contains(&"s1"));
        assert!(ids.contains(&"s2"));
    }

    #[test]
    fn multiple_shapes_different_connections_same_map() {
        let reg = ShapeRegistry::new();
        reg.register("s1".into(), 100, make_shape("users")).unwrap();
        reg.register("s2".into(), 200, make_shape("users")).unwrap();
        reg.register("s3".into(), 300, make_shape("users")).unwrap();

        let shapes = reg.shapes_for_map("users");
        assert_eq!(shapes.len(), 3);

        // Each should have a different connection_id
        let conn_ids: Vec<u64> = shapes.iter().map(|(_, s)| s.connection_id).collect();
        assert!(conn_ids.contains(&100));
        assert!(conn_ids.contains(&200));
        assert!(conn_ids.contains(&300));
    }
}
