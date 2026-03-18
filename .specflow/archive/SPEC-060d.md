# SPEC-060d: Cluster Protocol — Migration Service Implementation (Wave 2)

```yaml
id: SPEC-060d
type: feature
status: done
priority: P1
complexity: medium
parent: SPEC-060
depends_on: [SPEC-060c, TODO-064]
created: 2026-02-22
todo: TODO-066
```

## Context

This is the fourth sub-specification of SPEC-060, covering the Wave 2 (Dynamic Cluster) scope. It implements the 2-phase CRDT-aware migration protocol, partition state machine transitions during migration, NOT_OWNER response generation, and partition map push to connected clients.

**Activated:** Dependencies resolved — TODO-064 (networking layer) and SPEC-060c (cluster types/traits/algorithms) are both complete.

### Scope

- `MigrationCoordinator` struct implementing the `MigrationService` trait (2-phase CRDT-aware protocol per research Section 5.1–5.4)
- Partition state machine transitions: Active -> Migrating -> Draining -> Unassigned (source), Unassigned -> Receiving -> Active (destination)
- `not_owner_response()` free function returning current `PartitionMapPayload`
- `broadcast_partition_map()` free function pushing map to all `ConnectionKind::Client` connections
- Migration concurrency control via `ClusterConfig.max_parallel_migrations`
- Migration rollback on failure (source transitions back to Active)
- `RebalanceTrigger` background task watching `ClusterChange` events for membership changes

## Requirements

### R1: MigrationCoordinator Struct

```rust
pub struct MigrationCoordinator {
    state: Arc<ClusterState>,
    registry: Arc<ConnectionRegistry>,
    local_node_id: String,
    map_provider: Arc<dyn MapProvider>,
}
```

- `state: Arc<ClusterState>` — shared cluster state providing `partition_table`, `active_migrations`, `config`, and `local_node_id`
- `registry: Arc<ConnectionRegistry>` — for sending `ClusterMessage` to peer nodes and broadcasting `PartitionMapPayload` to clients
- `local_node_id: String` — cached local node ID (avoids repeated `state.local_node_id` cloning)
- `map_provider: Arc<dyn MapProvider>` — storage seam for serializing/deserializing CRDT map state during migration (see R2.4, R2.5)
- Derives: none (not serializable)
- Constructor: `pub fn new(state: Arc<ClusterState>, registry: Arc<ConnectionRegistry>, map_provider: Arc<dyn MapProvider>) -> Self`

### R2: MigrationService Trait Implementation

`MigrationCoordinator` implements `MigrationService` (defined in `traits.rs`) with the following behavior:

#### R2.1: `start_migrations(tasks: Vec<MigrationTask>) -> anyhow::Result<()>`

Role: **master node only.** Called when the master has computed a rebalance plan.

1. Acquire a write lock on `state.active_migrations`.
2. Count currently active migrations. If count >= `state.config.max_parallel_migrations`, skip remaining tasks and return `Ok(())` without starting them (tasks beyond the concurrency limit are silently dropped — the rebalance trigger will recompute them on the next membership change).
3. For each task (up to the concurrency limit), generate a UUID `migration_id`, construct an `ActiveMigration` with `state = MigrationPhase::Replicating`, `started_at_ms = current_unix_ms()`, and `new_backups` copied from `task.new_backups`, and insert into `state.active_migrations`.
4. Set partition state to `PartitionState::Migrating` on the source via `state.partition_table.set_state(task.partition_id, PartitionState::Migrating)`.
5. Send `ClusterMessage::MigrateStart(MigrateStartPayload { migration_id, partition_id, destination_node_id })` to the source node via `send_to_peer()` helper.
6. Return `Ok(())`.

#### R2.2: `cancel_migration(partition_id: u32) -> anyhow::Result<()>`

1. Acquire a write lock on `state.active_migrations`.
2. Remove the `ActiveMigration` entry for the given `partition_id`. If none exists, return `Ok(())`.
3. Set the source partition state back to `PartitionState::Active` via `state.partition_table.set_state()`.
4. Send `ClusterMessage::MigrateCancel(MigrateCancelPayload { migration_id, partition_id, reason: "cancelled".to_string() })` to both source and destination nodes.
5. Return `Ok(())`.

#### R2.3: `cancel_all() -> anyhow::Result<()>`

1. Acquire a write lock on `state.active_migrations`.
2. Collect all entries, clear the map.
3. For each cancelled migration: set source partition state to `PartitionState::Active`; send `MigrateCancel` to source and destination.
4. Return `Ok(())`.

#### R2.4: `handle_migrate_start(partition_id: u32, destination: &str) -> anyhow::Result<()>`

Role: **source node.** Called when the source node receives `MigrateStart` from the master.

1. Set partition state to `PartitionState::Migrating` via `state.partition_table.set_state()`.
2. Serialize CRDT state for the partition: iterate all maps owned by `partition_id`, serialize each to `MapStateChunk { map_name, data: rmp_serde::to_vec_named(&map)?, map_type }`. For this spec, the map state is represented as an opaque `Vec<u8>` (concrete CRDT map lookup is wired in the storage module, out of scope here — use a `MapProvider` trait stub injected into `MigrationCoordinator`).
3. Send `ClusterMessage::MigrateData(MigrateDataPayload { partition_id, map_states, delta_ops: vec![], source_version })` to destination node. **Note:** `source_version` is obtained from `state.partition_table.version()` — it captures the partition table version at the time of migration so the destination can detect stale transfers.
4. Determine the master node ID via `state.current_view().master().map(|m| m.node_id.clone())`. Send `ClusterMessage::MigrateReady(MigrateReadyPayload { migration_id: "".to_string(), partition_id, source_node_id: local_node_id })` to the master node. **Note:** The empty `migration_id` is intentional. The `handle_migrate_start` trait method does not receive the `migration_id` from the master, so the source node cannot populate it. This is safe because the master's `handle_migrate_ready` (R2.6) correlates migrations by `partition_id`, not `migration_id`.
5. Return `Ok(())`.

**Error handling:** Steps 3 and 4 call `send_to_peer()` which can fail. Propagate errors via `?` — if `send_to_peer` fails, the method returns `Err` to the caller. No rollback is needed because the partition state (`Migrating`) is correct regardless; the master will detect the missing `MigrateReady` and can cancel the migration via timeout (future spec).

**Note on MapProvider:** Because the storage module (Phase 4) is not yet implemented, `MigrationCoordinator` accepts a `map_provider: Arc<dyn MapProvider>` field. `MapProvider` is a new trait defined in `migration.rs`:

```rust
pub trait MapProvider: Send + Sync {
    fn get_partition_maps(&self, partition_id: u32) -> Vec<MapStateChunk>;
}
```

This allows unit tests to inject a stub without the full storage layer.

#### R2.5: `handle_migrate_data(data: MigrateDataPayload) -> anyhow::Result<()>`

Role: **destination node.** Called when the destination node receives `MigrateData`.

1. Set partition state to `PartitionState::Receiving` via `state.partition_table.set_state()`.
2. For each `MapStateChunk` in `data.map_states`: call `map_provider.receive_map_chunk(chunk)` (see `MapProvider` below — no-op in stub).
3. For each `DeltaOp` in `data.delta_ops`: call `map_provider.apply_delta_op(op)` (no-op in stub).
4. Return `Ok(())`.

