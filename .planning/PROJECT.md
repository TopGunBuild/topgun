# TopGun Hardening

## What This Is

A hardening milestone for TopGun — the hybrid offline-first in-memory data grid. This work addresses security vulnerabilities, fixes known bugs, and reduces technical debt identified in the codebase audit. The goal is production-readiness and improved maintainability.

## Core Value

Make TopGun safe to deploy in production environments while making the codebase easier to maintain and extend.

## Requirements

### Validated

- CRDT primitives (LWWMap, ORMap) with HLC conflict resolution — existing
- Real-time sync via WebSocket — existing
- Merkle tree delta synchronization — existing
- PostgreSQL and SQLite persistence — existing
- React hooks and bindings — existing
- Cluster partitioning with consistent hashing — existing
- Full-text search (BM25) — existing
- JWT authentication — existing

### Active

**Security Hardening:**
- [ ] JWT strict validation — reject default secret in production
- [ ] Clock drift strict mode — reject timestamps beyond threshold
- [ ] WebSocket message sanitization — validate all incoming messages

**Bug Fixes:**
- [ ] Fix skipped worker tests (CRDTMergeWorker, MerkleWorker, SerializationWorker)
- [ ] BetterAuth adapter cold start race condition
- [ ] Topic offline queue — implement message queueing instead of dropping
- [ ] Remove debug logging from getMapAsync

**Tech Debt:**
- [ ] Split ServerCoordinator (5086 lines) into focused modules
- [ ] Split SyncEngine (2540 lines) into focused classes
- [ ] Replace 100+ `as any` type casts with proper types
- [ ] Add LRU eviction for unbounded collections (maps, queries, clients)

### Out of Scope

- Data export/import functionality — defer to future milestone
- Schema migrations — defer to future milestone
- Per-client rate limiting — defer to future milestone
- Full cluster integration test suite — defer to future milestone
- Partition count change mechanism — architecture change too large

## Context

**Source of requirements:** `.planning/codebase/CONCERNS.md` audit dated 2026-01-18

**Existing architecture:**
- Local-first CRDT-based data grid with server-authoritative clustering
- 8 packages: core, client, server, react, adapters, native, adapter-better-auth, mcp-server
- ~5000 lines in ServerCoordinator, ~2500 in SyncEngine — primary refactor targets

**Known fragile areas:**
- ClusterMessage type union (30+ message types)
- IndexedLWWMap/IndexedORMap (complex query optimization)
- Storage loading race conditions

## Constraints

- **Backward compatibility**: Minor API breaks OK with migration path, but existing data must remain readable
- **Test coverage**: All fixes must include tests; no new skipped tests
- **No new dependencies**: Use existing stack unless absolutely necessary
- **Incremental commits**: Each change should be independently deployable

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Security first | Production deployments at risk | — Pending |
| Refactor after bugs | Stabilize behavior before restructuring | — Pending |
| LRU over hard limits | Graceful degradation preferred | — Pending |

---
*Last updated: 2026-01-18 after initialization*
