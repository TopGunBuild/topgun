# SPEC-060a: Cluster Protocol — Types, Traits, and Wire Messages

```yaml
id: SPEC-060a
type: feature
status: done
priority: P1
complexity: medium
parent: SPEC-060
depends_on: []
created: 2026-02-22
todo: TODO-066
```

## Context

This is the first sub-specification of SPEC-060 (Hazelcast-Informed Cluster Protocol). It defines all cluster domain types, service traits, and inter-node wire messages. These are pure definitions with no runtime behavior -- they compile independently and establish the contracts for all subsequent cluster implementation.

### Key Design Sources

| Source | Role |
|--------|------|
| `RUST_CLUSTER_ARCHITECTURE.md` Sections 2-3, 8-9 | Type definitions, trait hierarchy, wire protocol |
| Hazelcast `internal/cluster/` | MembersView, master election convention |
| TopGun TS `packages/server/src/cluster/` | Behavioral reference for phi-accrual FailureDetector |
| `packages/core-rust/src/messages/cluster.rs` | Existing `NodeStatus`, `PartitionMapPayload` (DO NOT modify) |
| `packages/server-rust/src/service/registry.rs` | `ManagedService` trait that `ClusterService` extends |

### Important Distinctions

**NodeState vs NodeStatus:** Core-rust already defines `NodeStatus` in `packages/core-rust/src/messages/cluster.rs` with variants `ACTIVE`, `JOINING`, `LEAVING`, `SUSPECTED`, `FAILED`. The new `NodeState` has 6 variants (`Joining`, `Active`, `Suspect`, `Leaving`, `Dead`, `Removed`) with Rust-idiomatic naming. These serve different purposes:
- `NodeStatus` is the client-facing wire type (SCREAMING_CASE to match TS)
- `NodeState` is the internal cluster FSM state (Rust-idiomatic, 2 extra variants for lifecycle)

Add a brief WHY-comment in `types.rs` explaining this distinction.

### MembersView::master() Clarification

The `master()` method MUST filter by `NodeState::Active` only. A Suspect or Leaving node must not be elected master even if it has the lowest `join_version`. This matches the research document (Section 3.2, line 267: `.filter(|m| m.state == NodeState::Active)`).

## Task

Create 3 files in `packages/server-rust/src/cluster/`:
1. `types.rs` -- All cluster domain types (enums, structs, config)
2. `traits.rs` -- All 5 cluster service traits
3. `messages.rs` -- `ClusterMessage` enum with all 18 payload structs

## Requirements

### File 1: `packages/server-rust/src/cluster/types.rs`

#### Enums

**`NodeState`** -- Internal cluster FSM state for a node:
- Variants: `Joining`, `Active`, `Suspect`, `Leaving`, `Dead`, `Removed`
- Derives: `Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize`
- Serde: `#[serde(rename_all = "camelCase")]`
- Include WHY-comment explaining this exists alongside `NodeStatus` in core-rust

**`PartitionState`** -- State of a partition on a specific node:
- Variants: `Unassigned`, `Active`, `Migrating`, `Receiving`, `Draining`, `Lost`
- Same derives as `NodeState`
- Serde: `#[serde(rename_all = "camelCase")]`

**`MigrationPhase`** -- Phase of an active migration:
- Variants: `Replicating`, `Ready`, `Finalizing`, `Failed`
- Derives: `Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize`
- Serde: `#[serde(rename_all = "camelCase")]`

#### Structs

**`MemberInfo`** -- Information about a single cluster member:
- `node_id: String`
- `host: String`
- `client_port: u16`
- `cluster_port: u16`
- `state: NodeState`
- `join_version: u64`
- Derives: `Debug, Clone, PartialEq, Serialize, Deserialize`
- Serde: `#[serde(rename_all = "camelCase")]`

**`MembersView`** -- Versioned snapshot of cluster membership:
- `version: u64`
- `members: Vec<MemberInfo>`
- Derives: `Debug, Clone, PartialEq, Serialize, Deserialize`
- Serde: `#[serde(rename_all = "camelCase")]`
- Methods:
  - `master() -> Option<&MemberInfo>` -- Returns the Active member with the lowest `join_version`; ties broken by lexicographic `node_id`. Filters by `NodeState::Active` only. Returns `None` for empty views or views with no Active members.
  - `is_master(node_id: &str) -> bool` -- Returns `true` only if the given node_id matches the computed master.
  - `active_members() -> Vec<&MemberInfo>` -- Returns all members with `state == NodeState::Active`.
  - `get_member(node_id: &str) -> Option<&MemberInfo>` -- Finds a member by node_id.

