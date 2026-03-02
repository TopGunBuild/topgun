# TopGun Rust Cluster Architecture

> **Date:** 2026-02-19
> **Status:** Design Complete (TODO-081)
> **Blocks:** TODO-063 (advanced partition management), TODO-066 (Cluster Protocol)
> **Research:** [RES-004](../research/RES-004.md)

---

## Table of Contents

1. [Design Philosophy](#1-design-philosophy)
2. [Cluster State Machine](#2-cluster-state-machine)
3. [Membership Protocol Design](#3-membership-protocol-design)
4. [Partition Management](#4-partition-management)
5. [Migration Lifecycle](#5-migration-lifecycle)
6. [Split-Brain Handling](#6-split-brain-handling)
7. [Rust Concurrency Design](#7-rust-concurrency-design)
8. [Trait Hierarchy](#8-trait-hierarchy)
9. [Wire Protocol](#9-wire-protocol)
10. [Client Integration](#10-client-integration)
11. [Migration Path](#11-migration-path)

---

## 1. Design Philosophy

### Triple Reference Application

| Concern | Source | What We Take |
|---------|--------|-------------|
| Conceptual architecture | Hazelcast | Versioned MembersView, master-centric coordination, migration ordering, heartbeat complaint protocol |
| Implementation patterns | TiKV, Quickwit | DashMap for partition lookup, ClusterChangeStream, Fsm trait for per-partition state, Arc<RwLock> for shared state |
| Behavioral specification | TopGun TS | Wire protocol (PartitionMapPayload), hash function (fnv1a % 271), FailureDetector phi-accrual algorithm, replication pipeline consistency levels |

### CRDT Advantage

TopGun's CRDT foundation fundamentally changes the cluster protocol compared to Hazelcast:

| Concern | Hazelcast (non-CRDT) | TopGun (CRDT) |
|---------|---------------------|---------------|
| Write during migration | **Blocked** -- source locks partition | **Allowed** -- both sides accept writes, merge later |
| Split-brain data conflict | **Explicit merge policy** required (LATEST_UPDATE, HIGHER_HITS, custom) | **Automatic** -- CRDTs converge deterministically |
| Migration data transfer | Full copy with fence | CRDT state + delta merge |
| Partition move rollback | Discard destination data | Merge destination data back to source |
| Replication consistency | Requires quorum for strong | Eventual consistency is safe; strong optional |

This means:
- **3-phase migration becomes 2-phase** (no lock phase needed)
- **Split-brain recovery is automatic** (CRDT merge on reconnect)
- **NOT_OWNER responses can be softer** (redirect, not reject)

---

## 2. Cluster State Machine

### 2.1 Node Lifecycle

```
                     +-----------+
        start() --> | Joining   |
                     +-----------+
                          |
                     join accepted by master
                          |
                          v
                     +-----------+
                     |  Active   | <--- heartbeat received (from Suspect)
                     +-----------+
                       |       |
            phi > threshold    graceful leave
                       |       |
                       v       v
                  +----------+  +----------+
                  | Suspect  |  | Leaving  |
                  +----------+  +----------+
                       |             |
              confirmed failed   leave ack / timeout
                       |             |
                       v             v
                  +----------+  +----------+
                  |  Dead    |  | Removed  |
                  +----------+  +----------+
                       |
                  grace period
                       |
                       v
                  +----------+
                  | Removed  |
                  +----------+
```

```rust
/// Lifecycle state of a cluster node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeState {
    /// Node is attempting to join the cluster. Awaiting master acceptance.
    Joining,
    /// Node is a full cluster member, serving partitions.
    Active,
    /// Failure detector suspects this node. Still in member list.
    /// If heartbeat resumes, transitions back to Active.
    Suspect,
    /// Node initiated graceful shutdown. Partitions being migrated away.
    Leaving,
    /// Node failure confirmed after suspicion timeout. Partitions reassigned.
    Dead,
    /// Node removed from member list entirely. Safe to forget.
    Removed,
}
```

**Key transitions:**
- `Joining -> Active`: Master sends `FinalizeJoin` with MembersView + PartitionTable
- `Active -> Suspect`: Local failure detector's phi exceeds threshold
- `Suspect -> Active`: Heartbeat received (clear suspicion)
- `Suspect -> Dead`: Confirmation timeout expires without heartbeat
- `Active -> Leaving`: Node calls `leave()`, master starts partition migration away
- `Dead -> Removed`: After grace period, master removes from MembersView

### 2.2 Partition Lifecycle

```
         +-------------+
         | Unassigned  |  (startup, before first assignment)
         +-------------+
               |
          assigned by master
               |
               v
         +-------------+
    +--> |   Active    | <--- finalize migration
    |    +-------------+
    |       |       |
    |  migration  migration
    |  source     destination
    |       |       |
    |       v       v
    |  +-----------+  +-----------+
    |  | Migrating |  | Receiving |
    |  | (source)  |  | (dest)    |
    |  +-----------+  +-----------+
    |       |               |
    |  migration complete   |
    |       |               |
    |       v               |
    |  +-----------+        |
    |  | Draining  |        |
    |  +-----------+        |
    |       |               |
    |  master finalize      |
    |       |               |
    |       v               |
    +-------+               |
         Active <-----------+
```

```rust
/// State of a partition on this node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PartitionState {
    /// Partition is not assigned to this node.
    Unassigned,
    /// Partition is fully operational on this node.
    Active,
    /// This node is the source of an ongoing migration.
    /// Writes are still accepted (CRDT -- no locking needed).
    Migrating,
    /// This node is receiving partition data from source.
    /// Writes may arrive from clients that have stale partition map.
    Receiving,
    /// Migration source: data transfer complete, waiting for master
    /// to finalize. New writes forwarded to destination.
    Draining,
    /// Partition data was lost (node crashed during migration, no backup).
    Lost,
}
```

**CRDT-specific behavior during migration:**
- `Migrating` state: source continues accepting writes, buffers delta since migration start
- `Receiving` state: destination accepts forwarded writes, merges with incoming migration data
- `Draining` state: source forwards any new writes to destination, master can safely finalize
- No write rejection needed at any point -- CRDTs handle concurrent writes

---

## 3. Membership Protocol Design

### 3.1 Versioned MembersView

Inspired by Hazelcast's `MembersView` but adapted for Rust:

```rust
use std::sync::Arc;

/// Immutable snapshot of cluster membership at a point in time.
/// New snapshots are created atomically on every membership change.
#[derive(Debug, Clone)]
pub struct MembersView {
    /// Monotonically increasing version. Clients and nodes use this
    /// to detect stale membership without comparing full member lists.
    pub version: u64,
    /// Ordered list of members (order is significant for master election).
    pub members: Vec<MemberInfo>,
}

/// Information about a single cluster member.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MemberInfo {
    /// Unique node identifier (UUID).
    pub node_id: String,
    /// Host address for client connections.
    pub host: String,
    /// Port for client WebSocket connections.
    pub client_port: u16,
    /// Port for inter-node cluster communication.
    pub cluster_port: u16,
    /// Current lifecycle state.
    pub state: NodeState,
    /// Version of MembersView when this node joined.
    pub join_version: u64,
}
```

**Atomic swap pattern (from Hazelcast MemberMap):**

```rust
use std::sync::atomic::AtomicPtr;
use std::sync::Arc;

/// Thread-safe, lock-free membership state.
/// Readers get a consistent snapshot. Writers (master only) create
/// a new Arc<MembersView> and swap atomically.
pub struct MembershipState {
    /// Current membership view. Lock-free read via Arc clone.
    current: ArcSwap<MembersView>,
}

impl MembershipState {
    /// Lock-free read of current membership.
    pub fn current(&self) -> arc_swap::Guard<Arc<MembersView>> {
        self.current.load()
    }

    /// Atomically replace membership view (master only).
    pub fn update(&self, new_view: MembersView) {
        self.current.store(Arc::new(new_view));
    }
}
```

We use `arc_swap::ArcSwap<MembersView>` for lock-free reads with atomic pointer swap on update. This mirrors Hazelcast's `AtomicReference<MemberMap>` but is more idiomatic in Rust.

### 3.2 Master Election

**Strategy: Oldest-member convention** (same as Hazelcast)

```rust
impl MembersView {
    /// The master is the member with the lowest join_version.
    /// Ties broken by node_id lexicographic order.
    /// This is deterministic: all nodes agree on master from the same MembersView.
    pub fn master(&self) -> Option<&MemberInfo> {
        self.members
            .iter()
            .filter(|m| m.state == NodeState::Active)
            .min_by_key(|m| (m.join_version, &m.node_id))
    }

    /// Check if the given node_id is the current master.
    pub fn is_master(&self, node_id: &str) -> bool {
        self.master().map(|m| m.node_id.as_str()) == Some(node_id)
    }
}
```

**Master failover:** When master dies, all nodes compute the new master from their (identical) MembersView. The new master takes over coordination immediately. No election protocol needed because the convention is deterministic.

**Mastership claim (from Hazelcast):** If nodes disagree about who is master (e.g., after network partition heals), a node can initiate a mastership claim:
1. Node suspects current master is dead
2. Node fetches MembersView from all reachable members
3. If a majority agrees on the same view, the oldest active member in that view becomes master
4. New master publishes updated MembersView to all

### 3.3 Join Ceremony

Adapted from Hazelcast's `ClusterJoinManager`, simplified for TopGun:

```
 Joining Node                    Master Node
      |                               |
      |--- JoinRequest -------------->|
      |    {node_id, host, ports,     |
      |     cluster_id, version}      |
      |                               |
      |                      validate config
      |                      check auth token
      |                      assign join_version
      |                               |
      |<--- JoinResponse -------------|
      |    {accepted: true,           |
      |     members_view: {...},      |
      |     partition_table: {...}}   |
      |                               |
      |                      broadcast MembersUpdate
      |                      to all existing members
      |                               |
```

**Validation steps (master side):**
1. **Cluster ID match:** Reject if different cluster
2. **Version compatibility:** Reject if protocol version mismatch
3. **Authentication:** Validate join token/credentials
4. **Duplicate check:** Reject if node_id already in member list (stale join prevention)
5. **Cluster state check:** Reject if cluster is frozen or in transition

**State transfer on join:**
- Master sends full `MembersView` + `PartitionTable` in `JoinResponse`
- Joining node applies both atomically, transitions to `Active`
- Master broadcasts `MembersUpdate` to existing members with new node

### 3.4 Heartbeat and Failure Detection

**Architecture:** Each node runs an independent `FailureDetector` that monitors all other members. Only the master acts on failure detection results.

```rust
/// Pluggable failure detection trait.
/// Ported from Hazelcast's ClusterFailureDetector interface.
pub trait FailureDetector: Send + Sync {
    /// Record a heartbeat from a member at the given timestamp (ms).
    fn heartbeat(&self, node_id: &str, timestamp_ms: u64);

    /// Check if a member is considered alive at the given timestamp.
    fn is_alive(&self, node_id: &str, timestamp_ms: u64) -> bool;

    /// Get the last heartbeat timestamp for a member.
    fn last_heartbeat(&self, node_id: &str) -> Option<u64>;

    /// Get the suspicion level (phi value) for a member.
    /// 0.0 = no suspicion. Higher = more suspicious.
    fn suspicion_level(&self, node_id: &str, timestamp_ms: u64) -> f64;

    /// Remove a member from tracking.
    fn remove(&self, node_id: &str);

    /// Clear all state.
    fn reset(&self);
}
```

**Phi-accrual implementation (ported from TS):**

```rust
use std::collections::HashMap;
use std::sync::RwLock;

pub struct PhiAccrualFailureDetector {
    config: PhiAccrualConfig,
    states: RwLock<HashMap<String, NodeHeartbeatState>>,
}

pub struct PhiAccrualConfig {
    /// Phi threshold above which a node is suspected. Default: 8.0
    pub phi_threshold: f64,
    /// Maximum samples to keep in history. Default: 200
    pub max_sample_size: usize,
    /// Minimum standard deviation (ms). Default: 100
    pub min_std_dev_ms: u64,
    /// Maximum no-heartbeat time (ms). Default: 5000
    pub max_no_heartbeat_ms: u64,
    /// Expected heartbeat interval (ms). Default: 1000
    pub heartbeat_interval_ms: u64,
}

struct NodeHeartbeatState {
    last_heartbeat_ms: u64,
    intervals: Vec<u64>,
}

impl PhiAccrualFailureDetector {
    /// Calculate phi using the proper CDF-based formula.
    /// phi = -log10(1 - CDF(t_now - t_last))
    /// where CDF is the normal distribution with mean and stddev
    /// computed from the heartbeat interval history.
    fn calculate_phi(&self, state: &NodeHeartbeatState, now_ms: u64) -> f64 {
        let elapsed = now_ms.saturating_sub(state.last_heartbeat_ms);

        if state.intervals.len() < 3 {
            // Fallback: simple ratio-based detection
            return elapsed as f64 / self.config.heartbeat_interval_ms as f64;
        }

        let mean = state.intervals.iter().sum::<u64>() as f64
            / state.intervals.len() as f64;
        let variance = state.intervals.iter()
            .map(|&i| {
                let diff = i as f64 - mean;
                diff * diff
            })
            .sum::<f64>() / state.intervals.len() as f64;
        let std_dev = variance.sqrt().max(self.config.min_std_dev_ms as f64);

        // Phi accrual: -log10(1 - CDF(elapsed))
        // Using the complementary error function for better numerical stability
        let y = (elapsed as f64 - mean) / std_dev;
        let p = 1.0 - 0.5 * erfc(y / std::f64::consts::SQRT_2);
        if p >= 1.0 {
            return f64::MAX;
        }
        -p.log10()
    }
}

/// Complementary error function approximation (Abramowitz and Stegun).
fn erfc(x: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.3275911 * x.abs());
    let poly = t * (0.254829592 + t * (-0.284496736 + t * (1.421413741
        + t * (-1.453152027 + t * 1.061405429))));
    let result = poly * (-x * x).exp();
    if x >= 0.0 { result } else { 2.0 - result }
}
```

**Heartbeat flow:**
1. All nodes send `Heartbeat` messages to all other members at `heartbeat_interval_ms`
2. Receiver records heartbeat in local `PhiAccrualFailureDetector`
3. Periodic check: if `suspicion_level() > phi_threshold`, node transitions to `Suspect`
4. Non-master nodes report suspicions to master (heartbeat complaint, from Hazelcast)
5. Master decides: if multiple nodes suspect the same member, master removes it

### 3.5 Leave/Removal Protocol

**Graceful leave:**
1. Leaving node sends `LeaveRequest` to master
2. Master marks node as `Leaving` in MembersView, broadcasts update
3. Master initiates partition migration for all partitions owned by leaving node
4. Once all migrations complete, master removes node from MembersView
5. Master broadcasts final MembersView update

**Ungraceful removal (node crash):**
1. Failure detector confirms failure (Suspect -> Dead)
2. Master marks node as `Dead` in MembersView, broadcasts update
3. Master initiates partition reassignment (backups promoted to owners)
4. After grace period, master removes node from MembersView entirely

---

## 4. Partition Management

### 4.1 PartitionTable with Version Tracking

```rust
use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// Fixed-size partition table for 271 partitions.
/// Uses DashMap for lock-free reads per partition (TiKV pattern).
pub struct PartitionTable {
    /// Monotonically increasing version. Incremented on every assignment change.
    version: AtomicU64,
    /// Per-partition metadata. Key = partition_id (0-270).
    partitions: DashMap<u32, PartitionMeta>,
    /// Total partition count (271, fixed).
    partition_count: u32,
}

/// Metadata for a single partition.
#[derive(Debug, Clone)]
pub struct PartitionMeta {
    /// Partition identifier (0-based).
    pub partition_id: u32,
    /// Node ID of the primary owner.
    pub owner: String,
    /// Node IDs of backup replicas, ordered by priority.
    pub backups: Vec<String>,
    /// Current state on this node.
    pub state: PartitionState,
    /// Per-partition version (incremented on ownership change).
    pub version: u32,
}

/// The constant partition count (prime number for better distribution).
pub const PARTITION_COUNT: u32 = 271;

/// Default number of backup replicas per partition.
pub const DEFAULT_BACKUP_COUNT: u32 = 1;

impl PartitionTable {
    pub fn new() -> Self {
        let partitions = DashMap::with_capacity(PARTITION_COUNT as usize);
        for i in 0..PARTITION_COUNT {
            partitions.insert(i, PartitionMeta {
                partition_id: i,
                owner: String::new(),
                backups: Vec::new(),
                state: PartitionState::Unassigned,
                version: 0,
            });
        }
        Self {
            version: AtomicU64::new(0),
            partitions,
            partition_count: PARTITION_COUNT,
        }
    }

    /// Hash a key to a partition ID. Must match TS client's implementation.
    /// Uses FNV-1a hash (UTF-16 compatible) modulo 271.
    pub fn hash_to_partition(key: &str) -> u32 {
        topgun_core::hash::fnv1a_hash(key) % PARTITION_COUNT
    }

    /// Get the owner node for a key.
    pub fn get_owner(&self, key: &str) -> Option<String> {
        let pid = Self::hash_to_partition(key);
        self.partitions.get(&pid).map(|m| m.owner.clone())
    }

    /// Get partition metadata (lock-free read).
    pub fn get_partition(&self, partition_id: u32) -> Option<PartitionMeta> {
        self.partitions.get(&partition_id).map(|m| m.clone())
    }

    /// Set partition owner (master only). Increments version.
    pub fn set_owner(&self, partition_id: u32, owner: &str) {
        if let Some(mut meta) = self.partitions.get_mut(&partition_id) {
            meta.owner = owner.to_string();
            meta.version += 1;
        }
        self.version.fetch_add(1, Ordering::Release);
    }

    /// Get current table version.
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    /// Generate a PartitionMapPayload for client consumption.
    /// Uses the existing wire type from core-rust messages.
    pub fn to_partition_map(&self, members: &MembersView) -> topgun_core::messages::cluster::PartitionMapPayload {
        use topgun_core::messages::cluster::*;

        let nodes: Vec<NodeInfo> = members.members.iter().map(|m| {
            NodeInfo {
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
            }
        }).collect();

        let partitions: Vec<PartitionInfo> = (0..self.partition_count).filter_map(|i| {
            self.partitions.get(&i).map(|m| PartitionInfo {
                partition_id: m.partition_id,
                owner_node_id: m.owner.clone(),
                backup_node_ids: m.backups.clone(),
            })
        }).collect();

        PartitionMapPayload {
            version: self.version() as u32,
            partition_count: self.partition_count,
            nodes,
            partitions,
            generated_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
        }
    }
}
```

### 4.2 Partition Assignment Algorithm

The assignment algorithm runs on the master when membership changes.

```rust
/// Compute the ideal partition assignment for a set of active members.
/// Uses simple modulo distribution (matches TS behavior) with
/// round-robin backup assignment.
pub fn compute_assignment(
    members: &[MemberInfo],
    partition_count: u32,
    backup_count: u32,
) -> Vec<PartitionAssignment> {
    // Sort members deterministically (all nodes must compute same result)
    let mut sorted: Vec<&MemberInfo> = members.iter()
        .filter(|m| m.state == NodeState::Active)
        .collect();
    sorted.sort_by_key(|m| &m.node_id);

    let n = sorted.len();
    if n == 0 {
        return Vec::new();
    }

    (0..partition_count).map(|pid| {
        let owner_idx = pid as usize % n;
        let backups: Vec<String> = (1..=backup_count as usize)
            .filter(|_| n > 1)
            .map(|b| {
                let backup_idx = (owner_idx + b) % n;
                sorted[backup_idx].node_id.clone()
            })
            .collect();

        PartitionAssignment {
            partition_id: pid,
            owner: sorted[owner_idx].node_id.clone(),
            backups,
        }
    }).collect()
}

pub struct PartitionAssignment {
    pub partition_id: u32,
    pub owner: String,
    pub backups: Vec<String>,
}
```

### 4.3 Rebalancing Strategy

When nodes join or leave, the master:

1. Computes new ideal assignment using `compute_assignment()`
2. Diffs current vs ideal to identify partitions that need to move
3. Creates migration plan: ordered list of `MigrationTask` items
4. Executes migrations in batches (configurable concurrency)

```rust
/// Compute the set of partition moves needed to reach the target assignment.
pub fn plan_rebalance(
    current: &PartitionTable,
    target: &[PartitionAssignment],
) -> Vec<MigrationTask> {
    let mut tasks = Vec::new();

    for assignment in target {
        if let Some(current_meta) = current.get_partition(assignment.partition_id) {
            if current_meta.owner != assignment.owner {
                tasks.push(MigrationTask {
                    partition_id: assignment.partition_id,
                    source: current_meta.owner.clone(),
                    destination: assignment.owner.clone(),
                    new_backups: assignment.backups.clone(),
                });
            }
        }
    }

    // Sort by partition_id for deterministic ordering
    tasks.sort_by_key(|t| t.partition_id);
    tasks
}

pub struct MigrationTask {
    pub partition_id: u32,
    pub source: String,
    pub destination: String,
    pub new_backups: Vec<String>,
}
```

---

## 5. Migration Lifecycle

### 5.1 CRDT-Aware 2-Phase Migration

Unlike Hazelcast's 3-phase migration (prepare/lock, replicate, finalize), TopGun uses a 2-phase protocol because CRDTs eliminate the need for write locks:

```
Phase 1: REPLICATE                    Phase 2: FINALIZE
  Master -> Source: MigrateStart        Master -> Source: MigrateFinalize
  Source -> Dest: data stream           Source: state -> Draining -> Unassigned
  Source: continues accepting writes    Dest: state -> Active
  Dest: merges incoming data + writes   Master: update PartitionTable
                                        Master: broadcast new PartitionMapPayload
```

**Detailed flow:**

```
   Master           Source Node         Destination Node     Clients
     |                   |                    |                 |
     |-- MigrateStart -->|                    |                 |
     |   {pid, dest}     |                    |                 |
     |                   |                    |                 |
     |              set state=Migrating       |                 |
     |                   |                    |                 |
     |                   |-- MigrateData ---->|                 |
     |                   |   {pid, crdt_state,|                 |
     |                   |    delta_ops}      |                 |
     |                   |                    |                 |
     |                   |   (source still    | set state=      |
     |                   |    accepts writes) | Receiving       |
     |                   |                    | merge CRDT data |
     |                   |                    |                 |
     |                   |   (may send delta  |                 |
     |                   |    of writes that  |                 |
     |                   |    arrived during  |                 |
     |                   |    transfer)       |                 |
     |                   |                    |                 |
     |                   |-- MigrateReady --->|                 |
     |                   |                    |                 |
     |<- MigrateReady --|                    |                 |
     |                   |                    |                 |
     |-- MigrateFinalize -->|                 |                 |
     |-- MigrateFinalize --|---------------->|                 |
     |                   |                    |                 |
     |              set state=Draining   set state=Active      |
     |              forward new writes        |                 |
     |              to destination            |                 |
     |                   |                    |                 |
     | update PartitionTable                  |                 |
     | broadcast PartitionMapPayload ---------|---------------->|
     |                   |                    |                 |
     |              set state=Unassigned      |                 |
     |              cleanup local data        |                 |
```

### 5.2 Migration Data Format

```rust
/// Data transferred during partition migration.
/// Contains the full CRDT state for the partition's maps.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrateData {
    /// Partition being migrated.
    pub partition_id: u32,
    /// Serialized CRDT map states (map_name -> msgpack bytes).
    pub map_states: Vec<MapStateChunk>,
    /// Delta operations that occurred during migration transfer.
    pub delta_ops: Vec<DeltaOp>,
    /// Source's partition version at start of migration.
    pub source_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapStateChunk {
    pub map_name: String,
    /// MsgPack-serialized LWWMap or ORMap state.
    pub data: Vec<u8>,
    /// Map type for deserialization.
    pub map_type: String, // "lww" | "or"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeltaOp {
    pub map_name: String,
    pub key: String,
    /// MsgPack-serialized entry value with HLC timestamp.
    pub entry: Vec<u8>,
}
```

### 5.3 CRDT Merge During Migration

The key insight: because both source and destination can accept writes during migration, the destination simply merges all incoming data:

```rust
/// Pseudocode for destination node handling migration data.
async fn handle_migrate_data(
    &self,
    data: MigrateData,
    map_provider: &dyn MapProvider,
) -> anyhow::Result<()> {
    for chunk in &data.map_states {
        // Load or create the local map
        let map_type = match chunk.map_type.as_str() {
            "lww" => MapType::LWW,
            "or" => MapType::OR,
            _ => continue,
        };
        let local_map = map_provider
            .get_or_load_map(&chunk.map_name, map_type)
            .await?;

        // Deserialize the incoming CRDT state
        let incoming_state = deserialize_map_state(&chunk.data, map_type)?;

        // CRDT merge: this is the magic -- merge is commutative,
        // associative, and idempotent. Order doesn't matter.
        local_map.merge(&incoming_state);
    }

    // Apply delta operations (writes that happened during transfer)
    for delta in &data.delta_ops {
        let local_map = map_provider
            .get_or_load_map(&delta.map_name, MapType::LWW) // type from context
            .await?;
        local_map.apply_entry(&delta.key, &delta.entry);
    }

    Ok(())
}
```

### 5.4 Rollback on Failure

If migration fails (network error, destination crash):

1. Master marks migration as `FAILED`
2. Source transitions back from `Migrating` to `Active`
3. Destination discards received data (or keeps it -- CRDT merge is safe either way)
4. Master retries migration after backoff
5. No data loss: source never stopped accepting writes

### 5.5 Migration Ordering

Adapted from Hazelcast's `MigrationPlanner` -- key property: **never decrease available replica count**.

```rust
/// Order migrations to maintain availability.
/// Rules:
/// 1. Backup promotions (backup -> owner) before new copies
/// 2. Partitions with fewer replicas migrate first (most at risk)
/// 3. Limit concurrent migrations per node (configurable)
pub fn order_migrations(tasks: &mut Vec<MigrationTask>, partition_table: &PartitionTable) {
    tasks.sort_by_key(|task| {
        let meta = partition_table.get_partition(task.partition_id);
        let replica_count = meta.map(|m| 1 + m.backups.len()).unwrap_or(0);
        let is_promotion = meta
            .map(|m| m.backups.contains(&task.destination))
            .unwrap_or(false);
        // Promotions first (0), then by ascending replica count
        (if is_promotion { 0 } else { 1 }, replica_count)
    });
}
```

---

## 6. Split-Brain Handling

### 6.1 Detection Strategy (Master-Centric)

Adapted from Hazelcast's `SplitBrainHandler`:

```rust
/// Runs periodically on the master node.
/// Searches for other clusters that might have formed from a network partition.
pub async fn check_for_split_brain(
    cluster_service: &ClusterService,
    seed_addresses: &[SocketAddr],
) -> Option<SplitBrainMergeDecision> {
    if !cluster_service.is_master() {
        return None;
    }

    // Contact seed addresses that are NOT in our current member list
    let our_members = cluster_service.members_view();
    let our_addresses: HashSet<_> = our_members.members.iter()
        .map(|m| format!("{}:{}", m.host, m.cluster_port))
        .collect();

    for seed in seed_addresses {
        let seed_str = seed.to_string();
        if our_addresses.contains(&seed_str) {
            continue;
        }

        // Try to contact this seed -- if it responds with a different
        // cluster view, we have a split brain
        if let Ok(remote_view) = probe_remote_cluster(seed).await {
            if remote_view.cluster_id == cluster_service.cluster_id() {
                // Same cluster, different view -- split brain detected!
                return Some(decide_merge(&our_members, &remote_view));
            }
        }
    }
    None
}

pub enum SplitBrainMergeDecision {
    /// This cluster should merge into the remote cluster.
    LocalShouldMerge { remote_master: SocketAddr },
    /// The remote cluster should merge into this cluster.
    RemoteShouldMerge,
    /// Cannot merge (different cluster IDs or incompatible versions).
    CannotMerge,
}

/// Decide which side merges into which.
/// The cluster with more members wins. Ties broken by oldest master.
fn decide_merge(local: &MembersView, remote: &RemoteClusterInfo) -> SplitBrainMergeDecision {
    if local.members.len() > remote.member_count {
        SplitBrainMergeDecision::RemoteShouldMerge
    } else if local.members.len() < remote.member_count {
        SplitBrainMergeDecision::LocalShouldMerge {
            remote_master: remote.master_address,
        }
    } else {
        // Tie-breaker: older master wins
        let local_master_join = local.master()
            .map(|m| m.join_version)
            .unwrap_or(u64::MAX);
        if local_master_join <= remote.master_join_version {
            SplitBrainMergeDecision::RemoteShouldMerge
        } else {
            SplitBrainMergeDecision::LocalShouldMerge {
                remote_master: remote.master_address,
            }
        }
    }
}
```

### 6.2 Recovery Protocol

**CRDT advantage:** Split-brain recovery is dramatically simpler than Hazelcast because CRDTs merge automatically.

```
1. Split-brain detected by master of smaller cluster
2. Smaller cluster initiates merge:
   a. Master sends MergeRequest to larger cluster's master
   b. Larger master assigns partition ownership for merged nodes
   c. Merged nodes receive new MembersView + PartitionTable
3. For partitions that existed on both sides during split:
   a. Both sides have independent CRDT states
   b. CRDT merge produces correct result automatically
   c. No "merge policy" needed (unlike Hazelcast)
4. For partitions that only existed on one side:
   a. Simply assign to the existing owner
   b. No data conflict possible
```

**Hazelcast vs TopGun split-brain recovery:**

| Step | Hazelcast | TopGun |
|------|-----------|--------|
| Detection | Master probes seeds | Same |
| Decision | Larger cluster wins | Same |
| Data reconciliation | Run `SplitBrainMergePolicy` per map entry | CRDT auto-merge |
| Conflict resolution | LATEST_UPDATE / HIGHER_HITS / CUSTOM | **Not needed** (LWW-Map: latest timestamp wins; OR-Map: union of entries) |
| Complexity | High (100+ lines of merge logic per policy) | **Zero** (merge is a CRDT property) |

---

## 7. Rust Concurrency Design

### 7.1 Decision: Shared State with DashMap (NOT Per-Partition FSM)

**Decision matrix:**

| Criterion | Per-Partition FSM (TiKV) | Shared State + DashMap | Winner |
|-----------|-------------------------|----------------------|--------|
| Partition count | TiKV: millions of regions | TopGun: 271 (fixed) | **DashMap** (271 FSMs is overkill) |
| Message routing | Mailbox per region | DashMap lookup | **DashMap** (simpler) |
| Write contention | Low (1 FSM per region) | Low (DashMap shards >> 271) | Tie |
| Implementation complexity | High (FSM trait, Poller, batch scheduler) | Low (DashMap + tokio tasks) | **DashMap** |
| Migration state machine | Natural (FSM transitions) | Manual state tracking | FSM (but not worth complexity) |
| Memory overhead | ~1KB per FSM + channel | ~100 bytes per DashMap entry | **DashMap** |
| Debugging | Hard (async FSM scheduling) | Easy (standard async) | **DashMap** |

**Verdict:** With only 271 partitions, the overhead of a per-partition FSM (mailbox, channel, scheduler) is not justified. DashMap provides the same lock-free concurrent access with far less complexity. TiKV uses FSMs because it manages millions of regions on a single node.

### 7.2 Concurrency Architecture

```rust
use dashmap::DashMap;
use arc_swap::ArcSwap;
use tokio::sync::{mpsc, watch, RwLock};

/// Top-level cluster state, shared across all components.
pub struct ClusterState {
    /// Current membership (lock-free read via ArcSwap).
    pub membership: ArcSwap<MembersView>,

    /// Partition table (lock-free per-partition access via DashMap).
    pub partition_table: PartitionTable,

    /// Active migrations (small map, RwLock is fine).
    pub active_migrations: RwLock<HashMap<u32, ActiveMigration>>,

    /// Channel for broadcasting cluster change events to interested
    /// components (Quickwit pattern: ClusterChangeStream).
    pub change_tx: mpsc::UnboundedSender<ClusterChange>,
}

/// Cluster change events for reactive components.
#[derive(Debug, Clone)]
pub enum ClusterChange {
    /// A new member joined the cluster.
    MemberAdded(MemberInfo),
    /// A member's state changed.
    MemberUpdated(MemberInfo),
    /// A member was removed from the cluster.
    MemberRemoved(MemberInfo),
    /// Partition ownership changed.
    PartitionMoved {
        partition_id: u32,
        old_owner: String,
        new_owner: String,
    },
    /// Full partition table was updated (after rebalance).
    PartitionTableUpdated { version: u64 },
}
```

### 7.3 How 271 Partitions Map to Tokio Tasks

```
                    tokio runtime (multi-threaded)
                    ================================

Master tasks (if this node is master):
  - heartbeat_sender: sends heartbeats to all members (1 task)
  - failure_checker: periodic phi check + suspicion (1 task)
  - split_brain_checker: periodic seed probing (1 task)
  - migration_coordinator: manages active migrations (1 task)

All-node tasks:
  - heartbeat_receiver: processes incoming heartbeats (1 task)
  - membership_listener: processes MembersUpdate messages (1 task)
  - cluster_connection_manager: manages inter-node WebSocket connections (1 task)

Per-partition work:
  - NOT separate tasks. Partition operations are dispatched via
    DashMap lookup within the request handler task.
  - Migration data transfer uses a bounded channel per active
    migration (at most `parallel_transfers` concurrent channels).

Client-facing:
  - WebSocket acceptor: 1 task
  - Per-client connection: 1 task each (handles all partitions)
```

**Total task count:** ~7 background tasks + 1 per peer connection + 1 per client connection. This is far simpler than a per-partition FSM model.

### 7.4 Channel-Based Communication

```rust
/// Inter-component communication channels.
pub struct ClusterChannels {
    /// Membership changes: master -> all components
    pub membership_changes: watch::Sender<Arc<MembersView>>,

    /// Cluster events: any -> interested components (reactive)
    pub cluster_events: mpsc::UnboundedSender<ClusterChange>,

    /// Migration control: master -> migration coordinator
    pub migration_commands: mpsc::Sender<MigrationCommand>,

    /// Inbound cluster messages: network layer -> cluster service
    pub inbound_messages: mpsc::Sender<InboundClusterMessage>,
}

#[derive(Debug)]
pub enum MigrationCommand {
    Start(MigrationTask),
    Cancel(u32), // partition_id
    CancelAll,
}

#[derive(Debug)]
pub struct InboundClusterMessage {
    pub sender_node_id: String,
    pub message: ClusterMessage,
}
```

---

## 8. Trait Hierarchy

### 8.1 ClusterService (top-level)

```rust
use async_trait::async_trait;

/// Top-level cluster service trait.
/// Coordinates membership, partitions, migration, and failure detection.
#[async_trait]
pub trait ClusterService: Send + Sync {
    /// Start the cluster service. Initiates join if peers are configured.
    async fn start(&self) -> anyhow::Result<()>;

    /// Initiate graceful shutdown. Migrates partitions away, then leaves.
    async fn shutdown(&self) -> anyhow::Result<()>;

    /// Get the local node's ID.
    fn node_id(&self) -> &str;

    /// Check if this node is the current master.
    fn is_master(&self) -> bool;

    /// Get the current master's node ID.
    fn master_id(&self) -> Option<String>;

    /// Get the current MembersView.
    fn members_view(&self) -> Arc<MembersView>;

    /// Get the partition table.
    fn partition_table(&self) -> &PartitionTable;

    /// Get a stream of cluster change events.
    fn subscribe_changes(&self) -> ClusterChangeStream;

    /// Get cluster health metrics.
    fn health(&self) -> ClusterHealth;
}

pub struct ClusterHealth {
    pub node_count: usize,
    pub active_nodes: usize,
    pub suspect_nodes: usize,
    pub partition_table_version: u64,
    pub active_migrations: usize,
    pub is_master: bool,
    pub master_node_id: Option<String>,
}
```

### 8.2 MembershipService

```rust
/// Manages the cluster membership list.
/// Only the master mutates membership; all nodes read it.
#[async_trait]
pub trait MembershipService: Send + Sync {
    /// Get the current MembersView (lock-free).
    fn current_view(&self) -> Arc<MembersView>;

    /// Get a specific member by node ID.
    fn get_member(&self, node_id: &str) -> Option<MemberInfo>;

    /// Get all active members.
    fn active_members(&self) -> Vec<MemberInfo>;

    /// [Master only] Process a join request from a new node.
    async fn handle_join_request(&self, request: JoinRequest) -> JoinResponse;

    /// [Master only] Process a leave request from a departing node.
    async fn handle_leave_request(&self, node_id: &str) -> anyhow::Result<()>;

    /// [Master only] Remove a failed node from membership.
    async fn remove_member(&self, node_id: &str) -> anyhow::Result<()>;

    /// [Non-master] Apply a MembersView update from master.
    fn apply_members_update(&self, view: MembersView);
}
```

### 8.3 PartitionService

```rust
/// Manages partition ownership and provides routing for operations.
/// Extends the basic PartitionTable (TODO-063) with full lifecycle management.
#[async_trait]
pub trait PartitionService: Send + Sync {
    /// Hash a key to its partition ID. Must be cross-language compatible.
    fn hash_to_partition(&self, key: &str) -> u32;

    /// Get the owner node for a partition.
    fn get_owner(&self, partition_id: u32) -> Option<String>;

    /// Check if the local node owns this partition.
    fn is_local_owner(&self, partition_id: u32) -> bool;

    /// Check if the local node is a backup for this partition.
    fn is_local_backup(&self, partition_id: u32) -> bool;

    /// Get partition state on the local node.
    fn get_state(&self, partition_id: u32) -> PartitionState;

    /// Get the full partition map for client consumption.
    fn get_partition_map(&self, members: &MembersView) -> PartitionMapPayload;

    /// Get the current partition table version.
    fn version(&self) -> u64;

    /// [Master only] Rebalance partitions after membership change.
    async fn rebalance(&self, members: &MembersView) -> Vec<MigrationTask>;

    /// Apply a partition table update from master.
    fn apply_partition_update(&self, assignments: &[PartitionAssignment]);

    /// Get all partitions owned by a specific node.
    fn partitions_for_node(&self, node_id: &str) -> Vec<u32>;
}
```

### 8.4 MigrationService

```rust
/// Manages the lifecycle of partition migrations.
/// Only the master initiates migrations; source and destination execute them.
#[async_trait]
pub trait MigrationService: Send + Sync {
    /// [Master] Start a batch of migrations.
    async fn start_migrations(&self, tasks: Vec<MigrationTask>) -> anyhow::Result<()>;

    /// [Master] Cancel a specific migration.
    async fn cancel_migration(&self, partition_id: u32) -> anyhow::Result<()>;

    /// [Master] Cancel all active migrations.
    async fn cancel_all(&self) -> anyhow::Result<()>;

    /// [Source] Handle migration start request from master.
    async fn handle_migrate_start(&self, partition_id: u32, destination: &str) -> anyhow::Result<()>;

    /// [Destination] Handle incoming migration data.
    async fn handle_migrate_data(&self, data: MigrateData) -> anyhow::Result<()>;

    /// [Master] Handle migration completion notification.
    async fn handle_migrate_ready(&self, partition_id: u32, source: &str) -> anyhow::Result<()>;

    /// Get migration status.
    fn status(&self) -> MigrationStatus;

    /// Check if a specific partition is currently migrating.
    fn is_migrating(&self, partition_id: u32) -> bool;
}
```

### 8.5 FailureDetector (trait with phi-accrual implementation)

```rust
/// Pluggable failure detection.
/// Implementations: PhiAccrualFailureDetector (default), DeadlineFailureDetector (simple).
pub trait FailureDetector: Send + Sync {
    /// Record a heartbeat from a member.
    fn heartbeat(&self, node_id: &str, timestamp_ms: u64);

    /// Check if a member is considered alive.
    fn is_alive(&self, node_id: &str, timestamp_ms: u64) -> bool;

    /// Get the last heartbeat timestamp for a member.
    fn last_heartbeat(&self, node_id: &str) -> Option<u64>;

    /// Get the suspicion level (phi value). 0.0 = no suspicion.
    fn suspicion_level(&self, node_id: &str, timestamp_ms: u64) -> f64;

    /// Remove a member from tracking.
    fn remove(&self, node_id: &str);

    /// Clear all state.
    fn reset(&self);
}

/// Simple deadline-based failure detector.
/// A member is dead if no heartbeat received within max_no_heartbeat_ms.
pub struct DeadlineFailureDetector {
    max_no_heartbeat_ms: u64,
    states: RwLock<HashMap<String, u64>>, // node_id -> last_heartbeat_ms
}

/// Phi-accrual failure detector (ported from TS, enhanced with proper CDF).
/// See Section 3.4 for full implementation.
pub struct PhiAccrualFailureDetector {
    config: PhiAccrualConfig,
    states: RwLock<HashMap<String, NodeHeartbeatState>>,
}
```

---

## 9. Wire Protocol

### 9.1 Cluster-Internal Message Types

These extend the existing `Message` enum from SPEC-052. They are used for inter-node communication only (not client-facing).

```rust
/// Cluster-internal messages. These are NOT part of the client-facing
/// Message enum. They travel on the inter-node WebSocket connections.
///
/// Serialized with rmp_serde::to_vec_named() (MsgPack, named fields).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ClusterMessage {
    // --- Membership ---

    /// Node requesting to join the cluster (sent to master).
    JoinRequest(JoinRequest),
    /// Master's response to a join request.
    JoinResponse(JoinResponse),
    /// Master broadcasting updated membership to all nodes.
    MembersUpdate(MembersUpdate),
    /// Node initiating graceful leave.
    LeaveRequest(LeaveRequest),

    // --- Heartbeat ---

    /// Periodic heartbeat between nodes.
    Heartbeat(HeartbeatPayload),
    /// Non-master reporting a heartbeat anomaly to master.
    HeartbeatComplaint(HeartbeatComplaint),
    /// Master sending explicit suspicion notification.
    ExplicitSuspicion(ExplicitSuspicion),

    // --- Partition Table ---

    /// Master broadcasting partition table update.
    PartitionTableUpdate(PartitionTableUpdate),
    /// Node requesting current partition table (e.g., after reconnect).
    FetchPartitionTable,

    // --- Migration ---

    /// Master instructing source to start migration.
    MigrateStart(MigrateStartPayload),
    /// Source sending partition data to destination.
    MigrateData(MigrateData),
    /// Source/destination signaling migration readiness.
    MigrateReady(MigrateReadyPayload),
    /// Master finalizing migration (commit ownership change).
    MigrateFinalize(MigrateFinalizePayload),
    /// Master canceling a migration.
    MigrateCancel(MigrateCancelPayload),

    // --- Split-Brain ---

    /// Master probing for other clusters.
    SplitBrainProbe(SplitBrainProbe),
    /// Response to split-brain probe.
    SplitBrainProbeResponse(SplitBrainProbeResponse),
    /// Merge request from smaller cluster to larger.
    MergeRequest(MergeRequest),

    // --- Existing cluster messages (from TS) ---
    // These already exist in the Message enum (SPEC-052) and are
    // forwarded on the inter-node connection:

    /// Forward a client operation to the partition owner.
    OpForward(OpForwardPayload),
    /// Cluster event broadcast.
    ClusterEvent(ClusterEventPayload),
}
```

### 9.2 Key Payload Structures

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinRequest {
    pub node_id: String,
    pub host: String,
    pub client_port: u16,
    pub cluster_port: u16,
    pub cluster_id: String,
    pub protocol_version: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub auth_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinResponse {
    pub accepted: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reject_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub members_view: Option<MembersView>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub partition_assignments: Option<Vec<PartitionAssignment>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MembersUpdate {
    /// Full MembersView with version.
    pub view: MembersView,
    /// Cluster timestamp at time of update.
    pub cluster_time_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatPayload {
    pub sender_id: String,
    pub timestamp_ms: u64,
    /// Sender's current MembersView version (for staleness detection).
    pub members_view_version: u64,
    /// Nodes this sender suspects are failed.
    pub suspected_nodes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatComplaint {
    /// The complaining node's metadata.
    pub complainer_id: String,
    pub complainer_view_version: u64,
    /// The node being complained about.
    pub suspect_id: String,
    pub suspect_view_version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartitionTableUpdate {
    /// Full set of partition assignments.
    pub assignments: Vec<PartitionAssignment>,
    /// Global partition table version.
    pub version: u64,
    /// List of completed migration IDs for deduplication.
    pub completed_migrations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrateStartPayload {
    pub migration_id: String,
    pub partition_id: u32,
    pub destination_node_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrateReadyPayload {
    pub migration_id: String,
    pub partition_id: u32,
    pub source_node_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrateFinalizePayload {
    pub migration_id: String,
    pub partition_id: u32,
    pub new_owner: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrateCancelPayload {
    pub migration_id: String,
    pub partition_id: u32,
    pub reason: String,
}
```

### 9.3 Mapping to Existing Message Enum

The cluster-internal messages are **separate** from the client-facing `Message` enum (SPEC-052). They share the same MsgPack wire format but travel on different connections:

| Connection Type | Message Type | Wire Format |
|----------------|-------------|-------------|
| Client WebSocket | `Message` enum (77 variants from SPEC-052) | MsgPack (rmp_serde, named fields) |
| Inter-node WebSocket | `ClusterMessage` enum (above) | MsgPack (rmp_serde, named fields) |

The existing client-facing message types that also travel inter-node (like `PartitionMapPayload`, `OpForward`) are wrapped in `ClusterMessage::OpForward` for inter-node transport.

---

## 10. Client Integration

### 10.1 Partition Map Discovery

Clients are first-class cluster participants in TopGun. They receive the full partition map and route operations directly to the owning node.

```
Client                    Any Server Node
  |                            |
  |--- PartitionMapRequest --->|   (existing message type)
  |    {currentVersion: 5}     |
  |                            |
  |                      check version
  |                      if server version > 5:
  |                            |
  |<--- PartitionMapPayload ---|   (existing message type)
  |    {version: 7,            |
  |     partitionCount: 271,   |
  |     nodes: [...],          |
  |     partitions: [...]}     |
  |                            |
```

### 10.2 Partition Map Push

When the partition table changes (rebalance, migration), the master broadcasts the updated map. Server nodes push it to all connected clients:

```rust
/// When partition table changes, push to all connected clients.
async fn broadcast_partition_map_to_clients(
    partition_service: &dyn PartitionService,
    members: &MembersView,
    client_connections: &[ClientConnection],
) {
    let map = partition_service.get_partition_map(members);
    let msg = Message::PartitionMapResponse {
        payload: map,
    };
    let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");

    for conn in client_connections {
        let _ = conn.send(&bytes).await;
    }
}
```

### 10.3 NOT_OWNER Handling

When a client sends an operation to the wrong node (stale partition map):

```rust
/// Handle an operation for a partition this node doesn't own.
async fn handle_not_owner(
    partition_id: u32,
    partition_service: &dyn PartitionService,
    members: &MembersView,
) -> Message {
    // Send the current partition map so client can update routing
    let map = partition_service.get_partition_map(members);
    Message::NotOwner {
        payload: NotOwnerPayload {
            partition_id,
            owner_node_id: partition_service.get_owner(partition_id),
            partition_map: Some(map),
        },
    }
}
```

The client receives `NotOwner`, updates its local partition map, and retries the operation to the correct node. This is already implemented in the TS client (`SyncEngine` handles `NOT_OWNER`).

### 10.4 Client-Side Routing

The TS client already has `PartitionRouter` that:
1. Maintains a local `PartitionMap`
2. Computes `fnv1a_hash(key) % 271` for each operation
3. Routes to the owning node's WebSocket connection
4. On `NOT_OWNER` response: updates map, retries

No changes needed to the TS client for the Rust server cluster protocol.

---

## 11. Migration Path

### 11.1 Phase 2 -> Phase 3 Evolution

```
Phase 2 (TODO-063 basic):
  - hash_to_partition(key) -> u32        [DONE: core-rust/hash.rs]
  - PartitionTable struct (DashMap)       [basic version]
  - PartitionMapPayload wire type         [DONE: core-rust/messages/cluster.rs]
  - No cluster, no migration, no state machine

Phase 3 (TODO-066 full, informed by this research):
  - ClusterService + MembershipService
  - PartitionService (extends basic table)
  - MigrationService with 2-phase CRDT-aware protocol
  - FailureDetector (phi-accrual, ported from TS)
  - Versioned MembersView with master election
  - Split-brain detection + CRDT auto-recovery
  - Wire protocol: ClusterMessage enum
  - Client partition map push
```

### 11.2 Incremental Implementation Plan

**Wave 1: Static cluster (no migration)**
1. `PartitionTable` with DashMap + `hash_to_partition()` (TODO-063)
2. `MembershipState` with ArcSwap<MembersView>
3. `PhiAccrualFailureDetector` (port from TS)
4. Master election (oldest-member convention)
5. Join ceremony (JoinRequest/JoinResponse)
6. Heartbeat loop + failure detection
7. Static partition assignment (compute on join, no migration)

**Wave 2: Dynamic cluster (with migration)**
1. `MigrationService` with 2-phase CRDT-aware protocol
2. Rebalancing on membership change
3. Migration ordering (availability-preserving)
4. Partition state machine (Active -> Migrating -> Active)
5. NOT_OWNER handling + partition map push

**Wave 3: Resilience**
1. Split-brain detection
2. CRDT auto-recovery on merge
3. Graceful leave protocol
4. Mastership claim (after master crash)
5. Heartbeat complaint protocol

### 11.3 Crate Dependencies

```toml
[dependencies]
# Concurrency
dashmap = "6"            # Lock-free per-partition lookup
arc-swap = "1"           # Lock-free ArcSwap<MembersView>
tokio = { version = "1", features = ["full"] }

# Serialization (already in use)
serde = { version = "1", features = ["derive"] }
rmp-serde = "1"          # MsgPack wire format

# Networking (from TODO-064)
axum = "0.7"
tokio-tungstenite = "0.24"

# Utilities
tracing = "0.1"          # Structured logging
uuid = { version = "1", features = ["v4"] }
```

No new major dependencies. `dashmap` and `arc-swap` are the only additions to the existing Cargo workspace.

---

## Appendix A: Hazelcast Concepts NOT Adopted

| Hazelcast Concept | Why Not Adopted |
|-------------------|----------------|
| Raft CP subsystem | TopGun uses CRDTs, not consensus |
| WAN replication | Out of scope for initial release |
| Hot Restart persistence | PostgreSQL + S3 handle persistence differently |
| ICMP ping failure detection | Unnecessary complexity for WebSocket-connected nodes |
| Lite members | All TopGun nodes are data members |
| Partition groups (rack-aware) | Future Phase 5 concern, not needed for 271 partitions |
| CP member list | No CP subsystem |

## Appendix B: Comparison with TS Implementation

| Feature | TS Server (current) | Rust Server (this design) |
|---------|-------------------|--------------------------|
| Membership versioning | None | Versioned MembersView with ArcSwap |
| Master election | None (implicit) | Oldest-member convention (explicit) |
| Join ceremony | HELLO message | Multi-step: validate, auth, state transfer |
| Failure detection | Phi-accrual (good) | Phi-accrual (ported, enhanced CDF) |
| Partition assignment | `i % members.length` | Same algorithm, but with version tracking |
| Migration | Basic chunk transfer | 2-phase CRDT-aware (no write locks) |
| Split-brain | None | Master-centric detection + CRDT auto-recovery |
| Partition state machine | 4 states (TS enum) | 6 states (richer: Unassigned, Draining, Lost) |
| Client routing | Partition map push | Same (wire-compatible PartitionMapPayload) |
| Concurrency model | Single-threaded event loop | Multi-threaded tokio + DashMap |

---

*This document is the primary architectural reference for TODO-066 (Cluster Protocol) and the advanced partition management parts of TODO-063. Implementation specs should reference specific sections of this document.*
