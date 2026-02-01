# Codebase Scan Report

**Date:** 2026-02-01
**Focus:** all
**Project:** TopGun v0.9.0 - Offline-First In-Memory Data Grid

## Executive Summary

TopGun is a mature TypeScript monorepo with 263K+ lines of production code and 63K+ test lines. The codebase demonstrates solid architectural patterns with modular design, comprehensive test coverage (195 test files, ~57% test-to-code ratio), and good tooling (ESLint, Prettier, TypeScript strict mode). However, several technical debt items and quality improvements have been identified, primarily around error handling, console logging in production, and empty catch blocks.

**Health Score:** Moderate

**Key Metrics:**
- Total source LOC: ~263K
- Test LOC: ~63K
- Test files: 195 (210 including e2e)
- Packages: 8 monorepo packages
- Classes: 57 extending classes
- Async functions: 34+ occurrences
- Type annotations issues: 9 files with `@ts-ignore`/`@ts-expect-error`

---

## Tech Debt

### High Priority

**Empty Catch Block - Silent Error Swallowing**
- Files: `packages/server/src/cluster/ClusterManager.ts:486`
- Problem: Empty catch block swallows exceptions during WebSocket close
- Impact: Failures during connection cleanup are silently ignored, making debugging cluster issues difficult
- Fix:
  ```typescript
  try {
    ws.close();
  } catch(e) {
    logger.debug({ error: e, remoteNodeId }, 'Failed to close stale WebSocket');
  }
  ```

**TODO: Custom Foreign Key Support**
- Files: `packages/adapter-better-auth/src/TopGunAdapter.ts:176`
- Problem: Adapter assumes standard `userId` relation but doesn't handle custom foreign keys
- Impact: BetterAuth integration breaks with non-standard schemas
- Fix: Add foreign key configuration support or dynamic field resolution

**Commented Debug Logging Throughout**
- Files:
  - `packages/adapter-better-auth/src/TopGunAdapter.ts:189`
  - `packages/server/src/cluster/ClusterManager.ts:557`
- Problem: Commented-out console.log statements indicate incomplete debug infrastructure
- Impact: Debugging requires code changes and redeployment
- Fix: Leverage existing `TOPGUN_DEBUG` environment variable and structured logging

### Medium Priority

**Phase Reference Without Completion Tracking**
- Files: `packages/client/src/SyncEngine.ts:183`
- Problem: Comment references "Phase 3 BUG-06" without link to issue tracker or completion status
- Impact: Tech debt tracking is informal and hard to audit
- Fix: Create GitHub issues for all phase references and link in comments

**Default Export Usage in Config Files**
- Files: `packages/core/tsup.config.ts`, `packages/core/vitest.config.ts`
- Problem: Config files use default exports, inconsistent with project's named export convention
- Impact: Minor inconsistency, but config files are an exception
- Fix: Document this as acceptable exception or refactor to named exports if possible

**Timer/Interval Cleanup**
- Files: 304 occurrences of `setTimeout`/`setInterval` across 97 files
- Problem: No systematic timer registry for cleanup during shutdown
- Impact: Potential memory leaks and zombie timers after server shutdown
- Fix: Already partially addressed with `TimerRegistry` in `packages/server/src/utils/TimerRegistry.ts` - ensure all timers use it

### Low Priority

**Deprecated Methods**
- Files:
  - `packages/core/src/query/QueryOptimizer.ts:79` - deprecated parameter in QueryOptimizer
  - `packages/client/src/cluster/ClusterClient.ts:483` - deprecated `send()` method
- Problem: Two deprecated APIs still in codebase without removal timeline
- Impact: API confusion for new developers
- Fix: Add deprecation warnings with removal version (e.g., v1.0.0) or remove if unused

**Minimal Package Documentation**
- Files: Only 3 README files found in 8 packages
  - `packages/core/src/__benchmarks__/README.md`
  - `packages/mcp-server/README.md`
  - `packages/adapter-better-auth/README.md`
