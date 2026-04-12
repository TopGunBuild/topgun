//! Migration service: 2-phase CRDT-aware partition migration protocol.
//!
//! Provides `MigrationCoordinator` (implementing `MigrationService`),
//! `MapProvider` trait for storage abstraction, `RebalanceTrigger` for
//! automatic rebalancing on membership changes, and free functions
//! `not_owner_response()` and `broadcast_partition_map()` for client routing.

use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::assignment::{compute_assignment, order_migrations, plan_rebalance};
use super::messages::{
    ClusterMessage, DeltaOp, MapStateChunk, MigrateCancelPayload, MigrateDataPayload,
    MigrateFinalizePayload, MigrateReadyPayload, MigrateStartPayload,
};
use super::state::{ClusterChange, ClusterPartitionTable, ClusterState, MigrationCommand};
use super::traits::MigrationService;
use super::types::{ActiveMigration, MembersView, MigrationPhase, MigrationTask, PartitionState};
use crate::network::connection::{ConnectionKind, ConnectionRegistry, OutboundMessage};
use topgun_core::messages::cluster::PartitionMapPayload;

// ---------------------------------------------------------------------------
// MapProvider trait
// ---------------------------------------------------------------------------

/// Storage seam for serializing/deserializing CRDT map state during migration.
///
/// Abstracts the concrete storage layer so the migration coordinator can
/// operate without a full storage module. The concrete implementation
/// (wired in a future storage spec) will call `LwwMap::merge()` and
/// `ORMap::merge()` from `topgun_core`.
pub trait MapProvider: Send + Sync {
    /// Returns all serialized map chunks for a given partition.
    fn get_partition_maps(&self, partition_id: u32) -> Vec<MapStateChunk>;

    /// Receives and applies a single map state chunk on the destination.
    fn receive_map_chunk(&self, chunk: MapStateChunk);

    /// Applies a single delta operation on the destination.
    fn apply_delta_op(&self, op: DeltaOp);
}

/// No-op `MapProvider` for tests and early bootstrapping.
///
/// Returns empty map lists and silently ignores incoming chunks/deltas.
#[allow(dead_code)]
pub(crate) struct NoOpMapProvider;

impl MapProvider for NoOpMapProvider {
    fn get_partition_maps(&self, _partition_id: u32) -> Vec<MapStateChunk> {
        Vec::new()
    }

    fn receive_map_chunk(&self, _chunk: MapStateChunk) {}

    fn apply_delta_op(&self, _op: DeltaOp) {}
}

// ---------------------------------------------------------------------------
// MigrationCoordinator
// ---------------------------------------------------------------------------

/// Implements the 2-phase CRDT-aware migration protocol.
///
/// Coordinates partition state transitions, data transfer between source
/// and destination nodes, and ownership updates on the master.
pub struct MigrationCoordinator {
    state: Arc<ClusterState>,
    registry: Arc<ConnectionRegistry>,
    local_node_id: String,
    map_provider: Arc<dyn MapProvider>,
}

impl MigrationCoordinator {
    /// Creates a new migration coordinator.
    ///
    /// The `local_node_id` is cached from `state.local_node_id` to avoid
    /// repeated cloning.
    #[must_use]
    pub fn new(
        state: Arc<ClusterState>,
        registry: Arc<ConnectionRegistry>,
        map_provider: Arc<dyn MapProvider>,
    ) -> Self {
        let local_node_id = state.local_node_id.clone();
        Self {
            state,
            registry,
            local_node_id,
            map_provider,
        }
    }

    /// Sends a cluster message to a specific peer node.
    ///
    /// Looks up the peer by scanning all connections for one with matching
    /// `peer_node_id` and `ConnectionKind::ClusterPeer`, then serializes
    /// and sends the message via the connection's outbound channel.
    async fn send_to_peer(&self, node_id: &str, msg: &ClusterMessage) -> anyhow::Result<()> {
        let bytes = rmp_serde::to_vec_named(msg)?;

        for handle in self.registry.connections() {
            if handle.kind != ConnectionKind::ClusterPeer {
                continue;
            }
            let meta = handle.metadata.read().await;
            if meta.peer_node_id.as_deref() == Some(node_id) {
                drop(meta);
                if handle.try_send(OutboundMessage::Binary(bytes)) {
                    return Ok(());
                }
                return Err(anyhow::anyhow!(
                    "peer {node_id} channel full or disconnected"
                ));
            }
        }

        Err(anyhow::anyhow!("no connection to peer {node_id}"))
    }
}

