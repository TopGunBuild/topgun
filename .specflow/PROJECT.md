# TopGun

## What This Is

A hybrid offline-first in-memory data grid providing zero-latency reads/writes via local CRDTs, real-time sync via WebSockets, and durable storage on PostgreSQL.

**Product positioning:** "The reactive data grid that extends the cluster into the browser." TopGun bridges two traditionally separate markets: server-side in-memory data grids (Hazelcast) and client-side offline-first sync engines (PowerSync). Clients are first-class cluster participants with CRDT replicas, not thin proxies.

## Core Value

Local-first data access that never waits for network, with automatic conflict resolution and seamless sync when connectivity is available.

## Tech Stack

| Layer | Technology |
|-------|------------|
| Language (client) | TypeScript |
| Language (server) | TypeScript (migrating to Rust) |
| Runtime (client) | Browser / Node.js |
| Runtime (server) | Node.js (migrating to tokio) |
| Package Manager | pnpm 10.13.1 (monorepo) |
| Build Tool | tsup (TS), Cargo (Rust) |
| Testing | Jest (TS), cargo test + proptest (Rust) |
| Server Transport | WebSocket (ws), HTTP sync |
| Database | PostgreSQL |
| Serialization | msgpackr / rmp-serde (MsgPack) |
| Schema Validation | Zod (TS) / serde (Rust) |

## Project Structure

```
packages/
├── core/               # TS: CRDTs (LWWMap, ORMap), HLC, MerkleTree, schemas
├── client/             # TS: Browser/Node SDK: TopGunClient, SyncEngine
├── server/             # TS: WebSocket server, clustering, PostgreSQL adapter
├── react/              # TS: React bindings: hooks (useQuery, useMap, etc.)
├── adapters/           # TS: Storage implementations (IndexedDB)
├── native/             # N-API: Native xxHash64 for Node.js performance
├── mcp-server/         # TS: MCP server integration
├── adapter-better-auth/ # TS: BetterAuth integration
├── core-rust/          # Rust (planned): CRDTs, HLC, MerkleTree
└── server-rust/        # Rust (planned): Server rewrite (axum, tokio, sqlx)
tests/
├── e2e/                # End-to-end tests
└── k6/                 # Load testing (k6)
```

## Rust Migration Status

**Current phase:** Phase 0 (TypeScript Completion) — finishing client cluster routing (SPEC-048b, 048c)

**Strategy:** Complete TypeScript Wave 1 → Bridge (Cargo workspace + traits) → Rust server rewrite using TS as executable specification.

**Key architectural decisions (2026-02-12):**
- 6 upfront traits gate all Rust architecture (ServerStorage, MapProvider, QueryNotifier, Processor, RequestContext, SchemaProvider)
- TypeScript-first schema strategy (developers define Zod schemas, build step generates Rust)
- Partial replication / Shapes (table stakes feature, SchemaProvider trait)
- MsgPack wire protocol (cross-language, NOT Bincode)
- Selective WASM (DAG executor, tantivy search — NOT basic CRDT ops)

**Full roadmap:** [TODO.md](.specflow/todos/TODO.md)
**Technical research:** [RUST_SERVER_MIGRATION_RESEARCH.md](.specflow/reference/RUST_SERVER_MIGRATION_RESEARCH.md)
**Product positioning:** [PRODUCT_POSITIONING_RESEARCH.md](.specflow/reference/PRODUCT_POSITIONING_RESEARCH.md)

### Reference Implementation: Hazelcast

**Path:** `/Users/koristuvac/Projects/hazelcast`

Hazelcast is an enterprise-grade in-memory data grid (Java). It serves as an **architectural reference** for server-side components that have no equivalent in the TopGun TypeScript implementation. When designing Rust server architecture, consult Hazelcast's patterns for:

| TopGun component | Hazelcast reference package | What to learn |
|------------------|-----------------------------|---------------|
| ClusterManager | `cluster/`, `internal/cluster/` | Cluster membership, heartbeat, split-brain detection |
| PartitionService | `partition/`, `internal/partition/` | Partition ownership, migration, rebalancing strategies |
| Distributed Map | `map/`, `replicatedmap/` | IMap lifecycle, near-cache invalidation, entry processing |
| Query engine | `query/`, `sql/`, `jet/` | Predicate indexing, DAG execution, query optimization |
| Pub/sub topics | `topic/` | Reliable topic delivery, ordering guarantees |
| Persistence | `persistence/`, `hotrestart/` | Snapshot strategies, WAL, hot restart |
| Security | `security/` | Authentication, authorization, TLS setup |
| Near Cache | `nearcache/` | Client-side caching with invalidation (analogous to TopGun client replicas) |
| Split-brain | `splitbrainprotection/` | Quorum policies, merge strategies |
| Networking | `nio/` | Non-blocking I/O, connection management |

**Existing analysis:** `HAZELCAST_SQL_ARCHITECTURE_AUDIT.md` (SQL/DAG subsystem audit)

