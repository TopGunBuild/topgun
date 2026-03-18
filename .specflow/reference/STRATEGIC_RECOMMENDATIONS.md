# TopGun — Strategic Recommendations

> **Purpose:** This document contains strategic analysis, architectural recommendations, and actionable TODO proposals. It is intended for the task-organizing agent to update the roadmap (`TODO.md`) and plan future work.
>
> **Context:** Based on audit of `PRODUCT_CAPABILITIES.md` by Gemini and Claude, review of current TODO list (28 items across 3 milestones), and analysis of current project state (SPEC-068 active, 6/7 domain services done, 468+ Rust tests).
>
> **Date:** 2026-02-28

---

## 1. Current TODO List — Assessment & Corrections

### What's Good

The TODO.md is well-structured: milestone-driven, dependency-tracked, wave-ordered. The Triple Reference Protocol and research-before-design philosophy have produced high-quality architecture. 67 completed specs, 900+ tests, zero failures — the execution quality is excellent.

### Issues to Fix

**1.1. STATE.md and TODO.md are out of sync.**
TODO.md says "Wave 5c — TODO-089 (PersistenceService) next. 5 of 7 domain services done." But SPEC-066 (TODO-089) and SPEC-067 (TODO-090) are both COMPLETED. The actual state is:
- 6/7 domain services done
- SPEC-068 (TODO-071, Search) is audited and ready for implementation
- TODO-090 (PostgreSQL) is DONE

**Action:** Update TODO.md execution order to reflect:
```
Current position: Wave 5d — TODO-071 (Search/SPEC-068) ready for implementation.
6 of 7 domain services done. PostgreSQL adapter complete.
v1.0 critical path: 071 (Search, ready) → 074/075 (bug fixes) → 093 (Admin) / 068 (Integration tests)
```

**1.2. TODO-089 label is confusing.**
In the "Phase 3b" section, TODO-089 is described as "PersistenceService (Counters + Entry Processing)" with P2 priority. But in the completed items log, SPEC-066 (PersistenceService) is marked done for TODO-089. These are seemingly the same TODO but described differently.

**Action:** Clarify whether TODO-089 is fully done or if the "Counters + Entry Processing" scope is a remaining subset.

**1.3. Missing TODOs identified by this audit (see sections below).**
The following new tasks should be added to the roadmap. Each is explained in detail in the relevant section.

---

## 2. Repository Structure for Dual Licensing (Open Core)

### Current Structure

```
topgun/
├── packages/
│   ├── core-rust/       (topgun-core)
│   ├── server-rust/     (topgun-server)
│   ├── client/          (TS SDK)
│   ├── react/           (React hooks)
│   ├── adapters/        (IndexedDB)
│   └── ...
├── LICENSE              (BSL 1.1 — applies to everything)
└── Cargo.toml           (workspace)
```

### Recommended Structure

Separate the workspace into `core` (Apache 2.0) and `enterprise` (BSL 1.1). The boundary is clear: v1.0 + v2.0 features are open, v3.0 features are enterprise.

```
topgun/
├── LICENSE                    # Apache 2.0 (default for everything)
├── Cargo.toml                 # workspace members: packages/* + enterprise/*
├── packages/                  # ALL open-source
│   ├── core-rust/
│   ├── server-rust/
│   ├── client/
│   ├── react/
│   ├── adapters/
│   └── ...
├── enterprise/                # BSL 1.1 (separate LICENSE file inside)
│   ├── LICENSE                # BSL 1.1
│   ├── multi-tenancy/         # TODO-041
│   ├── tiered-storage/        # TODO-040 + TODO-043
│   ├── vector-search/         # TODO-039
│   └── bi-temporal/           # TODO-044
└── apps/
    └── admin-dashboard/       # Apache 2.0 (marketing/adoption tool)
```

### Key Decisions

1. **`enterprise/` is a separate directory with its own LICENSE file.** This is the GitLab/Supabase model. Each enterprise crate depends on `topgun-server` via path dependency but lives in a different license zone.

