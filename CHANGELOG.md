# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> Versions below 0.12 are pre-release internal milestones — TopGun's public
> history begins at v2.0.0.

## [Unreleased]

- fix(query): a top-N page (a `limit` with no filter, cursor, or aggregation) over a
  map larger than RAM now streams the durable records through a bounded `limit+1` heap
  in the scan itself — it no longer materializes every non-resident row just to return a
  small page. Peak memory for such a page is O(resident) + O(limit) + one scan batch.
- fix(query): full-scan pager uses cap=limit+1 for correct pagination + mandatory _key
  tie-break shared by the scan pager and the sort stage (deterministic top-N boundary)
- **Behavior change (query result ordering):** sorted queries now break ties on the
  record key (`_key` ascending). Rows that compare equal on the sort field may surface
  in a different order than before, but the order is now deterministic across runs.
- **Behavior change (query result ordering):** a query with `limit` but no sort now
  returns rows in ascending key order. Previously the order was unspecified.
- feat(query): full-scan reads stream durable (non-resident) records and merge them
  HLC-LWW against the in-memory snapshot, so queries see records that were evicted from
  or never loaded into memory — without ever materializing the whole map in RAM.
- feat(query): new typed response code `QUERY_SNAPSHOT_OVERFLOW` (too many concurrent
  writes during a live snapshot; the client should resubscribe). A sort without a `limit`
  over a resident map is permitted (an O(result) in-memory sort, soft-capped by
  `max_query_records` — the same profile as a no-`limit` scan); the `QUERY_UNBOUNDED_SORT`
  code is reserved for a future size/residency-gated reject (sorting over a
  larger-than-RAM, non-resident match set).
- note(query): per-query Merkle is retired on the full-scan path (SYNC/QUERY separation);
  `merkleRootHash` is no longer returned. Reconnect falls back to a full snapshot
  (correct, less efficient) until the map-level Merkle tree-walk SYNC lands; clients
  degrade gracefully (`merkleRootHash` undefined → full snapshot).

## [2.0.0] - 2026-05-23

> First general-availability release of TopGun v2. Complete rewrite from the v1
> gun.js port. New Rust server, new client API, embedded backend by default,
> single-node production-ready.

### Breaking Changes

- **SDK API unified on `.subscribe` / `.onDelta`** — removed the old per-handle
  observe/onChange variants. `client.getMap(name).subscribe(fn)` returns an
  unsubscribe handle on both LWW and OR maps.
- **`TopGunClient` is now generic** — `new TopGunClient<TSchema>({...})`. Map
  and query accessors narrow types from the schema. Untyped (no generic) still
  works.
- **`TopGun` facade removed** — import and instantiate `TopGunClient` directly.
- **`@topgunbuild/client` IDBAdapter export removed** — use
  `@topgunbuild/adapters/IDBAdapter` (memory-first queueing implementation).
- **Server binary renamed `test-server` → `topgun-server`** — `cargo run --bin
  topgun-server --release` everywhere. The Docker image artifact path was
  already `/usr/local/bin/topgun-server`.
- **`JWT_SECRET` is required** — the server refuses to boot with the previous
  baked-in default and exits with a clear error if neither `JWT_SECRET` nor
  `TOPGUN_NO_AUTH=1` is set.
- **`@topgunbuild/client` `client.executeOnKey` / `executeOnKeys` and
  `ConflictResolverClient.register` / `unregister` / `list` now throw** —
  server-side custom entry processors and conflict resolvers require a WASM
  sandbox that is on the v2.x roadmap. Built-in CRDT merge logic and
  `useMergeRejections` / `ConflictResolverClient.onRejection` are unchanged.
- **`useEntryProcessor` and `useConflictResolver` React hooks removed.**
- **Default storage backend is embedded redb** (`STORAGE_BACKEND=redb`).
  Postgres still works via `STORAGE_BACKEND=postgres` + `DATABASE_URL`.
  The legacy SQLite server adapter was removed.

### Added

