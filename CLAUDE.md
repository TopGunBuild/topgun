# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

TopGun is a hybrid offline-first in-memory data grid. It provides zero-latency reads/writes via local CRDTs, real-time sync via WebSockets, and durable storage on PostgreSQL.

**Key Design Principles:**
- Local-first: Data lives in memory, reads/writes never wait for network
- CRDT conflict resolution using LWW-Map and OR-Map with Hybrid Logical Clocks (HLC)
- Merkle tree synchronization for efficient delta sync
- Server-authoritative cluster architecture with client offline capability

## Common Commands

```bash
# Install dependencies
pnpm install

# Build all packages
pnpm build

# Run all tests
pnpm test

# Run tests for a specific package
pnpm --filter @topgunbuild/core test
pnpm --filter @topgunbuild/client test
pnpm --filter @topgunbuild/server test

# Run a single test file (from package directory)
cd packages/server && pnpm test -- --testPathPattern="Cluster"

# Test coverage
pnpm test:coverage
pnpm --filter @topgunbuild/core test:coverage

# E2E tests
pnpm test:e2e

# k6 load tests (requires k6 installed)
pnpm test:k6:smoke
pnpm test:k6:throughput
pnpm test:k6:write
pnpm test:k6:connections

# CRDT micro-benchmarks
pnpm --filter @topgunbuild/core bench

# Start development server
pnpm start:server

# Start documentation site
pnpm start:docs
```

## Architecture

### Package Hierarchy

```
@topgunbuild/core (no internal deps)
    ↓
@topgunbuild/client, @topgunbuild/server (depend on core)
    ↓
@topgunbuild/adapters, @topgunbuild/react (depend on client)
```

### Packages

| Package | Purpose |
|---------|---------|
| `core` | CRDTs (LWWMap, ORMap), Hybrid Logical Clock, MerkleTree, message schemas (Zod), serialization (msgpackr) |
| `client` | Browser/Node SDK: TopGunClient, SyncEngine, QueryHandle, storage adapters interface |
| `server` | WebSocket server: ServerCoordinator, clustering (ClusterManager, PartitionService), WorkerPool, PostgreSQL adapter |
| `react` | React bindings: TopGunProvider, useQuery, useMap, useORMap, useMutation, useTopic hooks |
| `adapters` | Storage implementations: IDBAdapter (IndexedDB for browsers) |
| `native` | Native xxHash64 hashing for Node.js performance (optional, JS fallback available) |
| `adapter-better-auth` | BetterAuth integration |

### Key Abstractions

**Core:**
- `HLC` (Hybrid Logical Clock) - Global causality tracking: `{millis, counter, nodeId}`
- `LWWMap` - Last-Write-Wins Map, conflict resolution by highest timestamp
- `ORMap` - Observed-Remove Map, supports concurrent additions with unique tags
- `MerkleTree` - Efficient delta sync by comparing hashes

**Client:**
- `TopGunClient` - Main entry point, manages maps/queries/topics
- `SyncEngine` - Orchestrates synchronization, handles WebSocket connection and state machine
- `IStorageAdapter` - Interface for local persistence (IndexedDB, etc.)

**Server:**
- `ServerFactory` - Assembles server from domain modules via dependency injection
- `modules/` - Domain-specific factories (see `packages/server/src/modules/`):
  - `core-module` - HLC, nodeId, base configuration
  - `workers-module` - WorkerPool for CPU-intensive ops
  - `cluster-module` - ClusterManager, PartitionService (271 partitions)
  - `storage-module` - PostgreSQL adapter initialization
  - `network-module` - HTTP/WSS servers with deferred startup
  - `handlers-module` - 26 message handlers grouped by domain (CRDT, Sync, Query, Messaging, Coordination, Search, Persistence, Client, Server)
  - `lifecycle-module` - LifecycleManager with graceful shutdown hooks
- `ServerCoordinator` - Main entry point, routes requests, manages connections

### Data Flow

1. Client writes locally to LWWMap + OpLog (IndexedDB)
2. UI updates immediately
3. SyncEngine batches and sends to server when online
4. Server merges using HLC timestamps, broadcasts to subscribers
5. Clients reconnecting use MerkleTree for efficient delta sync

## Build System

- Package manager: pnpm 10.13.1 (monorepo with workspaces)
- Build tool: tsup (outputs CJS, ESM, and type declarations)
- Test runner: Jest with ts-jest
- Benchmarks: Vitest (bench mode)

## Commit Message Format

```
type(scope): description

Types: feat, fix, docs, test, chore, refactor, perf
Examples:
  feat(core): add new CRDT merge strategy
  fix(client): resolve sync race condition
  test(server): add cluster integration tests
```

## Code Comments Convention

- **Do NOT** add phase/spec/bug references in code comments (e.g., `// Phase 8.02`, `// BUG-06`, `// SPEC-011`)
- Such references belong in **commit messages**, not in code
- Instead, write WHY-comments explaining the reason for the code:
  ```typescript
  // BAD: // Merge defaults (Phase 3 BUG-06)
  // GOOD: // Merge defaults to prevent race condition when topics subscribe before connection
  ```

## Test Notes

- Server tests use ports 10000+ for servers, 11000+ for cluster nodes
- Run tests sequentially in CI to avoid port conflicts: `pnpm test -- --runInBand`
- Cluster tests include timing delays for node synchronization - don't interrupt early