2. **Feature gates, not binary separation.** Enterprise crates should implement traits defined in `topgun-server` (e.g., `MapDataStore` for S3, `StoragePolicy` for tiered). The server binary can be built with or without enterprise features:
   ```toml
   # topgun-server Cargo.toml
   [features]
   enterprise = ["topgun-multi-tenancy", "topgun-tiered-storage"]
   ```

3. **Admin Dashboard stays Apache 2.0.** It's a marketing/adoption tool — restricting it hurts growth. Enterprise features in the dashboard (tenant admin, tiered monitor) can be feature-flagged at the React level.

4. **Don't restructure now.** The physical separation only matters when enterprise crates exist (v3.0). For now, just change the LICENSE file from BSL to Apache 2.0 and plan the directory structure.

### Proposed TODO

```
TODO-094: Change LICENSE to Apache 2.0
- Priority: P1 (blocks community adoption)
- Complexity: Trivial
- Milestone: v1.0
- Scope: Replace LICENSE file content with Apache 2.0 text. Update Cargo.toml [workspace.package] license field. Add NOTICE file.
- Effort: 30 minutes

TODO-095: Enterprise Directory Structure
- Priority: P3 (not needed until v3.0)
- Complexity: Low
- Milestone: v3.0 (pre-work)
- Scope: Create enterprise/ directory with BSL LICENSE. Move TODO-041/040/043/039/044 crates there when implemented.
- Effort: 1 day (when v3.0 work starts)
```

---

## 3. PostgreSQL as Temporary Storage — Architectural Implications

### Current State

`PostgresDataStore` (SPEC-067) implements `MapDataStore` trait with write-through to PostgreSQL via sqlx. Feature-gated behind `postgres`. This is correct architecture — `MapDataStore` is the abstraction boundary.

### The Path Forward

The existing trait hierarchy already accounts for this:

```
StorageEngine (L1: in-memory)
  └── RecordStore (L2: metadata, TTL, eviction)
        └── MapDataStore (L3: persistence backend)
              ├── NullDataStore (no persistence)
              ├── PostgresDataStore (v1.0 — write-through)
              ├── S3DataStore (v3.0 — cold storage, TODO-043)
              └── TieredDataStore (v3.0 — hot/warm/cold, TODO-040)
```

### No Additional TODOs Needed

The storage abstraction is well-designed. TODO-043 (S3) and TODO-040 (Tiered) already exist with correct dependencies. The `MapDataStore` trait boundary ensures PostgreSQL can be replaced or augmented without touching `RecordStore` or `StorageEngine`. The research document `RUST_STORAGE_ARCHITECTURE.md` already covers this migration path.

**One minor recommendation:** When implementing S3 storage (TODO-043), consider `opendal` crate instead of raw `aws-sdk-s3`. OpenDAL provides a unified `Operator` trait that works with S3, GCS, Azure, R2, MinIO — all with the same API. This is already noted in TODO-043 context but worth emphasizing: it means "S3 storage" actually becomes "any object storage" for free.

---

## 4. Adoption Path — Overcoming "Replace Everything" Perception

### The Problem

PRODUCT_CAPABILITIES.md positions TopGun as replacing the entire stack: sync engine + database + compute layer + search engine + admin panel. This scares away potential adopters who already have a working stack.

### Direct Answers to the Three Questions

**Q: Can TopGun be used for just one feature while keeping Postgres for everything else?**

**Yes — and this should be the primary adoption story.** TopGun doesn't replace your existing database. It adds a real-time reactive layer on top. Your Postgres stays as your system of record. TopGun handles the subset of data that needs:
- Real-time sync across clients
- Offline capability
- Sub-millisecond reads
- Live query subscriptions

Example: A project management app keeps all data in Postgres. But the Kanban board state (card positions, assignments, comments) flows through TopGun for instant collaborative updates. The rest of the app (user management, billing, reports) stays on the existing Postgres + API stack.

**Q: Can TopGun be used as a caching layer in front of an existing DB?**

**Yes — this is literally what Hazelcast does.** The `MapDataStore` trait enables this exact pattern:
- `PostgresDataStore` can load data from an existing Postgres database on startup
- Reads come from in-memory (0ms)
- Writes go through to Postgres (write-through)
- TopGun adds CRDT sync, live queries, and offline support on top

This requires no schema migration. Just point `PostgresDataStore` at existing tables. The "cache + sync" use case is a natural entry point.