#### Embedded Storage (redb)
- **redb default** — server writes survive restart with no external
  dependency. File location configurable via `TOPGUN_REDB_PATH`
  (default `./topgun.redb`).
- **Production defaults for in-memory cache + write-behind buffer** —
  `TOPGUN_MAX_RAM_MB`, `TOPGUN_EVICTION_HIGH_PCT`, `TOPGUN_EVICTION_LOW_PCT`,
  `TOPGUN_EVICTION_INTERVAL_MS`, `TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS`,
  `TOPGUN_WRITEBEHIND_BATCH_SIZE`, `TOPGUN_WRITEBEHIND_CAPACITY`. See
  `CLAUDE.md` for ranges.
- **`TOPGUN_CORS_ORIGINS`** — comma-separated env var to enable browser
  cross-origin access (default deny-all).
- **Scalar index startup rebuild** with durability-before-ack contract.

#### Client SDK
- **`NullConnectionProvider`** — `TopGunClient` boots without a `serverUrl`
  for pure local-only apps.
- **`AuthRequiredError`** + `TopGunClientConfig.onAuthRequired` callback —
  when the server sends `AUTH_REQUIRED` with no token configured, the
  client now logs warn + invokes the callback instead of parking silently.
- **`useSyncState`** + `useMapWithSyncState` + `useORMapWithSyncState`
  React hooks for surface-level write-concern UX.
- **`RecordSyncStateTracker`** in `SyncEngine`.

#### Tooling
- **`create-topgun-app`** — `npx create-topgun-app my-app` scaffolds a
  working Vite + React + TopGun app with a hook-first todo demo.
- **Docker images on GHCR** — `ghcr.io/topgunbuild/topgun-server:latest`
  (multi-arch linux/amd64 + linux/arm64).
- **`@topgunbuild/mcp-server`** — MCP server for Claude Desktop and Cursor.
  Exposes eight tools (`topgun_query`, `topgun_mutate`, `topgun_search`,
  `topgun_subscribe`, `topgun_schema`, `topgun_stats`, `topgun_explain`,
  `topgun_list_maps`).

#### Cluster (single-node stable; cluster-mode partition-routing in progress)
- **Quorum-election state machine** replacing the discover-then-join protocol.
- **Cluster member lifecycle fixes** — `MemberAdded` events, disconnect on
  `MemberRemoved`, rejoin deduplication, no-auth bypass for dev clusters.

#### Docs
- New migration guides: Firebase, Y.js, Replicache, Supabase Realtime.
- New `/docs/guides/security` and `/docs/guides/mcp-server` guides.
- Roadmap, FAQ, troubleshooting, benchmarks rebuilt.
- `building-with-ai.mdx` AI Builder Guide with hook-first canon + MCP snippets.
- `llms.txt` + `llms-full.txt` for AI assistants.
- "Coming from X" comparison landing component.
- `SECURITY.md` responsible-disclosure policy.

### Changed

- Hero / landing positioning to outcome-first: "Build real-time apps that work
  offline."
- Landing fork (`landing-astro-next`) merged into the unified `docs-astro` app.
- Performance benchmarks: fire-and-forget reported as 483K ops/sec
  (corrected from a retired 560K figure).
- `TopGunClient.close()` is now async; all callers updated.

### Fixed

- 3 critical npm advisories in the prod dep tree resolved by bumping
  `better-auth >= 1.4.17` (2FA bypass), `@clerk/clerk-react >= 5.61.6`
  (middleware bypass), and forcing `fast-xml-parser >= 5.3.5` via pnpm
  override (RSS DOCTYPE bypass via `@astrojs/rss`).
- `ClusterState::update_view` made monotonic to prevent stale-view clobber.
- `topgun-server --port` default normalized to 8080; `--port 0` honored for
  ephemeral allocation in tests.
- `STORAGE_BACKEND=null` propagated through cluster spawn env (cluster test
  stability).
- Numerous cluster-routing teardown fixes (await `pool.close()`,
  `verifier.close()`, `removeNode` promises).