**`PartitionMeta`** -- Metadata for a single partition:
- `partition_id: u32`
- `owner: String`
- `backups: Vec<String>`
- `state: PartitionState`
- `version: u32`
- Derives: `Debug, Clone, PartialEq, Serialize, Deserialize`
- Serde: `#[serde(rename_all = "camelCase")]`

**`PartitionAssignment`** -- Target assignment for a partition (output of assignment algorithm):
- `partition_id: u32`
- `owner: String`
- `backups: Vec<String>`
- Derives: `Debug, Clone, PartialEq, Serialize, Deserialize`
- Serde: `#[serde(rename_all = "camelCase")]`

**`MigrationTask`** -- A single partition migration to execute:
- `partition_id: u32`
- `source: String`
- `destination: String`
- `new_backups: Vec<String>`
- Derives: `Debug, Clone, PartialEq, Serialize, Deserialize`
- Serde: `#[serde(rename_all = "camelCase")]`

**`ActiveMigration`** -- Tracking state for an in-progress migration:
- `migration_id: String`
- `partition_id: u32`
- `source: String`
- `destination: String`
- `state: MigrationPhase`
- `started_at_ms: u64`
- Derives: `Debug, Clone, PartialEq, Serialize, Deserialize`
- Serde: `#[serde(rename_all = "camelCase")]`

**`ClusterHealth`** -- Summary of cluster health for diagnostics:
- `node_count: usize`
- `active_nodes: usize`
- `suspect_nodes: usize`
- `partition_table_version: u64`
- `active_migrations: usize`
- `is_master: bool`
- `master_node_id: Option<String>` -- `#[serde(skip_serializing_if = "Option::is_none", default)]`
- Derives: `Debug, Clone, Default, PartialEq, Serialize, Deserialize`
- Serde: `#[serde(rename_all = "camelCase")]`

**`ClusterConfig`** -- Configuration for cluster behavior:
- `cluster_id: String` (default: empty string)
- `seed_addresses: Vec<String>` (default: empty vec)
- `heartbeat_interval_ms: u64` (default: 1000)
- `phi_threshold: f64` (default: 8.0)
- `max_sample_size: usize` (default: 200)
- `min_std_dev_ms: u64` (default: 100)
- `max_no_heartbeat_ms: u64` (default: 5000)
- `suspicion_timeout_ms: u64` (default: 10000)
- `backup_count: u32` (default: 1)
- `max_parallel_migrations: u32` (default: 2)
- `split_brain_check_interval_ms: u64` (default: 30000)
- Derives: `Debug, Clone, Serialize, Deserialize`
- Serde: `#[serde(rename_all = "camelCase")]`
- Implement `Default` manually (not derive) to set the production default values listed above.

### File 2: `packages/server-rust/src/cluster/traits.rs`

All traits use `#[async_trait]` for async methods. All traits are `Send + Sync`.

**`ClusterService: ManagedService`** -- Top-level cluster service:
- `fn node_id(&self) -> &str`
- `fn is_master(&self) -> bool`
- `fn master_id(&self) -> Option<String>`
- `fn members_view(&self) -> Arc<MembersView>`
- `fn partition_table(&self) -> &ClusterPartitionTable`
- `fn subscribe_changes(&self) -> tokio::sync::mpsc::UnboundedReceiver<ClusterChange>` -- uses `tokio::sync::mpsc` (not `std::sync::mpsc`, which has no `UnboundedReceiver`)
- `fn health(&self) -> ClusterHealth`

Note: `ClusterPartitionTable` and `ClusterChange` are defined in SPEC-060b (`state.rs`). For compilation in this spec, forward-declare them as opaque types or use a thin placeholder. **Preferred approach:** Since traits.rs will import from sibling modules, and SPEC-060b creates `state.rs`, the `ClusterService` trait should reference these types. The compilation gate for this spec will pass because traits.rs only defines the trait -- it does not implement it. Use `use super::state::{ClusterPartitionTable, ClusterChange};` imports that will resolve when SPEC-060b is complete. **For this spec, do NOT create state.rs. Instead, declare minimal placeholder types at the bottom of traits.rs behind a `// Placeholder types -- replaced by state.rs in SPEC-060b` comment:**
- `pub struct ClusterPartitionTable;` (empty struct)
- `pub enum ClusterChange {}` (empty enum)

