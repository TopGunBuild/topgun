# Codebase Scan Report

**Date:** 2026-01-20
**Focus:** all
**Project:** TopGun - Hybrid Offline-First In-Memory Data Grid

## Executive Summary

TopGun is a well-structured TypeScript monorepo with solid architecture for a distributed CRDT-based data system. The codebase has good test coverage (181 test files for 280 source files) and follows consistent patterns. Primary concerns are code complexity in core components (ServerCoordinator at 5000+ lines), type safety issues with `any` usage, and some silent error handling that could mask issues in production.

**Health Score:** Moderate (Good foundation, some areas need attention)

---

## Tech Debt

### High Priority

**Massive ServerCoordinator Class**
- Files: `packages/server/src/ServerCoordinator.ts` (5011 lines)
- Problem: Single class handles too many responsibilities (WebSocket server, clustering, storage, queries, topics, security, metrics)
- Impact: Hard to maintain, test, and extend. Changes risk regressions.
- Fix: Continue extracting responsibilities into dedicated managers (StorageManager, ConnectionManager, OperationHandler already started). Consider splitting into ServerCore + feature modules.

**Silent Error Swallowing**
- Files: `packages/server/src/ServerCoordinator.ts:2886`, `packages/server/src/cluster/ClusterManager.ts:486`
- Problem: Empty catch blocks `catch(e) {}` hide errors
- Impact: Debugging issues becomes difficult; errors go unnoticed in production
- Fix: At minimum log errors at debug level, or rethrow after cleanup

### Medium Priority

**Large SyncEngine Class**
- Files: `packages/client/src/SyncEngine.ts` (2015 lines)
- Problem: Client-side sync logic is becoming monolithic
- Impact: Similar maintainability concerns as ServerCoordinator
- Fix: Continue refactoring with WebSocketManager, QueryManager, BackpressureController pattern

**Deprecated API Parameters**
- Files: `packages/client/src/SyncEngine.ts:79`, `packages/client/src/sync/types.ts:120`, `packages/client/src/cluster/ClusterClient.ts:483`
- Problem: Deprecated `serverUrl` parameter still present alongside `connectionProvider`
- Impact: API surface confusion, maintenance burden
- Fix: Create migration guide and plan removal in next major version

**TODO Comments in Production Code**
- Files: `packages/adapter-better-auth/src/TopGunAdapter.ts:152`
- Problem: TODO for handling custom foreign keys not implemented
- Impact: Potential data relationship bugs with BetterAuth adapter
- Fix: Implement foreign key handling or document limitation

### Low Priority

**Console Statements in Production Code**
- Files: `packages/server/src/storage/BetterSqlite3Adapter.ts:110,155`, `packages/client/src/TopicHandle.ts:52`, `packages/client/src/crypto/EncryptionManager.ts:50`
- Problem: Console.log/error used instead of structured logger
- Impact: Inconsistent logging, harder to configure log levels
- Fix: Replace with logger utility calls

---

## Code Quality Issues

### Type Safety

**Excessive `any` Usage**
- Files: Multiple files (50+ occurrences)
- Key locations:
  - `packages/adapter-better-auth/src/TopGunAdapter.ts:88,125,136,161,168,184,192,193,278`
  - `packages/client/src/types.ts:22,75`
  - `packages/client/src/HybridQueryHandle.ts:309-321`
  - `packages/client/src/TopicHandle.ts:3,22,47`
  - `packages/client/src/cluster/ClusterClient.ts:72,120,128,133,343,379`
- Count: 50+ occurrences in source files
- Fix: Define proper interfaces, use generics, or use `unknown` with type guards

**@ts-ignore Directives**
- Files: `packages/client/src/__tests__/EncryptedStorageAdapter.test.ts:7,11,15`, `packages/client/src/__tests__/EncryptionManager.test.ts:6,10,14`
- Count: 6 occurrences
- Fix: Improve type definitions or use proper mocking patterns

### Error Handling

**Generic Error Messages**
- Files: `packages/client/src/cluster/ClusterClient.ts:176,227,272,324,371,672`
- Problem: Errors like "No healthy connection available" lack context (which cluster, why unhealthy)
- Fix: Include connection state, last error, and retry count in error messages

**Try-Catch Ratio**
- 226 try blocks vs 940 async functions
- Some async operations may lack proper error handling
- Fix: Audit async functions for missing try-catch wrappers

### Code Duplication

**Similar Connection/Retry Logic**
- Files: `packages/client/src/sync/WebSocketManager.ts`, `packages/client/src/cluster/ConnectionPool.ts`, `packages/client/src/cluster/ClusterClient.ts`
- Problem: Backoff/retry logic implemented multiple times
- Fix: Extract to shared utility or base class

