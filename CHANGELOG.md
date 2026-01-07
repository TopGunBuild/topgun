# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