- Problem: Missing README files for core, client, server, react, adapters packages
- Impact: Onboarding friction for contributors and users
- Fix: Add README with package purpose, API overview, and examples

---

## Code Quality Issues

### Type Safety

**Type Assertion Suppressions**
- Files: 9 files use `@ts-ignore` or `@ts-expect-error`
  - `packages/core/src/utils/hash.ts:1`
  - `packages/client/src/__tests__/EncryptedStorageAdapter.test.ts:3`
  - `packages/client/src/__tests__/EncryptionManager.test.ts:3`
  - `packages/server/src/settings/SettingsController.ts:1`
  - `packages/server/src/workers/worker-scripts/base.worker.ts:1`
- Count: 9 suppressions across 5 files
- Fix: Investigate each case and either fix the underlying type issue or add explanatory comments

**`as any` Type Casts**
- Files: 40+ occurrences in `packages/mcp-server/src/__tests__/mcp-integration.test.ts`
- Problem: Test file heavily uses `(result as any)` to access properties
- Impact: Test brittleness and potential runtime errors
- Fix: Define proper result type interfaces for MCP responses

**ESLint Warning: `@typescript-eslint/no-explicit-any: warn`**
- Files: ESLint configured to warn (not error) on `any` usage
- Problem: Allows `any` to creep into codebase
- Fix: Consider upgrading to `error` level after existing occurrences are fixed

### Error Handling

**No Skipped or Focused Tests**
- Files: Zero `.skip()` or `.only()` found in test files
- Status: Good - no test suite pollution

**Error Throwing Patterns**
- Count: 122 occurrences of `throw new Error` across 68 files
- Status: Appropriate - using proper error throwing, not string throws
- Observation: Could benefit from custom error classes for better categorization

### Console Logging in Production Code

**Console Statements in Source Code**
- Files: 40+ occurrences across production code (excluding tests)
  - `packages/core/src/EventJournal.ts:160,216` - error logging
  - `packages/mcp-server/src/transport/http.ts:321` - error logging
  - `packages/client/src/TopicHandle.ts:52` - error in listener
  - `packages/client/src/TopGunClient.ts` - multiple JSDoc examples with console
  - `packages/server/src/cluster/ClusterManager.ts:557` - commented console
- Problem: Production code uses `console.error()` instead of structured logger
- Impact: Inconsistent logging, missing context, hard to filter in production
- Fix: Replace all `console.*` with logger from `utils/logger` module

**ESLint `no-console: warn` Configuration**
- Files: `eslint.config.mjs:10`
- Problem: Console usage only warns, doesn't error
- Impact: New console statements can slip through code review
- Fix: Consider upgrading to `error` level for non-test files

### Code Duplication

**Message Handler Registration Pattern**
- Files: 12 message handler files in `packages/server/src`
- Observation: Handlers use consistent registration pattern via `handlers-module.ts`
- Status: Good architectural pattern, domain-grouped for Rust portability

**Export Re-export Pattern**
- Files: 14 `index.ts` files with `export * from` statements
- Status: Standard barrel export pattern, acceptable

---

## Security Considerations

**Environment Variable Usage**
- Files: 97+ files use `process.env.*`
- Risk: Environment variables used throughout (see `packages/server/src/start-server.ts:8-32`)
- Severity: Medium
- Observations:
  - Good: `JWT_SECRET` validation exists (`validateJwtSecret()`)
  - Good: `.env` is in `.gitignore` (checked)
  - Concern: No validation for required env vars at startup
- Mitigation:
  - Add centralized env validation at server startup
  - Use schema validation (Zod) for environment variables
  - Document required vs optional env vars in README

**Hardcoded Secrets Search**
- Files: Searched for `password|secret|api_key` patterns
- Results: Found references in debug commands and env config examples only
- Status: No hardcoded secrets detected
- Note: All references are for configuration or documentation