- `@topgunbuild/client` `pino-pretty` redirected to stderr.
- `apps/admin-dashboard` Server-Unavailable overlay no longer displays a
  non-existent `cargo run --bin topgun-server` command (the name now
  matches the actual binary).
- `apps/docs-astro/src/content/docs/guides/index.mdx` rewritten to list only
  guides that actually exist (was 22 cards, 9+ pointed at 404).
- 13 stale code samples in docs-astro corrected (`map.getAll()`, `auth: {token}`,
  `useTopic` destructuring, `create({...})`, `client.getTopic`, `cargo run --bin
  test-server`, landing "one binary" claim, `IDBAdapter('my-app')`, port 8090,
  `topgun_get/set`, quick-start slug).
- 14 SPEC-NNN references removed from `packages/server-rust/src/` to honor
  the CLAUDE.md code-comments convention.

### Removed

- **`TopGun` facade** and `TopGun.test.ts`.
- **`@topgunbuild/client` `IDBAdapter` re-export** (use `@topgunbuild/adapters`).
- **Legacy SQLite server adapter** (`BetterSqlite3Adapter`) — superseded by redb.
- **`WHITEPAPER.md`, `MIGRATION.md`, `TESTING.md`, `TODO_NEXT.md`** — outcome-
  level positioning conflicted with `CLAUDE.md`; superseded by docs-astro.
- **`specifications/` directory** (15 monolithic spec files) and `docs/`
  directory (4 legacy markdown pages) — content fully ported to
  `apps/docs-astro/`.
- **`start-clerk-server.sh`** — TS-server-era leftover; the notes-app README
  was rewritten to use `pnpm start:server` directly.
- **`scripts/analyze-bottlenecks.md`** — referenced a non-existent
  `profile-server.sh` and cited stale benchmark numbers.
- **`apps/docs-astro/src/content/blog/conflict-resolution-beyond-lww.mdx`** —
  blog promoted a feature whose backend (WASM sandbox) is on the roadmap.
- **Personal Cloudflare account_id and `easysolpro.workers.dev` URLs** stripped
  from `examples/push-worker`, `examples/storage-worker`, `examples/notes-app`.
- Dead `Hero.tsx` from landing (zero imports).
- 488 phase/spec/bug references from code comments (CLAUDE.md convention).

### Legal

- Apache ICLA at `.github/CLA.md`; cla-assistant.io bot enforcement.
- Grandfathered committers snapshot at `legal/GRANDFATHERED_COMMITTERS.md`.

### Tests

- Parallel 3-node cluster bootstrap integration tests with `serial_test`
  annotation for shared-port suites.
- Cluster failover, partition-routing, rebalance k6 scenarios.
- 4 Jest scaffold tests for `create-topgun-app`.

---

## [0.11.0] - 2026-02-07

### Added

#### HTTP Sync Protocol (`@topgunbuild/core`, `@topgunbuild/client`, `@topgunbuild/server`)
- **Stateless HTTP sync** — new `POST /sync` endpoint enables CRDT synchronization without WebSocket connections, designed for serverless environments
- **`HttpSyncProvider`** — client-side connection provider that translates sync operations into HTTP POST requests with configurable polling interval (`pollIntervalMs`)
- **`AutoConnectionProvider`** — automatically selects WebSocket or HTTP transport with transparent fallback after configurable retry attempts (`maxWsAttempts`)
- **`HttpSyncHandler`** — lightweight, stateless server-side handler for processing sync requests in serverless functions (Vercel Edge, AWS Lambda, Cloudflare Workers)
- **HTTP sync Zod schemas** — `HttpSyncRequestSchema` and `HttpSyncResponseSchema` for request/response validation with msgpackr serialization
- **Delta computation via HLC** — efficient timestamp-based filtering using `HLC.compare()` instead of Merkle tree round-trips

#### Deterministic Simulation Testing (`@topgunbuild/core`)
- **`VirtualClock`** — injectable time source for deterministic test execution
- **`clockSource` injection** — HLC, LWWMap, and ORMap now accept pluggable clock sources
- **DST infrastructure** — `SeededRNG`, `VirtualNetwork`, `InvariantChecker`, `ScenarioRunner` exported from core package