**Error handling:** This method does not call `send_to_peer()`, so no network errors are possible. `MapProvider` methods are infallible (no `Result` return). If a future `MapProvider` revision adds fallible methods, errors should propagate via `?`.

`MapProvider` gains two additional methods:

```rust
pub trait MapProvider: Send + Sync {
    fn get_partition_maps(&self, partition_id: u32) -> Vec<MapStateChunk>;
    fn receive_map_chunk(&self, chunk: MapStateChunk);
    fn apply_delta_op(&self, op: DeltaOp);
}
```

#### R2.6: `handle_migrate_ready(partition_id: u32, source: &str) -> anyhow::Result<()>`

Role: **master node.** Called when master receives `MigrateReady` from source.

1. Acquire a write lock on `state.active_migrations`.
2. Find the `ActiveMigration` for `partition_id`. Return `Ok(())` if not found (idempotent).
3. Advance phase to `MigrationPhase::Finalizing`.
4. Set source partition state to `PartitionState::Draining` via `state.partition_table.set_state()`.
5. Send `ClusterMessage::MigrateFinalize(MigrateFinalizePayload { migration_id, partition_id, new_owner: destination })` to both source and destination nodes.
6. Update partition ownership in `state.partition_table.set_owner(partition_id, destination, active_migration.new_backups.clone())`.
7. Increment partition table version via `state.partition_table.increment_version()`.
8. Emit `ClusterChange::PartitionMoved { partition_id, old_owner: source, new_owner: destination }` via `state.change_sender().send()`.
9. Remove the `ActiveMigration` entry from `state.active_migrations`.
10. Call `broadcast_partition_map(&state.partition_table, &state.current_view(), &registry)` to push updated map to all clients.
11. Return `Ok(())`.

**Rollback path:** If any step in R2.6 returns an error, the implementation must:
- Set source partition state back to `PartitionState::Active`.
- Set `ActiveMigration.state` to `MigrationPhase::Failed`.
- Send `MigrateCancel` to source and destination.
- Remove the migration from `state.active_migrations`.

#### R2.7: `is_migrating(partition_id: u32) -> bool`

Returns `true` if `state.active_migrations` (blocking read via `try_read()` or synchronous read) contains an entry for `partition_id`.

### R3: `send_to_peer()` Helper (private)

```rust
async fn send_to_peer(&self, node_id: &str, msg: &ClusterMessage) -> anyhow::Result<()>
```

1. Serialize `msg` via `rmp_serde::to_vec_named(msg)?`.
2. Iterate `registry.connections()`, find the connection where `metadata.read().await.peer_node_id == Some(node_id)` and `kind == ConnectionKind::ClusterPeer`.
3. Call `handle.try_send(OutboundMessage::Binary(bytes))`. If `false`, return `Err(anyhow::anyhow!("peer {} channel full or disconnected", node_id))`.
4. If no connection is found, return `Err(anyhow::anyhow!("no connection to peer {}", node_id))`.

### R4: `not_owner_response()` Free Function

```rust
pub fn not_owner_response(
    table: &ClusterPartitionTable,
    members: &MembersView,
) -> PartitionMapPayload
```

Calls `table.to_partition_map(members)` and returns the result. This function is stateless — callers provide the current table and view snapshot. Used by message handlers when a client operation targets a partition not owned by the local node.

### R5: `broadcast_partition_map()` Free Function

```rust
pub fn broadcast_partition_map(
    table: &ClusterPartitionTable,
    members: &MembersView,
    registry: &ConnectionRegistry,
)
```

1. Build `PartitionMapPayload` via `table.to_partition_map(members)`.
2. Wrap in a server message envelope (placeholder: serialize as `rmp_serde::to_vec_named(&map)?` — the exact envelope type is wired in the handlers module, out of scope here; this function serializes the raw payload).
3. Call `registry.broadcast(&bytes, ConnectionKind::Client)`.

### R6: `RebalanceTrigger` Struct

```rust
pub struct RebalanceTrigger {
    state: Arc<ClusterState>,
    migration_tx: mpsc::Sender<MigrationCommand>,
}
```

- Constructor: `pub fn new(state: Arc<ClusterState>, migration_tx: mpsc::Sender<MigrationCommand>) -> Self`
- Method: `pub async fn run(self, mut change_rx: mpsc::UnboundedReceiver<ClusterChange>)`

`run()` loop:
1. `while let Some(change) = change_rx.recv().await`
2. Match on `ClusterChange::MemberAdded | MemberRemoved | MemberUpdated`: if the local node is master (`state.is_master()`), compute rebalance in two steps:
   - Step 1: Call `compute_assignment(&view.members, state.partition_table.partition_count(), state.config.backup_count)` using `state.current_view()` to produce target `Vec<PartitionAssignment>`. **Note:** `compute_assignment` internally filters for `NodeState::Active` members, so passing the full `view.members` slice is correct.
   - Step 2: Call `plan_rebalance(&state.partition_table, &assignments)` with the computed assignments to produce `Vec<MigrationTask>`.
   - Then send `MigrationCommand::Start(task)` for each resulting `MigrationTask` via `migration_tx.send()`.
3. Other variants (`PartitionMoved`, `PartitionTableUpdated`): no action.

**Note:** `RebalanceTrigger` does not directly call `MigrationService::start_migrations()`. It sends `MigrationCommand::Start` messages into the channel; the event loop that drives `MigrationCoordinator` is responsible for dispatching these commands. This decouples the trigger from the service implementation.

### R7: Migration Ordering

Before starting migrations, `start_migrations()` must call `order_migrations(&mut tasks, &state.partition_table)` (from `assignment.rs`, re-exported from `cluster::mod`) to sort tasks by availability priority: backup promotions first, then partitions with fewest replicas.

### R8: CRDT Merge Path

`MigrateDataPayload.map_states` contains `Vec<MapStateChunk>` where each chunk's `data: Vec<u8>` is a MsgPack-serialized CRDT map produced by `rmp_serde::to_vec_named()`. On the destination:

- `MapType::Lww` chunks: deserialize as `LwwMap` from `topgun_core` via `rmp_serde::from_slice::<LwwMap>(&chunk.data)`, then call `.merge()` with the local map if one exists, or initialize from the deserialized value.
- `MapType::Or` chunks: same pattern with `ORMap`.
- Delta ops (`DeltaOp.entry: Vec<u8>`): MsgPack-serialized single CRDT entries applied via the map's `set_entry()` method.

For this spec, `MapProvider` abstracts the concrete map lookup. The stub implementation in tests returns empty chunks from `get_partition_maps()` and is a no-op for `receive_map_chunk()` and `apply_delta_op()`. The concrete implementation that calls `LwwMap::merge()` and `ORMap::merge()` is wired in the storage module (future spec).

## Files

| File | Action | Purpose |
|------|--------|---------|
| `packages/server-rust/src/cluster/migration.rs` | **Create** | `MigrationCoordinator`, `MapProvider` trait, `NoOpMapProvider` (pub(crate)), `not_owner_response()`, `broadcast_partition_map()`, `RebalanceTrigger`, unit tests |
| `packages/server-rust/src/cluster/types.rs` | **Modify** | Add `new_backups: Vec<String>` field to `ActiveMigration` struct |
| `packages/server-rust/src/cluster/mod.rs` | **Modify** | Add `pub mod migration;` declaration; add re-exports for `MigrationCoordinator`, `MapProvider`, `RebalanceTrigger`, `not_owner_response`, `broadcast_partition_map` |