These placeholders will be removed in SPEC-060b when the real types are created, and the imports in traits.rs will be updated to `use super::state::*`.

**`MembershipService: Send + Sync`** -- Membership management:
- `fn current_view(&self) -> Arc<MembersView>`
- `fn get_member(&self, node_id: &str) -> Option<MemberInfo>`
- `fn active_members(&self) -> Vec<MemberInfo>`
- `async fn handle_join_request(&self, request: JoinRequestPayload) -> JoinResponsePayload`
- `async fn handle_leave_request(&self, node_id: &str) -> anyhow::Result<()>`
- `async fn remove_member(&self, node_id: &str) -> anyhow::Result<()>`
- `fn apply_members_update(&self, view: MembersView)`

**`ClusterPartitionService: Send + Sync`** -- Extended partition management:
- `fn hash_to_partition(&self, key: &str) -> u32`
- `fn get_owner(&self, partition_id: u32) -> Option<String>`
- `fn is_local_owner(&self, partition_id: u32) -> bool`
- `fn is_local_backup(&self, partition_id: u32) -> bool`
- `fn get_state(&self, partition_id: u32) -> PartitionState`
- `fn get_partition_map(&self, members: &MembersView) -> PartitionMapPayload`
- `fn version(&self) -> u64`
- `async fn rebalance(&self, members: &MembersView) -> Vec<MigrationTask>`
- `fn apply_partition_update(&self, assignments: &[PartitionAssignment])`
- `fn partitions_for_node(&self, node_id: &str) -> Vec<u32>`

Note: `PartitionMapPayload` is imported from `topgun_core::messages::cluster`.

**`MigrationService: Send + Sync`** -- Migration lifecycle (trait only, implementation in SPEC-060d):
- `async fn start_migrations(&self, tasks: Vec<MigrationTask>) -> anyhow::Result<()>`
- `async fn cancel_migration(&self, partition_id: u32) -> anyhow::Result<()>`
- `async fn cancel_all(&self) -> anyhow::Result<()>`
- `async fn handle_migrate_start(&self, partition_id: u32, destination: &str) -> anyhow::Result<()>`
- `async fn handle_migrate_data(&self, data: MigrateDataPayload) -> anyhow::Result<()>`
- `async fn handle_migrate_ready(&self, partition_id: u32, source: &str) -> anyhow::Result<()>`
- `fn is_migrating(&self, partition_id: u32) -> bool`

Note: `MigrateDataPayload` is the wire message struct defined in messages.rs (not the `MigrateData` state struct).

**`FailureDetector: Send + Sync`** -- Pluggable failure detection:
- `fn heartbeat(&self, node_id: &str, timestamp_ms: u64)`
- `fn is_alive(&self, node_id: &str, timestamp_ms: u64) -> bool`
- `fn last_heartbeat(&self, node_id: &str) -> Option<u64>`
- `fn suspicion_level(&self, node_id: &str, timestamp_ms: u64) -> f64`
- `fn remove(&self, node_id: &str)`
- `fn reset(&self)`

### File 3: `packages/server-rust/src/cluster/messages.rs`

**`ClusterMessage` enum** -- 18-variant serde-tagged enum:
- Serde: `#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]`
- Derives: `Debug, Clone, Serialize, Deserialize`

Variants and their payload structs (fields fully enumerated):

#### Membership (4 variants)

**`JoinRequest(JoinRequestPayload)`**
```
JoinRequestPayload:
  node_id: String
  host: String
  client_port: u16
  cluster_port: u16
  cluster_id: String
  protocol_version: u32
  auth_token: Option<String>  // skip_serializing_if, default
```

**`JoinResponse(JoinResponsePayload)`**
```
JoinResponsePayload:
  accepted: bool
  reject_reason: Option<String>  // skip_serializing_if, default
  members_view: Option<MembersView>  // skip_serializing_if, default
  partition_assignments: Option<Vec<PartitionAssignment>>  // skip_serializing_if, default
```
Derives: `Debug, Clone, Default, Serialize, Deserialize` (3+ optional fields)

**`MembersUpdate(MembersUpdatePayload)`**
```
MembersUpdatePayload:
  view: MembersView
  cluster_time_ms: u64
```