#### Query Optimizer Enhancements (`@topgunbuild/core`)
- **Point lookup optimization** — `QueryOptimizer` detects equality filters on indexed fields and generates `PointLookupStep`/`MultiPointLookupStep` execution plans
- **Network-aware cost model** — distributed query cost estimation accounting for network latency, partition count, and data transfer
- **Index hints** — `QueryOptions.indexHints` allows explicit index selection (`force`, `prefer`, `ignore`) for query planning

### Fixed
- **server**: Correct `HttpSyncError` field types and add optionality markers
- **server**: Preserve TLS status message in HTTP sync handler
- **client**: Align `requestTimeoutMs` default with spec (30s)
- **core**: Consolidate `ClockSource` interface to single definition

### Documentation
- Serverless deployment guide with Vercel Edge, AWS Lambda, and Cloudflare Workers examples
- `HttpSyncProvider` and `AutoConnectionProvider` API reference
- `POST /sync` endpoint reference with request/response schemas
- HTTP Sync section in transport decision guide
- "TopGun Goes Serverless" blog post
- "Merkle Trees & Delta Sync" blog post

---

## [0.10.1] - 2026-01-20

### Fixed
- Minor fixes and stability improvements

---

## [0.10.0] - 2026-02-04

### Breaking Changes

- **client**: Remove deprecated `ClusterClient.sendMessage()` - use `send(data, key)` instead
- **core**: Remove legacy constructor from `QueryOptimizer` - use options object
- **core**: Remove legacy array format from `CRDTDebugger.importHistory()` - use v1.0 format

### Added

#### Observability & Debugging
- **CRDTDebugger** - Full CRDT operation recording, replay, conflict detection, and export (JSON/CSV/NDJSON)
- **SearchDebugger** - BM25 TF-IDF breakdown, RRF rank contributions, timing analysis
- **PrometheusExporter** - Comprehensive metrics: connections, operations, CRDT merges, sync, cluster, storage, queries
- **Grafana Dashboard** - Pre-built dashboard JSON for TopGun monitoring

#### Admin Dashboard
- **SetupWizard** - 6-step guided setup for new deployments
- **DataExplorer** - Browse and edit map data with JSON editor
- **QueryPlayground** - Monaco editor with Cmd+Enter execution
- **ClusterTopology** - Visual cluster ring and partition distribution
- **CommandPalette** - Cmd+K quick navigation
- **Settings** - Hot-reloadable runtime configuration
- **Login/Auth** - JWT-based admin authentication

#### Distributed Search & Subscriptions
- **ClusterSearchCoordinator** - Scatter-gather distributed FTS with RRF merge
- **DistributedSubscriptionCoordinator** - Live query subscriptions across cluster nodes
- **QueryCursor** - Cursor-based pagination for distributed queries
- **SearchCursor** - Cursor-based pagination for distributed search
- **CLUSTER_SEARCH_REQ/RESP** - New cluster protocol messages
- **CLUSTER_SUB_REGISTER/UPDATE** - Distributed subscription protocol

#### CLI & Developer Experience
- **CLI Commands** - `topgun doctor`, `setup`, `dev`, `test`, `config`, `cluster`, `debug:crdt`, `debug:search`
- **BetterSqlite3Adapter** - SQLite storage adapter for server-side persistence
- **DevContainer** - VS Code/Codespaces development environment
- **Docker Compose Profiles** - `admin`, `monitoring`, `dbtools` profiles

#### Infrastructure
- **Environment Validation** - Zod schema for server configuration
- **TOPGUN_DEBUG_ENDPOINTS** - Security control for debug endpoints (disabled by default)
- **foreignKeyMap** - Custom database schema support for better-auth adapter

### Changed

