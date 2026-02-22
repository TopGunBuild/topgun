//! Shared cluster state structures.
//!
//! Provides lock-free, concurrent data structures for cluster membership
//! and partition ownership:
//! - `ClusterPartitionTable`: per-partition metadata via `DashMap` with atomic versioning
//! - `ClusterState`: `ArcSwap<MembersView>` for lock-free membership reads
//! - `ClusterChange`: reactive event enum for cluster state notifications
//! - `ClusterChannels`/`ClusterChannelReceivers`: typed channel pairs for inter-component messaging

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use arc_swap::ArcSwap;
use dashmap::DashMap;
use tokio::sync::{mpsc, watch};

use topgun_core::messages::cluster::{
    NodeEndpoints, NodeInfo, NodeStatus, PartitionInfo, PartitionMapPayload,
};

use super::messages::ClusterMessage;
use super::types::{
    ActiveMigration, ClusterConfig, MemberInfo, MembersView, MigrationTask, NodeState,
    PartitionAssignment, PartitionMeta, PartitionState,
};

// ---------------------------------------------------------------------------
// ClusterPartitionTable
// ---------------------------------------------------------------------------

/// Concurrent partition table tracking per-partition ownership and state.
///
/// Uses `DashMap` for lock-free per-partition reads and an `AtomicU64` version
/// counter to track table mutations. This design allows readers to access
/// individual partitions without blocking writers on other partitions.
pub struct ClusterPartitionTable {
    partitions: DashMap<u32, PartitionMeta>,
    version: AtomicU64,
    partition_count: u32,
}

impl fmt::Debug for ClusterPartitionTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClusterPartitionTable")
            .field("partition_count", &self.partition_count)
            .field("version", &self.version.load(Ordering::Relaxed))
            .field("populated", &self.partitions.len())
            .finish()
    }
}

impl ClusterPartitionTable {
    /// Creates an empty partition table with the given total partition count.
    pub fn new(partition_count: u32) -> Self {
        Self {
            partitions: DashMap::new(),
            version: AtomicU64::new(0),
            partition_count,
        }
    }

    /// Returns a clone of the metadata for the given partition, if it exists.
    pub fn get_partition(&self, partition_id: u32) -> Option<PartitionMeta> {
        self.partitions.get(&partition_id).map(|r| r.clone())
    }

    /// Updates or inserts partition ownership.
    pub fn set_owner(&self, partition_id: u32, owner: String, backups: Vec<String>) {
        match self.partitions.get_mut(&partition_id) {
            Some(mut entry) => {
                entry.owner = owner;
                entry.backups = backups;
            }
            None => {
                self.partitions.insert(
                    partition_id,
                    PartitionMeta {
                        partition_id,
                        owner,
                        backups,
                        state: PartitionState::Active,
                        version: 0,
                    },
                );
            }
        }
    }

    /// Updates the state of an existing partition.
    pub fn set_state(&self, partition_id: u32, state: PartitionState) {
        if let Some(mut entry) = self.partitions.get_mut(&partition_id) {
            entry.state = state;
        }
    }

    /// Returns the current table version.
    ///
    /// Uses `Acquire` ordering to synchronize with `Release` writes,
    /// ensuring all partition mutations before the version bump are visible.
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    /// Atomically increments the version and returns the new value.
    ///
    /// Uses `Release` ordering so that all prior partition mutations are
    /// visible to readers that observe the new version via `Acquire`.
    pub fn increment_version(&self) -> u64 {
        self.version.fetch_add(1, Ordering::Release) + 1
    }

    /// Bulk-applies partition assignments and increments the table version.
    pub fn apply_assignments(&self, assignments: &[PartitionAssignment]) {
        for a in assignments {
            self.set_owner(a.partition_id, a.owner.clone(), a.backups.clone());
        }
        self.increment_version();
    }

    /// Converts the partition table to a client-facing `PartitionMapPayload`.
    ///
    /// Maps internal `NodeState` to wire `NodeStatus` and constructs
    /// `NodeEndpoints` from member host/port information.
    pub fn to_partition_map(&self, members: &MembersView) -> PartitionMapPayload {
        let nodes: Vec<NodeInfo> = members
            .members
            .iter()
            .map(|m| NodeInfo {
                node_id: m.node_id.clone(),
                endpoints: NodeEndpoints {
                    websocket: format!("ws://{}:{}", m.host, m.client_port),
                    http: None,
                },
                status: match m.state {
                    NodeState::Active => NodeStatus::ACTIVE,
                    NodeState::Joining => NodeStatus::JOINING,
                    NodeState::Leaving => NodeStatus::LEAVING,
                    NodeState::Suspect => NodeStatus::SUSPECTED,
                    NodeState::Dead | NodeState::Removed => NodeStatus::FAILED,
                },
            })
            .collect();

        let mut partitions: Vec<PartitionInfo> = Vec::new();
        for entry in self.partitions.iter() {
            let meta = entry.value();
            partitions.push(PartitionInfo {
                partition_id: meta.partition_id,
                owner_node_id: meta.owner.clone(),
                backup_node_ids: meta.backups.clone(),
            });
        }
        // Sort by partition_id for deterministic output.
        partitions.sort_by_key(|p| p.partition_id);

        let generated_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        PartitionMapPayload {
            version: self.version() as u32,
            partition_count: self.partition_count,
            nodes,
            partitions,
            generated_at,
        }
    }