Total: 3 files (1 created, 2 modified). Well within the 5-file limit.

## Task

### Wave A: Core Implementation

**Task A1 — `MapProvider` trait and stub**
- File: `migration.rs`
- Define `MapProvider` trait with 3 methods: `get_partition_maps`, `receive_map_chunk`, `apply_delta_op`
- Define `NoOpMapProvider` struct as `pub(crate)` implementing `MapProvider` with empty/no-op methods (test-only helper, not re-exported from the cluster module)

**Task A2 — `MigrationCoordinator` struct and constructor**
- File: `migration.rs`
- Define struct fields: `state: Arc<ClusterState>`, `registry: Arc<ConnectionRegistry>`, `local_node_id: String`, `map_provider: Arc<dyn MapProvider>`
- Implement `MigrationCoordinator::new(state, registry, map_provider) -> Self`
- Implement `async fn send_to_peer()` private helper

**Task A2.1 — Add `new_backups` to `ActiveMigration`**
- File: `types.rs`
- Add `pub new_backups: Vec<String>` field to `ActiveMigration` struct (after `started_at_ms`)
- Update any existing test constructors of `ActiveMigration` in `mod.rs` to include `new_backups: vec![]`

**Task A3 — `MigrationService` trait implementation**
- File: `migration.rs`
- Implement `async_trait` block for `MigrationCoordinator`
- Implement all 7 methods: `start_migrations`, `cancel_migration`, `cancel_all`, `handle_migrate_start`, `handle_migrate_data`, `handle_migrate_ready`, `is_migrating`
- Include rollback path in `handle_migrate_ready` error handling

**Task A4 — Free functions**
- File: `migration.rs`
- Implement `not_owner_response(table, members) -> PartitionMapPayload`
- Implement `broadcast_partition_map(table, members, registry)`

**Task A5 — `RebalanceTrigger`**
- File: `migration.rs`
- Define struct: `state: Arc<ClusterState>`, `migration_tx: mpsc::Sender<MigrationCommand>`
- Implement `new()` constructor
- Implement `async fn run(self, change_rx)` with two-step rebalance: `compute_assignment()` then `plan_rebalance()`

### Wave B: Module Wiring

**Task B1 — Module declaration**
- File: `mod.rs`
- Add `pub mod migration;` to module list

**Task B2 — Re-exports**
- File: `mod.rs`
- Add `pub use migration::{MigrationCoordinator, MapProvider, RebalanceTrigger, not_owner_response, broadcast_partition_map};`

### Wave C: Tests

**Task C1 — Unit tests in `migration.rs`**

Tests live in `#[cfg(test)] mod tests { ... }` inside `migration.rs`.

| Test | What it verifies |
|------|-----------------|
| `start_migrations_sets_partition_to_migrating` | After `start_migrations()`, partition state is `Migrating` |
| `start_migrations_respects_concurrency_limit` | Tasks beyond `max_parallel_migrations` are not started immediately |
| `start_migrations_orders_by_priority` | `order_migrations()` is called; backup promotions precede new copies |
| `cancel_migration_restores_active_state` | Source partition returns to `Active` after cancel |
| `cancel_all_clears_all_migrations` | All active migrations are cleared and states restored |
| `handle_migrate_start_sets_migrating_state` | Partition transitions to `Migrating` on source |
| `handle_migrate_data_sets_receiving_state` | Partition transitions to `Receiving` on destination |
| `handle_migrate_ready_completes_migration` | Partition transitions to `Draining` then ownership updated; `PartitionMoved` event emitted |
| `handle_migrate_ready_rollback_on_send_failure` | If `send_to_peer` fails, source partition returns to `Active` and migration marked `Failed` |
| `is_migrating_returns_true_during_migration` | Returns `true` for active partition, `false` for idle |
| `not_owner_response_returns_partition_map` | Returns `PartitionMapPayload` matching current table state |
| `broadcast_partition_map_sends_to_clients_only` | `registry.broadcast()` called with `ConnectionKind::Client`; cluster peers not targeted |
| `rebalance_trigger_sends_commands_on_member_change` | `MemberAdded` event causes `MigrationCommand::Start` to be sent when local node is master |
| `rebalance_trigger_ignores_partition_events` | `PartitionMoved` and `PartitionTableUpdated` events produce no commands |

**Context Estimate:** ~400 lines for `migration.rs` (struct + trait impl + free functions + tests), ~10 lines for `mod.rs` changes, ~5 lines for `types.rs` changes. Fits comfortably in one implementation session.

## Constraints

1. Uses `ConnectionRegistry` from `packages/server-rust/src/network/connection.rs` (completed as part of TODO-064). Specifically: `registry.connections()` to iterate peers, `handle.metadata.read().await.peer_node_id` to identify peers, `handle.try_send(OutboundMessage::Binary(_))` to send, and `registry.broadcast(&bytes, ConnectionKind::Client)` for client push.
2. Uses `ClusterState` from `cluster/state.rs`: `state.partition_table` (for `set_state`, `set_owner`, `increment_version`, `to_partition_map`), `state.active_migrations` (RwLock<HashMap<u32, ActiveMigration>>), `state.config` (for `max_parallel_migrations`), `state.is_master()`, `state.change_sender()`.
3. Must implement the `MigrationService` trait defined in `cluster/traits.rs` exactly (all 7 method signatures, `#[async_trait]`).
4. Must use `ClusterMessage` migration variants (`MigrateStart`, `MigrateData`, `MigrateReady`, `MigrateFinalize`, `MigrateCancel`) from `cluster/messages.rs`.
5. Wire format: `rmp_serde::to_vec_named()` for all serialization.
6. `MapProvider` is a new trait in `migration.rs` — it is a seam for the storage module, not a permanent abstraction; concrete storage wiring is deferred to the storage spec.
7. Max 5 files per Language Profile: this spec uses 3 files.

## Assumptions

1. `ConnectionRegistry` (from `network/connection.rs`) provides peer lookup via `connections()` returning `Vec<Arc<ConnectionHandle>>`. `ConnectionHandle.metadata` is an `Arc<RwLock<ConnectionMetadata>>` where `ConnectionMetadata.peer_node_id: Option<String>` identifies the remote cluster node. This is the concrete interface used by `send_to_peer()`.
2. CRDT merge during migration uses `LwwMap::merge()` and `ORMap::merge()` from `topgun_core`. Serialized map state in `MapStateChunk.data` is a MsgPack-encoded map produced by `rmp_serde::to_vec_named()`. Deserialization is `rmp_serde::from_slice::<LwwMap>()` / `rmp_serde::from_slice::<ORMap>()`. This path is abstracted by `MapProvider` so the concrete CRDT types are not required in `migration.rs` itself.
3. `plan_rebalance()`, `compute_assignment()`, and `order_migrations()` from `cluster/assignment.rs` are already implemented and re-exported via `cluster::mod`. `start_migrations()` calls `order_migrations()` before processing tasks. `RebalanceTrigger::run()` calls `compute_assignment()` then `plan_rebalance()`.
4. Master node identity is determined by `ClusterState::is_master()` which compares `local_node_id` against the lowest-`join_version` active member in the current `MembersView`.
5. `RebalanceTrigger` receives a clone of the `ClusterChange` receiver from `ClusterChannels::cluster_events` (unbounded mpsc). The caller is responsible for creating the channel and passing the receiver to `run()`.

