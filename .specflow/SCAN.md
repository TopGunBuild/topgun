# Codebase Scan Report

**Date:** 2026-01-30
**Focus:** all
**Project:** TopGun - Hybrid Offline-First In-Memory Data Grid

## Executive Summary

TopGun is a well-architected TypeScript monorepo implementing a distributed CRDT-based data grid with offline-first capabilities. The codebase demonstrates strong engineering practices with good test coverage (203 test files for ~447 source files, ~45% ratio) and clear package separation. Primary concerns include code complexity in core components (SyncEngine: 1415 lines, ServerFactory: 1066 lines), extensive use of `any` types compromising type safety, and some silent error handling that could mask production issues.

**Health Score:** Moderate (Solid foundation with areas needing attention)

---

## Tech Debt

### High Priority

**Massive Client SyncEngine Class**
- Files: `packages/client/src/SyncEngine.ts`
- Lines: 1,415
- Problem: Central orchestrator handles WebSocket management, state machine, query handling, topic pub/sub, locks, counters, entry processors, search, conflict resolution, merkle sync, backpressure, and heartbeats
- Impact: Single point of failure for client-side sync; difficult to test, maintain, and extend; changes risk breaking multiple features
- Fix: Continue modularization pattern already started with WebSocketManager, QueryManager, TopicManager. Extract remaining responsibilities into focused modules. Target <500 lines.

**Complex Server Factory**
- Files: `packages/server/src/ServerFactory.ts`
- Lines: 1,066
- Problem: Factory method creates and wires 40+ dependencies with complex initialization logic
- Impact: Hard to understand initialization order; difficult to test in isolation; change impact unclear
- Fix: Consider builder pattern or dependency injection container to manage wiring complexity

**Silent Error Swallowing**
- Files:
  - `packages/server/src/cluster/ClusterManager.ts:486` - `catch(e) {}`
  - `packages/server/src/coordinator/client-message-handler.ts:43` - `catch (e) { }`
- Problem: Empty catch blocks hide errors with no logging or handling
- Impact: Failures go unnoticed in production; debugging becomes extremely difficult
- Fix: Replace with `catch (e) { logger.error({ err: e }, 'Context here'); }` minimum

**Focused Tests With .only()/.skip()**
- Files: 10 test files contain `.only()`, `.skip()`, `fdescribe`, `fit`, etc.
  - `packages/server/src/__tests__/GC.test.ts`
  - `packages/server/src/__tests__/SubscriptionRouting.test.ts`
  - `packages/server/src/__tests__/Resilience.test.ts`
  - `packages/server/src/__tests__/Chaos.test.ts`
  - `packages/core/src/__tests__/query/adaptive/CompoundIndexDetection.test.ts`
  - `packages/core/src/query/adaptive/__tests__/IndexAdvisor.test.ts`
  - `packages/core/src/__tests__/ConflictResolver.test.ts`
  - `packages/core/src/__tests__/EntryProcessor.test.ts`
  - `packages/client/src/__tests__/ClientFailover.test.ts`
  - `packages/server/src/__tests__/workers/SharedMemoryManager.test.ts`
- Problem: Tests are excluded from normal runs or focused, indicating incomplete test suite cleanup
- Impact: CI may not run all tests; test coverage metrics misleading
- Fix: Remove all `.only()` and `.skip()` from committed code, use CI flags for selective runs

### Medium Priority

**Large Core Schema File**
- Files: `packages/core/src/schemas.ts`
- Lines: 1,159 (all Zod schemas)
- Problem: Single file defines all message schemas for the protocol
- Impact: Hard to navigate; every protocol change touches one massive file
- Fix: Split into logical groups (auth-schemas.ts, sync-schemas.ts, query-schemas.ts, search-schemas.ts, cluster-schemas.ts)

**Deprecated API Still Present**
- Files: `packages/client/src/SyncEngine.ts:72-73`
- Problem: `/** @deprecated Use connectionProvider instead */ serverUrl?: string;`
- Impact: API surface confusion; users may use wrong pattern; maintenance burden
- Fix: Document migration path, plan removal in v2.0

**TODO Comments in Production Code**
- Count: 20+ occurrences across codebase
- Key examples:
  - `packages/adapter-better-auth/src/TopGunAdapter.ts:152` - "TODO: Handle custom foreign keys"
  - `packages/server/src/__tests__/LiveQuery.test.ts:24` - "TODO(sf-002): Evaluate removing after test suite hardening"
