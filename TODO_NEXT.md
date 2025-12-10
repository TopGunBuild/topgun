# Roadmap: TopGun V2 (Offline-First IMDG)

Based on the audit of the current codebase (`packages/`) and comparison with the reference architecture (`hazelcast` & `specifications`), here is the prioritized plan for the next phase of development.

## Phase 1: Critical Integrity & Features (Immediate)

## Phase 3: Documentation & Developer Experience

### 9. Platform Overview & Whitepaper [NEW]
**Problem:** The project has detailed specs but lacks a high-level overview explaining "Why TopGun?" and architecture decisions (CRDTs, HLC, Offline-First) for new users.
**Task:**
- Create a `WHITEPAPER.md` or updated `README.md` summarizing the architecture.
- Create diagrams (Mermaid/SVG) showing the Data Flow, Sync Protocol, and Cluster Topology.

### 10. Documentation Website [COMPLETED]
**Problem:** Documentation is scattered across markdown files.
**Task:**
- [x] Setup a documentation site (Vite + React).
- [x] Implement Diátaxis structure (Intro, Guides, API, Concepts).
- [x] Publish API Reference, Guides (Getting Started, Auth, React Integration), and Deployment Best Practices.
- [x] Create a "TopGun vs X" comparison page.
- [x] Write Authentication Guide.
- [x] Implement "Quick Start" tutorial with verified code.

### 11. Documentation Expansion (Phase 2) [COMPLETED]
**Problem:** Only ~30% of features are documented. Critical gaps in Data Structures, Operations, and Security.
**Task:**
- [x] **Core Features:** Document OR-Map, TTL, Pub/Sub.
- [x] **Operations:** Write "Deploying Server" (Docker/K8s) and "Observability" guides.
- [x] **Security:** Document RBAC and Permissions.
- [x] **Reference:** Auto-generate or fill API Reference.

### 12. Basic Admin Dashboard (Web UI) [COMPLETED]
**Problem:** Developers have no visual way to inspect data or cluster state, making debugging difficult.
**Task:**
- [x] Create a lightweight React app (`apps/admin-dashboard`) that connects as an admin client.
- [x] Implement JWT authentication with session restore.
- [x] **Dashboard:** Real-time cluster stats (Ops, Memory, Connected Clients).
- [x] **Map Browser:** JSON tree view with search/filter by keys.
- [x] **Cluster Details:** Node list with status, metrics, and "YOU" indicator.
- [x] **UI States:** Loading spinners, empty states, error boundaries.
- [x] **TypeScript:** Full type safety with system types.

## Phase 5: Advanced AI Capabilities (Research)

### 12. Vector Embeddings & Similarity Search [FUTURE]
**Goal:** Enable Retrieval-Augmented Generation (RAG) directly from TopGun.
**Concept:** Store float32 arrays (vectors) in `LWWRecord`.
**Task:**
- Implement distance metrics (Cosine Similarity, Euclidean) in `Matcher.ts`.
- (Research) Implement HNSW index for efficient k-NN search in-memory.
- Allow queries like `sort: { vectorField: { near: [0.1, 0.5, ...], metric: 'cosine' } }`.

## Phase 4: Completed / Verified

- [x] **Basic Admin Dashboard:** Implemented in `apps/admin-dashboard` with Dashboard, Map Browser, Cluster Details, JWT auth, and full TypeScript types.
- [x] **Chaos Testing Expansion:** Implemented "Flaky Connection" and "Slow Consumer" scenarios in `Chaos.test.ts`.
- [x] **Cloud-Native Deployment Kit:** Created Dockerfile, Helm Chart, and `KubernetesDiscoveryStrategy`.
- [x] **Observability & Metrics:** Implemented `/metrics` endpoint (Prometheus) and `MetricsService`.
- [x] **Distributed Messaging (Pub/Sub):** Implemented `TopicManager` in server and client SDK.
- [x] **Data Expiration (TTL):** Implemented support for `ttlMs` in `LWWRecord`/`ORMapRecord` and `GarbageCollector`.
- [x] **Distributed Coordination Primitives:** Implemented `DistributedLock` (Fenced Lock) with `LockManager` in server and client SDK.
- [x] **Distributed Tombstone Garbage Collection:** Implemented consensus-based GC in `GarbageCollector.ts` and `ServerCoordinator.ts`.
- [x] **Server-Side Hooks & Middleware (Enrichment):** Implemented `IInterceptor` pipeline in `ServerCoordinator.ts`.
- [x] **Better Auth Integration:** implemented in `@topgunbuild/adapter-better-auth` (Phase 0).
- [x] **Server-Side OR-Map Support:** Implemented in `ServerCoordinator.ts` (supports `OR_ADD`/`OR_REMOVE` and storage persistence).
- [x] **Field-Level Permissions (RBAC):** Implemented in `SecurityManager.ts` (`filterObject` with whitelist support).
- [x] **Query Indexing:** implemented in `QueryRegistry` (Reverse Index).
- [x] **OR-Map Data Structure:** implemented in `ORMap.ts` (Client-side logic).
- [x] **Basic Sync Protocol:** implemented (Batch Ops, Merkle Trees for LWW).
- [x] **Clustering:** implemented in `ClusterManager`.
- [x] **React SDK:** implemented in `packages/react` (Hooks & Provider).
- [x] **React SDK Tests:** Unit tests for `useQuery` and `useMutation`.