## Acceptance Criteria

### Structural

- [x] `migration.rs` exists at `packages/server-rust/src/cluster/migration.rs`
- [x] `MapProvider` trait is defined in `migration.rs` with exactly 3 methods: `get_partition_maps`, `receive_map_chunk`, `apply_delta_op`
- [x] `NoOpMapProvider` struct is defined as `pub(crate)` in `migration.rs` and implements `MapProvider` (all methods are no-ops or return empty); it is not re-exported from `cluster::mod`
- [x] `MigrationCoordinator` struct has exactly 4 fields: `state: Arc<ClusterState>`, `registry: Arc<ConnectionRegistry>`, `local_node_id: String`, `map_provider: Arc<dyn MapProvider>`
- [x] `MigrationCoordinator::new()` takes 3 parameters: `state: Arc<ClusterState>`, `registry: Arc<ConnectionRegistry>`, `map_provider: Arc<dyn MapProvider>`
- [x] `MigrationCoordinator` implements `MigrationService` trait (all 7 methods present)
- [x] `send_to_peer()` is an `async fn` (not synchronous) to support `metadata.read().await`
- [x] `not_owner_response(table: &ClusterPartitionTable, members: &MembersView) -> PartitionMapPayload` is a public free function in `migration.rs`
- [x] `broadcast_partition_map(table: &ClusterPartitionTable, members: &MembersView, registry: &ConnectionRegistry)` is a public free function in `migration.rs`
- [x] `RebalanceTrigger` struct is defined with fields `state: Arc<ClusterState>` and `migration_tx: mpsc::Sender<MigrationCommand>`
- [x] `RebalanceTrigger::run()` is an `async fn` accepting `mpsc::UnboundedReceiver<ClusterChange>`
- [x] `ActiveMigration` in `types.rs` has a `new_backups: Vec<String>` field
- [x] `mod.rs` declares `pub mod migration;`
- [x] `mod.rs` re-exports `MigrationCoordinator`, `MapProvider`, `RebalanceTrigger`, `not_owner_response`, `broadcast_partition_map` from `migration`

### Behavioral (verified by unit tests)

- [x] `start_migrations()` sets partition state to `PartitionState::Migrating` for each started task
- [x] `start_migrations()` does not start migrations beyond `config.max_parallel_migrations`
- [x] `cancel_migration()` sets the source partition state back to `PartitionState::Active`
- [x] `cancel_all()` clears `state.active_migrations` and sets all source partitions back to `Active`
- [x] `handle_migrate_start()` sets partition state to `PartitionState::Migrating`
- [x] `handle_migrate_data()` sets partition state to `PartitionState::Receiving`
- [x] `handle_migrate_ready()` sets source partition state to `PartitionState::Draining`, updates ownership, increments partition table version, and emits `ClusterChange::PartitionMoved`
- [x] `handle_migrate_ready()` rollback: on `send_to_peer` failure, source partition returns to `PartitionState::Active` and migration transitions to `MigrationPhase::Failed`
- [x] `is_migrating()` returns `true` for a partition with an active migration entry, `false` otherwise
- [x] `not_owner_response()` returns a `PartitionMapPayload` whose `version` matches the partition table version
- [x] `broadcast_partition_map()` sends to connections with `kind == ConnectionKind::Client` and not to `ConnectionKind::ClusterPeer`
- [x] `RebalanceTrigger::run()` sends `MigrationCommand::Start` on `ClusterChange::MemberAdded` when node is master
- [x] `RebalanceTrigger::run()` does not send commands on `ClusterChange::PartitionMoved`

### Quality

- [x] `cargo build -p topgun-server` succeeds with no errors
- [x] `cargo test -p topgun-server` passes all 14 migration tests plus all pre-existing cluster tests (272 total)
- [x] `cargo clippy -p topgun-server -- -D warnings` reports no warnings

## Audit History

### Audit v1 (2026-02-23 09:00)
**Status:** NEEDS_REVISION

**Context Estimate:** Not estimable (Task section is TBD)

**Critical:**
1. **Task section is empty (TBD).** The entire Task section reads "TBD -- Full requirements will be defined after TODO-064 (networking layer) is complete." This makes the specification unimplementable. A developer cannot build anything from this spec. The Task section must define: which files to create/modify, what structs/impls to write, the migration protocol flow (REPLICATE phase steps, FINALIZE phase steps), how partition state transitions are driven, the NOT_OWNER response generation mechanism, and the partition map push mechanism.
2. **No Requirements section.** Sibling specs (SPEC-060a, 060b) include detailed Requirements sections enumerating every field, derive, method signature, and serde annotation. This spec has none. The 7 acceptance criteria from the parent spec are high-level behavioral statements (e.g., "source continues accepting writes during migration") that cannot be verified without concrete implementation requirements.
3. **No file list.** The spec does not specify which files will be created or modified. For a Rust spec subject to the Language Profile (max 5 files per spec), this is essential. The implementor needs to know whether this creates a single `migration.rs`, or multiple files, and which existing files (e.g., `state.rs`, `mod.rs`) need modification.
4. **No Implementation Tasks section.** For a medium-complexity spec, task groups with wave assignments, dependency tracking, and context estimates are needed to enable execution planning. This section is entirely absent.
5. **Acceptance criteria are not self-contained.** The spec references "Acceptance Criteria (from parent SPEC-060)" with numbers 23-29, but these are behavioral descriptions, not testable implementation criteria. Completed sibling specs (060a) had 14 concrete, measurable criteria (e.g., "NodeState enum has exactly 6 variants"). This spec needs its own implementation-level acceptance criteria that a reviewer can verify against source code.
6. **Constraint #1 is stale.** Constraint 1 says "Depends on TODO-064 (networking layer) for inter-node communication" but the Context section states "Dependencies resolved -- TODO-064 ... and SPEC-060c ... are both complete." The constraint text should reflect that TODO-064 is complete and describe what interface from the networking layer this spec depends on, rather than stating a dependency that has already been satisfied.

**Recommendations:**
7. [Strategic] The Scope Preview lists 7 major capabilities (2-phase protocol, state machine transitions, NOT_OWNER, partition map push, migration ordering, rollback, rebalancing trigger). For a medium-complexity spec with a max of 5 files, verify this scope is achievable. If the networking layer (TODO-064) introduced abstractions that constrain the design, the scope may need adjustment.
8. Goal Analysis section is recommended for medium-complexity specs. The parent SPEC-060 has one, but this sub-spec should inherit or refine the relevant observable truths (Truth 5: Partition migration, Truth 7: Client routing) to ensure coverage.
9. Assumption #1 ("TODO-064 will provide a trait or channel-based interface") should be replaced with concrete references to the actual interface now that TODO-064 is complete. The spec should name the specific types/traits from the networking layer it will use.
10. Assumption #2 references "existing LWWMap/ORMap merge methods from core-rust" but does not specify which methods or how serialized CRDT data (as `Vec<u8>` in `MigrateDataPayload.map_states`) will be deserialized and merged. This needs elaboration in the Requirements section.

### Response v1 (2026-02-23)
**Applied:** All 6 critical issues and all 4 recommendations.

