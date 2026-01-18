# Codebase Concerns

**Analysis Date:** 2026-01-18

## Tech Debt

**ServerCoordinator God Object (5086 lines):**
- Issue: `packages/server/src/ServerCoordinator.ts` is a massive 5086-line file handling WebSocket connections, authentication, CRDT operations, clustering, queries, persistence, and more
- Files: `packages/server/src/ServerCoordinator.ts`
- Impact: Difficult to maintain, test, and reason about. High cognitive load for developers. Changes risk unintended side effects.
- Fix approach: Extract into focused modules (AuthHandler, ConnectionManager, OperationHandler, StorageManager). Use composition pattern.

**SyncEngine Complexity (2540 lines):**
- Issue: `packages/client/src/SyncEngine.ts` handles too many responsibilities: WebSocket, state machine, queries, topics, backpressure, write concerns, conflict resolution
- Files: `packages/client/src/SyncEngine.ts`
- Impact: Similar to ServerCoordinator - hard to maintain and test in isolation
- Fix approach: Extract WebSocket management, query handling, and backpressure into separate classes

**Excessive `any` Type Usage:**
- Issue: Over 100 uses of `as any` type casts throughout the codebase, bypassing TypeScript safety
- Files: `packages/adapter-better-auth/src/TopGunAdapter.ts`, `packages/server/src/ServerCoordinator.ts`, `tests/e2e/*.ts`, `examples/*.ts`
- Impact: Runtime type errors, reduced IDE assistance, harder refactoring
- Fix approach: Define proper interfaces. Use generics. Create adapter-specific types.

**TODO Comments (Incomplete Features):**
- Issue: Critical TODO for topic message queuing when offline
- Files: `packages/client/src/SyncEngine.ts:639` - "TODO: Queue topic messages or drop?"
- Impact: Topic publishes are silently dropped when client is offline - data loss
- Fix approach: Implement queue with configurable max size and persistence option

**TODO in Better Auth Adapter:**
- Issue: Foreign key handling not fully implemented
- Files: `packages/adapter-better-auth/src/TopGunAdapter.ts:148` - "TODO: Handle custom foreign keys"
- Impact: Join queries may not work correctly for non-standard schemas
- Fix approach: Implement foreign key inference from Better Auth schema

**Deprecated API Still in Use:**
- Issue: `serverUrl` option deprecated in favor of `connectionProvider` but still widely used
- Files: `packages/client/src/SyncEngine.ts:68`, all examples and tests
- Impact: Inconsistent API, migration burden for users
- Fix approach: Complete migration to connectionProvider, update docs and examples

## Known Bugs

**Skipped Tests Indicate Broken Features:**
- Symptoms: Multiple test.skip and describe.skip in worker tests
- Files:
  - `packages/server/src/__tests__/workers/CRDTMergeWorker.test.ts:182,371`
  - `packages/server/src/__tests__/workers/MerkleWorker.test.ts:100,223,288`
  - `packages/server/src/__tests__/workers/SerializationWorker.test.ts:334`
  - `packages/server/src/__tests__/DistributedSearch.e2e.test.ts:21`
- Trigger: Worker thread operations with large batches
- Workaround: Tests use main thread fallback

**Race Condition in BetterAuth Adapter:**
- Symptoms: Cold start may miss data
- Files: `packages/adapter-better-auth/src/TopGunAdapter.ts:130`
- Trigger: First request before data loads from storage
- Workaround: None documented

## Security Considerations

**Default JWT Secret in Production:**
- Risk: Hardcoded fallback `'topgun-secret-dev'` if JWT_SECRET not set
- Files: `packages/server/src/ServerCoordinator.ts:301`
- Current mitigation: Console warning recommended
- Recommendations: Throw error in production if no secret provided. Add startup validation.

**Clock Drift Warning Only (No Rejection):**
- Risk: Accepting remote timestamps far in the future allows timestamp manipulation attacks
- Files: `packages/core/src/HLC.ts:57`
- Current mitigation: `console.warn` only - timestamps are still accepted
- Recommendations: Add configurable strict mode that rejects excessive drift. Document security implications.

**No Input Validation on WebSocket Messages:**
- Risk: Malformed messages could cause crashes or unexpected behavior
- Files: `packages/server/src/ServerCoordinator.ts` (handleMessage)
- Current mitigation: Zod schemas exist but may not cover all paths
- Recommendations: Add comprehensive message validation at ingress. Rate limit invalid messages.

**Processor Sandbox Limited:**
- Risk: Entry processors run user code with limited isolation
- Files: `packages/server/src/ProcessorSandbox.ts:282` - "isolated-vm not available"
- Current mitigation: Falls back to direct execution
- Recommendations: Make isolated-vm a hard requirement for production. Add resource limits.

## Performance Bottlenecks

