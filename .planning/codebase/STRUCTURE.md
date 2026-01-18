# Codebase Structure

**Analysis Date:** 2026-01-18

## Directory Layout

```
topgun/
├── packages/              # Monorepo packages (core code)
│   ├── core/              # CRDT primitives, HLC, schemas, query engine
│   ├── client/            # Browser/Node SDK
│   ├── server/            # WebSocket server, clustering
│   ├── react/             # React hooks and provider
│   ├── adapters/          # Storage implementations (IndexedDB)
│   ├── native/            # Native bindings (xxHash64)
│   ├── adapter-better-auth/  # BetterAuth integration
│   └── mcp-server/        # Model Context Protocol server
├── bin/                   # CLI entry points
│   ├── topgun.js          # Main CLI binary
│   └── commands/          # CLI command implementations
├── tests/                 # Integration and E2E tests
│   ├── e2e/               # End-to-end tests
│   ├── k6/                # Load testing with k6
│   └── cli/               # CLI tests
├── examples/              # Example applications
│   ├── notes-app/         # React notes application
│   ├── todo-app/          # React todo application
│   ├── push-worker/       # Cloudflare push worker
│   └── storage-worker/    # Cloudflare storage worker
├── deploy/                # Deployment configurations
│   ├── k8s/               # Kubernetes Helm chart
│   └── grafana/           # Grafana dashboards
├── scripts/               # Build and utility scripts
├── apps/                  # Documentation apps
├── specifications/        # Technical specifications
└── .planning/             # GSD planning documents
```

## Directory Purposes

**packages/core/:**
- Purpose: Zero-dependency foundation layer
- Contains: CRDTs, HLC, MerkleTree, schemas, predicate engine, query engine, FTS
- Key files:
  - `src/HLC.ts` - Hybrid Logical Clock implementation
  - `src/LWWMap.ts` - Last-Write-Wins Map CRDT
  - `src/ORMap.ts` - Observed-Remove Map CRDT
  - `src/MerkleTree.ts` - Delta sync tree structure
  - `src/schemas.ts` - Zod message schemas
  - `src/predicate.ts` - Query predicate evaluation
  - `src/query/` - Query engine (indexes, optimizer, cursors)
  - `src/fts/` - Full-text search (BM25, tokenizer, inverted index)

**packages/client/:**
- Purpose: Application SDK for browser and Node.js
- Contains: Client API, sync engine, query handles, storage adapters
- Key files:
  - `src/TopGunClient.ts` - Main client class
  - `src/SyncEngine.ts` - WebSocket sync orchestration
  - `src/QueryHandle.ts` - Live query subscription
  - `src/SyncState.ts` - Connection state enum
  - `src/SyncStateMachine.ts` - State transitions
  - `src/IStorageAdapter.ts` - Storage interface
  - `src/adapters/IDBAdapter.ts` - IndexedDB implementation
  - `src/cluster/` - Cluster client (connection pool, partition router)

**packages/server/:**
- Purpose: Server-side WebSocket coordination
- Contains: Server coordinator, cluster management, handlers, search
- Key files:
  - `src/ServerCoordinator.ts` - Main server class (~2500 lines)
  - `src/cluster/ClusterManager.ts` - Peer-to-peer mesh
  - `src/cluster/PartitionService.ts` - Consistent hashing
  - `src/cluster/ReplicationPipeline.ts` - Data replication
  - `src/cluster/MigrationManager.ts` - Partition migration
  - `src/handlers/` - Message handlers (Counter, EntryProcessor, ConflictResolver)
  - `src/workers/` - Thread pool (CRDT merge, Merkle, serialization)
  - `src/search/` - Search coordination
  - `src/query/` - Query registry and matcher
  - `src/storage/` - Server storage interfaces

**packages/react/:**
- Purpose: React integration layer
- Contains: Context provider and hooks
- Key files:
  - `src/TopGunProvider.tsx` - React context provider
  - `src/hooks/useQuery.ts` - Query subscription hook
  - `src/hooks/useMap.ts` - LWWMap hook
  - `src/hooks/useORMap.ts` - ORMap hook
  - `src/hooks/useMutation.ts` - Write operations hook
  - `src/hooks/useTopic.ts` - Pub/sub hook
  - `src/hooks/useSearch.ts` - FTS hook
  - `src/hooks/useHybridQuery.ts` - Combined FTS + filter hook

**packages/adapters/:**
- Purpose: Pluggable storage backends
- Contains: IndexedDB adapter
- Key files:
  - `src/IDBAdapter.ts` - IndexedDB implementation for browsers

**packages/native/:**
- Purpose: Native performance optimizations
- Contains: xxHash64 native binding with JS fallback
- Key files:
  - `src/hash.ts` - Hash function with native detection

