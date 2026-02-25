//! Sync domain service implementing Merkle delta sync protocol.
//!
//! Handles 6 sync-related `Operation` variants across two CRDT types
//! (LWW-Map and OR-Map), using `MerkleSyncManager` for per-partition tree
//! management and `RecordStoreFactory` for record access.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use tower::Service;

use crate::network::connection::ConnectionRegistry;
use crate::service::operation::{
    service_names, Operation, OperationError, OperationResponse,
};
use crate::service::registry::{ManagedService, ServiceContext};
use crate::storage::merkle_sync::MerkleSyncManager;
use crate::storage::RecordStoreFactory;

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
        // Handler bodies implemented in G3 (LWW) and G4 (OR-Map).
        let _ = op;
        Box::pin(async move { Err(OperationError::WrongService) })
    }
}