- Problem: Unfinished features or workarounds in production code
- Impact: Potential bugs, incomplete features, maintenance debt
- Fix: Convert TODOs into tracked issues/specs or implement them

**Comment in Production Code (Phase 3 BUG-06)**
- Files: `packages/client/src/SyncEngine.ts:185`
- Problem: `// Merge topic queue config with defaults (Phase 3 BUG-06)` - references internal phase numbering
- Impact: Code archaeology requires understanding internal sprint/phase system
- Fix: Remove phase references from code comments; use git history/docs for context

### Low Priority

**Console Statements in Source Code**
- Count: 40+ occurrences
- Locations:
  - `tests/e2e/cluster/helpers.ts:203,209,221` - Used for test debugging
  - `examples/distributed-query-test.ts:9,36,47-66` - Example code (acceptable)
  - `packages/adapter-better-auth/src/TopGunAdapter.ts:165` - Commented out, but present
- Problem: Should use structured logger instead of console
- Impact: Cannot control log levels, format inconsistent, harder to filter
- Fix: Replace with logger utility or remove commented code

**No Linting/Formatting Configuration**
- Files: No `.eslintrc*`, `.prettierrc*`, or `biome.json` found
- Problem: Code style not enforced automatically
- Impact: Inconsistent formatting, potential code quality issues slip through
- Suggestion: Add ESLint + Prettier configuration to enforce consistent code style

---

## Code Quality Issues

### Type Safety

**Excessive `any` Usage**
- Count: 40+ occurrences in source files (excluding tests)
- Critical locations:
  - `packages/adapter-better-auth/src/TopGunAdapter.ts:88,125,136,161,168,184,192,193` - Data transformation lacks types
  - `tests/e2e/helpers/index.ts:20,22,67,69,83,143,146,189,290,304` - Test helpers use `any` extensively
  - `packages/client/src/SyncEngine.ts:32,509` - Generic record/message handling
- Impact: Type safety compromised; refactoring risks hidden; IDE autocomplete limited
- Fix:
  - Define proper interfaces for BetterAuth adapter
  - Use generics with constraints
  - Replace `any` with `unknown` + type guards where type is truly dynamic

**@ts-ignore Suppression**
- Files:
  - `packages/client/src/__tests__/EncryptedStorageAdapter.test.ts:7,11,15`
  - `packages/client/src/__tests__/EncryptionManager.test.ts:6,10,14`
- Count: 6 total
- Problem: Type errors suppressed rather than fixed
- Fix: Improve type definitions for test mocks or use proper casting

### Error Handling

**No Centralized Error Types**
- Observation: Errors thrown as generic `new Error(message)` throughout codebase
- Impact: Difficult to handle errors programmatically; no error codes or categories
- Suggestion: Define custom error types (e.g., `ConnectionError`, `AuthenticationError`, `PartitionError`) for structured error handling

**Promise Rejection Handling**
- Files: Numerous async functions without try-catch
- Example: `packages/server/src/cluster/ClusterManager.ts:486` catches errors but ignores them
- Impact: Unhandled promise rejections can crash Node.js processes
- Fix: Audit all async functions for proper error handling

### Code Duplication

**Backoff/Retry Logic**
- Files:
  - `packages/client/src/sync/WebSocketManager.ts:85-92` - BackoffConfig
  - `packages/client/src/SyncEngine.ts:45-56` - BackoffConfig (duplicate)
- Problem: Same configuration pattern defined in multiple places
- Fix: Extract to shared config module

**Similar Test Setup Code**
- Files: E2E tests in `tests/e2e/` have repeated setup patterns
- Impact: Changes to test infrastructure require updates in multiple places
- Fix: Extract common setup into shared test utilities (already partially done in `tests/e2e/helpers/`)

---

## Security Considerations

**Hardcoded Secrets in Test Code**
- Files:
  - `scripts/profile-runner.js:23` - `'benchmark-secret-key-for-testing'`
  - `scripts/profile-server.js:29` - `'topgun-secret-dev'`
  - `tests/e2e/helpers/index.ts:8` - `'test-e2e-secret'`
  - `scripts/generate-k6-token.js:28` - `'topgun-secret-dev'` (default fallback)
- Risk: Low (test/dev only)
- Severity: Low
- Mitigation: These are test secrets, but ensure they're never used in production. Server validates against default secrets in production mode (see `validateJwtSecret`)

