# Requirements: TopGun Hardening

**Defined:** 2026-01-18
**Core Value:** Make TopGun safe for production and easier to maintain

## v1 Requirements

Requirements for this hardening milestone. Each maps to roadmap phases.

### Security

- [x] **SEC-01**: Server throws error on startup if JWT_SECRET is missing or uses default in production mode
- [x] **SEC-02**: HLC provides configurable strict mode that rejects timestamps beyond threshold (e.g., 5 minutes)
- [x] **SEC-03**: All WebSocket messages validated via Zod schemas before processing
- [x] **SEC-04**: Invalid messages logged and rate-limited (not just rejected)

### Bug Fixes

- [x] **BUG-01**: CRDTMergeWorker tests pass (no test.skip)
- [x] **BUG-02**: MerkleWorker tests pass (no test.skip)
- [x] **BUG-03**: SerializationWorker tests pass (no test.skip)
- [x] **BUG-04**: DistributedSearch E2E test passes (no describe.skip)
- [x] **BUG-05**: BetterAuth adapter waits for data load before accepting requests
- [x] **BUG-06**: Topic messages queued when offline with configurable max size
- [x] **BUG-07**: getMapAsync debug logging gated behind TOPGUN_DEBUG env var

### Refactoring

- [x] **REF-01**: ServerCoordinator auth logic extracted to AuthHandler module
- [x] **REF-02**: ServerCoordinator connection logic extracted to ConnectionManager module
- [x] **REF-03**: ServerCoordinator operation handling extracted to OperationHandler module
- [x] **REF-04**: ServerCoordinator storage logic extracted to StorageManager module
- [ ] **REF-05**: SyncEngine WebSocket logic extracted to WebSocketManager class
- [ ] **REF-06**: SyncEngine query handling extracted to QueryManager class
- [ ] **REF-07**: SyncEngine backpressure logic extracted to BackpressureController class
- [ ] **REF-08**: All `as any` casts in packages/server replaced with proper types
- [ ] **REF-09**: All `as any` casts in packages/client replaced with proper types
- [ ] **REF-10**: All `as any` casts in packages/adapter-better-auth replaced with proper types
- [ ] **REF-11**: LRU eviction added to client maps collection with configurable max size
- [ ] **REF-12**: LRU eviction added to client queries collection with configurable max size
- [ ] **REF-13**: LRU eviction added to server clients collection with configurable max size

## v2 Requirements

Deferred to future milestones.

### Infrastructure

- **INF-01**: Data export/import API for backup and migration
- **INF-02**: Schema migration system for stored records
- **INF-03**: Per-client rate limiting for multi-tenant deployments

### Testing

- **TEST-01**: Full cluster partition rebalancing integration tests
- **TEST-02**: E2E tests for custom conflict resolvers
- **TEST-03**: Large offline queue sync stress tests

## Out of Scope

| Feature | Reason |
|---------|--------|
| Partition count change mechanism | Major architecture change, requires data migration tooling |
| Full test coverage for all cluster scenarios | Time-boxed milestone, focus on critical paths |
| deprecated serverUrl removal | Breaking change too large, document migration path only |

## Traceability

Which phases cover which requirements. Updated during roadmap creation.

| Requirement | Phase | Status |
|-------------|-------|--------|
| SEC-01 | Phase 1 | Complete |
| SEC-02 | Phase 1 | Complete |
| SEC-03 | Phase 1 | Complete |
| SEC-04 | Phase 1 | Complete |
| BUG-01 | Phase 2 | Complete |
| BUG-02 | Phase 2 | Complete |
| BUG-03 | Phase 2 | Complete |
| BUG-04 | Phase 2 | Complete |
| BUG-05 | Phase 3 | Complete |
| BUG-06 | Phase 3 | Complete |
| BUG-07 | Phase 3 | Complete |
| REF-01 | Phase 4 | Complete |
| REF-02 | Phase 4 | Complete |
| REF-03 | Phase 4 | Complete |
| REF-04 | Phase 4 | Complete |
| REF-05 | Phase 5 | Pending |
| REF-06 | Phase 5 | Pending |
| REF-07 | Phase 5 | Pending |
| REF-08 | Phase 6 | Pending |
| REF-09 | Phase 6 | Pending |
| REF-10 | Phase 6 | Pending |
| REF-11 | Phase 7 | Pending |
| REF-12 | Phase 7 | Pending |
| REF-13 | Phase 7 | Pending |

**Coverage:**
- v1 requirements: 24 total
- Mapped to phases: 24
- Unmapped: 0

---
*Requirements defined: 2026-01-18*
*Last updated: 2026-01-19 after Phase 4 completion*