**bin/:**
- Purpose: CLI tools
- Contains: Server management, Docker orchestration
- Key files:
  - `topgun.js` - Main CLI entry
  - `commands/` - Subcommands (cluster, debug)

**tests/:**
- Purpose: Integration and load testing
- Contains: E2E tests, k6 load tests
- Key files:
  - `e2e/` - Jest E2E tests
  - `k6/scenarios/` - Load test scripts
  - `k6/docker-compose.cluster.yml` - Cluster test setup

## Key File Locations

**Entry Points:**
- `packages/client/src/index.ts`: Client SDK exports
- `packages/server/src/index.ts`: Server exports
- `packages/core/src/index.ts`: Core primitives exports
- `packages/react/src/index.ts`: React bindings exports
- `bin/topgun.js`: CLI entry point

**Configuration:**
- `package.json`: Root monorepo config
- `pnpm-workspace.yaml`: Workspace definition
- `tsconfig.json`: TypeScript base config
- `jest.config.js`: Test runner config
- `docker-compose.yml`: Local development stack
- `.env.example`: Environment variables template

**Core Logic:**
- `packages/core/src/HLC.ts`: Timestamp generation
- `packages/core/src/LWWMap.ts`: Primary CRDT
- `packages/client/src/SyncEngine.ts`: Sync orchestration
- `packages/server/src/ServerCoordinator.ts`: Server logic

**Testing:**
- `packages/*/src/__tests__/`: Unit tests (co-located)
- `tests/e2e/`: End-to-end integration tests
- `tests/k6/`: Performance load tests

## Naming Conventions

**Files:**
- PascalCase for classes: `LWWMap.ts`, `SyncEngine.ts`, `TopGunClient.ts`
- camelCase for utilities: `logger.ts`, `hash.ts`, `base64url.ts`
- kebab-case for configs: `jest.config.js`, `docker-compose.yml`
- Test files: `*.test.ts` co-located in `__tests__/` directories

**Directories:**
- lowercase for feature areas: `cluster/`, `handlers/`, `hooks/`
- `__tests__/` for test directories
- `__benchmarks__/` for performance benchmarks

**Exports:**
- Named exports preferred over default exports
- Index files (`index.ts`) barrel-export public API
- Types exported alongside implementations

## Where to Add New Code

**New CRDT Type:**
- Implementation: `packages/core/src/NewCRDT.ts`
- Tests: `packages/core/src/__tests__/NewCRDT.test.ts`
- Export: Add to `packages/core/src/index.ts`

**New Client Feature:**
- Implementation: `packages/client/src/NewFeature.ts`
- Tests: `packages/client/src/__tests__/NewFeature.test.ts`
- Export: Add to `packages/client/src/index.ts`
- If handle class: `packages/client/src/NewFeatureHandle.ts`

**New Server Handler:**
- Implementation: `packages/server/src/handlers/NewHandler.ts`
- Tests: `packages/server/src/__tests__/NewHandler.test.ts`
- Registration: Add to `ServerCoordinator.ts` constructor

**New React Hook:**
- Implementation: `packages/react/src/hooks/useNewFeature.ts`
- Tests: `packages/react/src/__tests__/useNewFeature.test.tsx`
- Export: Add to `packages/react/src/index.ts`

**New Storage Adapter:**
- Implementation: `packages/adapters/src/NewAdapter.ts`
- Interface: Implement `IStorageAdapter` from `packages/client/src/IStorageAdapter.ts`
- Tests: `packages/adapters/src/__tests__/NewAdapter.test.ts`

**Utilities:**
- Core utils: `packages/core/src/utils/`
- Client utils: `packages/client/src/utils/`
- Server utils: `packages/server/src/utils/`

**New Cluster Feature:**
- Implementation: `packages/server/src/cluster/NewFeature.ts`
- Export: Add to `packages/server/src/cluster/index.ts`

## Special Directories

**node_modules/:**
- Purpose: pnpm workspace dependencies
- Generated: Yes (pnpm install)
- Committed: No (gitignored)

**dist/:**
- Purpose: Compiled output (CJS, ESM, types)
- Generated: Yes (pnpm build via tsup)
- Committed: No (gitignored)

**coverage/:**
- Purpose: Test coverage reports
- Generated: Yes (pnpm test:coverage)
- Committed: No (gitignored)

**.planning/:**
- Purpose: GSD planning documents
- Generated: Manual/GSD commands
- Committed: Varies by project

**specifications/:**
- Purpose: Technical design documents
- Generated: Manual
- Committed: Yes

**data/:**
- Purpose: Local development data (SQLite, etc.)
- Generated: Runtime
- Committed: No (gitignored)

**profiles/:**
- Purpose: Performance profiling output
- Generated: pnpm profile:* commands
- Committed: No (gitignored)

---

*Structure analysis: 2026-01-18*