---

## Security Considerations

**ProcessorSandbox Fallback Mode**
- Files: `packages/server/src/ProcessorSandbox.ts:34-49`
- Risk: Falls back to less secure Node.js vm module when isolated-vm unavailable
- Severity: High (in production)
- Mitigation: The code already warns in production. Consider refusing to start without isolated-vm in production mode.

**JWT Secret Validation**
- Files: `packages/server/src/bootstrap/BootstrapController.ts:118`, `packages/server/src/ServerCoordinator.ts:194`
- Risk: JWT secret handling requires careful validation
- Severity: Medium
- Mitigation: `validateJwtSecret` utility exists - ensure it enforces minimum length and complexity

**Secrets in Environment Variables**
- Files: `packages/server/src/bootstrap/BootstrapController.ts:257-261`
- Risk: Secrets from env vars visible in process listing
- Severity: Low (standard practice)
- Mitigation: Support for Docker/K8s secrets via file mounting already implemented

**Entry Processor Code Execution**
- Files: `packages/server/src/ProcessorSandbox.ts:162,240`
- Risk: Executes user-provided code
- Severity: Medium (mitigated by sandbox)
- Mitigation: Isolated-vm with memory/CPU limits. Ensure `validateProcessorCode` from core is comprehensive.

---

## Test Coverage Gaps

**React Package Has No Tests**
- Files: `packages/react/src/hooks/*.ts` (13 hook files)
- What's missing: Unit tests for useMap, useQuery, useORMap, useTopic, useMutation, useSearch, useHybridQuery, usePNCounter, useEntryProcessor, useEventJournal, useConflictResolver, useMergeRejections
- Priority: High - React hooks are user-facing API

**MCP Server Package Limited Tests**
- Files: `packages/mcp-server/src/` (tools, transport, CLI)
- What's missing: Integration tests for MCP protocol compliance
- Priority: Medium

**Native Package Tests**
- Files: `packages/native/__tests__/hash.test.ts`
- Current: Only hash tests exist
- What's missing: Performance regression tests, cross-platform validation
- Priority: Low

---

## Architecture Observations

**Good: Clear Package Hierarchy**
- Current: core -> client/server -> adapters/react
- Concern: None - well designed
- Suggestion: Document dependency direction in CLAUDE.md (already done)

**Good: Coordinator Pattern Emerging**
- Current: ServerCoordinator delegates to StorageManager, ConnectionManager, OperationHandler
- Concern: Refactor incomplete - ServerCoordinator still 5000+ lines
- Suggestion: Continue extraction, target <1000 lines for coordinator

**Concern: Mixed Storage Patterns**
- Current: IServerStorage interface with PostgresAdapter, BetterSqlite3Adapter, MemoryServerAdapter
- Concern: Some adapters have different feature support (SQLite has verbose logging, Postgres doesn't)
- Suggestion: Standardize adapter capabilities and feature flags

**Concern: Worker Pool Optional Dependency**
- Current: Workers are optional (workerPoolEnabled config)
- Concern: Performance characteristics differ significantly with/without workers
- Suggestion: Document performance implications clearly

---

## Suggested Specifications

Based on this scan, consider creating specs for:

1. **React Hooks Test Suite** - Add comprehensive tests for all React hooks
   - Priority: High
   - Complexity: medium

2. **ServerCoordinator Continued Refactor** - Extract remaining responsibilities into dedicated managers
   - Priority: High
   - Complexity: large

3. **Type Safety Improvement** - Replace `any` types with proper interfaces in public APIs
   - Priority: Medium
   - Complexity: medium

4. **Error Handling Audit** - Review all async functions for proper error handling
   - Priority: Medium
   - Complexity: medium

5. **Silent Catch Fix** - Replace empty catch blocks with proper logging/handling
   - Priority: Medium
   - Complexity: small

6. **Deprecation Cleanup** - Remove deprecated serverUrl parameter path
   - Priority: Low
   - Complexity: small

7. **MCP Server Integration Tests** - Add MCP protocol compliance tests
   - Priority: Low
   - Complexity: medium

---

## Metrics Summary

| Metric | Value |
|--------|-------|
| Total Source Files | 280 |
| Total Test Files | 181 |
| Test/Source Ratio | 0.65 |
| Largest File | ServerCoordinator.ts (5011 lines) |
| `any` Occurrences | 50+ |
| TODO/FIXME/BUG Comments | 12 |
| Empty Catch Blocks | 2 |
| Deprecated APIs | 4 |
| Packages with Tests | 6/8 |

---

*Scan completed: 2026-01-20T00:00:00Z*