**Changes:**
1. [✓] Task section was empty (TBD) — replaced with full Task section (Waves A/B/C) covering struct definition, trait implementation, free functions, RebalanceTrigger, module wiring, and 14 unit tests with descriptions.
2. [✓] No Requirements section — added complete Requirements section (R1–R8) with struct fields, method-by-method protocol flow, send_to_peer helper, rollback path, MapProvider trait, CRDT merge path, and migration ordering.
3. [✓] No file list — added Files table: 2 files (migration.rs created, mod.rs modified), explicitly within the 5-file limit.
4. [✓] No Implementation Tasks section — added Task section with Wave A (core impl, 5 tasks), Wave B (module wiring, 2 tasks), Wave C (14 unit tests), and context estimate.
5. [✓] Acceptance criteria were not self-contained — replaced parent-spec references with 14 structural criteria, 14 behavioral criteria, and 3 quality criteria, all verifiable against source code.
6. [✓] Constraint #1 was stale — replaced with concrete reference to `ConnectionRegistry` from `network/connection.rs` and its specific methods used (`connections()`, `metadata.peer_node_id`, `try_send()`, `broadcast()`).
7. [✓] Scope feasibility (recommendation) — confirmed: 7 capabilities fit in 2 files. MapProvider trait provides a storage seam without requiring the storage module. RebalanceTrigger is a lightweight async task. All scoped items are achievable at medium complexity.
8. [✗] Goal Analysis section (recommendation) — skipped per revision scope note: acceptable for sub-specs. Parent SPEC-060 goal analysis applies.
9. [✓] Assumption #1 updated — replaced vague "trait or channel-based interface" with concrete reference to `ConnectionRegistry.connections()`, `ConnectionHandle.metadata.peer_node_id`, `handle.try_send(OutboundMessage::Binary(_))`, and `registry.broadcast()`.
10. [✓] CRDT merge path elaborated — added R8 specifying `rmp_serde::to_vec_named()` for serialization, `rmp_serde::from_slice::<LwwMap>()` / `rmp_serde::from_slice::<ORMap>()` for deserialization, `LwwMap::merge()` / `ORMap::merge()` for merging, `DeltaOp.entry` as MsgPack-serialized single entries applied via `set_entry()`, and `MapProvider` as the abstraction layer.

### Audit v2 (2026-02-23 11:30)
**Status:** NEEDS_REVISION

**Context Estimate:** ~25% total (1 new file ~400 lines + 1 small mod ~10 lines + async/state management 1.5x multiplier)

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields
- [x] No `r#type: String` on message structs (no new message structs)
- [N/A] `Default` derived on payload structs (no new payload structs)
- [N/A] Enums used for known value sets (no new enums)
- [x] Wire compatibility: `rmp_serde::to_vec_named()`
- [N/A] `#[serde(rename_all = "camelCase")]` (MigrationCoordinator not serialized)
- [N/A] `#[serde(skip_serializing_if = ...)]` on `Option<T>` (no serialized Option fields)

**Critical:**
1. **`send_to_peer()` cannot be synchronous as specified.** R3 defines `send_to_peer` as `fn send_to_peer(&self, ...) -> anyhow::Result<()>` (synchronous). However, Constraint #1 and R3 step 2 require reading `handle.metadata.read().await.peer_node_id` to identify peer connections. `ConnectionMetadata` is behind a `tokio::sync::RwLock`, which requires `.await` and therefore an async context. Either: (a) make `send_to_peer` an `async fn` (valid since all callers are async trait methods), or (b) use `metadata.try_read()` (can fail if lock is held). The spec must explicitly choose one approach. Recommendation: `async fn` is simpler and more reliable.

2. **`ActiveMigration` lacks `new_backups` field -- data lost between R2.1 and R2.6.** In R2.1 step 3, `start_migrations()` constructs an `ActiveMigration` from a `MigrationTask`, but `ActiveMigration` (in `types.rs`) has no `new_backups` field. Then R2.6 step 6 calls `set_owner(partition_id, destination, new_backups)` which needs the target backup list. Since `ActiveMigration` does not store `new_backups`, this value is unavailable at finalization time. Fix options: (a) add `new_backups: Vec<String>` to the `ActiveMigration` struct in `types.rs` (requires listing `types.rs` as a modified file in the Files table), or (b) recompute backups from the current assignment at finalization time using `compute_assignment()`. Option (a) is cleaner. This also means the Files table needs updating to 3 files.

3. **R1 struct definition contradicts the rest of the spec.** The R1 code block shows only 3 fields (`state`, `registry`, `local_node_id`) and a 2-parameter constructor (`state, registry`). But Task A2 and acceptance criteria specify 4 fields (adding `map_provider: Arc<dyn MapProvider>`) and a 3-parameter constructor (`state, registry, map_provider`). The R1 code block must be updated to match the actual 4-field struct and 3-parameter constructor to avoid confusing implementors.

**Recommendations:**
4. **R2.1 step 2 uses contradictory language.** It says "enqueue remaining tasks" but immediately clarifies in parentheses that tasks are "silently deferred" and "the rebalance loop will retry." There is no queue structure defined. Replace "enqueue" with "skip" to match the actual behavior: tasks beyond the concurrency limit are dropped and the rebalance trigger will recompute them on the next membership change.

5. **R2.4 step 4 sends empty `migration_id`.** `MigrateReadyPayload { migration_id: "".to_string(), ... }` uses an empty string because the `handle_migrate_start` method signature (from the trait) does not receive the `migration_id`. The master's `handle_migrate_ready` in R2.6 looks up the migration by `partition_id`, not `migration_id`, so this works functionally. However, the empty string is a code smell that could confuse future maintainers. Consider adding a comment in the spec noting this is a known limitation of the trait signature and that the empty string is intentional because the master correlates by `partition_id`.

6. **`RebalanceTrigger.run()` calls `plan_rebalance()` which requires `&[PartitionAssignment]` but the spec says it uses `active_members()`.** R6 says "compute rebalance via `plan_rebalance(&state.partition_table, &assignments)` using the current view's `active_members()`." The `plan_rebalance()` function takes `&[PartitionAssignment]` (target assignments), not member info directly. The trigger must first call `compute_assignment(active_members, partition_count, backup_count)` to produce target assignments, then pass those to `plan_rebalance()`. The spec should make this two-step process explicit: (1) `compute_assignment()` from active members, (2) `plan_rebalance()` from assignments.

7. **`NoOpMapProvider` should also be re-exported.** The acceptance criteria specify `NoOpMapProvider` is defined in `migration.rs`, but the mod.rs re-exports do not include it. While it is primarily for tests, external consumers writing integration tests may need it. Consider adding it to the re-export list, or explicitly noting it is `pub(crate)` and test-only.

### Response v2 (2026-02-23)
**Applied:** All 3 critical issues and all 4 recommendations.