**`LeaveRequest(LeaveRequestPayload)`**
```
LeaveRequestPayload:
  node_id: String
  reason: Option<String>  // skip_serializing_if, default
```

#### Heartbeat (3 variants)

**`Heartbeat(HeartbeatPayload)`**
```
HeartbeatPayload:
  sender_id: String
  timestamp_ms: u64
  members_view_version: u64
  suspected_nodes: Vec<String>
```

**`HeartbeatComplaint(HeartbeatComplaintPayload)`**
```
HeartbeatComplaintPayload:
  complainer_id: String
  complainer_view_version: u64
  suspect_id: String
  suspect_view_version: u64
```

**`ExplicitSuspicion(ExplicitSuspicionPayload)`**
```
ExplicitSuspicionPayload:
  suspect_id: String
  reason: String
  master_view_version: u64
```

#### Partition (2 variants)

**`PartitionTableUpdate(PartitionTableUpdatePayload)`**
```
PartitionTableUpdatePayload:
  assignments: Vec<PartitionAssignment>
  version: u64
  completed_migrations: Vec<String>
```

**`FetchPartitionTable`** -- Unit variant (no payload).

#### Migration (5 variants)

**`MigrateStart(MigrateStartPayload)`**
```
MigrateStartPayload:
  migration_id: String
  partition_id: u32
  destination_node_id: String
```

**`MigrateData(MigrateDataPayload)`**
```
MigrateDataPayload:
  partition_id: u32
  map_states: Vec<MapStateChunk>
  delta_ops: Vec<DeltaOp>
  source_version: u32
```

Supporting structs:

```
MapStateChunk:
  map_name: String
  data: Vec<u8>
  map_type: MapType  // enum: Lww | Or
```

```
MapType:  // enum, not String -- avoids unchecked string values for a known 2-variant set
  Lww
  Or
  Serde: #[serde(rename_all = "lowercase")]  // wire values: "lww" | "or"
  Derives: Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize
```

```
DeltaOp:
  map_name: String
  key: String
  entry: Vec<u8>
```

**`MigrateReady(MigrateReadyPayload)`**
```
MigrateReadyPayload:
  migration_id: String
  partition_id: u32
  source_node_id: String
```

**`MigrateFinalize(MigrateFinalizePayload)`**
```
MigrateFinalizePayload:
  migration_id: String
  partition_id: u32
  new_owner: String
```

**`MigrateCancel(MigrateCancelPayload)`**
```
MigrateCancelPayload:
  migration_id: String
  partition_id: u32
  reason: String
```

#### Split-Brain (3 variants)

**`SplitBrainProbe(SplitBrainProbePayload)`**
```
SplitBrainProbePayload:
  sender_cluster_id: String
  sender_master_id: String
  sender_member_count: u32
  sender_view_version: u64
```

**`SplitBrainProbeResponse(SplitBrainProbeResponsePayload)`**
```
SplitBrainProbeResponsePayload:
  responder_cluster_id: String
  responder_master_id: String
  responder_member_count: u32
  responder_view_version: u64
  responder_master_join_version: u64
```

**`MergeRequest(MergeRequestPayload)`**
```
MergeRequestPayload:
  source_cluster_id: String
  source_members: Vec<MemberInfo>
  source_view_version: u64
```

#### Forwarding (1 variant)

**`OpForward(OpForwardPayload)`**
```
OpForwardPayload:
  source_node_id: String
  target_partition_id: u32
  client_id: Option<String>  // skip_serializing_if, default
  payload: Vec<u8>  // MsgPack-serialized client Message
```

**All payload structs** derive `Debug, Clone, Serialize, Deserialize` and use `#[serde(rename_all = "camelCase")]`. Structs with 2+ `Option<T>` fields additionally derive `Default`. All `Option<T>` fields use `#[serde(skip_serializing_if = "Option::is_none", default)]`.

## Acceptance Criteria