**Q: Is there a CDC connector from Postgres → TopGun for gradual migration?**

**Not in v1.0, but designed for v2.0.** TODO-092 (Connector Framework) includes "PostgreSQL CDC source" in scope. This would enable:
1. Postgres changes stream into TopGun maps via logical replication
2. TopGun serves real-time subscriptions to clients
3. Gradual migration: more data moves to TopGun-primary over time

### Adoption Path Strategy (3 tiers)

| Tier | Use Case | What Customer Keeps | What TopGun Adds |
|------|----------|--------------------|-----------------|
| **Tier 1: Real-Time Layer** | Add collaborative features to existing app | Existing DB, API, auth | Real-time sync, offline, live queries for specific features |
| **Tier 2: Cache + Sync** | Accelerate reads, add offline | Existing DB (Postgres) | In-memory cache with automatic sync, CRDT conflict resolution |
| **Tier 3: Full Platform** | Greenfield or platform replacement | Nothing (TopGun is primary) | Everything: storage, compute, sync, search, streaming |

### Impact on PRODUCT_CAPABILITIES.md

The document currently only describes Tier 3. Add a section "Getting Started" or "Adoption Path" that leads with Tier 1. The competitive comparison table should include a row for "Works alongside existing DB" — TopGun should show "Yes" here.

### Impact on TODO List

```
TODO-096: Adoption Path Documentation + Tier 1 Example
- Priority: P1 (critical for first users)
- Complexity: Medium
- Milestone: v1.0
- Scope:
  - Add "Adoption Path" section to PRODUCT_CAPABILITIES.md (3-tier model)
  - Create example app: existing Express+Postgres app + TopGun for one collaborative feature
  - Document how to use PostgresDataStore with existing tables (no migration required)
  - Show pattern: TopGun for real-time subset, REST API for everything else
- Effort: 1 week
```

---

## 5. Security Model

### Current State

No security section in PRODUCT_CAPABILITIES.md. JWT auth exists for Admin Dashboard. No data-level authorization.

### The Gap

"Client as Replica" is a marketing concept. In production, clients are untrusted. Without server-side validation:
- Any client can overwrite any key in any map
- A client can set HLC timestamps in the future to "win" all LWW conflicts
- No per-user or per-role access control

### Minimum Viable Security for v1.0

1. **HLC sanitization:** Server replaces client-provided HLC timestamps with server-generated ones. Client HLC is used for local ordering only; server HLC is authoritative for merge decisions.

2. **Map-level ACL:** Simple allow/deny per map per authenticated user. Not field-level, not row-level — just "can this user read/write this map?"

3. **Server-side write validation:** Before accepting a CRDT merge, server checks: (a) user is authenticated, (b) user has write access to this map, (c) value is within size limits.

### What This Means for CRDTs

The CRDT merge semantics remain the same. The security layer sits BEFORE the CRDT merge in the pipeline:

```
Client write → Auth check → Map ACL check → HLC sanitization → CRDT merge → Persist
```

This is the standard approach used by Ditto and Firebase. CRDTs handle conflict resolution; the security layer handles authorization.

### Impact on TODO List

```
TODO-097: Server-Side Write Validation + HLC Sanitization
- Priority: P0 (blocks production use)
- Complexity: Medium
- Milestone: v1.0
- Scope:
  - HLC sanitization in CrdtService: replace client HLC with server HLC on received operations
  - Map-level ACL: MapPermissions struct (read: bool, write: bool) per connection per map
  - Write validation middleware: auth check → ACL check → size limit → pass to CRDT merge
  - Reject unauthenticated writes (currently all clients can write without auth)
- Depends on: TODO-085 (CrdtService) — already done
- Effort: 1 week

TODO-098: Security Model Documentation
- Priority: P1 (trust signal for adoption)
- Complexity: Low
- Milestone: v1.0
- Scope:
  - Add "Security Model" section to PRODUCT_CAPABILITIES.md
  - Document trust boundary: clients are untrusted, server is authoritative
  - Document HLC sanitization, map-level ACL, write validation
  - Document authentication flow (JWT, how to integrate with existing auth)
- Effort: 2-3 days
```

---