**Changes:**
1. [✓] `send_to_peer()` made async — R3 signature updated from `fn send_to_peer(...)` to `async fn send_to_peer(...)`. R3 step 2 updated to show `metadata.read().await.peer_node_id`. Task A2 updated to specify "async fn". Acceptance criteria updated with new criterion: "`send_to_peer()` is an `async fn` (not synchronous) to support `metadata.read().await`".
2. [✓] `ActiveMigration` gets `new_backups` field — R2.1 step 3 updated to copy `new_backups` from `MigrationTask` into `ActiveMigration`. R2.6 step 6 updated to use `active_migration.new_backups.clone()`. Files table updated to 3 files (added `types.rs` modify). New Task A2.1 added for `types.rs` change. Constraint #7 updated to "3 files". Acceptance criteria updated with "`ActiveMigration` in `types.rs` has a `new_backups: Vec<String>` field".
3. [✓] R1 code block updated — now shows 4 fields (added `map_provider: Arc<dyn MapProvider>`) and field description list updated. Constructor updated to 3-parameter signature. Acceptance criteria split to verify both 4 fields and 3-parameter constructor separately.
4. [✓] R2.1 step 2 "enqueue" replaced with "skip" — wording changed to "skip remaining tasks" with parenthetical clarified to "silently dropped -- the rebalance trigger will recompute them on the next membership change".
5. [✓] R2.4 step 4 empty `migration_id` explained — added bold **Note** inline explaining the empty string is intentional: trait signature does not provide `migration_id` to source, and master correlates by `partition_id`.
6. [✓] R6 `compute_assignment()` -> `plan_rebalance()` two-step made explicit — `run()` loop step 2 rewritten with explicit Step 1 (`compute_assignment`) and Step 2 (`plan_rebalance`). Assumption #3 updated to mention `compute_assignment()`.
7. [✓] `NoOpMapProvider` clarified as `pub(crate)` test-only — Task A1 updated to specify `pub(crate)` and "not re-exported from the cluster module". Acceptance criteria updated: `NoOpMapProvider` is `pub(crate)` and not re-exported. Files table `migration.rs` purpose column updated to mention `NoOpMapProvider (pub(crate))`.

### Audit v3 (2026-02-23 14:00)
**Status:** APPROVED

**Context Estimate:** ~25% total (1 new file ~400 lines + 2 small modifications ~15 lines + async/state management 1.5x multiplier)

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields
- [x] No `r#type: String` on message structs (no new message structs)
- [N/A] `Default` derived on payload structs (no new payload structs)
- [N/A] Enums used for known value sets (no new enums)
- [x] Wire compatibility: `rmp_serde::to_vec_named()`
- [N/A] `#[serde(rename_all = "camelCase")]` (MigrationCoordinator not serialized)
- [N/A] `#[serde(skip_serializing_if = ...)]` on `Option<T>` (no serialized Option fields)

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~25% | <=50% | OK |
| Largest task group | ~20% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | <-- Current estimate |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**Assumptions Validated Against Source Code:**
- [x] Assumption #1: `ConnectionRegistry.connections()` returns `Vec<Arc<ConnectionHandle>>` (verified in `connection.rs` line 220)
- [x] Assumption #1: `ConnectionHandle.metadata` is `Arc<RwLock<ConnectionMetadata>>` with `peer_node_id: Option<String>` (verified lines 62, 127)
- [x] Assumption #1: `handle.try_send(OutboundMessage::Binary(_))` returns `bool` (verified line 75)
- [x] Assumption #1: `registry.broadcast(&bytes, ConnectionKind::Client)` exists (verified line 231)
- [x] Assumption #3: `plan_rebalance()`, `compute_assignment()`, `order_migrations()` implemented and re-exported (verified in `assignment.rs` and `mod.rs` line 49)
- [x] Assumption #4: `ClusterState::is_master()` implemented correctly (verified in `state.rs` line 303)
- [x] Constraint #2: `state.partition_table` is public `ClusterPartitionTable` field (verified `state.rs` line 256)
- [x] Constraint #2: `state.active_migrations` is `tokio::sync::RwLock<HashMap<u32, ActiveMigration>>` (verified `state.rs` line 257)
- [x] `MigrationService` trait has exactly 7 methods matching R2.1-R2.7 signatures (verified in `traits.rs` lines 129-158)

**Strategic fit:** Aligned with project goals. Migration service is a necessary component for the Rust server's cluster protocol (Phase 3). The `MapProvider` trait seam for storage is consistent with the project's trait-first approach and defers concrete storage to a future spec.

**Project compliance:** Honors PROJECT.md decisions. Uses `rmp_serde::to_vec_named()`, follows trait-first language profile, 3 files within the 5-file limit, no new dependencies introduced.

**Comment:** The specification is now thorough and implementable. All previous critical issues (v1: 6 issues, v2: 3 issues) have been resolved. Requirements R1-R8 are detailed with step-by-step protocol flows. The 14 structural + 13 behavioral + 3 quality acceptance criteria are concrete and verifiable. Three minor issues were caught and fixed inline during this audit (see Recommendations below); they were applied directly to the spec text since they were non-controversial factual corrections.

**Inline fixes applied during audit (non-critical corrections):**
- R6 Step 1: Changed `state.config.partition_count` to `state.partition_table.partition_count()` -- `ClusterConfig` has no `partition_count` field; the count lives on `ClusterPartitionTable`.
- R6 Step 1: Changed `active_members()` to `&view.members` with explanatory note -- `compute_assignment` takes `&[MemberInfo]` and filters Active internally; `active_members()` returns `Vec<&MemberInfo>` which does not match the expected type.
- R2.4 step 4: Added explicit master node lookup via `state.current_view().master()` -- the source node needs the master's node ID to send `MigrateReady`, but the spec previously omitted how the source determines who the master is.
- R2.6 step 10: Added explicit arguments to `broadcast_partition_map()` call -- previously stated the bare function name without showing how to obtain the `MembersView` parameter.

**Recommendations:**
1. [Minor] R2.4 `source_version` in `MigrateDataPayload` is referenced but never defined. The implementor needs to know where to get this value. It likely comes from `state.partition_table.version()`. Consider adding a note clarifying this.
2. [Minor] The `handle_migrate_start` and `handle_migrate_data` methods send messages via `send_to_peer()` which can fail, but there is no error handling guidance for these methods (unlike R2.6 which has a detailed rollback path). The implementor should propagate `send_to_peer` errors via `?`. Consider noting this explicitly to maintain consistency with R2.6's error handling documentation.

### Response v3 (2026-02-23)
**Applied:** Both audit v3 recommendations.

**Changes:**
1. [✓] `source_version` clarified — added inline **Note** to R2.4 step 3 explaining `source_version` is obtained from `state.partition_table.version()` and its purpose (detecting stale transfers).
2. [✓] Error handling guidance added — added **Error handling** paragraphs to R2.4 (propagate `send_to_peer` errors via `?`, no rollback needed since `Migrating` state is correct) and R2.5 (no network errors possible; `MapProvider` methods are infallible).

### Audit v4 (2026-02-23 16:30)
**Status:** APPROVED

**Context Estimate:** ~25% total (1 new file ~400 lines + 2 small modifications ~15 lines + async/state management 1.5x multiplier)

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields
- [x] No `r#type: String` on message structs (no new message structs)
- [N/A] `Default` derived on payload structs (no new payload structs)
- [N/A] Enums used for known value sets (no new enums)
- [x] Wire compatibility: `rmp_serde::to_vec_named()`
- [N/A] `#[serde(rename_all = "camelCase")]` (MigrationCoordinator not serialized)
- [N/A] `#[serde(skip_serializing_if = ...)]` on `Option<T>` (no serialized Option fields)

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~25% | <=50% | OK |
| Largest task group | ~20% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | <-- Current estimate |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