**Production Debug Endpoints**
- Files:
  - `packages/server/src/ServerCoordinator.ts:137` - debug endpoints gated by `debugEnabled`
  - `packages/server/src/ServerFactory.ts:155` - defaults to `TOPGUN_DEBUG === 'true'`
- Risk: Debug endpoints (`/debug/crdt/*`, `/debug/search/*`) expose internal state
- Severity: Medium
- Mitigation: Ensure `TOPGUN_DEBUG` is never enabled in production, add warning in docs

**TLS Configuration**
- Files: `packages/server/src/start-server.ts:19-32`
- Observation: TLS support exists for both client and cluster connections
- Status: Good - mTLS available for cluster (`TOPGUN_CLUSTER_MTLS`)
- Recommendation: Document TLS setup and provide example certificates for dev

---

## Test Coverage Gaps

**Test-to-Code Ratio**
- Source files: 343 TypeScript files (non-test)
- Test files: 195 test files in packages + 15 e2e tests
- Ratio: ~57% files have corresponding tests
- Status: Good coverage

**E2E Test Coverage**
- Files: 15 e2e tests in `tests/e2e/`
  - `basic-sync.test.ts` (706 LOC)
  - `live-queries.test.ts` (1223 LOC)
  - `offline-online.test.ts` (1180 LOC)
  - `pubsub.test.ts` (1100 LOC)
  - `multi-client.test.ts` (1096 LOC)
  - `fulltext-search.test.ts` (892 LOC)
  - Cluster tests: `node-failure`, `partition-routing`, `replication`
  - Security: `uat-security-hardening.test.ts`
- Status: Excellent - comprehensive e2e coverage

**Missing Test Areas**

**CLI Commands**
- Files: Only 1 CLI test found (`tests/cli/doctor.test.ts`)
- What's missing: Tests for `bin/commands/*.js` (cluster start/stop, dev, setup, config, docker)
- Priority: Medium
- Fix: Add integration tests for CLI commands

**Load Testing Infrastructure**
- Files: k6 scenarios exist but require manual execution
- What's missing: Automated performance regression detection
- Priority: Low
- Fix: Integrate k6 tests into CI with baseline thresholds

**Browser Adapter Testing**
- Files: `packages/adapters/src/` has IDB adapter
- What's missing: Browser-based tests (currently only Node.js tests)
- Priority: Medium
- Fix: Add Playwright/Puppeteer tests for browser storage adapters

---

## Architecture Observations

**Module Boundary Clarity**

**Positive: Strong Package Hierarchy**
- Current: `core` → `client`/`server` → `adapters`/`react`
- Concern: None
- Status: Clean dependency graph enforced by TypeScript paths

**Negative: Large Core Files**
- Files:
  - `packages/client/src/SyncEngine.ts` (1319 LOC)
  - `packages/server/src/subscriptions/DistributedSubscriptionCoordinator.test.ts` (1282 LOC)
  - `packages/server/src/subscriptions/DistributedSubscriptionCoordinator.ts` (1064 LOC)
  - `packages/server/src/search/SearchCoordinator.ts` (1058 LOC)
  - `packages/server/src/coordinator/types.ts` (1051 LOC)
  - `packages/core/src/IndexedORMap.ts` (988 LOC)
  - `packages/core/src/IndexedLWWMap.ts` (969 LOC)
- Concern: Files >1000 LOC are complexity hotspots
- Suggestion: Consider splitting by responsibility (e.g., SyncEngine → SyncEngine + SyncHandlers)

**Modular Refactoring in Progress**

**Positive: ServerFactory Modularization**
- Files: `packages/server/src/modules/handlers-module.ts` (861 LOC)
- Current: Handlers grouped by domain (CRDT, Sync, Query, Messaging, etc.)
- Status: Excellent architectural pattern for Rust portability (SPEC-011d)
- Suggestion: Document this pattern as reference for future modularization