## 6. Testing & Observability

### Current State

- 468 server-rust tests + 431 core-rust tests, clippy-clean
- TODO-068 (Integration Test Suite) exists but hasn't started
- No observability TODOs beyond admin dashboard
- `tracing` crate is already a dependency in both Rust crates

### What's Missing

**6.1. Structured Logging**
`tracing` is a dependency but likely not systematically used across all services. Every domain service operation should emit structured spans:
```rust
#[instrument(skip(self), fields(map_name, key, caller_origin))]
async fn handle_put(&self, ...) { ... }
```

**6.2. Metrics Export**
The Tower middleware pipeline includes `MetricsMiddleware` but there's no metrics endpoint for Prometheus scraping. The admin dashboard shows stats, but external monitoring tools need a `/metrics` endpoint.

**6.3. CRDT Debug Tools**
The TS CLI has `debug:crdt` commands (export, stats, conflicts, timeline, replay, tail). These need Rust server equivalents. When a customer reports "data looks wrong", you need tools to inspect CRDT state, HLC timelines, and Merkle tree divergences.

**6.4. Client-Side Developer Tools**
Browser DevTools panel or Chrome extension that shows: local replica state, pending sync queue, HLC timeline, connection status, CRDT merge history. This is how Ditto, PowerSync, and ElectricSQL differentiate on developer experience.

### Impact on TODO List

```
TODO-099: Structured Tracing + Metrics Endpoint
- Priority: P1 (operational necessity)
- Complexity: Medium
- Milestone: v1.0
- Scope:
  - Add #[instrument] spans to all domain service handlers
  - Add Prometheus /metrics endpoint (metrics crate + axum handler)
  - Export: operation count, latency histograms, active connections, map sizes, sync operations
  - Correlate client request → server operation via trace IDs
- Depends on: Network module (TODO-064, done)
- Effort: 3-5 days

TODO-100: CRDT Debug API (Rust equivalent of debug:crdt CLI)
- Priority: P2 (essential for production support)
- Complexity: Medium
- Milestone: v1.0 (post-release hardening) or v2.0
- Scope:
  - Admin API endpoints: GET /api/debug/crdt/{map}/stats, /conflicts, /timeline, /export
  - Expose HLC state per map, merge history, Merkle tree summary
  - Wire into admin dashboard (data explorer tab)
- Depends on: TODO-093 (Admin Dashboard), TODO-085 (CrdtService)
- Effort: 1 week

TODO-101: Client DevTools
- Priority: P2 (DX differentiator)
- Complexity: Medium
- Milestone: v2.0
- Scope:
  - Browser DevTools panel (Chrome extension) or in-app debug overlay
  - Shows: local replica state, pending sync queue, HLC timeline, connection status
  - CRDT merge visualization: what merged, which timestamp won, conflict count
  - Offline queue inspector: pending ops, retry state
- Depends on: Client SDK stabilization
- Effort: 2-3 weeks
```

---

## 7. Client SDK Size Strategy

### Your Hypothesis: Correct

Base SDK = sync engine + CRDT types + predicate filtering only. Everything heavy is optional.

### Detailed Recommendation

| Module | Inclusion | Estimated Size | Rationale |
|--------|-----------|---------------|-----------|
| CRDT types (LWW-Map, OR-Map) | **Core** | ~15KB gzipped | Essential — this IS TopGun |
| HLC | **Core** | ~2KB gzipped | Required for CRDT |
| SyncEngine + WebSocket | **Core** | ~20KB gzipped | Required for sync |
| Predicate filtering (client-side) | **Core** | ~5KB gzipped | Basic query, no WASM |
| IndexedDB adapter | **Core** | ~8KB gzipped | Offline persistence |
| React hooks | **Separate package** | ~5KB gzipped | Already `@topgunbuild/react` |
| DataFusion WASM (SQL) | **Optional import** | ~2-5MB gzipped | Only if client-side SQL needed |
| Tantivy WASM (search) | **Optional import** | ~1-3MB gzipped | Only if client-side FTS needed |

**Total core SDK: ~50KB gzipped.** This is competitive with PowerSync (~40KB) and smaller than RxDB (~80KB).

### Counterarguments and Responses