**All assumptions re-validated against source code (fresh audit):**
- [x] `ConnectionRegistry.connections()` returns `Vec<Arc<ConnectionHandle>>` (`connection.rs` line 220)
- [x] `ConnectionHandle.metadata` is `Arc<RwLock<ConnectionMetadata>>` with `peer_node_id: Option<String>` (`connection.rs` lines 62, 127)
- [x] `handle.try_send(OutboundMessage::Binary(_))` returns `bool` (`connection.rs` line 75)
- [x] `registry.broadcast(&bytes, ConnectionKind::Client)` exists (`connection.rs` line 231)
- [x] `compute_assignment(&[MemberInfo], u32, u32)` signature correct (`assignment.rs` line 19)
- [x] `plan_rebalance()` and `order_migrations()` implemented and re-exported (`assignment.rs` lines 73, 106; `mod.rs` line 49)
- [x] `ClusterState::is_master()` uses `MembersView::is_master()` with `local_node_id` (`state.rs` line 303)
- [x] `state.partition_table` is public `ClusterPartitionTable` field (`state.rs` line 256)
- [x] `state.active_migrations` is `tokio::sync::RwLock<HashMap<u32, ActiveMigration>>` (`state.rs` line 257)
- [x] `state.change_sender()` returns `&mpsc::UnboundedSender<ClusterChange>` (`state.rs` line 312)
- [x] `state.current_view()` returns `Arc<MembersView>` via `ArcSwap` (`state.rs` line 292)
- [x] `MigrationService` trait has exactly 7 async methods matching R2.1-R2.7 signatures (`traits.rs` lines 129-158)
- [x] `ActiveMigration` currently has no `new_backups` field (confirmed in `types.rs` lines 153-160) -- spec correctly adds it
- [x] Existing `ActiveMigration` test constructor in `mod.rs` (line 360) needs `new_backups: vec![]` added -- spec correctly notes this

**Strategic fit:** Aligned with project goals. Migration service is the core dynamic cluster capability for Phase 3.

**Project compliance:** Honors PROJECT.md decisions. Uses `rmp_serde::to_vec_named()`, trait-first language profile respected, 3 files within 5-file limit, no new dependencies.

**Language profile:** Compliant with Rust profile. Trait defined before implementation (Task A1 before A3). 3 files within max 5.

**Comment:** This specification has matured through 3 revision cycles and is now thoroughly implementable. All 9 critical issues from prior audits (v1: 6, v2: 3) have been resolved. Requirements R1-R8 provide step-by-step protocol flows with explicit error handling, rollback paths, and edge case documentation. The 14 structural + 13 behavioral + 3 quality acceptance criteria are concrete and verifiable against source code. All assumptions have been independently re-validated against the current codebase. No critical issues or recommendations remain.

---

## Execution Summary

**Executed:** 2026-02-23
**Commits:** 2

### Files Created
- `packages/server-rust/src/cluster/migration.rs` -- MigrationCoordinator, MapProvider trait, NoOpMapProvider (pub(crate)), not_owner_response(), broadcast_partition_map(), RebalanceTrigger, 14 unit tests

### Files Modified
- `packages/server-rust/src/cluster/types.rs` -- Added `new_backups: Vec<String>` field to ActiveMigration struct
- `packages/server-rust/src/cluster/mod.rs` -- Added `pub mod migration;` declaration and re-exports for MigrationCoordinator, MapProvider, RebalanceTrigger, not_owner_response, broadcast_partition_map

### Files Deleted
(none)

### Acceptance Criteria Status

**Structural:**
- [x] `migration.rs` exists at `packages/server-rust/src/cluster/migration.rs`
- [x] `MapProvider` trait is defined in `migration.rs` with exactly 3 methods: `get_partition_maps`, `receive_map_chunk`, `apply_delta_op`
- [x] `NoOpMapProvider` struct is defined as `pub(crate)` in `migration.rs` and implements `MapProvider` (all methods are no-ops or return empty); it is not re-exported from `cluster::mod`
- [x] `MigrationCoordinator` struct has exactly 4 fields: `state: Arc<ClusterState>`, `registry: Arc<ConnectionRegistry>`, `local_node_id: String`, `map_provider: Arc<dyn MapProvider>`
- [x] `MigrationCoordinator::new()` takes 3 parameters: `state: Arc<ClusterState>`, `registry: Arc<ConnectionRegistry>`, `map_provider: Arc<dyn MapProvider>`
- [x] `MigrationCoordinator` implements `MigrationService` trait (all 7 methods present)
- [x] `send_to_peer()` is an `async fn` (not synchronous) to support `metadata.read().await`
- [x] `not_owner_response(table: &ClusterPartitionTable, members: &MembersView) -> PartitionMapPayload` is a public free function in `migration.rs`
- [x] `broadcast_partition_map(table: &ClusterPartitionTable, members: &MembersView, registry: &ConnectionRegistry)` is a public free function in `migration.rs`
- [x] `RebalanceTrigger` struct is defined with fields `state: Arc<ClusterState>` and `migration_tx: mpsc::Sender<MigrationCommand>`
- [x] `RebalanceTrigger::run()` is an `async fn` accepting `mpsc::UnboundedReceiver<ClusterChange>`
- [x] `ActiveMigration` in `types.rs` has a `new_backups: Vec<String>` field
- [x] `mod.rs` declares `pub mod migration;`
- [x] `mod.rs` re-exports `MigrationCoordinator`, `MapProvider`, `RebalanceTrigger`, `not_owner_response`, `broadcast_partition_map` from `migration`

**Behavioral (verified by unit tests):**
- [x] `start_migrations()` sets partition state to `PartitionState::Migrating` for each started task
- [x] `start_migrations()` does not start migrations beyond `config.max_parallel_migrations`
- [x] `cancel_migration()` sets the source partition state back to `PartitionState::Active`
- [x] `cancel_all()` clears `state.active_migrations` and sets all source partitions back to `Active`
- [x] `handle_migrate_start()` sets partition state to `PartitionState::Migrating`
- [x] `handle_migrate_data()` sets partition state to `PartitionState::Receiving`
- [x] `handle_migrate_ready()` sets source partition state to `PartitionState::Draining`, updates ownership, increments partition table version, and emits `ClusterChange::PartitionMoved`
- [x] `handle_migrate_ready()` rollback: on `send_to_peer` failure, source partition returns to `PartitionState::Active` and migration transitions to `MigrationPhase::Failed`
- [x] `is_migrating()` returns `true` for a partition with an active migration entry, `false` otherwise
- [x] `not_owner_response()` returns a `PartitionMapPayload` whose `version` matches the partition table version
- [x] `broadcast_partition_map()` sends to connections with `kind == ConnectionKind::Client` and not to `ConnectionKind::ClusterPeer`
- [x] `RebalanceTrigger::run()` sends `MigrationCommand::Start` on `ClusterChange::MemberAdded` when node is master
- [x] `RebalanceTrigger::run()` does not send commands on `ClusterChange::PartitionMoved`

**Quality:**
- [x] `cargo build -p topgun-server` succeeds with no errors
- [x] `cargo test -p topgun-server` passes all 14 migration tests plus all pre-existing cluster tests (272 total)
- [x] `cargo clippy -p topgun-server -- -D warnings` reports no warnings

