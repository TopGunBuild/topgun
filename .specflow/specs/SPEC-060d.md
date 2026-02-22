# SPEC-060d: Cluster Protocol — Migration Service Implementation (Wave 2)

```yaml
id: SPEC-060d
type: feature
status: deferred
priority: P1
complexity: medium
parent: SPEC-060
depends_on: [SPEC-060c, TODO-064]
created: 2026-02-22
todo: TODO-066
```

## Context

This is the fourth sub-specification of SPEC-060, covering the Wave 2 (Dynamic Cluster) scope. It implements the 2-phase CRDT-aware migration protocol, partition state machine transitions during migration, NOT_OWNER response generation, and partition map push to connected clients.

**Deferred:** This spec depends on TODO-064 (networking layer) which provides inter-node WebSocket transport. The migration service requires sending `MigrateStart`, `MigrateData`, `MigrateReady`, and `MigrateFinalize` messages between nodes, which requires the networking layer.

### Scope Preview

- `MigrationService` implementation (2-phase CRDT-aware protocol per research Section 5.1-5.4)
- Partition state machine transitions: Active -> Migrating -> Draining -> Unassigned (source), Unassigned -> Receiving -> Active (destination)
- NOT_OWNER response with current `PartitionMapPayload`
- Partition map push to connected clients on ownership change
- Migration ordering for availability preservation
- Migration rollback on failure (source transitions back to Active)
- Rebalancing trigger on membership change

### Acceptance Criteria (from parent SPEC-060)

23. Migration service executes 2-phase protocol: REPLICATE then FINALIZE.
24. Source partition state transitions: Active -> Migrating -> Draining -> Unassigned.
25. Destination partition state transitions: Unassigned -> Receiving -> Active.
26. Source continues accepting writes during migration (CRDT advantage).
27. Migration rollback: source transitions back to Active on failure.
28. NOT_OWNER response includes current `PartitionMapPayload`.
29. Partition map is pushed to connected clients on ownership change.

## Task

TBD -- Full requirements will be defined after TODO-064 (networking layer) is complete, since the migration protocol requires inter-node message transport.

## Constraints

1. Depends on TODO-064 (networking layer) for inter-node communication.
2. Depends on SPEC-060c (all cluster types, traits, and algorithms must be complete).
3. Must use the `MigrationService` trait defined in SPEC-060a.
4. Must use the `ClusterMessage` variants (MigrateStart, MigrateData, MigrateReady, MigrateFinalize, MigrateCancel) defined in SPEC-060a.

## Assumptions

1. TODO-064 will provide a trait or channel-based interface for sending `ClusterMessage` to specific nodes.
2. CRDT merge during migration uses the existing LWWMap/ORMap merge methods from core-rust.