**"Why not include SQL in core?"** — Because 90%+ of queries are simple predicates (Eq, In, OrderBy, Limit). The `PredicateEngine` handles these without WASM overhead. SQL is for power users running JOINs and GROUP BYs — they can opt in.

**"Won't lazy-loading WASM create a jarring UX?"** — Use the pattern: first query triggers WASM download, show a one-time loading indicator. After that, WASM is cached. Alternative: preload in a Web Worker on idle.

### Architecture

```typescript
// Core SDK — always loaded
import { TopGunClient } from '@topgunbuild/client';

// Optional SQL — lazy-loaded WASM
import { enableSQL } from '@topgunbuild/client/sql';
await enableSQL(client); // downloads + initializes WASM
const results = await client.sql('SELECT ...');

// Optional search — lazy-loaded WASM
import { enableSearch } from '@topgunbuild/client/search';
await enableSearch(client);
const hits = client.search('products', { query: 'shoes' });
```

### Impact on TODO List

No new TODO needed. TODO-072 (Selective WASM Modules) already covers this. But its description should be updated to emphasize the lazy-loading architecture and target bundle size budget (~50KB core).

---

## 8. CLI — Migration to Rust

### Current State

`bin/commands/` contains 14 CLI commands implemented in JavaScript using `commander`. Commands cover: doctor, setup, dev, test, config, cluster management, CRDT debug, search debug, Docker management.

### Recommendation: Don't Rewrite Now, Plan for Later

**Reasoning:**
1. The CLI is a developer tool, not a production component. Performance doesn't matter.
2. The JS CLI already works and covers the needed functionality.
3. Rewriting in Rust (via `clap`) would take 1-2 weeks for zero user-facing benefit.
4. The CLI will need significant changes AFTER the TS server is removed anyway (different config, different startup, different debug endpoints).

**When to rewrite:**
- After v1.0, when the TS server is deprecated and the CLI needs to manage only the Rust server
- When the `topgun` binary becomes the single entry point (Rust server + CLI in one binary)
- Natural timing: v2.0 milestone, alongside other CLI-surface work

### Impact on TODO List

```
TODO-102: Rust CLI (clap)
- Priority: P3 (not blocking)
- Complexity: Medium
- Milestone: v2.0
- Scope:
  - Rewrite CLI in Rust using clap
  - Merge with server binary: `topgun serve`, `topgun status`, `topgun debug crdt`
  - Drop Node.js dependency for server operation
  - Keep JS CLI for TS client development tools
- Depends on: v1.0 completion (CLI shape depends on final server API)
- Effort: 1-2 weeks
```

---

## 9. Legacy TypeScript Code Removal

### What to Remove (After Rust Server Passes All Integration Tests)

| Package | Action | Rationale |
|---------|--------|-----------|
| `packages/server/` | **Remove entirely** | Replaced by `packages/server-rust/` |
| `packages/native/` | **Remove entirely** | xxHash64 native addon — Rust handles hashing |
| `packages/mcp-server/` | **Evaluate** | MCP server for Claude — may still be useful if it wraps TS client |
| `packages/core/` | **Keep** | TS client SDK still uses it |
| `packages/client/` | **Keep** | TS client SDK — this IS the product for web developers |
| `packages/react/` | **Keep** | React hooks — core DX |
| `packages/adapters/` | **Keep** | IndexedDB adapter for client |
| `packages/adapter-better-auth/` | **Keep** | Auth integration |

### When to Remove

NOT during v1.0 development. The TS server serves as:
1. Behavioral oracle for integration tests (TODO-068)
2. Fallback if Rust server has critical bugs
3. Reference for developers reading the codebase

Remove AFTER:
1. TODO-068 (Integration Tests) proves Rust server matches TS server behavior
2. At least one real application runs against Rust server successfully
3. All CLI commands work against Rust server

### Impact on TODO List

```
TODO-103: Remove Legacy TS Server Code
- Priority: P2 (cleanup)
- Complexity: Low
- Milestone: v1.0 (final step, after TODO-068)
- Scope:
  - Remove packages/server/ entirely
  - Remove packages/native/ entirely
  - Update pnpm workspace config
  - Update CI workflows (remove TS server test jobs)
  - Update CLAUDE.md package hierarchy
  - Update examples/ to use Rust server
  - Evaluate packages/mcp-server/ — keep if still useful
- Depends on: TODO-068 (Integration Tests — proves behavioral equivalence)
- Effort: 1-2 days
```

