//! Persistence domain service for counters, journal, entry processing, and
//! conflict resolvers.
//!
//! Handles 10 `Operation` variants grouped into four sub-domains:
//! - **PN-Counter CRDT** (2 ops): `CounterRequest`, `CounterSync`
//! - **Journal** (3 ops): `JournalSubscribe`, `JournalUnsubscribe`, `JournalRead`
//! - **Entry Processing** (2 ops): `EntryProcess`, `EntryProcessBatch` (stub -- WASM sandbox required)
//! - **Conflict Resolvers** (3 ops): `RegisterResolver`, `UnregisterResolver`, `ListResolvers` (stub -- WASM sandbox required)

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use tower::Service;

use topgun_core::messages::{
    CounterRequestPayload, CounterStatePayload, EntryProcessBatchData,
    EntryProcessBatchResponseData, EntryProcessData, EntryProcessKeyResult,
    EntryProcessResponseData, JournalReadData, JournalReadResponseData, JournalSubscribeData,
    JournalUnsubscribeData, ListResolversData, ListResolversResponseData, Message,
    RegisterResolverData, RegisterResolverResponseData, UnregisterResolverData,
    UnregisterResolverResponseData,
};

use crate::network::connection::{ConnectionRegistry, OutboundMessage};
use crate::service::domain::counter::CounterRegistry;
use crate::service::domain::journal::{JournalStore, JournalSubscription};
use crate::service::operation::{
    service_names, Operation, OperationContext, OperationError, OperationResponse,
};
use crate::service::registry::{ManagedService, ServiceContext};

/// Error message returned by all stub handlers that require WASM sandbox.
const WASM_SANDBOX_ERROR: &str = "Entry processing not available: WASM sandbox required";
const WASM_RESOLVER_ERROR: &str = "Conflict resolvers not available: WASM sandbox required";

/// Default journal ring buffer capacity.
const DEFAULT_JOURNAL_CAPACITY: usize = 10_000;

// ---------------------------------------------------------------------------
// PersistenceService
// ---------------------------------------------------------------------------

/// Real persistence domain service replacing the `domain_stub!` macro.
///
/// Fully implements Counter and Journal operations. Entry Processing and
/// Resolver operations return structured error responses (sandbox not available).
pub struct PersistenceService {
    counter_registry: Arc<CounterRegistry>,
    journal_store: Arc<JournalStore>,
    connection_registry: Arc<ConnectionRegistry>,
}

impl PersistenceService {
    /// Creates a new `PersistenceService` with its required dependencies.
    #[must_use]
    pub fn new(connection_registry: Arc<ConnectionRegistry>, node_id: String) -> Self {
        Self {
            counter_registry: Arc::new(CounterRegistry::new(node_id)),
            journal_store: Arc::new(JournalStore::new(DEFAULT_JOURNAL_CAPACITY)),
            connection_registry,
        }
    }

    /// Returns a reference to the inner counter registry (for testing).
    #[must_use]
    pub fn counter_registry(&self) -> &CounterRegistry {
        &self.counter_registry
    }

    /// Returns a reference to the inner journal store (for testing).
    #[must_use]
    pub fn journal_store(&self) -> &JournalStore {
        &self.journal_store
    }
}

// ---------------------------------------------------------------------------
// ManagedService implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl ManagedService for PersistenceService {
    fn name(&self) -> &'static str {
        service_names::PERSISTENCE
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

impl Service<Operation> for Arc<PersistenceService> {
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
                // Counter operations
                Operation::CounterRequest { ctx, payload } => {
                    svc.handle_counter_request(&ctx, &payload)
                }
                Operation::CounterSync { ctx, payload } => {
                    svc.handle_counter_sync(&ctx, &payload)
                }

                // Journal operations
                Operation::JournalSubscribe { ctx, payload } => {
                    svc.handle_journal_subscribe(&ctx, &payload)
                }
                Operation::JournalUnsubscribe { ctx: _, payload } => {
                    svc.handle_journal_unsubscribe(&payload)
                }
                Operation::JournalRead { ctx: _, payload } => {
                    svc.handle_journal_read(&payload)
                }