    /// Returns all partition IDs owned by the given node.
    pub fn partitions_for_node(&self, node_id: &str) -> Vec<u32> {
        self.partitions
            .iter()
            .filter(|entry| entry.value().owner == node_id)
            .map(|entry| *entry.key())
            .collect()
    }

    /// Returns the total partition count.
    pub fn partition_count(&self) -> u32 {
        self.partition_count
    }
}

// ---------------------------------------------------------------------------
// ClusterChange
// ---------------------------------------------------------------------------

/// Events emitted when cluster state changes.
///
/// Subscribers receive these via an unbounded mpsc channel to react to
/// membership and partition topology changes.
#[derive(Debug, Clone, PartialEq)]
pub enum ClusterChange {
    MemberAdded(MemberInfo),
    MemberUpdated(MemberInfo),
    MemberRemoved(MemberInfo),
    PartitionMoved {
        partition_id: u32,
        old_owner: String,
        new_owner: String,
    },
    PartitionTableUpdated {
        version: u64,
    },
}

// ---------------------------------------------------------------------------
// MigrationCommand
// ---------------------------------------------------------------------------

/// Commands sent to the migration service to control partition migrations.
#[derive(Debug, Clone)]
pub enum MigrationCommand {
    Start(MigrationTask),
    /// Cancel a single migration by partition ID.
    Cancel(u32),
    CancelAll,
}

// ---------------------------------------------------------------------------
// InboundClusterMessage
// ---------------------------------------------------------------------------

/// An inbound cluster message tagged with the sender's node ID.
#[derive(Debug, Clone)]
pub struct InboundClusterMessage {
    pub sender_node_id: String,
    pub message: ClusterMessage,
}

// ---------------------------------------------------------------------------
// ClusterState
// ---------------------------------------------------------------------------

/// Central cluster state combining membership and partition data.
///
/// Uses `ArcSwap<MembersView>` for lock-free membership reads (readers never
/// block writers) and an unbounded channel for reactive change notifications.
pub struct ClusterState {
    membership: ArcSwap<MembersView>,
    pub partition_table: ClusterPartitionTable,
    pub active_migrations: tokio::sync::RwLock<HashMap<u32, ActiveMigration>>,
    change_tx: mpsc::UnboundedSender<ClusterChange>,
    pub config: Arc<ClusterConfig>,
    pub local_node_id: String,
}

impl ClusterState {
    /// Creates a new cluster state and returns the change event receiver.
    ///
    /// The initial membership view is empty (version 0, no members).
    pub fn new(
        config: Arc<ClusterConfig>,
        local_node_id: String,
    ) -> (Self, mpsc::UnboundedReceiver<ClusterChange>) {
        let (change_tx, change_rx) = mpsc::unbounded_channel();
        let initial_view = MembersView {
            version: 0,
            members: Vec::new(),
        };

        let state = Self {
            membership: ArcSwap::new(Arc::new(initial_view)),
            partition_table: ClusterPartitionTable::new(271),
            active_migrations: tokio::sync::RwLock::new(HashMap::new()),
            change_tx,
            config,
            local_node_id,
        };

        (state, change_rx)
    }

    /// Returns the current membership view via lock-free `ArcSwap` load.
    pub fn current_view(&self) -> Arc<MembersView> {
        self.membership.load_full()
    }

    /// Replaces the current membership view atomically.
    pub fn update_view(&self, view: MembersView) {
        self.membership.store(Arc::new(view));
    }

    /// Returns `true` if the local node is the current cluster master.
    pub fn is_master(&self) -> bool {
        let view = self.membership.load();
        view.is_master(&self.local_node_id)
    }

    /// Returns a reference to the change event sender.
    ///
    /// Used by cluster subsystems to emit change notifications.
    pub fn change_sender(&self) -> &mpsc::UnboundedSender<ClusterChange> {
        &self.change_tx
    }
}

// ---------------------------------------------------------------------------
// ClusterChannels / ClusterChannelReceivers
// ---------------------------------------------------------------------------