---

## 10. Monetization Strategy

### Your Position: Build First, Monetize Later

This is the correct approach for infrastructure products. The reasoning:
1. Infrastructure adoption is trust-based. Trust requires production usage, which requires time.
2. Premature monetization kills growth. Every pricing page is a bounce.
3. Open Core model means the free tier IS the growth engine. Enterprise features (v3.0) are the monetization trigger.

### The Timeline

```
v1.0 (Working IMDG) → Free, open source (Apache 2.0)
  Goal: Get 10-50 production users. Build trust. Collect case studies.

v2.0 (Data Platform) → Free, open source (Apache 2.0)
  Goal: Get 100-500 users. SQL + streaming makes it a "real" platform.
  Announce: "TopGun Cloud coming soon" (managed hosting waitlist).

v3.0 (Enterprise) → Open Core
  Monetization starts:
  - Enterprise features (BSL): multi-tenancy, tiered storage, vector search
  - TopGun Cloud (managed): usage-based pricing
  - Support contracts: SLA, priority fixes
```

### No TODOs Needed Now

Monetization planning happens when v2.0 is near completion. Adding commercial infrastructure too early is a distraction.

---

## 11. Competitive Intelligence — Additions

### Missing from Competitive Comparison

**Cloudflare Durable Objects + D1**
- Stateful edge compute with actor model
- D1 (SQLite at edge) for persistence
- Not local-first, not CRDT, but solves low-latency distributed state
- Different trust model (Cloudflare-hosted, not self-hosted)
- Worth adding to the "When NOT to Use TopGun" table: "If you need edge-native serverless state → Durable Objects"

**Replicache / Zero (by Rocicorp)**
- Closest competitor in the "sync engine with server authority" space
- Replicache uses server-authoritative mutations (not CRDTs)
- Zero is their new product: offline-first with server reconciliation
- Different trade-off: server authority (simpler security) vs CRDTs (automatic merge)
- Should be in the competitive table

**Impact:** Update PRODUCT_CAPABILITIES.md competitive comparison. No TODO needed — this is a documentation update that can happen alongside TODO-096 (Adoption Path docs).

---

## 12. Documentation Site & Demo Apps

### 12.1. Documentation Site (`apps/docs-astro/`)

**Framework:** Custom Astro 5 + React 19 (NOT Starlight). Tailwind CSS v4, Shiki syntax highlighting, OG image generation. Deployed at `https://topgun.build`.

**Assessment: High quality, needs content fixes.**

The site structure is sound — custom `DocsSidebar`, content collections, interactive `TacticalDemo` on the homepage that simulates HLC/LWW merge with mock data. 21 guides, 8 reference pages, 6 blog posts. No need to restructure.

**Content issues to fix:**

| Issue | Location | Fix |
|-------|----------|-----|
| "Postgres/Mongo" — Mongo not supported | `intro.mdx`, `comparison.mdx` | Remove Mongo references |
| Orphaned page | `guides/rbac.mdx` exists but not in `guides/index.mdx` | Add to index or remove |
| Docs describe TS server API | `reference/server.mdx`, `reference/cli.mdx` | Update after Rust migration |
| No Security Model section | Missing entirely | Add after TODO-098 |
| No Adoption Path section | Missing entirely | Add after TODO-096 |
| Replicache/Zero missing from comparison | `comparison.mdx` | Add to competitive table |
| Durable Objects missing | `comparison.mdx` | Add to "When NOT to use" |

### 12.2. Notes App (`examples/notes-app/`)

**Assessment: Excellent showcase, poor getting-started experience.**

Production-quality PWA demonstrating: `EncryptedStorageAdapter`, Clerk auth, useQuery/useMutation, offline PWA, per-user data isolation, R2 file upload, push notifications.

