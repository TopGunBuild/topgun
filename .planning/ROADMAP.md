# Roadmap: TopGun Hardening

## Overview

This hardening milestone takes TopGun from development-ready to production-safe. We start with security hardening (blocking production deployments with default secrets, validating timestamps and messages), then stabilize worker tests and fix known bugs, and finally reduce technical debt through focused refactoring of the two largest files (ServerCoordinator and SyncEngine), type safety improvements, and LRU eviction for unbounded collections.

## Phases

**Phase Numbering:**
- Integer phases (1, 2, 3): Planned milestone work
- Decimal phases (2.1, 2.2): Urgent insertions (marked with INSERTED)

Decimal phases appear between their surrounding integers in numeric order.

- [x] **Phase 1: Security Hardening** - Block unsafe production deployments and validate all inputs
- [x] **Phase 2: Worker Test Fixes** - Fix skipped worker tests to restore CI confidence
- [x] **Phase 3: Bug Fixes** - Address BetterAuth race, topic queueing, and debug logging
- [ ] **Phase 4: ServerCoordinator Refactor** - Split 5086-line god object into focused modules
- [ ] **Phase 5: SyncEngine Refactor** - Split 2540-line file into focused classes
- [ ] **Phase 6: Type Safety** - Replace 100+ `as any` casts with proper types
- [ ] **Phase 7: LRU Eviction** - Add eviction policies to unbounded collections

## Phase Details

### Phase 1: Security Hardening
**Goal**: Production deployments cannot run with unsafe defaults; all inputs are validated
**Depends on**: Nothing (first phase)
**Requirements**: SEC-01, SEC-02, SEC-03, SEC-04
**Success Criteria** (what must be TRUE):
  1. Server refuses to start in production mode without explicit JWT_SECRET
  2. HLC can be configured to reject timestamps beyond a configurable drift threshold
  3. All WebSocket messages are validated against Zod schemas before processing
  4. Invalid messages are logged with rate limiting to prevent log flooding
**Plans**: 3 plans in 2 waves

Plans:
- [x] 01-01-PLAN.md — Rate-limited logger utility and integration (SEC-04)
- [x] 01-02-PLAN.md — JWT secret production validation (SEC-01)
- [x] 01-03-PLAN.md — HLC strict mode for clock drift rejection (SEC-02)

**Note:** SEC-03 (WebSocket message validation) is already implemented in ServerCoordinator line 1365 via `MessageSchema.safeParse()`. Verification only needed.

**Wave Structure:**
| Wave | Plans | Can Run Parallel |
|------|-------|------------------|
| 1 | 01-01, 01-02 | Yes |
| 2 | 01-03 | After wave 1 |

### Phase 2: Worker Test Fixes
**Goal**: All worker tests pass without skipping; CI is fully green
**Depends on**: Phase 1
**Requirements**: BUG-01, BUG-02, BUG-03, BUG-04
**Success Criteria** (what must be TRUE):
  1. CRDTMergeWorker tests run without test.skip
  2. MerkleWorker tests run without test.skip
  3. SerializationWorker tests run without test.skip
  4. DistributedSearch E2E test runs without describe.skip
**Plans**: 3 plans in 2 waves

Plans:
- [x] 02-01-PLAN.md — Build infrastructure for compiled worker scripts (BUG-01, BUG-02, BUG-03, BUG-04 foundation)
- [x] 02-02-PLAN.md — Unskip CRDTMergeWorker, MerkleWorker, SerializationWorker tests (BUG-01, BUG-02, BUG-03)
- [x] 02-03-PLAN.md — Fix DistributedSearch E2E with proper JWT auth (BUG-04)

**Wave Structure:**
| Wave | Plans | Can Run Parallel |
|------|-------|------------------|
| 1 | 02-01 | Foundation (must run first) |
| 2 | 02-02, 02-03 | Yes (after wave 1) |

### Phase 3: Bug Fixes
**Goal**: Known bugs in production code paths are fixed
**Depends on**: Phase 2
**Requirements**: BUG-05, BUG-06, BUG-07
**Success Criteria** (what must be TRUE):
  1. BetterAuth adapter waits for storage load before accepting requests
  2. Topic messages are queued when offline with configurable max queue size
  3. getMapAsync debug logging only appears when TOPGUN_DEBUG is set