1. `NodeState` enum has exactly 6 variants: `Joining`, `Active`, `Suspect`, `Leaving`, `Dead`, `Removed`. All derive `Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize`.
2. `PartitionState` enum has exactly 6 variants: `Unassigned`, `Active`, `Migrating`, `Receiving`, `Draining`, `Lost`. Same derives as `NodeState`.
3. `MembersView::master()` returns the Active member with the lowest `join_version`; ties broken by lexicographic `node_id`. Filters by `NodeState::Active` only. Returns `None` for empty views or views with no Active members.
4. `MembersView::is_master(node_id)` returns `true` only for the computed master.
5. `ClusterMessage` enum has exactly 18 variants covering membership (4), heartbeat (3), partition (2), migration (5), split-brain (3), and forwarding (1).
6. All `ClusterMessage` payload structs round-trip through `rmp_serde::to_vec_named()` / `rmp_serde::from_slice()` without data loss.
7. All 5 cluster traits (`ClusterService`, `MembershipService`, `ClusterPartitionService`, `MigrationService`, `FailureDetector`) are defined with the specified method signatures and are `Send + Sync`.
8. `ClusterConfig` has `Default` with production defaults: `heartbeat_interval_ms = 1000`, `phi_threshold = 8.0`, `max_sample_size = 200`, `min_std_dev_ms = 100`, `max_no_heartbeat_ms = 5000`, `suspicion_timeout_ms = 10000`, `backup_count = 1`, `max_parallel_migrations = 2`, `split_brain_check_interval_ms = 30000`.
9. `types.rs` contains a WHY-comment explaining the distinction between `NodeState` (internal FSM) and `NodeStatus` (client-facing wire type in core-rust).
10. No `f64` used for integer-semantic fields. No `r#type: String` on message structs. Enums used for known value sets (including `MapType` for `MapStateChunk.map_type`).
11. `cargo test` and `cargo clippy` produce no regressions against the existing crate (tests and lint checks that passed before this spec must still pass). The 3 new files (`types.rs`, `traits.rs`, `messages.rs`) are not wired into the module tree in this spec; their standalone compilation is deferred to SPEC-060c which creates `mod.rs`.
12. `MigrationPhase` enum has exactly 4 variants: `Replicating`, `Ready`, `Finalizing`, `Failed`.
13. Placeholder types `ClusterPartitionTable` and `ClusterChange` exist in traits.rs for compilation. They are clearly marked as placeholders for SPEC-060b.
14. `MapType` enum has exactly 2 variants (`Lww`, `Or`) with `#[serde(rename_all = "lowercase")]`, serializing to wire values `"lww"` and `"or"` respectively.

## Constraints

1. **DO NOT** implement any runtime behavior. This spec is types, traits, and message definitions only.
2. **DO NOT** modify the existing `PartitionTable` in core-rust or the existing `Message` enum in core-rust.
3. **DO NOT** add `tokio::spawn` background tasks.
4. Follow all Rust Type Mapping Rules from PROJECT.md.
5. All structs that cross wire boundaries use `#[serde(rename_all = "camelCase")]`.
6. **DO NOT** add phase/spec references in code comments. Use WHY-comments only.
7. **DO NOT** create `mod.rs` or modify `lib.rs` -- that is SPEC-060c's scope. These files compile as standalone modules for now. `cargo test` / `cargo clippy` are checked for regressions against the existing module tree only (see AC #11).

## Assumptions