**Problems:**
- **Requires Clerk API key** — app won't start without it. Zero "time to first value."
- **Push notification UI in Russian** — `"Уведомления включены"`, `"Включить уведомления"` — development artifact.
- **Doesn't show CRDT conflict resolution** — no multi-tab/multi-device sync visualization.
- **Too complex for first contact** — auth + encryption + R2 + push. New users want to see the core magic, not infrastructure integrations.

**Recommendation:** Keep as a showcase/portfolio demo. Localize strings to English. Add a "Sync Status" indicator. But do NOT position this as the getting-started example.

### 12.3. Todo App (`examples/todo-app/`)

**Assessment: Too minimal, doesn't sell TopGun.**

Shows useQuery with predicates, CRUD. But:
- Hardcoded JWT token in source code
- Incomplete `vite.config.ts` aliases (missing `@topgunbuild/react` and `@topgunbuild/adapters`)
- No offline/reconnect demonstration
- No multi-tab sync visualization

**Recommendation:** Fix technical issues (aliases, JWT). But this app cannot serve as the primary demo — it looks like any other React CRUD app.

### 12.4. The Missing "Perfect Demo"

Neither existing app demonstrates TopGun's core differentiator. What's needed:

**A "Collaborative Kanban" or "Sync Board" demo** that:

1. **Starts in 30 seconds** — `pnpm install && pnpm dev`, no auth provider, no external services
2. **Shows offline → reconnect** — visual indicator ("Offline" / "2 pending ops" / "Synced"), click a button to simulate disconnect
3. **Shows multi-tab sync** — banner: "Open this URL in another tab to see real-time sync"
4. **Shows CRDT conflict resolution** — split-screen mode: left panel = "Device A", right panel = "Device B", both edit the same item while "offline", click "Reconnect" to see automatic LWW merge with visual highlight of which version won
5. **Shows sync API** — `map.get()` without `await`, with a latency counter showing "0ms read"

This is the "offline Wi-Fi video" that Gemini recommended — but as an interactive web app, embeddable on the docs homepage replacing the mock TacticalDemo.

### Proposed TODOs

```
TODO-104: Fix Demo App Issues
- Priority: P2 (quality)
- Complexity: Low
- Milestone: v1.0
- Scope:
  - todo-app: fix vite.config.ts aliases, remove hardcoded JWT, add .env.example
  - notes-app: localize Russian strings to English, add .env.example
  - docs-astro: remove Mongo references, add rbac.mdx to guides index
- Effort: 1-2 days

TODO-105: Sync Showcase Demo App
- Priority: P1 (marketing — "Show, Don't Tell")
- Complexity: Medium
- Milestone: v1.0
- Scope:
  - New example: collaborative board (Kanban or shared list)
  - Zero external deps (no Clerk, no R2, no push)
  - Visual sync status indicator (online/offline/pending ops count)
  - "Simulate Offline" button — disconnects WebSocket, shows pending queue
  - Multi-tab awareness: banner prompting to open another tab
  - Split-screen conflict demo: two "devices" editing same data
  - Designed to be embedded as iframe/video on docs homepage
- Depends on: TODO-071 (Search — server must work), or can use TS server for initial demo
- Effort: 1 week

TODO-106: Update Documentation Content for Rust Server
- Priority: P2 (post-migration)
- Complexity: Medium
- Milestone: v1.0 (after TODO-068 integration tests)
- Scope:
  - Update reference/server.mdx for Rust server API
  - Update reference/cli.mdx for new CLI commands
  - Update reference/protocol.mdx if wire format changed
  - Add Security Model section (from TODO-098 output)
  - Add Adoption Path section (from TODO-096 output)
  - Update comparison.mdx: add Replicache/Zero, Durable Objects
  - Verify all code snippets work with current SDK
- Depends on: TODO-096, TODO-098, TODO-103 (legacy removal finalizes API surface)
- Effort: 1 week
```

---

## 13. Summary of Proposed New TODOs

### v1.0 (Add to current milestone)