/// Sender halves of all cluster inter-component channels.
pub struct ClusterChannels {
    pub membership_changes: watch::Sender<Arc<MembersView>>,
    pub cluster_events: mpsc::UnboundedSender<ClusterChange>,
    pub migration_commands: mpsc::Sender<MigrationCommand>,
    pub inbound_messages: mpsc::Sender<InboundClusterMessage>,
}

/// Receiver halves of all cluster inter-component channels.
pub struct ClusterChannelReceivers {
    pub membership_changes: watch::Receiver<Arc<MembersView>>,
    pub cluster_events: mpsc::UnboundedReceiver<ClusterChange>,
    pub migration_commands: mpsc::Receiver<MigrationCommand>,
    pub inbound_messages: mpsc::Receiver<InboundClusterMessage>,
}

impl ClusterChannels {
    /// Creates all cluster channels and returns both sender and receiver halves.
    ///
    /// The `buffer_size` parameter controls the bounded channels
    /// (`migration_commands` and `inbound_messages`). The `membership_changes`
    /// watch channel is initialized with an empty `MembersView` (version 0).
    pub fn new(buffer_size: usize) -> (Self, ClusterChannelReceivers) {
        let initial_view = Arc::new(MembersView {
            version: 0,
            members: Vec::new(),
        });

        let (membership_tx, membership_rx) = watch::channel(initial_view);
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let (migration_tx, migration_rx) = mpsc::channel(buffer_size);
        let (inbound_tx, inbound_rx) = mpsc::channel(buffer_size);

        let channels = Self {
            membership_changes: membership_tx,
            cluster_events: events_tx,
            migration_commands: migration_tx,
            inbound_messages: inbound_tx,
        };

        let receivers = ClusterChannelReceivers {
            membership_changes: membership_rx,
            cluster_events: events_rx,
            migration_commands: migration_rx,
            inbound_messages: inbound_rx,
        };

        (channels, receivers)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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

    // -- ClusterPartitionTable --

    #[test]
    fn partition_table_new_is_empty() {
        let table = ClusterPartitionTable::new(271);
        assert_eq!(table.partition_count(), 271);
        assert_eq!(table.version(), 0);
        assert!(table.get_partition(0).is_none());
    }

    #[test]
    fn partition_table_set_and_get_owner() {
        let table = ClusterPartitionTable::new(271);
        table.set_owner(0, "node-1".to_string(), vec!["node-2".to_string()]);

        let meta = table.get_partition(0).unwrap();
        assert_eq!(meta.owner, "node-1");
        assert_eq!(meta.backups, vec!["node-2"]);
        assert_eq!(meta.state, PartitionState::Active);
    }

    #[test]
    fn partition_table_set_state() {
        let table = ClusterPartitionTable::new(271);
        table.set_owner(0, "node-1".to_string(), vec![]);
        table.set_state(0, PartitionState::Migrating);

        let meta = table.get_partition(0).unwrap();
        assert_eq!(meta.state, PartitionState::Migrating);
    }

    #[test]
    fn partition_table_version_increments() {
        let table = ClusterPartitionTable::new(271);
        assert_eq!(table.version(), 0);

        let v1 = table.increment_version();
        assert_eq!(v1, 1);
        assert_eq!(table.version(), 1);

        let v2 = table.increment_version();
        assert_eq!(v2, 2);
        assert_eq!(table.version(), 2);
    }

    #[test]
    fn partition_table_apply_assignments() {
        let table = ClusterPartitionTable::new(271);

        let assignments = vec![
            PartitionAssignment {
                partition_id: 0,
                owner: "node-1".to_string(),
                backups: vec!["node-2".to_string()],
            },
            PartitionAssignment {
                partition_id: 1,
                owner: "node-2".to_string(),
                backups: vec!["node-1".to_string()],
            },
        ];

        table.apply_assignments(&assignments);

        assert_eq!(table.version(), 1);
        assert_eq!(table.get_partition(0).unwrap().owner, "node-1");
        assert_eq!(table.get_partition(1).unwrap().owner, "node-2");
    }

    #[test]
    fn partition_table_partitions_for_node() {
        let table = ClusterPartitionTable::new(271);
        table.set_owner(0, "node-1".to_string(), vec![]);
        table.set_owner(1, "node-2".to_string(), vec![]);
        table.set_owner(2, "node-1".to_string(), vec![]);

        let mut pids = table.partitions_for_node("node-1");
        pids.sort();
        assert_eq!(pids, vec![0, 2]);
    }

    #[test]
    fn partition_table_to_partition_map() {
        let table = ClusterPartitionTable::new(271);
        table.set_owner(0, "node-1".to_string(), vec!["node-2".to_string()]);
        table.increment_version();

        let members = MembersView {
            version: 1,
            members: vec![
                make_member("node-1", NodeState::Active),
                make_member("node-2", NodeState::Joining),
            ],
        };

        let map = table.to_partition_map(&members);

        assert_eq!(map.version, 1);
        assert_eq!(map.partition_count, 271);
        assert_eq!(map.nodes.len(), 2);
        assert_eq!(map.nodes[0].status, NodeStatus::ACTIVE);
        assert_eq!(map.nodes[1].status, NodeStatus::JOINING);
        assert_eq!(map.partitions.len(), 1);
        assert_eq!(map.partitions[0].owner_node_id, "node-1");
    }

    #[test]
    fn partition_table_node_state_to_node_status_mapping() {
        let table = ClusterPartitionTable::new(1);

        let members = MembersView {
            version: 1,
            members: vec![
                make_member("n1", NodeState::Active),
                make_member("n2", NodeState::Joining),
                make_member("n3", NodeState::Leaving),
                make_member("n4", NodeState::Suspect),
                make_member("n5", NodeState::Dead),
                make_member("n6", NodeState::Removed),
            ],
        };

        let map = table.to_partition_map(&members);

        assert_eq!(map.nodes[0].status, NodeStatus::ACTIVE);
        assert_eq!(map.nodes[1].status, NodeStatus::JOINING);
        assert_eq!(map.nodes[2].status, NodeStatus::LEAVING);
        assert_eq!(map.nodes[3].status, NodeStatus::SUSPECTED);
        assert_eq!(map.nodes[4].status, NodeStatus::FAILED);
        assert_eq!(map.nodes[5].status, NodeStatus::FAILED);
    }

    // -- ClusterChange --

    #[test]
    fn cluster_change_has_five_variants() {
        // Verify all 5 variants can be constructed.
        let _v1 = ClusterChange::MemberAdded(make_member("n1", NodeState::Active));
        let _v2 = ClusterChange::MemberUpdated(make_member("n1", NodeState::Active));
        let _v3 = ClusterChange::MemberRemoved(make_member("n1", NodeState::Active));
        let _v4 = ClusterChange::PartitionMoved {
            partition_id: 0,
            old_owner: "n1".to_string(),
            new_owner: "n2".to_string(),
        };
        let _v5 = ClusterChange::PartitionTableUpdated { version: 1 };
    }

    // -- ClusterState --

    #[tokio::test]
    async fn cluster_state_initial_view_is_empty() {
        let config = Arc::new(ClusterConfig::default());
        let (state, _rx) = ClusterState::new(config, "node-1".to_string());

        let view = state.current_view();
        assert_eq!(view.version, 0);
        assert!(view.members.is_empty());
    }

    #[tokio::test]
    async fn cluster_state_update_view() {
        let config = Arc::new(ClusterConfig::default());
        let (state, _rx) = ClusterState::new(config, "node-1".to_string());

        let new_view = MembersView {
            version: 1,
            members: vec![make_member("node-1", NodeState::Active)],
        };
        state.update_view(new_view.clone());

        let view = state.current_view();
        assert_eq!(view.version, 1);
        assert_eq!(view.members.len(), 1);
    }

    #[tokio::test]
    async fn cluster_state_is_master() {
        let config = Arc::new(ClusterConfig::default());
        let (state, _rx) = ClusterState::new(config, "node-1".to_string());

        // No members -> not master.
        assert!(!state.is_master());

        // Add node-1 as active -> it is master (only active member).
        state.update_view(MembersView {
            version: 1,
            members: vec![make_member("node-1", NodeState::Active)],
        });
        assert!(state.is_master());

        // Add node-0 with lower join_version -> node-0 becomes master.
        state.update_view(MembersView {
            version: 2,
            members: vec![
                MemberInfo {
                    node_id: "node-0".to_string(),
                    host: "127.0.0.1".to_string(),
                    client_port: 8080,
                    cluster_port: 9090,
                    state: NodeState::Active,
                    join_version: 0,
                },
                make_member("node-1", NodeState::Active),
            ],
        });
        assert!(!state.is_master());
    }

    // -- ClusterChannels --

    #[tokio::test]
    async fn cluster_channels_creation() {
        let (channels, mut receivers) = ClusterChannels::new(16);

        // Membership watch is initialized with empty view.
        let initial = receivers.membership_changes.borrow_and_update().clone();
        assert_eq!(initial.version, 0);
        assert!(initial.members.is_empty());

        // Cluster events channel works.
        let change = ClusterChange::PartitionTableUpdated { version: 1 };
        channels.cluster_events.send(change.clone()).unwrap();
        let received = receivers.cluster_events.recv().await.unwrap();
        assert_eq!(received, change);
    }
}