**Plans**: 3 plans in 1 wave

Plans:
- [x] 03-01-PLAN.md — BetterAuth adapter cold start fix (BUG-05)
- [x] 03-02-PLAN.md — Topic offline queue implementation (BUG-06)
- [x] 03-03-PLAN.md — Debug logging gating (BUG-07)

**Wave Structure:**
| Wave | Plans | Can Run Parallel |
|------|-------|------------------|
| 1 | 03-01, 03-02, 03-03 | Yes (all independent) |

### Phase 4: ServerCoordinator Refactor
**Goal**: ServerCoordinator is split into maintainable, testable modules
**Depends on**: Phase 3
**Requirements**: REF-01, REF-02, REF-03, REF-04
**Success Criteria** (what must be TRUE):
  1. Auth logic lives in AuthHandler module, not ServerCoordinator
  2. Connection management lives in ConnectionManager module
  3. CRDT operation handling lives in OperationHandler module
  4. Storage/persistence logic lives in StorageManager module
  5. ServerCoordinator orchestrates modules but delegates all logic
**Plans**: TBD

Plans:
- [ ] 04-01: Extract AuthHandler module
- [ ] 04-02: Extract ConnectionManager module
- [ ] 04-03: Extract OperationHandler module
- [ ] 04-04: Extract StorageManager module

### Phase 5: SyncEngine Refactor
**Goal**: SyncEngine is split into focused, single-responsibility classes
**Depends on**: Phase 4
**Requirements**: REF-05, REF-06, REF-07
**Success Criteria** (what must be TRUE):
  1. WebSocket logic lives in WebSocketManager class
  2. Query handling lives in QueryManager class
  3. Backpressure control lives in BackpressureController class
  4. SyncEngine orchestrates classes but delegates all logic
**Plans**: TBD

Plans:
- [ ] 05-01: Extract WebSocketManager class
- [ ] 05-02: Extract QueryManager class
- [ ] 05-03: Extract BackpressureController class

### Phase 6: Type Safety
**Goal**: TypeScript provides full type safety; no runtime type surprises
**Depends on**: Phase 5
**Requirements**: REF-08, REF-09, REF-10
**Success Criteria** (what must be TRUE):
  1. packages/server has zero `as any` casts
  2. packages/client has zero `as any` casts
  3. packages/adapter-better-auth has zero `as any` casts
**Plans**: TBD

Plans:
- [ ] 06-01: Type safety fixes for packages/server
- [ ] 06-02: Type safety fixes for packages/client
- [ ] 06-03: Type safety fixes for packages/adapter-better-auth

### Phase 7: LRU Eviction
**Goal**: Unbounded collections have graceful eviction policies
**Depends on**: Phase 6
**Requirements**: REF-11, REF-12, REF-13
**Success Criteria** (what must be TRUE):
  1. Client maps collection has LRU eviction with configurable max size
  2. Client queries collection has LRU eviction with configurable max size
  3. Server clients collection has LRU eviction with configurable max size
  4. Eviction is configurable and disabled by default (opt-in)
**Plans**: TBD

Plans:
- [ ] 07-01: LRU eviction for client collections (maps and queries)
- [ ] 07-02: LRU eviction for server clients collection

## Progress

**Execution Order:**
Phases execute in numeric order: 1 -> 2 -> 3 -> 4 -> 5 -> 6 -> 7

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. Security Hardening | 3/3 | ✓ Complete | 2026-01-18 |
| 2. Worker Test Fixes | 3/3 | ✓ Complete | 2026-01-18 |
| 3. Bug Fixes | 3/3 | ✓ Complete | 2026-01-18 |
| 4. ServerCoordinator Refactor | 0/4 | Not started | - |
| 5. SyncEngine Refactor | 0/3 | Not started | - |
| 6. Type Safety | 0/3 | Not started | - |
| 7. LRU Eviction | 0/2 | Not started | - |

---
*Roadmap created: 2026-01-18*
*Phase 1 planned: 2026-01-18*
*Phase 2 planned: 2026-01-18*
*Phase 3 planned: 2026-01-18*
*Depth: standard (7 phases)*
*Coverage: 24/24 requirements mapped*