- **server**: Refactor DistributedSubscriptionCoordinator into focused coordinators (Base, Search, Query)
- **server**: Wire SearchCoordinator.dispose() into LifecycleManager for timer cleanup
- **server**: Extract handlers from ServerCoordinator into domain modules
- **core**: Split schemas.ts (1160 lines) into domain-focused modules
- **core**: Structured logging with pino throughout codebase

### Removed

- Dead code from ServerCoordinator (~1,263 lines)
- Phase/Spec/Bug references from code comments (488 occurrences)
- `jest.retryTimes` from flaky tests (proper fixes applied)

### Fixed

- Timer cleanup in SearchCoordinator preventing memory leaks
- Type safety improvements across client message handlers
- WebCrypto polyfill for consistent test environment

### Tests

- CLI command tests (28 tests covering all commands)
- 1814 tests in core package
- Cluster E2E tests for partition routing, node failure, replication

## [0.9.0] - 2026-01-06

### Added

#### MCP Server Package
- **@topgunbuild/mcp-server** - New package enabling AI assistants (Claude Desktop, Cursor) to interact with TopGun databases via Model Context Protocol (MCP)
- Query, mutate, and search data using natural language
- Full-text hybrid search (BM25 + exact matching)
- Real-time subscriptions for watching changes
- Schema inspection and query explanation
- HTTP and stdio transports
- Security controls: restricted maps, read-only mode, auth tokens

#### Documentation
- Comprehensive MCP Server guide

## [0.8.1] - 2026-01-05

### Added

#### Hybrid Search with RRF Fusion (Phase 12)
- **Reciprocal Rank Fusion (RRF)** for merging search results from multiple sources
- **QueryExecutor** with parallel FTS + filter execution
- **LiveFTSIndex** for live hybrid query updates
- **HybridQueryHandle** for client-side hybrid queries
- **useHybridQuery** React hook for faceted search UIs
- **_score sorting** for relevance-based results

#### New Predicates
- FTS: `match()`, `matchPhrase()`, `matchPrefix()`
- Filters: `eq()`, `gt()`, `lt()`, `gte()`, `lte()`, `contains()`
- Logical: `and()`, `or()`

## [0.8.0] - 2026-01-04

### Added

#### Full-Text Search (Phase 11)
- **BM25 Ranked Search** - Industry-standard relevance algorithm with Porter stemming and 174 English stopwords
- **Server-side FTS Indexes** - Centralized indexes with automatic updates on data changes
- **Live Search Subscriptions** - Real-time search results with delta updates (ENTER/UPDATE/LEAVE events)
- **useSearch React Hook** - Easy integration with debounce support for search-as-you-type UIs
- **O(1) Live Updates** - scoreSingleDocument optimization for 1000x faster live search updates
- **Notification Batching** - 16ms batching window to prevent update storms

### Performance
- scoreDocument (1K docs): 1000× faster
- scoreDocument (10K docs): 10000× faster

## [0.7.0] - 2026-01-01

### Added

#### Cluster Replication (Phase 10)
- **ReplicationPipeline** - Async and sync replication with configurable consistency levels (EVENTUAL, QUORUM, STRONG)
- **Gossip Protocol** - Automatic cluster member discovery
- **Partition-aware routing** - Direct operations to partition owners

#### Anti-Entropy Repair
- **MerkleTreeManager** - Efficient hash-based data verification using Merkle trees
- **RepairScheduler** - Automated periodic repair with configurable scan intervals
- **Bucket-level comparison** - Minimizes network traffic

#### Failover Handling
- **PartitionReassigner** - Automatic partition rebalancing on node failure
- **Phi Accrual Failure Detector** - Adaptive failure detection
- **ReadReplicaHandler** - Support for reading from replica nodes

#### Documentation
- New "Cluster Replication" guide

### Tests
- 38 E2E cluster tests covering replication, failover, and partition routing

## [0.6.0] - 2025-12-31

### Added

#### Lazy Index Building (Phase 9.01)
- `LazyHashIndex`, `LazyNavigableIndex`, `LazyInvertedIndex` classes
- Deferred index construction until first query for fast startup
- Progress callbacks for index materialization
- `lazyIndexBuilding` option in `IndexedLWWMap`