**Message Handler Architecture**
- Files: 26 message handlers across 12 handler files
- Pattern: Clean separation via `createMessageRegistry()`
- Status: Good - supports message routing and testing

**Cluster Architecture**

**271 Partitions for Scalability**
- Files: `packages/server/src/cluster/PartitionService.ts`
- Observation: Fixed partition count for consistent hashing
- Status: Good - industry-standard partition count
- Concern: No dynamic repartitioning documented

**Failure Detection and Recovery**
- Files:
  - `packages/server/src/cluster/FailureDetector.ts`
  - `packages/server/src/cluster/RepairScheduler.ts`
  - `packages/server/src/cluster/MigrationManager.ts`
- Status: Comprehensive cluster resilience
- Tests: E2E tests exist (`tests/e2e/cluster/node-failure.test.ts`)

**Worker Pool for CPU-Intensive Operations**
- Files:
  - `packages/server/src/workers/WorkerPool.ts`
  - Worker types: MerkleWorker, CRDTMergeWorker, SerializationWorker
- Status: Good - offloads CPU work from event loop
- Concern: Worker script dependencies unclear (`worker-scripts/base.worker.ts`)

**Observability & Debugging**

**Debug Infrastructure (Phase 14C)**
- Files:
  - `packages/core/src/debug/CRDTDebugger.ts`
  - `packages/core/src/debug/SearchDebugger.ts`
- Features: Operation recording, conflict tracking, replay capability
- Status: Excellent - production-grade debugging
- Activation: `TOPGUN_DEBUG=true` env variable

**Metrics Service**
- Files: Referenced in `ServerFactory.ts` but implementation not scanned
- Observation: Metrics port configurable (`metricsPort`)
- Recommendation: Document metrics endpoints and available metrics

---

## Suggested Specifications

Based on this scan, consider creating specs for:

1. **Error Handling Standardization** — Replace all console.error with structured logging, fix empty catch blocks
   - Priority: High
   - Complexity: medium
   - Run: `/sf:new "Error Handling Standardization"`

2. **Environment Variable Validation** — Add startup validation for required env vars using Zod
   - Priority: High
   - Complexity: small
   - Run: `/sf:new "Environment Variable Validation"`

3. **Package Documentation** — Add README files to core, client, server, react, adapters packages
   - Priority: Medium
   - Complexity: medium
   - Run: `/sf:new "Package Documentation"`

4. **CLI Test Coverage** — Add integration tests for all CLI commands
   - Priority: Medium
   - Complexity: medium
   - Run: `/sf:new "CLI Test Coverage"`

5. **Type Safety Cleanup** — Remove all `@ts-ignore` and `as any` with proper typing
   - Priority: Medium
   - Complexity: small
   - Run: `/sf:new "Type Safety Cleanup"`

6. **Large File Refactoring** — Split SyncEngine and SearchCoordinator files by responsibility
   - Priority: Low
   - Complexity: large
   - Run: `/sf:new "Large File Refactoring"`

7. **Timer Cleanup System** — Ensure all timers use TimerRegistry for proper cleanup
   - Priority: Medium
   - Complexity: small
   - Run: `/sf:new "Timer Cleanup System"`

8. **BetterAuth Custom Foreign Keys** — Add support for custom foreign key configuration
   - Priority: Medium
   - Complexity: small
   - Run: `/sf:new "BetterAuth Custom Foreign Keys"`

9. **Debug Endpoint Security Audit** — Document and harden debug endpoint protection
   - Priority: High
   - Complexity: small
   - Run: `/sf:new "Debug Endpoint Security Audit"`

10. **Browser Storage Testing** — Add Playwright tests for IndexedDB adapter
    - Priority: Medium
    - Complexity: medium
    - Run: `/sf:new "Browser Storage Testing"`

---

*Scan completed: 2026-02-01 - Analyzed 343 source files, 195 test files, 8 packages*