| TODO | Title | Priority | Effort | Rationale |
|------|-------|----------|--------|-----------|
| TODO-094 | Change LICENSE to Apache 2.0 | P1 | 30 min | Blocks community adoption |
| TODO-096 | Adoption Path Docs + Tier 1 Example | P1 | 1 week | Critical for first users |
| TODO-097 | Server-Side Write Validation + HLC Sanitization | P0 | 1 week | Blocks production use |
| TODO-098 | Security Model Documentation | P1 | 2-3 days | Trust signal |
| TODO-099 | Structured Tracing + Metrics Endpoint | P1 | 3-5 days | Operational necessity |
| TODO-104 | Fix Demo App Issues (aliases, i18n, Mongo refs) | P2 | 1-2 days | Quality |
| TODO-105 | Sync Showcase Demo App (offline/conflict/multi-tab) | P1 | 1 week | "Show, Don't Tell" marketing |
| TODO-106 | Update Documentation Content for Rust Server | P2 | 1 week | Post-migration docs sync |

### v1.0 (post-release) or v2.0

| TODO | Title | Priority | Effort | Rationale |
|------|-------|----------|--------|-----------|
| TODO-100 | CRDT Debug API | P2 | 1 week | Production support |
| TODO-101 | Client DevTools | P2 | 2-3 weeks | DX differentiator |
| TODO-102 | Rust CLI (clap) | P3 | 1-2 weeks | Drop Node.js dependency |
| TODO-103 | Remove Legacy TS Server Code | P2 | 1-2 days | Cleanup |

### v3.0

| TODO | Title | Priority | Effort | Rationale |
|------|-------|----------|--------|-----------|
| TODO-095 | Enterprise Directory Structure | P3 | 1 day | License separation |

---

## 14. Revised v1.0 Execution Order

With the new TODOs integrated:

```
Wave 5d (current):
  TODO-071 (Search/SPEC-068) — READY FOR IMPLEMENTATION

Wave 5e (parallel):
  TODO-074 (HLC colon fix) · TODO-075 (ORMap hash fix) — trivial, parallel
  TODO-094 (LICENSE change) — trivial, parallel
  TODO-104 (Fix Demo App Issues) — trivial, parallel

Wave 5f (after 071):
  TODO-097 (Write Validation + HLC Sanitization) — blocks production use
  TODO-099 (Structured Tracing + Metrics) — parallel with 097

Wave 5g (parallel with 5f):
  TODO-093 (Admin Dashboard v1.0)
  TODO-098 (Security Model Docs) — parallel with 093
  TODO-096 (Adoption Path Docs + Example) — parallel with 093
  TODO-105 (Sync Showcase Demo) — parallel with 093

Wave 5h (gates release):
  TODO-068 (Integration Tests) — incremental, but gates v1.0
  TODO-106 (Update Docs for Rust Server) — after 068 finalizes API surface
  TODO-103 (Remove Legacy TS) — after 068 proves equivalence
```

### Updated v1.0 Critical Path

```
071 (Search) → 097 (Security) → 068 (Integration Tests) → v1.0 Release
               ↓ parallel
              099 (Tracing) + 093 (Admin) + 094 (License) + 096 (Docs)
              105 (Sync Demo) + 104 (Fix Demos) + 106 (Update Docs)
```

### Effort Impact

New tasks add approximately **3-4 weeks** to v1.0 timeline. This is a worthwhile trade-off — releasing without server-side security, observability, or a compelling demo would damage trust and adoption more than a delay.

---

## 15. Strategic Priorities (Ranked)

1. **Finish SPEC-068 (Search)** — Last domain service, unblocks everything
2. **Security (TODO-097)** — Without this, TopGun is not production-usable
3. **License change (TODO-094)** — Without Apache 2.0, no community adoption
4. **Sync Showcase Demo (TODO-105)** — "Show, Don't Tell" — most impactful marketing asset
5. **Integration Tests (TODO-068)** — Proves the Rust server works
6. **Observability (TODO-099)** — Needed for anyone running TopGun in production
7. **Adoption Path (TODO-096)** — Needed for anyone evaluating TopGun
8. **Admin Dashboard (TODO-093)** — Differentiator, but not a blocker
9. **Bug fixes (TODO-074/075)** — Important but low effort
10. **Fix Demos (TODO-104)** — Quick wins, parallel work
11. **Update Docs (TODO-106)** — After API surface is finalized
12. **Legacy cleanup (TODO-103)** — Last step before v1.0 announcement
13. **Security Docs (TODO-098)** — Can be written alongside implementation