**JWT Secret Validation**
- Files: `packages/server/src/utils/validateConfig.ts`
- Risk: Default secrets rejected in production mode
- Severity: Medium (already mitigated)
- Status: ✅ Good - `validateJwtSecret` enforces strong secrets in production

**Environment Variable Usage**
- Count: 195 occurrences of `process.env` across 34 files
- Files:
  - `packages/server/src/bootstrap/BootstrapController.ts:6` - Reads sensitive config
  - `packages/server/src/ServerFactory.ts:228` - `TOPGUN_DEBUG=true`
  - `examples/simple-server.ts:12` - `DATABASE_URL`
- Risk: Medium (standard practice, but requires proper deployment security)
- Mitigation: Document required env vars and use secret management (K8s secrets, Docker secrets)

**Password Handling in Admin Dashboard**
- Files: `apps/admin-dashboard/src/pages/Login.tsx:12,21,22,28,63-71`
- Risk: Low (UI code, transmitted to backend)
- Observation: Passwords handled in plain text in UI (expected for login form), ensure HTTPS in production
- Mitigation: Document requirement for TLS in production

**No Rate Limiting on Entry Processor**
- Files: `packages/server/src/handlers/EntryProcessorHandler.ts`
- Risk: Medium
- Problem: User-submitted code execution without apparent rate limiting
- Mitigation: Ensure ProcessorSandbox has CPU/memory limits, consider per-client rate limits

---

## Test Coverage Gaps

**React Package Has NO Tests**
- Files: `packages/react/src/hooks/*.ts` (13+ hook files)
- Missing:
  - `useMap.ts`, `useQuery.ts`, `useORMap.ts`, `useTopic.ts`
  - `useMutation.ts`, `useSearch.ts`, `useHybridQuery.ts`
  - `usePNCounter.ts`, `useEntryProcessor.ts`, `useEventJournal.ts`
  - `useConflictResolver.ts`, `useMergeRejections.ts`
- Priority: **High** - These are user-facing APIs
- Risk: Breaking changes undetected; edge cases not covered
- Suggested: Add React Testing Library tests for all hooks

**MCP Server Package Limited Coverage**
- Files: `packages/mcp-server/src/` (tools, transport, CLI)
- Current: Only basic unit tests exist
- Missing: MCP protocol compliance tests, integration tests
- Priority: Medium

**Native Package Performance Tests**
- Files: `packages/native/__tests__/hash.test.ts`
- Current: Basic correctness tests for xxHash64
- Missing: Performance regression tests, cross-platform validation
- Priority: Low

**Test File Ratio**
- Source Files: ~447
- Test Files: 203
- Ratio: 45% (moderate)
- Observation: Client and server packages have good coverage, but adapters/react/mcp-server lack tests

---

## Architecture Observations

**Good: Clear Package Hierarchy**
- Current: `core` → `client`/`server` → `adapters`/`react`
- Observation: Well-designed layered architecture with clear dependencies
- Concern: None
- Suggestion: Continue documenting in CLAUDE.md

**Good: Coordinator Pattern Emerging**
- Current: `ServerFactory` creates modular handlers (AuthHandler, OperationHandler, QueryHandler, etc.)
- Observation: Refactoring from monolithic coordinator toward composition
- Concern: Factory is now very complex (1066 lines)
- Suggestion: Consider DI container or builder pattern to simplify wiring

**Concern: Message Routing Complexity**
- Current: `MessageRouter` (Phase 09d) routes 40+ message types
- Files: `packages/client/src/SyncEngine.ts:297-404` - registerMessageHandlers
- Observation: Switch-case replaced with registry pattern (good), but 40+ handlers is complex
- Suggestion: Group related message types (auth, sync, query, search, cluster) into sub-routers

**Concern: Schema File Too Large**
- Files: `packages/core/src/schemas.ts` (1,159 lines)
- Problem: Single file defines all Zod schemas for 40+ message types
- Suggestion: Split into domain-specific schema files for maintainability

**Concern: Dual Sync Protocols**
- Current: Both LWW (Merkle-based) and ORMap sync protocols exist
- Files: `packages/client/src/sync/MerkleSyncHandler.ts`, `packages/client/src/sync/ORMapSyncHandler.ts`
- Observation: Necessary for different CRDT types, but doubles complexity
- Suggestion: Document trade-offs and usage guidelines