**Usage rule:** When a spec involves server-side architecture (clustering, partitioning, persistence, query execution), the spec creator and auditor SHOULD review the corresponding Hazelcast package for proven patterns before finalizing the design. The goal is not to copy Java code, but to learn from Hazelcast's battle-tested architecture decisions and adapt them idiomatically for Rust.

## Rust Migration Principles

- **Fix-on-port, don't copy bugs:** When migrating TS code to Rust, fix discovered issues rather than reproducing them. No active clients means breaking changes are free. Known TS bugs are tagged "covered by rewrite" in TODO.md.
- **TS as executable spec, not gospel:** The TS implementation defines *what* the system does (behavior, wire protocol, test vectors), but not necessarily *how* it should be done. Rust should improve architecture where the TS design was expedient.
- **Audit before implementing:** Before porting a domain to Rust, audit the TS source for bugs, dead code, and inconsistencies. Fix them in TS first (so the TS test suite validates the fix), then port the corrected version.
- **No JS-isms in Rust:** Do not reproduce JavaScript language limitations in Rust. Every Rust struct/field must be evaluated: "Is this the best Rust representation, or am I copying a JS constraint?" Specific rules below.

### Rust Type Mapping Rules (mandatory for all Rust specs)

These rules apply to every spec that creates or modifies Rust structs in `packages/core-rust/` or `packages/server-rust/`.

**1. Integer types, not f64:**
JS has no integer type, so TS uses `z.number()` for everything. Rust MUST use proper integer types:

| Semantic meaning | JS/TS type | Rust type | Rationale |
|------------------|------------|-----------|-----------|
| Hash value | `number` | `u64` | Hashes are unsigned integers |
| Count, length | `number` | `u32` or `u64` | Counts are non-negative integers |
| Error code | `number` | `u32` | HTTP-style codes are integers |
| Timeout (ms) | `number` | `u64` | Milliseconds are non-negative integers |
| Timestamp (ms since epoch) | `number` | `i64` | Already correct in `Timestamp.millis` |
| Score, weight, ratio | `number` | `f64` | Genuinely fractional values stay as f64 |
| Page size, offset, limit | `number` | `u32` or `u64` | Pagination values are non-negative integers |

This produces **better** wire compatibility: TS `msgpackr` encodes whole numbers as MsgPack integers. Rust `u64` decodes them directly. Using `f64` forces coercion on read and emits MsgPack float64 on write — a different binary format.

**2. No `type` field in message structs:**
The `Message` enum uses `#[serde(tag = "type")]` — serde owns the `type` discriminant. Inner structs MUST NOT have a `type` / `r#type` field. Having both produces duplicate keys on serialization (undefined behavior in MsgPack).

**3. Default derives for payload structs:**
Payload structs with 2+ optional fields should derive `Default` for ergonomic construction.

**4. Enums over strings for known value sets:**
If TS uses `z.literal('X')` or `z.enum([...])`, Rust should use an enum, not `String`.

### Auditor Checklist for Rust Specs

When auditing a Rust spec, verify each of these. Flag violations as **critical**:

- [ ] No `f64` for integer-semantic fields (see type mapping table above)
- [ ] No `r#type: String` on message structs (enum owns the tag)
- [ ] `Default` derived on payload structs with 2+ optional fields
- [ ] Enums used for known value sets (not raw `String`)
- [ ] Wire compatibility: uses `rmp_serde::to_vec_named()`, not `to_vec()`
- [ ] `#[serde(rename_all = "camelCase")]` on every struct
- [ ] `#[serde(skip_serializing_if = "Option::is_none", default)]` on every `Option<T>`

## Patterns & Conventions

- Monorepo with package hierarchy: core -> client/server -> adapters/react
- TypeScript with strict mode
- Commit format: `type(scope): description` (feat, fix, docs, test, chore, refactor, perf)
- Test files co-located with source in `__tests__/` directories
- Server tests use ports 10000+ for servers, 11000+ for cluster nodes
- CRDTs use Hybrid Logical Clocks for causality tracking
- No phase/spec/bug references in code comments — use WHY-comments instead

## Constraints

- Local-first: Reads and writes must never wait for network
- CRDT conflict resolution using LWW-Map and OR-Map
- Must work offline with IndexedDB persistence
- Server-authoritative cluster architecture
- MsgPack wire format for client-server protocol (cross-language compatibility)

## Language Profile

| Setting | Value |
|---------|-------|
| Language | Rust |
| Max files per spec | 5 |
| Trait-first | Yes |

**Notes:**
- Rust specs use trait-first ordering: G1 (Wave 1) defines traits/types only, implementation groups depend on G1
- Max 5 files per spec to limit borrow checker cascade risk
- Applies to `packages/core-rust/` and `packages/server-rust/` only
- TypeScript packages continue using existing conventions (no file limit, no trait-first)

---
*Generated by SpecFlow on 2026-01-20. Updated 2026-02-12 with Rust migration status and product positioning.*