                // Entry processing stubs (WASM sandbox required)
                Operation::EntryProcess { ctx: _, payload } => {
                    svc.handle_entry_process(&payload)
                }
                Operation::EntryProcessBatch { ctx: _, payload } => {
                    svc.handle_entry_process_batch(&payload)
                }

                // Conflict resolver stubs (WASM sandbox required)
                Operation::RegisterResolver { ctx: _, payload } => {
                    svc.handle_register_resolver(&payload)
                }
                Operation::UnregisterResolver { ctx: _, payload } => {
                    svc.handle_unregister_resolver(&payload)
                }
                Operation::ListResolvers { ctx: _, payload } => {
                    svc.handle_list_resolvers(&payload)
                }

                // Not a persistence operation
                _ => Err(OperationError::WrongService),
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Handler implementations
// ---------------------------------------------------------------------------

#[allow(clippy::unnecessary_wraps, clippy::unused_self)]
impl PersistenceService {
    // --- Counter handlers ---

    /// Handles `CounterRequest`: returns the current state of the named counter,
    /// creating it if absent. Auto-subscribes the caller's connection.
    fn handle_counter_request(
        &self,
        ctx: &OperationContext,
        payload: &CounterRequestPayload,
    ) -> Result<OperationResponse, OperationError> {
        let state = self.counter_registry.get_or_create(&payload.name);

        // Auto-subscribe the caller if connection_id is available.
        if let Some(conn_id) = ctx.connection_id {
            self.counter_registry.subscribe(&payload.name, conn_id);
        }

        Ok(OperationResponse::Message(Box::new(
            Message::CounterResponse {
                payload: CounterStatePayload {
                    name: payload.name.clone(),
                    state,
                },
            },
        )))
    }

    /// Handles `CounterSync`: merges incoming state using max-per-node semantics,
    /// auto-subscribes the caller, broadcasts `CounterUpdate` to other subscribers,
    /// and returns the merged state to the sender.
    fn handle_counter_sync(
        &self,
        ctx: &OperationContext,
        payload: &CounterStatePayload,
    ) -> Result<OperationResponse, OperationError> {
        let merged = self.counter_registry.merge(&payload.name, &payload.state);

        // Auto-subscribe the caller if connection_id is available.
        if let Some(conn_id) = ctx.connection_id {
            self.counter_registry.subscribe(&payload.name, conn_id);
        }

        // Build the update message for broadcasting.
        let update_msg = Message::CounterUpdate {
            payload: CounterStatePayload {
                name: payload.name.clone(),
                state: merged.clone(),
            },
        };

        // Serialize for broadcasting to other subscribers.
        let bytes = rmp_serde::to_vec_named(&update_msg)
            .map_err(|e| OperationError::Internal(anyhow::anyhow!("serialize error: {e}")))?;

        // Broadcast to all subscribers except the sender.
        let sender_conn_id = ctx.connection_id;
        let subscribers = self.counter_registry.subscribers(&payload.name);
        for sub_conn_id in subscribers {
            if Some(sub_conn_id) == sender_conn_id {
                continue;
            }
            if let Some(handle) = self.connection_registry.get(sub_conn_id) {
                // Best-effort delivery: skip full channels.
                let _ = handle.try_send(OutboundMessage::Binary(bytes.clone()));
            }
        }

        // Return the merged state to the sender.
        Ok(OperationResponse::Message(Box::new(
            Message::CounterUpdate {
                payload: CounterStatePayload {
                    name: payload.name.clone(),
                    state: merged,
                },
            },
        )))
    }

    // --- Journal handlers ---

    /// Handles `JournalSubscribe`: registers subscription metadata and returns
    /// `Empty`. Active push delivery is deferred.
    fn handle_journal_subscribe(
        &self,
        ctx: &OperationContext,
        payload: &JournalSubscribeData,
    ) -> Result<OperationResponse, OperationError> {
        // Use request_id as the subscription ID.
        let conn_id = ctx.connection_id.ok_or_else(|| {
            OperationError::Internal(anyhow::anyhow!(
                "JournalSubscribe requires a connection_id"
            ))
        })?;

        let sub = JournalSubscription {
            connection_id: conn_id,
            map_name: payload.map_name.clone(),
            types: payload.types.clone(),
        };

        self.journal_store.subscribe(payload.request_id.clone(), sub);

        Ok(OperationResponse::Empty)
    }