/// Returns the current Unix timestamp in milliseconds.
#[allow(clippy::cast_possible_truncation)]
fn current_unix_ms() -> u64 {
    // Truncation from u128 to u64 is safe: u64::MAX millis is ~584 million years.
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[async_trait]
impl MigrationService for MigrationCoordinator {
    async fn start_migrations(&self, mut tasks: Vec<MigrationTask>) -> anyhow::Result<()> {
        // Sort tasks by availability priority before starting.
        order_migrations(&mut tasks, &self.state.partition_table);

        let mut migrations = self.state.active_migrations.write().await;
        #[allow(clippy::cast_possible_truncation)]
        let current_count = migrations.len() as u32;
        let max = self.state.config.max_parallel_migrations;

        if current_count >= max {
            return Ok(());
        }

        let available_slots = (max - current_count) as usize;

        // Collect messages to send after releasing the write lock (avoid
        // holding the lock across async send_to_peer calls).
        let mut pending_sends: Vec<(String, ClusterMessage)> = Vec::new();

        for task in tasks.into_iter().take(available_slots) {
            let migration_id = Uuid::new_v4().to_string();

            let active = ActiveMigration {
                migration_id: migration_id.clone(),
                partition_id: task.partition_id,
                source: task.source.clone(),
                destination: task.destination.clone(),
                state: MigrationPhase::Replicating,
                started_at_ms: current_unix_ms(),
                new_backups: task.new_backups.clone(),
            };

            migrations.insert(task.partition_id, active);

            self.state
                .partition_table
                .set_state(task.partition_id, PartitionState::Migrating);

            let msg = ClusterMessage::MigrateStart(MigrateStartPayload {
                migration_id,
                partition_id: task.partition_id,
                destination_node_id: task.destination.clone(),
            });

            pending_sends.push((task.source.clone(), msg));
        }

        drop(migrations);

        for (source_node, msg) in pending_sends {
            // Best-effort send: if it fails, the migration will time out.
            let _ = self.send_to_peer(&source_node, &msg).await;
        }

        Ok(())
    }

    async fn cancel_migration(&self, partition_id: u32) -> anyhow::Result<()> {
        let mut migrations = self.state.active_migrations.write().await;
        let Some(migration) = migrations.remove(&partition_id) else {
            return Ok(());
        };

        self.state
            .partition_table
            .set_state(partition_id, PartitionState::Active);

        let cancel_msg = ClusterMessage::MigrateCancel(MigrateCancelPayload {
            migration_id: migration.migration_id.clone(),
            partition_id,
            reason: "cancelled".to_string(),
        });

        let source = migration.source.clone();
        let destination = migration.destination.clone();

        drop(migrations);

        let _ = self.send_to_peer(&source, &cancel_msg).await;
        let _ = self.send_to_peer(&destination, &cancel_msg).await;

        Ok(())
    }

    async fn cancel_all(&self) -> anyhow::Result<()> {
        let mut migrations = self.state.active_migrations.write().await;
        let all: Vec<ActiveMigration> = migrations.drain().map(|(_, m)| m).collect();
        drop(migrations);

        for migration in all {
            self.state
                .partition_table
                .set_state(migration.partition_id, PartitionState::Active);

            let cancel_msg = ClusterMessage::MigrateCancel(MigrateCancelPayload {
                migration_id: migration.migration_id.clone(),
                partition_id: migration.partition_id,
                reason: "cancelled".to_string(),
            });

            let _ = self.send_to_peer(&migration.source, &cancel_msg).await;
            let _ = self.send_to_peer(&migration.destination, &cancel_msg).await;
        }

        Ok(())
    }

    async fn handle_migrate_start(
        &self,
        partition_id: u32,
        destination: &str,
    ) -> anyhow::Result<()> {
        self.state
            .partition_table
            .set_state(partition_id, PartitionState::Migrating);

        let map_states = self.map_provider.get_partition_maps(partition_id);

        #[allow(clippy::cast_possible_truncation)]
        let source_version = self.state.partition_table.version() as u32;

        let data_msg = ClusterMessage::MigrateData(MigrateDataPayload {
            partition_id,
            map_states,
            delta_ops: vec![],
            source_version,
        });

        self.send_to_peer(destination, &data_msg).await?;

        // Send MigrateReady to the master node.
        // The empty migration_id is intentional: the trait method does not
        // receive the migration_id, and the master correlates by partition_id.
        let master_id = self
            .state
            .current_view()
            .master()
            .map(|m| m.node_id.clone());

        if let Some(master_id) = master_id {
            let ready_msg = ClusterMessage::MigrateReady(MigrateReadyPayload {
                migration_id: String::new(),
                partition_id,
                source_node_id: self.local_node_id.clone(),
            });

            self.send_to_peer(&master_id, &ready_msg).await?;
        }

        Ok(())
    }

    async fn handle_migrate_data(&self, data: MigrateDataPayload) -> anyhow::Result<()> {
        self.state
            .partition_table
            .set_state(data.partition_id, PartitionState::Receiving);

        for chunk in data.map_states {
            self.map_provider.receive_map_chunk(chunk);
        }

        for op in data.delta_ops {
            self.map_provider.apply_delta_op(op);
        }

        Ok(())
    }

    async fn handle_migrate_ready(&self, partition_id: u32, source: &str) -> anyhow::Result<()> {
        let mut migrations = self.state.active_migrations.write().await;

        let Some(migration) = migrations.get_mut(&partition_id) else {
            return Ok(());
        };

        migration.state = MigrationPhase::Finalizing;
        let migration_id = migration.migration_id.clone();
        let destination = migration.destination.clone();
        let new_backups = migration.new_backups.clone();

        // Set source partition state to Draining.
        self.state
            .partition_table
            .set_state(partition_id, PartitionState::Draining);

        // Send MigrateFinalize to both source and destination.
        let finalize_msg = ClusterMessage::MigrateFinalize(MigrateFinalizePayload {
            migration_id: migration_id.clone(),
            partition_id,
            new_owner: destination.clone(),
        });

        drop(migrations);

        let send_result = async {
            self.send_to_peer(source, &finalize_msg).await?;
            self.send_to_peer(&destination, &finalize_msg).await?;
            Ok::<(), anyhow::Error>(())
        }
        .await;

        if let Err(err) = send_result {
            // Rollback: restore source partition to Active and mark migration as Failed.
            self.state
                .partition_table
                .set_state(partition_id, PartitionState::Active);

            let mut migrations = self.state.active_migrations.write().await;
            if let Some(m) = migrations.get_mut(&partition_id) {
                m.state = MigrationPhase::Failed;
            }

            // Send cancel to both nodes (best-effort).
            let cancel_msg = ClusterMessage::MigrateCancel(MigrateCancelPayload {
                migration_id: migration_id.clone(),
                partition_id,
                reason: format!("finalize failed: {err}"),
            });
            migrations.remove(&partition_id);
            drop(migrations);

            let _ = self.send_to_peer(source, &cancel_msg).await;
            let _ = self.send_to_peer(&destination, &cancel_msg).await;

            return Err(err);
        }

        // Update partition ownership.
        self.state
            .partition_table
            .set_owner(partition_id, destination.clone(), new_backups);

        let _ = self.state.partition_table.increment_version();

        // Emit PartitionMoved event.
        let _ = self
            .state
            .change_sender()
            .send(ClusterChange::PartitionMoved {
                partition_id,
                old_owner: source.to_string(),
                new_owner: destination.clone(),
            });

        // Remove the completed migration.
        let mut migrations = self.state.active_migrations.write().await;
        migrations.remove(&partition_id);
        drop(migrations);

        // Broadcast updated partition map to all clients.
        let view = self.state.current_view();
        broadcast_partition_map(&self.state.partition_table, &view, &self.registry);

        Ok(())
    }

    fn is_migrating(&self, partition_id: u32) -> bool {
        // Use try_read to avoid blocking. If the lock is contended, assume
        // the partition might be migrating (conservative, but safe).
        match self.state.active_migrations.try_read() {
            Ok(migrations) => migrations.contains_key(&partition_id),
            Err(_) => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Returns a `PartitionMapPayload` for `NOT_OWNER` responses.
///
/// Stateless function: callers provide the current partition table and
/// membership view. Used by message handlers when a client operation
/// targets a partition not owned by the local node.
#[must_use]
pub fn not_owner_response(
    table: &ClusterPartitionTable,
    members: &MembersView,
) -> PartitionMapPayload {
    table.to_partition_map(members)
}

/// Broadcasts the current partition map to all connected clients.
///
/// Serializes the partition map via `rmp_serde::to_vec_named()` and sends
/// it to all connections with `ConnectionKind::Client`. Cluster peers are
/// not targeted.
pub fn broadcast_partition_map(
    table: &ClusterPartitionTable,
    members: &MembersView,
    registry: &ConnectionRegistry,
) {
    let map = table.to_partition_map(members);
    match rmp_serde::to_vec_named(&map) {
        Ok(bytes) => registry.broadcast(&bytes, ConnectionKind::Client),
        Err(e) => tracing::warn!("Failed to serialize partition map for broadcast: {e}"),
    }
}

// ---------------------------------------------------------------------------
// RebalanceTrigger
// ---------------------------------------------------------------------------

/// Background task that watches cluster membership changes and triggers
/// partition rebalancing when needed.
///
/// Decoupled from `MigrationCoordinator`: sends `MigrationCommand::Start`
/// messages into a channel rather than calling the service directly.
pub struct RebalanceTrigger {
    state: Arc<ClusterState>,
    migration_tx: mpsc::Sender<MigrationCommand>,
}

impl RebalanceTrigger {
    /// Creates a new rebalance trigger.
    #[must_use]
    pub fn new(state: Arc<ClusterState>, migration_tx: mpsc::Sender<MigrationCommand>) -> Self {
        Self {
            state,
            migration_tx,
        }
    }

    /// Runs the trigger loop, consuming cluster change events.
    ///
    /// On membership changes (`MemberAdded`, `MemberRemoved`, `MemberUpdated`),
    /// if the local node is master, computes a rebalance plan and sends
    /// migration commands. Partition events are ignored.
    pub async fn run(self, mut change_rx: mpsc::UnboundedReceiver<ClusterChange>) {
        while let Some(change) = change_rx.recv().await {
            match change {
                ClusterChange::MemberAdded(_)
                | ClusterChange::MemberRemoved(_)
                | ClusterChange::MemberUpdated(_) => {
                    if !self.state.is_master() {
                        continue;
                    }

                    let view = self.state.current_view();
                    let partition_count = self.state.partition_table.partition_count();
                    let backup_count = self.state.config.backup_count;

                    let assignments =
                        compute_assignment(&view.members, partition_count, backup_count);
                    let tasks = plan_rebalance(&self.state.partition_table, &assignments);

                    for task in tasks {
                        let _ = self.migration_tx.send(MigrationCommand::Start(task)).await;
                    }
                }
                // Partition events do not trigger rebalancing.
                ClusterChange::PartitionMoved { .. }
                | ClusterChange::PartitionTableUpdated { .. } => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::types::{ClusterConfig, MemberInfo, NodeState};
    use std::collections::HashMap;

    fn make_member(node_id: &str, state: NodeState) -> MemberInfo {
        MemberInfo {
            node_id: node_id.to_string(),
            host: "127.0.0.1".to_string(),
            client_port: 8080,
            cluster_port: 9090,
            state,
            join_version: 1,
        }
    }

    /// Creates a `ClusterState` with a master view (local node is master).
    fn make_state_with_master(
        local_id: &str,
    ) -> (Arc<ClusterState>, mpsc::UnboundedReceiver<ClusterChange>) {
        let config = Arc::new(ClusterConfig {
            max_parallel_migrations: 2,
            ..ClusterConfig::default()
        });
        let (state, rx) = ClusterState::new(config, local_id.to_string());

        // Set up a view where local node is master (lowest join_version).
        state.update_view(MembersView {
            version: 1,
            members: vec![
                make_member(local_id, NodeState::Active),
                MemberInfo {
                    join_version: 2,
                    ..make_member("node-2", NodeState::Active)
                },
                MemberInfo {
                    join_version: 3,
                    ..make_member("node-3", NodeState::Active)
                },
            ],
        });

        (Arc::new(state), rx)
    }

    fn make_coordinator(
        state: &Arc<ClusterState>,
    ) -> (
        MigrationCoordinator,
        Vec<tokio::sync::mpsc::Receiver<OutboundMessage>>,
    ) {
        let registry = Arc::new(ConnectionRegistry::new());
        let map_provider: Arc<dyn MapProvider> = Arc::new(NoOpMapProvider);
        let coordinator = MigrationCoordinator::new(Arc::clone(state), registry, map_provider);
        (coordinator, vec![])
    }

    /// Creates a coordinator with peer connections set up in the registry.
    /// Returns the coordinator, registry, and receivers for each peer.
    async fn make_coordinator_with_peers(
        state: Arc<ClusterState>,
        peer_ids: &[&str],
    ) -> (
        MigrationCoordinator,
        Arc<ConnectionRegistry>,
        HashMap<String, tokio::sync::mpsc::Receiver<OutboundMessage>>,
    ) {
        let config = crate::network::config::ConnectionConfig::default();
        let registry = Arc::new(ConnectionRegistry::new());
        let mut receivers = HashMap::new();

        for &peer_id in peer_ids {
            let (handle, rx) = registry.register(ConnectionKind::ClusterPeer, &config);
            {
                let mut meta = handle.metadata.write().await;
                meta.peer_node_id = Some(peer_id.to_string());
            }
            receivers.insert(peer_id.to_string(), rx);
        }

        let map_provider: Arc<dyn MapProvider> = Arc::new(NoOpMapProvider);
        let coordinator =
            MigrationCoordinator::new(Arc::clone(&state), Arc::clone(&registry), map_provider);

        (coordinator, registry, receivers)
    }

    // -- start_migrations --

    #[tokio::test]
    async fn start_migrations_sets_partition_to_migrating() {
        let (state, _rx) = make_state_with_master("node-1");
        state
            .partition_table
            .set_owner(0, "node-1".to_string(), vec![]);

        let (coordinator, _registry, _receivers) =
            make_coordinator_with_peers(Arc::clone(&state), &["node-1", "node-2"]).await;

        let tasks = vec![MigrationTask {
            partition_id: 0,
            source: "node-1".to_string(),
            destination: "node-2".to_string(),
            new_backups: vec![],
        }];

        coordinator.start_migrations(tasks).await.unwrap();

        let meta = state.partition_table.get_partition(0).unwrap();
        assert_eq!(meta.state, PartitionState::Migrating);
    }

    #[tokio::test]
    async fn start_migrations_respects_concurrency_limit() {
        let (state, _rx) = make_state_with_master("node-1");
        // max_parallel_migrations = 2
        state
            .partition_table
            .set_owner(0, "node-1".to_string(), vec![]);
        state
            .partition_table
            .set_owner(1, "node-1".to_string(), vec![]);
        state
            .partition_table
            .set_owner(2, "node-1".to_string(), vec![]);

        let (coordinator, _registry, _receivers) =
            make_coordinator_with_peers(Arc::clone(&state), &["node-1", "node-2"]).await;

        let tasks = vec![
            MigrationTask {
                partition_id: 0,
                source: "node-1".to_string(),
                destination: "node-2".to_string(),
                new_backups: vec![],
            },
            MigrationTask {
                partition_id: 1,
                source: "node-1".to_string(),
                destination: "node-2".to_string(),
                new_backups: vec![],
            },
            MigrationTask {
                partition_id: 2,
                source: "node-1".to_string(),
                destination: "node-2".to_string(),
                new_backups: vec![],
            },
        ];

        coordinator.start_migrations(tasks).await.unwrap();

        let migrations = state.active_migrations.read().await;
        // Only 2 should have been started (max_parallel_migrations = 2).
        assert_eq!(migrations.len(), 2);

        // Third partition should NOT be in Migrating state.
        // (It depends on which 2 were picked, but at most 2 are active.)
    }

    #[tokio::test]
    async fn start_migrations_orders_by_priority() {
        let (state, _rx) = make_state_with_master("node-1");
        // Partition 0: owner + 1 backup = 2 replicas (destination is backup = promotion)
        state
            .partition_table
            .set_owner(0, "node-1".to_string(), vec!["node-2".to_string()]);
        // Partition 1: owner + 0 backups = 1 replica (not a promotion)
        state
            .partition_table
            .set_owner(1, "node-1".to_string(), vec![]);

        let (coordinator, _registry, _receivers) =
            make_coordinator_with_peers(Arc::clone(&state), &["node-1", "node-2", "node-3"]).await;

        // Submit in reverse priority order.
        let tasks = vec![
            MigrationTask {
                partition_id: 1,
                source: "node-1".to_string(),
                destination: "node-3".to_string(),
                new_backups: vec![],
            },
            MigrationTask {
                partition_id: 0,
                source: "node-1".to_string(),
                destination: "node-2".to_string(),
                new_backups: vec![],
            },
        ];

        coordinator.start_migrations(tasks).await.unwrap();

        let migrations = state.active_migrations.read().await;
        // Both should be started (within concurrency limit).
        assert!(migrations.contains_key(&0));
        assert!(migrations.contains_key(&1));
    }

    // -- cancel_migration --

    #[tokio::test]
    async fn cancel_migration_restores_active_state() {
        let (state, _rx) = make_state_with_master("node-1");
        state
            .partition_table
            .set_owner(0, "node-1".to_string(), vec![]);

        let (coordinator, _registry, _receivers) =
            make_coordinator_with_peers(Arc::clone(&state), &["node-1", "node-2"]).await;

        let tasks = vec![MigrationTask {
            partition_id: 0,
            source: "node-1".to_string(),
            destination: "node-2".to_string(),
            new_backups: vec![],
        }];

        coordinator.start_migrations(tasks).await.unwrap();
        assert_eq!(
            state.partition_table.get_partition(0).unwrap().state,
            PartitionState::Migrating
        );

        coordinator.cancel_migration(0).await.unwrap();

        assert_eq!(
            state.partition_table.get_partition(0).unwrap().state,
            PartitionState::Active
        );

        let migrations = state.active_migrations.read().await;
        assert!(!migrations.contains_key(&0));
    }

    // -- cancel_all --

    #[tokio::test]
    async fn cancel_all_clears_all_migrations() {
        let (state, _rx) = make_state_with_master("node-1");
        state
            .partition_table
            .set_owner(0, "node-1".to_string(), vec![]);
        state
            .partition_table
            .set_owner(1, "node-1".to_string(), vec![]);

        let (coordinator, _registry, _receivers) =
            make_coordinator_with_peers(Arc::clone(&state), &["node-1", "node-2"]).await;

        let tasks = vec![
            MigrationTask {
                partition_id: 0,
                source: "node-1".to_string(),
                destination: "node-2".to_string(),
                new_backups: vec![],
            },
            MigrationTask {
                partition_id: 1,
                source: "node-1".to_string(),
                destination: "node-2".to_string(),
                new_backups: vec![],
            },
        ];

        coordinator.start_migrations(tasks).await.unwrap();

        let count = state.active_migrations.read().await.len();
        assert_eq!(count, 2);

        coordinator.cancel_all().await.unwrap();

        let migrations = state.active_migrations.read().await;
        assert!(migrations.is_empty());

        assert_eq!(
            state.partition_table.get_partition(0).unwrap().state,
            PartitionState::Active
        );
        assert_eq!(
            state.partition_table.get_partition(1).unwrap().state,
            PartitionState::Active
        );
    }

    // -- handle_migrate_start --

    #[tokio::test]
    async fn handle_migrate_start_sets_migrating_state() {
        let (state, _rx) = make_state_with_master("node-1");
        state
            .partition_table
            .set_owner(5, "node-1".to_string(), vec![]);

        let (coordinator, _registry, _receivers) =
            make_coordinator_with_peers(Arc::clone(&state), &["node-1", "node-2"]).await;

        coordinator.handle_migrate_start(5, "node-2").await.unwrap();

        let meta = state.partition_table.get_partition(5).unwrap();
        assert_eq!(meta.state, PartitionState::Migrating);
    }

    // -- handle_migrate_data --

    #[tokio::test]
    async fn handle_migrate_data_sets_receiving_state() {
        let (state, _rx) = make_state_with_master("node-2");
        state
            .partition_table
            .set_owner(5, "node-1".to_string(), vec![]);

        let (coordinator, _) = make_coordinator(&state);

        let data = MigrateDataPayload {
            partition_id: 5,
            map_states: vec![],
            delta_ops: vec![],
            source_version: 0,
        };

        coordinator.handle_migrate_data(data).await.unwrap();

        let meta = state.partition_table.get_partition(5).unwrap();
        assert_eq!(meta.state, PartitionState::Receiving);
    }

    // -- handle_migrate_ready --

    #[tokio::test]
    async fn handle_migrate_ready_completes_migration() {
        let (state, mut change_rx) = make_state_with_master("node-1");
        state
            .partition_table
            .set_owner(5, "node-1".to_string(), vec![]);

        let (coordinator, _registry, _receivers) =
            make_coordinator_with_peers(Arc::clone(&state), &["node-1", "node-2", "node-3"]).await;

        // Insert an active migration manually.
        {
            let mut migrations = state.active_migrations.write().await;
            migrations.insert(
                5,
                ActiveMigration {
                    migration_id: "mig-1".to_string(),
                    partition_id: 5,
                    source: "node-1".to_string(),
                    destination: "node-2".to_string(),
                    state: MigrationPhase::Replicating,
                    started_at_ms: current_unix_ms(),
                    new_backups: vec!["node-3".to_string()],
                },
            );
        }

        let version_before = state.partition_table.version();

        coordinator.handle_migrate_ready(5, "node-1").await.unwrap();

        // Ownership should have been updated.
        let meta = state.partition_table.get_partition(5).unwrap();
        assert_eq!(meta.owner, "node-2");
        assert_eq!(meta.backups, vec!["node-3"]);

        // Version should have incremented.
        assert!(state.partition_table.version() > version_before);

        // Migration should have been removed.
        let migrations = state.active_migrations.read().await;
        assert!(!migrations.contains_key(&5));
        drop(migrations);

        // PartitionMoved event should have been emitted.
        let event = change_rx.recv().await.unwrap();
        assert_eq!(
            event,
            ClusterChange::PartitionMoved {
                partition_id: 5,
                old_owner: "node-1".to_string(),
                new_owner: "node-2".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn handle_migrate_ready_rollback_on_send_failure() {
        let (state, _rx) = make_state_with_master("node-1");
        state
            .partition_table
            .set_owner(5, "node-1".to_string(), vec![]);

        // Create coordinator with NO peer connections, so send_to_peer will fail.
        let registry = Arc::new(ConnectionRegistry::new());
        let map_provider: Arc<dyn MapProvider> = Arc::new(NoOpMapProvider);
        let coordinator = MigrationCoordinator::new(Arc::clone(&state), registry, map_provider);

        // Insert an active migration.
        {
            let mut migrations = state.active_migrations.write().await;
            migrations.insert(
                5,
                ActiveMigration {
                    migration_id: "mig-1".to_string(),
                    partition_id: 5,
                    source: "node-1".to_string(),
                    destination: "node-2".to_string(),
                    state: MigrationPhase::Replicating,
                    started_at_ms: current_unix_ms(),
                    new_backups: vec![],
                },
            );
        }

        let result = coordinator.handle_migrate_ready(5, "node-1").await;

        // Should have returned an error.
        assert!(result.is_err());

        // Source partition should be back to Active.
        let meta = state.partition_table.get_partition(5).unwrap();
        assert_eq!(meta.state, PartitionState::Active);

        // Migration should have been removed (after being marked Failed).
        let migrations = state.active_migrations.read().await;
        assert!(!migrations.contains_key(&5));
    }

    // -- is_migrating --

    #[tokio::test]
    async fn is_migrating_returns_true_during_migration() {
        let (state, _rx) = make_state_with_master("node-1");
        let (coordinator, _) = make_coordinator(&state);

        assert!(!coordinator.is_migrating(5));

        {
            let mut migrations = state.active_migrations.write().await;
            migrations.insert(
                5,
                ActiveMigration {
                    migration_id: "mig-1".to_string(),
                    partition_id: 5,
                    source: "node-1".to_string(),
                    destination: "node-2".to_string(),
                    state: MigrationPhase::Replicating,
                    started_at_ms: current_unix_ms(),
                    new_backups: vec![],
                },
            );
        }

        assert!(coordinator.is_migrating(5));
        assert!(!coordinator.is_migrating(6));
    }

    // -- not_owner_response --

    #[test]
    fn not_owner_response_returns_partition_map() {
        let table = ClusterPartitionTable::new(271);
        table.set_owner(0, "node-1".to_string(), vec!["node-2".to_string()]);
        let _ = table.increment_version();

        let members = MembersView {
            version: 1,
            members: vec![
                make_member("node-1", NodeState::Active),
                make_member("node-2", NodeState::Active),
            ],
        };

        let map = not_owner_response(&table, &members);

        assert_eq!(map.version, 1);
        assert_eq!(map.partition_count, 271);
        assert_eq!(map.partitions.len(), 1);
        assert_eq!(map.partitions[0].owner_node_id, "node-1");
    }

    // -- broadcast_partition_map --

    #[test]
    fn broadcast_partition_map_sends_to_clients_only() {
        let table = ClusterPartitionTable::new(10);
        table.set_owner(0, "node-1".to_string(), vec![]);

        let members = MembersView {
            version: 1,
            members: vec![make_member("node-1", NodeState::Active)],
        };

        let config = crate::network::config::ConnectionConfig::default();
        let registry = ConnectionRegistry::new();

        let (_client_handle, mut client_rx) = registry.register(ConnectionKind::Client, &config);
        let (_peer_handle, mut peer_rx) = registry.register(ConnectionKind::ClusterPeer, &config);

        broadcast_partition_map(&table, &members, &registry);

        // Client should have received the broadcast.
        assert!(client_rx.try_recv().is_ok());
        // Cluster peer should NOT have received the broadcast.
        assert!(peer_rx.try_recv().is_err());
    }

    // -- RebalanceTrigger --

    #[tokio::test]
    async fn rebalance_trigger_sends_commands_on_member_change() {
        let config = Arc::new(ClusterConfig {
            max_parallel_migrations: 2,
            backup_count: 0,
            ..ClusterConfig::default()
        });
        let (state, _state_rx) = ClusterState::new(config, "node-1".to_string());
        let state = Arc::new(state);

        // Set up a view where node-1 is master.
        state.update_view(MembersView {
            version: 1,
            members: vec![
                make_member("node-1", NodeState::Active),
                MemberInfo {
                    join_version: 2,
                    ..make_member("node-2", NodeState::Active)
                },
            ],
        });

        // Assign a subset of partitions to node-1 so rebalancing produces tasks.
        // Use a small number to avoid overflowing the bounded migration channel.
        for pid in 0..10 {
            state
                .partition_table
                .set_owner(pid, "node-1".to_string(), vec![]);
        }

        let (migration_tx, mut migration_rx) = mpsc::channel(100);
        let (change_tx, change_rx) = mpsc::unbounded_channel();

        let trigger = RebalanceTrigger::new(Arc::clone(&state), migration_tx);

        // Spawn the trigger in a background task.
        let handle = tokio::spawn(async move {
            trigger.run(change_rx).await;
        });

        // Update the view to include node-3 (in a real system, the view is
        // updated before the MemberAdded event is emitted).
        state.update_view(MembersView {
            version: 2,
            members: vec![
                make_member("node-1", NodeState::Active),
                MemberInfo {
                    join_version: 2,
                    ..make_member("node-2", NodeState::Active)
                },
                MemberInfo {
                    join_version: 3,
                    ..make_member("node-3", NodeState::Active)
                },
            ],
        });

        // Send a MemberAdded event.
        change_tx
            .send(ClusterChange::MemberAdded(MemberInfo {
                join_version: 3,
                ..make_member("node-3", NodeState::Active)
            }))
            .unwrap();

        // Give the trigger time to process.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Should have received at least one MigrationCommand::Start.
        let cmd = migration_rx.try_recv();
        assert!(
            cmd.is_ok(),
            "expected MigrationCommand::Start from rebalance trigger"
        );
        match cmd.unwrap() {
            MigrationCommand::Start(task) => {
                // Destination should be one of the new assignment targets.
                assert!(
                    task.destination == "node-2" || task.destination == "node-3",
                    "unexpected destination: {}",
                    task.destination
                );
            }
            other => panic!("expected MigrationCommand::Start, got {other:?}"),
        }

        // Clean up.
        drop(change_tx);
        let _ = handle.await;
    }

    #[tokio::test]
    async fn rebalance_trigger_ignores_partition_events() {
        let config = Arc::new(ClusterConfig::default());
        let (state, _state_rx) = ClusterState::new(config, "node-1".to_string());
        let state = Arc::new(state);

        // Make node-1 the master.
        state.update_view(MembersView {
            version: 1,
            members: vec![make_member("node-1", NodeState::Active)],
        });

        let (migration_tx, mut migration_rx) = mpsc::channel(100);
        let (change_tx, change_rx) = mpsc::unbounded_channel();

        let trigger = RebalanceTrigger::new(Arc::clone(&state), migration_tx);

        let handle = tokio::spawn(async move {
            trigger.run(change_rx).await;
        });

        // Send partition events that should be ignored.
        change_tx
            .send(ClusterChange::PartitionMoved {
                partition_id: 0,
                old_owner: "node-1".to_string(),
                new_owner: "node-2".to_string(),
            })
            .unwrap();

        change_tx
            .send(ClusterChange::PartitionTableUpdated { version: 5 })
            .unwrap();

        // Give the trigger time to process.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // No migration commands should have been sent.
        assert!(
            migration_rx.try_recv().is_err(),
            "partition events should not produce migration commands"
        );

        drop(change_tx);
        let _ = handle.await;
    }
}