**Good: Modular Client Architecture**
- Current: SyncEngine delegates to 11+ specialized managers
  - WebSocketManager, QueryManager, TopicManager, LockManager
  - WriteConcernManager, CounterManager, EntryProcessorClient
  - SearchClient, MerkleSyncHandler, ORMapSyncHandler, MessageRouter
- Observation: Phase 09 refactor successfully extracted concerns
- Suggestion: Continue until SyncEngine is <500 lines (currently 1,415)

---

## Suggested Specifications

Based on this scan, consider creating specs for:

1. **React Hooks Test Suite**
   - Priority: High
   - Complexity: medium
   - Description: Add comprehensive tests for all 13+ React hooks using React Testing Library
   - Run: `/sf:new "React Hooks Test Suite"`

2. **SyncEngine Final Refactor**
   - Priority: High
   - Complexity: large
   - Description: Complete modularization of SyncEngine to <500 lines, extract auth/state management
   - Run: `/sf:new "SyncEngine Final Refactor"`

3. **Silent Error Handling Audit**
   - Priority: High
   - Complexity: small
   - Description: Replace all empty catch blocks with proper logging, add error context
   - Run: `/sf:new "Silent Error Handling Audit"`

4. **Remove Focused Tests**
   - Priority: High
   - Complexity: small
   - Description: Remove all `.only()`, `.skip()`, `fdescribe`, `fit` from committed test files
   - Run: `/sf:new "Remove Focused Tests"`

5. **Type Safety Improvement - BetterAuth Adapter**
   - Priority: Medium
   - Complexity: medium
   - Description: Replace `any` types in TopGunAdapter with proper interfaces
   - Run: `/sf:new "Type Safety Improvement - BetterAuth Adapter"`

6. **Schema File Splitting**
   - Priority: Medium
   - Complexity: small
   - Description: Split schemas.ts into auth, sync, query, search, cluster schema modules
   - Run: `/sf:new "Schema File Splitting"`

7. **ServerFactory Simplification**
   - Priority: Medium
   - Complexity: large
   - Description: Introduce builder pattern or DI container to reduce factory complexity
   - Run: `/sf:new "ServerFactory Simplification"`

8. **Add ESLint + Prettier**
   - Priority: Medium
   - Complexity: small
   - Description: Add linting and formatting configuration to enforce code style
   - Run: `/sf:new "Add ESLint + Prettier"`

9. **Deprecation Cleanup - serverUrl**
   - Priority: Low
   - Complexity: small
   - Description: Remove deprecated serverUrl parameter, document migration to connectionProvider
   - Run: `/sf:new "Deprecation Cleanup - serverUrl"`

10. **MCP Protocol Compliance Tests**
    - Priority: Low
    - Complexity: medium
    - Description: Add integration tests for MCP server package
    - Run: `/sf:new "MCP Protocol Compliance Tests"`

---

## Metrics Summary

| Metric | Value |
|--------|-------|
| Total TypeScript/JavaScript Files | ~447 |
| Total Test Files | 203 |
| Test/Source Ratio | 45% |
| Largest Files | SyncEngine.ts (1,415), ServerFactory.ts (1,066), schemas.ts (1,159) |
| Packages | 8 (core, client, server, react, adapters, mcp-server, native, adapter-better-auth) |
| `any` Occurrences | 40+ in source files |
| TODO/FIXME Comments | 20+ |
| Empty Catch Blocks | 2 confirmed |
| Focused Tests | 10 files |
| Environment Variable Usage | 195 occurrences across 34 files |
| Console Statements | 40+ |
| Missing Linter Config | Yes (no ESLint/Prettier found) |

---

## Positive Observations

- **Strong Test Coverage**: 203 test files for ~447 source files (45%)
- **E2E Testing**: Comprehensive end-to-end tests for sync, cluster, search, security
- **Load Testing**: k6 integration for throughput, connection storms, failover scenarios
- **Type-Safe Messaging**: Zod schemas validate all protocol messages
- **Security Awareness**: JWT validation, TLS support, sandbox for user code
- **Structured Logging**: Pino-based logger with structured fields (mostly)
- **Documentation**: CLAUDE.md provides clear guidance for contributors
- **Refactoring In Progress**: Evidence of ongoing modularization (Phase 09d)

---

*Scan completed: 2026-01-30T00:00:00Z*