    /// Handles `JournalUnsubscribe`: removes the subscription and returns `Empty`.
    fn handle_journal_unsubscribe(
        &self,
        payload: &JournalUnsubscribeData,
    ) -> Result<OperationResponse, OperationError> {
        self.journal_store.unsubscribe(&payload.subscription_id);
        Ok(OperationResponse::Empty)
    }

    /// Handles `JournalRead`: reads events from the ring buffer with optional
    /// `map_name` filter and returns a paginated response.
    fn handle_journal_read(
        &self,
        payload: &JournalReadData,
    ) -> Result<OperationResponse, OperationError> {
        // Parse from_sequence from string to u64.
        let from_sequence: u64 = payload.from_sequence.parse().unwrap_or(0);
        let limit = payload.limit.unwrap_or(100);

        let (events, has_more) =
            self.journal_store
                .read(from_sequence, limit, payload.map_name.as_deref());

        Ok(OperationResponse::Message(Box::new(
            Message::JournalReadResponse(JournalReadResponseData {
                request_id: payload.request_id.clone(),
                events,
                has_more,
            }),
        )))
    }

    // --- Entry processing stubs ---

    /// Handles `EntryProcess`: returns a "not implemented" error response.
    fn handle_entry_process(
        &self,
        payload: &EntryProcessData,
    ) -> Result<OperationResponse, OperationError> {
        Ok(OperationResponse::Message(Box::new(
            Message::EntryProcessResponse(EntryProcessResponseData {
                request_id: payload.request_id.clone(),
                success: false,
                result: None,
                new_value: None,
                error: Some(WASM_SANDBOX_ERROR.to_string()),
            }),
        )))
    }

    /// Handles `EntryProcessBatch`: returns per-key "not implemented" errors.
    fn handle_entry_process_batch(
        &self,
        payload: &EntryProcessBatchData,
    ) -> Result<OperationResponse, OperationError> {
        let results: HashMap<String, EntryProcessKeyResult> = payload
            .keys
            .iter()
            .map(|key| {
                (
                    key.clone(),
                    EntryProcessKeyResult {
                        success: false,
                        result: None,
                        new_value: None,
                        error: Some(WASM_SANDBOX_ERROR.to_string()),
                    },
                )
            })
            .collect();

        Ok(OperationResponse::Message(Box::new(
            Message::EntryProcessBatchResponse(EntryProcessBatchResponseData {
                request_id: payload.request_id.clone(),
                results,
            }),
        )))
    }

    // --- Conflict resolver stubs ---

    /// Handles `RegisterResolver`: returns a "not implemented" error response.
    fn handle_register_resolver(
        &self,
        payload: &RegisterResolverData,
    ) -> Result<OperationResponse, OperationError> {
        Ok(OperationResponse::Message(Box::new(
            Message::RegisterResolverResponse(RegisterResolverResponseData {
                request_id: payload.request_id.clone(),
                success: false,
                error: Some(WASM_RESOLVER_ERROR.to_string()),
            }),
        )))
    }

    /// Handles `UnregisterResolver`: returns a "not implemented" error response.
    fn handle_unregister_resolver(
        &self,
        payload: &UnregisterResolverData,
    ) -> Result<OperationResponse, OperationError> {
        Ok(OperationResponse::Message(Box::new(
            Message::UnregisterResolverResponse(UnregisterResolverResponseData {
                request_id: payload.request_id.clone(),
                success: false,
                error: Some(WASM_RESOLVER_ERROR.to_string()),
            }),
        )))
    }

    /// Handles `ListResolvers`: returns an empty resolver list.
    fn handle_list_resolvers(
        &self,
        payload: &ListResolversData,
    ) -> Result<OperationResponse, OperationError> {
        Ok(OperationResponse::Message(Box::new(
            Message::ListResolversResponse(ListResolversResponseData {
                request_id: payload.request_id.clone(),
                resolvers: vec![],
            }),
        )))
    }
}