#### Attribute Factory (Phase 9.02)
- `generateAttributes<V>()` with curried type inference
- `attr()` and `multiAttr()` helper functions
- `createSchema<V>()` fluent builder
- Nested path support (e.g., `'address.city'`)

#### Compound Index Auto-Detection (Phase 9.03)
- `CompoundIndex` class for multi-attribute O(1) lookups
- Compound query tracking in `QueryPatternTracker`
- Compound index suggestions in `IndexAdvisor`
- Automatic `CompoundIndex` usage in `QueryOptimizer`

### Tests
- 140+ new tests for Phase 9 features
- 1264 total tests in core package

## [0.5.0] - 2025-12-30

### Added

#### Query Engine (Phase 7)
- **HashIndex** - O(1) equality lookups
- **NavigableIndex** - O(log N) range queries with SkipList-based SortedMap
- **QueryOptimizer** - Cost-based query planning with automatic index selection
- **StandingQueryIndex** - Pre-computed results for live queries
- **ResultSet Pipeline** - Lazy, composable query results

#### Full-Text Search Foundation (Phase 8.01)
- **InvertedIndex** - Token-based indexing for text search
- New query predicates: `contains`, `containsAll`, `containsAny`
- Configurable tokenization with filters (lowercase, stop words, min length)
- Multi-value attribute support

#### Adaptive Indexing (Phase 8.02)
- **QueryPatternTracker** - Runtime statistics collection
- **IndexAdvisor** - Intelligent index suggestions based on query patterns
- **AutoIndexManager** - Optional automatic index creation
- **DefaultIndexingStrategy** - Auto-index scalar fields

#### New Classes
- `IndexedLWWMap` - LWWMap with index support
- `IndexedORMap` - ORMap with index support

### Performance
- Equality queries: **100-1000× faster** than full scan
- Range queries: **O(log N)** instead of O(N)
- Text search: Sub-millisecond on 100K+ documents

## [0.4.0] - 2025-12-28

### Added

#### New CRDTs and Data Structures
- **PN Counter** (Phase 5.2) - Positive-Negative counter CRDT supporting increment/decrement operations with eventual consistency
- **Event Journal** (Phase 5.04) - Append-only event log for event sourcing patterns with full CRDT synchronization
- **Entry Processor** (Phase 5.03) - Atomic server-side map operations with sandboxed execution via isolated-vm

#### API Enhancements
- **Custom Conflict Resolvers** (Phase 5.05) - Register custom conflict resolution strategies for domain-specific merge logic
- **Delta Updates for Subscriptions** (Phase 5.1) - Incremental updates instead of full data snapshots for improved performance
- **Offline Persistence for PNCounter** (Phase 5.02) - Full offline support for counter operations

#### React Hooks
- `usePNCounter` - Hook for working with PN Counter in React applications
- `useEventJournal` - Hook for subscribing to Event Journal streams
- Enhanced `useQuery` with delta update support

#### Documentation
- PNCounter guide and API reference
- Entry Processor guide and API reference
- Event Journal guide and API reference
- Conflict resolution blog post with ML/AI integration examples

### Fixed
- **react**: Added `maxChanges` option to prevent memory leaks in change tracking
- **server**: Mark isolated-vm as external for bundling
- **server**: Add production warning when isolated-vm is unavailable

### Changed
- **client**: Replace console.log with logger in QueryHandle for better debugging

## [0.3.0] - 2025-12-25

### Added
- Phase 4 completion: Cluster support with split-brain protection
- FailureDetector and FencingManager for cluster resilience
- Cluster Client documentation

### Fixed
- Test alignment with current implementation

## [0.2.1] - 2025-12-24

### Fixed
- Minor bug fixes and stability improvements

## [0.2.0] - 2025-12-23

### Added
- Phase 0 complete: Core CRDT implementation
- LWWMap and ORMap with HLC timestamps
- MerkleTree for efficient delta sync
- Basic client-server synchronization