### Deviations

1. [Rule 1 - Bug] Fixed `start_migrations()` message construction: removed dead code that built messages inside the loop but never used them, and collected send targets separately from the migration map to avoid sending to all existing replicating migrations (not just newly started ones).
2. [Rule 1 - Bug] Fixed test receiver lifetime: changed `_` destructuring to `_receivers` in all `make_coordinator_with_peers` calls to prevent channel receivers from being immediately dropped, which caused `try_send` to return false (channel disconnected).
3. [Rule 1 - Bug] Fixed rebalance trigger test: updated state view to include the new node before sending `MemberAdded` event (matching real system behavior where view is updated before event emission), and reduced partition count to 10 to avoid overflowing the bounded migration command channel (capacity 100 vs 180+ tasks from 271 partitions).

### Notes
- Total test count increased from 258 to 272 (14 new migration tests).
- `NoOpMapProvider` uses `#[allow(dead_code)]` since it is `pub(crate)` but only constructed in `#[cfg(test)]` code.
- `current_unix_ms()` uses `#[allow(clippy::cast_possible_truncation)]` for u128-to-u64 cast (safe: u64::MAX millis exceeds 584 million years).
- The `start_migrations()` implementation drops the write lock before calling `send_to_peer()` to avoid holding the lock across await points.

---

## Review History

### Review v1 (2026-02-23 18:30)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Minor:**
1. `broadcast_partition_map()` silently swallows serialization errors via `if let Ok(bytes)` at line 433 of `migration.rs`. In practice `rmp_serde::to_vec_named` on a `PartitionMapPayload` should not fail, but a `log::warn!` or at least a debug assertion would help catch unexpected failures during development. This is consistent with the spec's placeholder wording ("serialize as `rmp_serde::to_vec_named(&map)?`" -- note the `?` in the spec, but the implementation uses `if let Ok`).

2. The `start_migrations_orders_by_priority` test (line 667) verifies that both tasks are started but does not directly assert the ordering in which they were processed. Since both fit within the concurrency limit, the test proves `order_migrations()` was called (otherwise it would fail) but does not verify that partition 0 (backup promotion) was processed before partition 1. This is a minor test coverage gap -- the ordering logic itself is tested in `assignment.rs`.

**Passed:**
- [PASS] `migration.rs` exists at `packages/server-rust/src/cluster/migration.rs`
- [PASS] `MapProvider` trait has exactly 3 methods: `get_partition_maps`, `receive_map_chunk`, `apply_delta_op` -- verified at lines 36-45
- [PASS] `NoOpMapProvider` is `pub(crate)` (line 51), implements `MapProvider` (lines 53-61), not re-exported from `cluster::mod`
- [PASS] `MigrationCoordinator` has 4 fields matching spec (lines 71-76): `state`, `registry`, `local_node_id`, `map_provider`
- [PASS] `MigrationCoordinator::new()` takes 3 parameters (lines 84-87): `state`, `registry`, `map_provider`; caches `local_node_id` from state
- [PASS] `MigrationCoordinator` implements `MigrationService` with all 7 methods (lines 136-403)
- [PASS] `send_to_peer()` is `async fn` (line 103) using `metadata.read().await` (line 110)
- [PASS] `not_owner_response()` is public free function with correct signature (lines 415-420)
- [PASS] `broadcast_partition_map()` is public free function with correct signature (lines 427-436)
- [PASS] `RebalanceTrigger` struct has `state: Arc<ClusterState>` and `migration_tx: mpsc::Sender<MigrationCommand>` (lines 447-449)
- [PASS] `RebalanceTrigger::run()` is `async fn` accepting `mpsc::UnboundedReceiver<ClusterChange>` (line 470)
- [PASS] `ActiveMigration` in `types.rs` has `new_backups: Vec<String>` field (line 160)
- [PASS] `mod.rs` declares `pub mod migration;` (line 10)
- [PASS] `mod.rs` re-exports all 5 items: `MigrationCoordinator`, `MapProvider`, `RebalanceTrigger`, `not_owner_response`, `broadcast_partition_map` (lines 53-56)
- [PASS] `start_migrations()` sets partition to `Migrating` -- test at line 597
- [PASS] `start_migrations()` respects concurrency limit -- test at line 619
- [PASS] `cancel_migration()` restores `Active` state -- test at line 708
- [PASS] `cancel_all()` clears all migrations -- test at line 744
- [PASS] `handle_migrate_start()` sets `Migrating` -- test at line 794
- [PASS] `handle_migrate_data()` sets `Receiving` -- test at line 812
- [PASS] `handle_migrate_ready()` completes migration (Draining, ownership, version, PartitionMoved) -- test at line 837
- [PASS] `handle_migrate_ready()` rollback on send failure -- test at line 895
- [PASS] `is_migrating()` correct behavior -- test at line 944
- [PASS] `not_owner_response()` returns correct `PartitionMapPayload` -- test at line 973
- [PASS] `broadcast_partition_map()` sends to clients only -- test at line 997
- [PASS] `RebalanceTrigger` sends commands on member change -- test at line 1025
- [PASS] `RebalanceTrigger` ignores partition events -- test at line 1116
- [PASS] `cargo build -p topgun-server` succeeds
- [PASS] `cargo test -p topgun-server` -- 272 tests passed, 0 failed
- [PASS] `cargo clippy -p topgun-server -- -D warnings` -- no warnings
- [PASS] No `.unwrap()` in production code (all in tests only)
- [PASS] No `unsafe` blocks
- [PASS] No unnecessary `.clone()` calls -- all clones are across lock boundaries or into message payloads
- [PASS] Error handling uses `?` operator correctly in `send_to_peer`, `handle_migrate_start`, `handle_migrate_data`
- [PASS] `rmp_serde::to_vec_named()` used for all serialization (wire compatibility)
- [PASS] Lock held minimally -- write locks dropped before async operations (`send_to_peer`)
- [PASS] No hardcoded secrets or sensitive data
- [PASS] Follows existing codebase patterns (DashMap, ArcSwap, async_trait, module re-exports)
- [PASS] No code duplication -- reuses `compute_assignment`, `plan_rebalance`, `order_migrations`
- [PASS] Files within 5-file limit (3 files)
- [PASS] No files to delete per spec; no deletions needed

**Summary:** The implementation is clean, complete, and fully compliant with the specification. All 14 structural, 13 behavioral, and 3 quality acceptance criteria are met. The code demonstrates good async patterns (lock release before await, conservative `try_read` fallback in `is_migrating`). The 3 documented deviations are legitimate bug fixes that improve correctness. The 2 minor findings are optional improvements that do not affect correctness or functionality.

### Fix Response v1 (2026-02-23)
**Applied:** Minor issue #1

**Fixes:**
1. [✓] `broadcast_partition_map()` silent error swallowing — replaced `if let Ok(bytes)` with `match` + `tracing::warn!` for serialization failures
   - Commit: f2ffb7b

**Skipped:**
2. [✗] `start_migrations_orders_by_priority` test ordering assertion — ordering logic is already covered by `assignment.rs` tests; adding a direct ordering assertion here would require tracking send order via a mock, which is out of scope for a minor test coverage gap

---

## Completion

**Completed:** 2026-02-23
**Total Commits:** 3
**Audit Cycles:** 4
**Review Cycles:** 1