1. `ClusterService` extends `ManagedService` from `packages/server-rust/src/service/registry.rs`.
2. Cluster-internal messages are separate from the client `Message` enum (different connections, same MsgPack format).
3. Protocol version for join ceremony starts at `1`.
4. Placeholder types for `ClusterPartitionTable` and `ClusterChange` are acceptable in traits.rs; they will be replaced by real types in SPEC-060b.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create `types.rs`: all enums (`NodeState`, `PartitionState`, `MigrationPhase`), all structs (`MemberInfo`, `MembersView` with methods, `PartitionMeta`, `PartitionAssignment`, `MigrationTask`, `ActiveMigration`, `ClusterHealth`, `ClusterConfig` with manual Default) | -- | ~12% |
| G2 | 1 | Create `messages.rs`: `ClusterMessage` enum (18 variants), all 18 payload structs (`JoinRequestPayload`, `JoinResponsePayload`, `MembersUpdatePayload`, `LeaveRequestPayload`, `HeartbeatPayload`, `HeartbeatComplaintPayload`, `ExplicitSuspicionPayload`, `PartitionTableUpdatePayload`, `MigrateStartPayload`, `MigrateDataPayload`, `MigrateReadyPayload`, `MigrateFinalizePayload`, `MigrateCancelPayload`, `SplitBrainProbePayload`, `SplitBrainProbeResponsePayload`, `MergeRequestPayload`, `OpForwardPayload`), supporting structs (`MapStateChunk`, `MapType`, `DeltaOp`) | -- | ~15% |
| G3 | 2 | Create `traits.rs`: 5 traits (`ClusterService`, `MembershipService`, `ClusterPartitionService`, `MigrationService`, `FailureDetector`) with placeholder types for `ClusterPartitionTable` and `ClusterChange` | G1, G2 (logical ordering only -- cross-file imports are not enforced by the compiler during this spec since files are outside the module tree per Constraint #7) | ~10% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3 | No | 1 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-02-22)
**Status:** NEEDS_REVISION

**Context Estimate:** ~37% total

**Critical:**
1. `MapStateChunk.map_type: String` with known values `"lww" | "or"` violates PROJECT.md Rust Type Mapping Rule #4 ("Enums over strings for known value sets"). Replace with a `MapType` enum (e.g., variants `Lww`, `Or`) and use `#[serde(rename_all = "lowercase")]` for wire compatibility.
2. AC #11 ("cargo test passes; cargo clippy is clean") contradicts Constraint #7 ("DO NOT create mod.rs or modify lib.rs"). Without module wiring, the 3 new files are not part of the crate's module tree and will not be compiled or checked by cargo. Either: (a) clarify AC #11 to mean "no regressions to existing cargo test/clippy" and note that new file compilation is deferred to SPEC-060c, or (b) permit creating a minimal `mod.rs` that declares the 3 submodules (and remove this from Constraint #7).

**Recommendations:**
3. G2 lists `G1 (imports types)` as a dependency yet is placed in the same wave (Wave 1, parallel). Since these files are not in the module tree during this spec (per Constraint #7), cross-file imports are irrelevant during this spec's execution. Consider removing the stated dependency so the wave assignment is consistent, or add a note that dependencies describe logical (not compilation) ordering.
4. [Strategic] Goal Analysis section is recommended for medium-complexity specs. Not critical since this is a sub-spec of SPEC-060 which was decomposed with goal context at the parent level.
5. The `ClusterService` trait's `subscribe_changes` method returns `mpsc::UnboundedReceiver<ClusterChange>`. Explicitly state the import as `tokio::sync::mpsc` in the spec to avoid ambiguity with `std::sync::mpsc` (which has no `UnboundedReceiver`).

### Response v1 (2026-02-22)
**Applied:** All 2 critical issues and all 3 recommendations

**Changes:**
1. [✓] `MapStateChunk.map_type: String` replaced with `MapType` enum — Added `MapType` enum definition (variants `Lww`, `Or`, `#[serde(rename_all = "lowercase")]`) to the `MapStateChunk` supporting structs in messages.rs requirements. Updated `MapStateChunk.map_type` field type from `String` to `MapType`. Added new AC #14 to verify the enum has exactly 2 variants with correct serde configuration. Updated G2 task list to include `MapType` in supporting structs. Updated AC #10 to explicitly call out `MapType` for `MapStateChunk.map_type`.
2. [✓] AC #11 contradiction with Constraint #7 resolved — Rewrote AC #11 to clarify it means "no regressions to existing cargo test/clippy" and explicitly states that the 3 new files are not wired into the module tree in this spec, with compilation deferred to SPEC-060c. Updated Constraint #7 to add the same clarification parenthetically.
3. [✓] G2 dependency note clarified — Changed G2 Dependencies column from `G1 (imports types)` to `--` (same as G1). Added a parenthetical note to G3's dependency column clarifying that G1/G2 dependencies are logical ordering only and not enforced by the compiler during this spec.
4. [✓] Goal Analysis section — Not added. This is a sub-spec of SPEC-060 which carried full goal context at the parent level. The audit itself rated this as non-critical for sub-specs.
5. [✓] `tokio::sync::mpsc` import clarified — Updated `subscribe_changes` method signature in the `ClusterService` trait to use the fully-qualified `tokio::sync::mpsc::UnboundedReceiver<ClusterChange>` and added an inline note explaining the tokio vs std distinction.

### Audit v2 (2026-02-22)
**Status:** APPROVED

**Context Estimate:** ~47% total

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~47% | <=50% | OK |
| Largest task group | ~15% (G2) | <=30% | OK |
| Worker overhead | ~10% | <=10% | OK |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | |
| 30-50% | GOOD | <-- Current estimate |
| 50-70% | DEGRADING | |
| 70%+ | POOR | |

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields (`phi_threshold: f64` is correct -- genuinely fractional)
- [x] No `r#type: String` on message structs (`ClusterMessage` uses `#[serde(tag = "type")]`)
- [x] `Default` derived on payload structs with 2+ optional fields (`JoinResponsePayload`)
- [x] Enums used for known value sets (`MapType`, `NodeState`, `PartitionState`, `MigrationPhase`)
- [x] Wire compatibility: AC #6 specifies `rmp_serde::to_vec_named()` / `from_slice()`
- [x] `#[serde(rename_all = "camelCase")]` on every struct
- [x] `#[serde(skip_serializing_if = "Option::is_none", default)]` on every `Option<T>`

**Project compliance:** Honors PROJECT.md decisions
**Strategic fit:** Aligned with project goals (Phase 3 Rust Server, TODO-066)
**Language profile:** Compliant with Rust profile (3 files <= 5 max, types-first ordering)

**Comment:** Exceptionally well-specified. All v1 critical issues have been properly resolved. Every type, field, derive, and serde annotation is explicitly enumerated, leaving no ambiguity for the implementor. The 14 acceptance criteria are concrete and measurable. The separation between types (G1), messages (G2), and traits (G3) is clean and the wave structure is sound. The spec correctly navigates the `NodeState` vs `NodeStatus` distinction and the compilation-without-module-tree constraint. Ready for implementation.

## Execution Summary

**Executed:** 2026-02-22
**Mode:** orchestrated (direct -- subagent CLI unavailable)
**Commits:** 3

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2 | complete |
| 2 | G3 | complete |

### Files Created
- `packages/server-rust/src/cluster/types.rs` -- NodeState, PartitionState, MigrationPhase enums; MemberInfo, MembersView (with master election), PartitionMeta, PartitionAssignment, MigrationTask, ActiveMigration, ClusterHealth, ClusterConfig structs
- `packages/server-rust/src/cluster/messages.rs` -- ClusterMessage enum (18 variants), all payload structs, MapType enum, MapStateChunk, DeltaOp
- `packages/server-rust/src/cluster/traits.rs` -- ClusterService, MembershipService, ClusterPartitionService, MigrationService, FailureDetector traits; placeholder ClusterPartitionTable and ClusterChange types

### Files Modified
(none)

### Acceptance Criteria Status
- [x] AC1: NodeState enum has exactly 6 variants with correct derives
- [x] AC2: PartitionState enum has exactly 6 variants with correct derives
- [x] AC3: MembersView::master() filters Active, lowest join_version, ties by node_id
- [x] AC4: MembersView::is_master() returns true only for computed master
- [x] AC5: ClusterMessage enum has exactly 18 variants (4+3+2+5+3+1)
- [x] AC6: Payload structs use rmp_serde-compatible serde annotations (round-trip verified at SPEC-060c)
- [x] AC7: All 5 cluster traits defined with specified signatures, Send + Sync
- [x] AC8: ClusterConfig Default has all production defaults
- [x] AC9: WHY-comment explaining NodeState vs NodeStatus distinction
- [x] AC10: No f64 for integer fields, no r#type, MapType enum for known values
- [x] AC11: cargo test (183 pass) and cargo clippy (clean) -- no regressions
- [x] AC12: MigrationPhase enum has exactly 4 variants
- [x] AC13: Placeholder ClusterPartitionTable and ClusterChange in traits.rs
- [x] AC14: MapType enum has 2 variants (Lww, Or) with lowercase serde

### Deviations
(none)

---

## Review History

### Review v1 (2026-02-22)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: `NodeState` has exactly 6 variants (`Joining`, `Active`, `Suspect`, `Leaving`, `Dead`, `Removed`) with all required derives (`Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize`) and `#[serde(rename_all = "camelCase")]`
- [✓] AC2: `PartitionState` has exactly 6 variants (`Unassigned`, `Active`, `Migrating`, `Receiving`, `Draining`, `Lost`) with identical derives
- [✓] AC3: `MembersView::master()` correctly chains `.filter(|m| m.state == NodeState::Active)` then `.min_by()` comparing `join_version` first, then `node_id` lexicographically. Returns `None` for empty/all-non-Active views.
- [✓] AC4: `MembersView::is_master()` delegates to `master()` and compares `node_id` -- cannot return `true` when no master exists
- [✓] AC5: `ClusterMessage` enum has exactly 18 variants in the correct groups (4+3+2+5+3+1)
- [✓] AC6: All payload structs use `#[serde(rename_all = "camelCase")]`; serde internally-tagged enum with `rmp_serde::to_vec_named()` is wire-compatible; all `Option<T>` fields carry `#[serde(skip_serializing_if = "Option::is_none", default)]`
- [✓] AC7: All 5 traits defined -- `ClusterService` (extends `ManagedService`, `#[async_trait]`), `MembershipService` (`Send + Sync`, `#[async_trait]`), `ClusterPartitionService` (`Send + Sync`, `#[async_trait]`), `MigrationService` (`Send + Sync`, `#[async_trait]`), `FailureDetector` (`Send + Sync`, no async methods)
- [✓] AC8: `ClusterConfig::default()` sets all 11 production values exactly as specified
- [✓] AC9: Clear WHY-comment in `types.rs` distinguishing `NodeState` (internal FSM, Rust-idiomatic naming, 2 extra lifecycle variants) from `NodeStatus` (client-facing wire type, SCREAMING_CASE)
- [✓] AC10: `phi_threshold: f64` is the only `f64` field and is correctly float-semantic (accrual phi value). No `r#type` fields on structs. `MapType`, `NodeState`, `PartitionState`, `MigrationPhase` all use enums for known value sets.
- [✓] AC11: `cargo test` 183 passed, 0 failed. `cargo clippy -- -D warnings` clean. No regressions. The 3 new files correctly remain outside the module tree (no `mod.rs`, `lib.rs` not modified).
- [✓] AC12: `MigrationPhase` has exactly 4 variants: `Replicating`, `Ready`, `Finalizing`, `Failed`
- [✓] AC13: `ClusterPartitionTable` (empty struct) and `ClusterChange` (empty enum) declared in `traits.rs` under a clearly labeled placeholder comment
- [✓] AC14: `MapType` enum has variants `Lww` and `Or` with `#[serde(rename_all = "lowercase")]`; wire values will be `"lww"` and `"or"`
- [✓] Constraint 1: No runtime behavior -- all three files are pure type/trait definitions with no function bodies
- [✓] Constraint 2: `core-rust` package unmodified (verified via git status)
- [✓] Constraint 3: No `tokio::spawn` calls present
- [✓] Constraint 4: Rust Type Mapping Rules fully satisfied -- integers for integer fields, no `r#type`, `Default` on structs with 2+ Options, enums for known value sets
- [✓] Constraint 5: Every struct that crosses wire boundaries has `#[serde(rename_all = "camelCase")]`
- [✓] Constraint 6: No SPEC/TODO/Phase references in code comments beyond those explicitly required by AC13 (the spec itself mandated "clearly marked as placeholders for SPEC-060b")
- [✓] Constraint 7: No `mod.rs` created, `lib.rs` not modified
- [✓] `subscribe_changes` correctly uses `tokio::sync::mpsc::UnboundedReceiver` (not `std::sync::mpsc`)
- [✓] `PartitionMapPayload` correctly imported from `topgun_core::messages::cluster` in `traits.rs`
- [✓] `MigrateDataPayload` (wire message) correctly used in `MigrationService::handle_migrate_data` signature
- [✓] `ClusterService` correctly extends `ManagedService` from `crate::service::registry`
- [✓] `JoinResponsePayload` derives `Default` (has 3 `Option<T>` fields, meeting the 2+ threshold)
- [✓] `FetchPartitionTable` unit variant correctly works with serde's internally-tagged representation (serializes as `{"type": "FETCH_PARTITION_TABLE"}`)
- [✓] Commit messages follow project format (`feat(sf-060a): ...`) and describe the why, not just the what

**Summary:** The implementation faithfully delivers all 14 acceptance criteria with zero deviations. Code quality is high -- the `MembersView::master()` implementation is idiomatic and correct, the placeholder type pattern is clean and clearly labeled, all serde annotations are consistent, and the file organization matches the spec's wave structure. `cargo test` and `cargo clippy -- -D warnings` both pass clean against the existing 183-test suite.

---

## Completion

**Completed:** 2026-02-22
**Total Commits:** 3
**Audit Cycles:** 2
**Review Cycles:** 1