**getMapAsync Debug Logging:**
- Problem: Verbose logging on every map access
- Files: `packages/server/src/ServerCoordinator.ts:4236-4252`
- Cause: Debug logging left in production code path
- Improvement path: Gate behind TOPGUN_DEBUG flag or remove

**Full Table Scan on getAllKeys:**
- Problem: `runLocalQuery` retrieves all keys before filtering
- Files: `packages/client/src/SyncEngine.ts:657-659`
- Cause: No index support in client storage
- Improvement path: Add index support to IStorageAdapter interface

**Unbounded In-Memory Collections:**
- Problem: Maps, queries, topics stored without limits
- Files:
  - `packages/client/src/SyncEngine.ts:99` - `private maps: Map<string, ...>`
  - `packages/client/src/SyncEngine.ts:100` - `private queries: Map<string, ...>`
  - `packages/server/src/ServerCoordinator.ts:186` - `private clients: Map<string, ...>`
- Cause: No eviction policy
- Improvement path: Add LRU eviction for inactive maps/queries. Add max client limits.

## Fragile Areas

**ClusterMessage Type Union:**
- Files: `packages/server/src/cluster/ClusterManager.ts:34`
- Why fragile: Single massive union type with 30+ message types. Easy to miss handling new types.
- Safe modification: Add TypeScript exhaustiveness checks. Use discriminated unions properly.
- Test coverage: Partial - not all message types have tests

**IndexedLWWMap/IndexedORMap:**
- Files: `packages/core/src/IndexedLWWMap.ts` (969 lines), `packages/core/src/IndexedORMap.ts` (988 lines)
- Why fragile: Complex query optimization with multiple index types. Many code paths.
- Safe modification: Add comprehensive property-based tests. Benchmark before/after changes.
- Test coverage: Good unit tests, but integration gaps

**Storage Loading Race:**
- Files: `packages/server/src/ServerCoordinator.ts:4210-4218`
- Why fragile: Async loading with promise tracking. Easy to introduce race conditions.
- Safe modification: Add explicit loading states. Use semaphores for concurrent loads.
- Test coverage: Limited - hard to test timing issues

## Scaling Limits

**Client Connection Map:**
- Current capacity: No explicit limit (default maxConnections: 10000)
- Limit: Memory-bound, each client has CoalescingWriter, subscriptions
- Scaling path: Implement connection shedding. Add cluster-aware load balancing.

**Partition Count Fixed:**
- Current capacity: 271 partitions (PARTITION_COUNT constant)
- Limit: Cannot be changed without data migration
- Scaling path: Design partition splitting mechanism. Document upgrade path.

**Single EventLoop Bottleneck:**
- Current capacity: ~50K ops/sec per node (estimated)
- Limit: Node.js single-threaded event loop
- Scaling path: WorkerPool exists but disabled by default. Enable and tune.

## Dependencies at Risk

**ws (WebSocket):**
- Risk: Core dependency, any breaking change affects entire system
- Impact: Client and server connectivity
- Migration plan: Abstract behind IConnectionProvider (partially done)

**msgpackr:**
- Risk: Serialization format locked to this implementation
- Impact: All data persistence and network protocol
- Migration plan: Version serialization format. Add format negotiation.

**better-sqlite3:**
- Risk: Native module, platform-specific builds
- Impact: SQLite storage adapter
- Migration plan: Already have PostgreSQL alternative. Document when to use each.

## Missing Critical Features

**No Data Export/Import:**
- Problem: No way to backup or migrate data programmatically
- Blocks: Disaster recovery, environment cloning

**No Schema Migrations:**
- Problem: Adding fields to stored records has no migration path
- Blocks: Application schema evolution

**No Rate Limiting Per-Client:**
- Problem: RateLimitInterceptor exists but not per-client
- Blocks: Multi-tenant deployments, abuse prevention

## Test Coverage Gaps

**Integration Tests for Cluster:**
- What's not tested: Full cluster partition rebalancing under load
- Files: `packages/server/src/cluster/`
- Risk: Partition reassignment may fail in production
- Priority: High

**E2E Tests for Conflict Resolution:**
- What's not tested: Custom conflict resolvers in real multi-client scenario
- Files: `packages/server/src/ConflictResolverService.ts`
- Risk: Resolver execution may fail under real conditions
- Priority: Medium

**Client Offline Queue:**
- What's not tested: Large offline operation queue sync
- Files: `packages/client/src/SyncEngine.ts` (opLog handling)
- Risk: Memory issues with large offline queues
- Priority: Medium

**Worker Pool Under Load:**
- What's not tested: Worker pool behavior with concurrent requests at scale
- Files: `packages/server/src/workers/WorkerPool.ts`
- Risk: Worker exhaustion, memory leaks
- Priority: High (tests are skipped)

---

*Concerns audit: 2026-01-18*
