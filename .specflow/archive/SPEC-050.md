# SPEC-050: Define 6 Foundational Rust Traits

---
id: SPEC-050
type: feature
status: done
priority: P0
complexity: small
created: 2026-02-13
todo_ref: TODO-060
---

## Context

The Rust migration strategy identifies 6 trait abstractions that gate all architectural decisions. These traits define the boundaries between subsystems and prevent 95% of future rework. They are design-only: no implementations, just trait definitions, structs, enums, and placeholder types. The TypeScript server (`packages/server/`) serves as the behavioral reference, particularly `IServerStorage` in `packages/server/src/storage/IServerStorage.ts`.

Both `packages/core-rust/` and `packages/server-rust/` were scaffolded in SPEC-049 with empty `lib.rs` files, workspace lints (clippy all+pedantic, `unsafe_code = "forbid"`), and Cargo.toml. No external dependencies exist yet.

**Reference:** RUST_SERVER_MIGRATION_RESEARCH.md Section 7, PRODUCT_POSITIONING_RESEARCH.md Section 7.5.

## Goal Statement

After this spec is implemented, every future Rust feature crate and implementation has well-defined trait boundaries to code against, enabling parallel development of storage backends, CRDT maps, DAG executor vertices, schema validation, and query notifications without cross-cutting design decisions.

### Observable Truths

1. `cargo build --workspace` compiles without errors or warnings.
2. `cargo clippy --workspace` passes with zero warnings under `all + pedantic` lint profile.
3. `cargo test --workspace` passes (existing crate_loads tests still work, plus new compile-gate tests).
4. A developer can write `impl ServerStorage for PostgresAdapter` in `topgun-server` by depending only on the trait and `StorageValue` from `topgun-core`.
5. A developer can write `impl Processor for MyVertex` in any crate by depending only on `topgun-core`.
6. `RequestContext` can be constructed in both `topgun-core` tests and `topgun-server` code.
7. `SchemaProvider.get_shape` returns `SyncShape` referencing `RequestContext`, proving the cross-type wiring compiles.

### Required Artifacts

| Artifact | Crate | Purpose |
|----------|-------|---------|
| `src/types.rs` | topgun-core | `StorageValue`, `Value`, `MapType`, `CrdtMap` (placeholder), `Principal` |
| `src/traits.rs` | topgun-core | `QueryNotifier`, `Processor` traits + `ProcessorContext`, `Inbox` |
| `src/context.rs` | topgun-core | `RequestContext` struct |
| `src/schema.rs` | topgun-core | `MapSchema`, `ValidationResult`, `SyncShape`, `Predicate` |
| `src/lib.rs` | topgun-core | Module declarations, re-exports |
| `src/traits.rs` | topgun-server | `ServerStorage`, `MapProvider`, `SchemaProvider` traits |
| `src/lib.rs` | topgun-server | Module declarations, re-exports |
| `Cargo.toml` | topgun-core | Add `async-trait`, `serde` dependencies |
| `Cargo.toml` | topgun-server | Add `async-trait`, `anyhow` dependencies |

### Key Links

- `topgun-server::ServerStorage` references `topgun-core::StorageValue` (cross-crate type dependency).
- `topgun-server::MapProvider` references `topgun-core::{CrdtMap, MapType}`.
- `topgun-server::SchemaProvider` references `topgun-core::{MapSchema, ValidationResult, SyncShape, RequestContext, Value}`.
- `topgun-core::Processor` references `topgun-core::{ProcessorContext, Inbox}`.

## Task

Define 6 foundational trait abstractions and their supporting types across 2 Rust crates. All items are design-only: trait definitions, struct/enum declarations, and type aliases. No method bodies beyond `todo!()` or default impls. No runtime logic.

## Requirements

### R1: Core Types (topgun-core/src/types.rs)

Define the following types:

```rust
use serde::{Deserialize, Serialize};

/// Opaque serialized CRDT record stored in persistence.
/// Placeholder: will be refined when CRDTs are ported (Phase 2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageValue {
    pub data: Vec<u8>,
}

/// Generic runtime value type for CRDT map entries.
/// Placeholder: will become a proper enum (Null, Bool, Int, Float, String, Bytes, Array, Map)
/// when message schemas are ported.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Value {
    pub data: Vec<u8>,
}

/// Discriminant for CRDT map types (LWW vs OR).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MapType {
    Lww,
    Or,
}

/// Placeholder for the unified CRDT map abstraction.
/// Will be replaced with actual LWWMap/ORMap implementations in Phase 2.
#[derive(Debug)]
pub struct CrdtMap {
    pub map_type: MapType,
}

/// Authentication principal for multi-tenancy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principal {
    pub id: String,
    pub roles: Vec<String>,
}
```

### R2: RequestContext (topgun-core/src/context.rs)

```rust
use crate::types::Principal;

/// Per-request context carrying identity, tenancy, and tracing information.
/// Threaded through all server operations for auth, audit, and multi-tenant isolation.
#[derive(Debug, Clone)]
pub struct RequestContext {
    pub node_id: String,
    pub tenant_id: Option<String>,
    pub principal: Option<Principal>,
    pub trace_id: String,
}
```

### R3: Schema Types (topgun-core/src/schema.rs)

```rust
use serde::{Deserialize, Serialize};

/// Schema definition for a map. Placeholder: will carry field definitions,
/// version info, and validation rules when the schema system is built.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapSchema {
    pub version: u32,
    pub fields: Vec<FieldDef>,
}

/// Single field definition within a schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDef {
    pub name: String,
    pub required: bool,
}

/// Result of validating a value against a schema.
#[derive(Debug, Clone)]
pub enum ValidationResult {
    Valid,
    Invalid { errors: Vec<String> },
}

/// Row-level filter predicate for sync shapes.
/// Placeholder: will become an expression tree when query filtering is built.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Predicate {
    pub expression: String,
}

/// Defines what subset of a map's data a client receives.
/// Used for partial replication (shapes).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncShape {
    pub map_name: String,
    pub filter: Option<Predicate>,
    pub fields: Option<Vec<String>>,
    pub limit: Option<usize>,
}
```

### R4: Core Traits (topgun-core/src/traits.rs)

```rust
use async_trait::async_trait;
use crate::types::Value;

/// Context provided to a Processor vertex on initialization.
#[derive(Debug)]
pub struct ProcessorContext {
    pub vertex_index: usize,
    pub total_parallelism: usize,
}

/// Inbound message queue for a Processor vertex.
/// Placeholder: will carry typed messages when DAG executor is built.
#[derive(Debug)]
pub struct Inbox {
    pub ordinal: usize,
}

/// Vertex in a DAG execution graph (Hazelcast-style distributed query processing).
/// Each vertex receives items from an Inbox, processes them, and signals completion.
#[async_trait]
pub trait Processor: Send {
    /// One-time initialization with execution context.
    async fn init(&mut self, ctx: ProcessorContext) -> anyhow::Result<()>;
    /// Process a batch from the inbox. Returns true when this ordinal is fully processed.
    async fn process(&mut self, ordinal: usize, inbox: &mut Inbox) -> anyhow::Result<bool>;
    /// Called after all ordinals are processed. Returns true if complete.
    async fn complete(&mut self) -> anyhow::Result<bool>;
    /// Whether this processor yields cooperatively (affects scheduling).
    fn is_cooperative(&self) -> bool;
    /// Release resources.
    async fn close(&mut self) -> anyhow::Result<()>;
}

/// Write-path notification for live query updates.
/// Implementations observe all writes and notify active query subscriptions.
pub trait QueryNotifier: Send + Sync {
    /// Called on every write. `old_value` enables delta-based optimizations.
    fn notify_change(
        &self,
        map_name: &str,
        key: &str,
        old_value: Option<&Value>,
        new_value: &Value,
    );
}
```

### R5: Server Traits (topgun-server/src/traits.rs)

```rust
use std::sync::Arc;
use async_trait::async_trait;
use topgun_core::{
    StorageValue, MapType, CrdtMap, Value,
    MapSchema, ValidationResult, SyncShape, RequestContext,
};

/// Pluggable persistence backend for the server.
/// Implementations: PostgreSQL, SQLite, S3 (future), memory (tests).
#[async_trait]
pub trait ServerStorage: Send + Sync {
    async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<StorageValue>>;
    async fn load_all(&self, map: &str, keys: &[String]) -> anyhow::Result<Vec<(String, StorageValue)>>;
    async fn load_all_keys(&self, map: &str) -> anyhow::Result<Vec<String>>;
    async fn store(&self, map: &str, key: &str, value: &StorageValue) -> anyhow::Result<()>;
    async fn store_all(&self, map: &str, records: &[(String, StorageValue)]) -> anyhow::Result<()>;
    async fn delete(&self, map: &str, key: &str) -> anyhow::Result<()>;
    async fn delete_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()>;
    async fn initialize(&self) -> anyhow::Result<()>;
    async fn close(&self) -> anyhow::Result<()>;
}

/// Async map access with tiered storage awareness.
/// Abstracts whether a map is in memory, being loaded from disk, or evicted.
#[async_trait]
pub trait MapProvider: Send + Sync {
    async fn get_map(&self, name: &str) -> Option<Arc<CrdtMap>>;
    async fn get_or_load_map(&self, name: &str, type_hint: MapType) -> anyhow::Result<Arc<CrdtMap>>;
    fn has_map(&self, name: &str) -> bool;
}

/// Schema validation and partial replication shape computation.
/// Controls what data clients receive and validates writes against schemas.
#[async_trait]
pub trait SchemaProvider: Send + Sync {
    async fn get_schema(&self, map_name: &str) -> Option<MapSchema>;
    async fn register_schema(&self, map_name: &str, schema: MapSchema) -> anyhow::Result<()>;
    fn validate(&self, map_name: &str, value: &Value) -> ValidationResult;
    async fn get_shape(&self, map_name: &str, client_ctx: &RequestContext) -> Option<SyncShape>;
}
```

### R6: Module Structure and Re-exports

**topgun-core/src/lib.rs** declares modules and re-exports all public types at crate root:
- `mod types;` -- `StorageValue`, `Value`, `MapType`, `CrdtMap`, `Principal`
- `mod context;` -- `RequestContext`
- `mod schema;` -- `MapSchema`, `FieldDef`, `ValidationResult`, `Predicate`, `SyncShape`
- `mod traits;` -- `Processor`, `ProcessorContext`, `Inbox`, `QueryNotifier`

**topgun-server/src/lib.rs** declares modules and re-exports:
- `mod traits;` -- `ServerStorage`, `MapProvider`, `SchemaProvider`

### R7: Cargo.toml Dependencies

**topgun-core/Cargo.toml** adds:
- `async-trait = "0.1"` (dependencies)
- `serde = { version = "1", features = ["derive"] }` (dependencies)
- `anyhow = "1"` (dependencies)

**topgun-server/Cargo.toml** adds:
- `async-trait = "0.1"` (dependencies)
- `anyhow = "1"` (dependencies)
- Existing `topgun-core` path dependency already present

### R8: No Implementation Code

All trait methods remain unimplemented. No `impl` blocks for any trait. Struct fields use simple types (`Vec<u8>`, `String`, `Vec<String>`, `u32`, `usize`, `bool`). No runtime logic. No `unsafe` code.

## Files

| File | Action | Description |
|------|--------|-------------|
| `packages/core-rust/Cargo.toml` | MODIFY | Add async-trait, serde, anyhow dependencies |
| `packages/core-rust/src/lib.rs` | MODIFY | Add module declarations and re-exports |
| `packages/core-rust/src/types.rs` | CREATE | StorageValue, Value, MapType, CrdtMap, Principal |
| `packages/core-rust/src/context.rs` | CREATE | RequestContext struct |
| `packages/core-rust/src/schema.rs` | CREATE | MapSchema, FieldDef, ValidationResult, Predicate, SyncShape |
| `packages/core-rust/src/traits.rs` | CREATE | Processor, ProcessorContext, Inbox, QueryNotifier |
| `packages/server-rust/Cargo.toml` | MODIFY | Add async-trait, anyhow dependencies |
| `packages/server-rust/src/lib.rs` | MODIFY | Add module declaration and re-exports |
| `packages/server-rust/src/traits.rs` | CREATE | ServerStorage, MapProvider, SchemaProvider |

**Total files: 9** (4 modified, 5 created)

**Note:** This exceeds the Language Profile "Max files per spec: 5" guideline. However, 4 of the 9 are trivial modifications (Cargo.toml dependency additions, lib.rs module declarations), and the actual design content lives in 5 source files. All files are tightly coupled (trait definitions referencing each other's types) and cannot be split without creating circular dependency issues. Splitting would force artificial boundaries between types that must be designed together.

## Acceptance Criteria

1. **AC-1:** `cargo build --workspace` succeeds with zero errors.
2. **AC-2:** `cargo clippy --workspace -- -D warnings` passes with zero warnings.
3. **AC-3:** `cargo test --workspace` passes (existing `crate_loads` tests plus any new compile-gate tests).
4. **AC-4:** `topgun-core` exports exactly these public items: `StorageValue`, `Value`, `MapType`, `CrdtMap`, `Principal`, `RequestContext`, `MapSchema`, `FieldDef`, `ValidationResult`, `Predicate`, `SyncShape`, `Processor`, `ProcessorContext`, `Inbox`, `QueryNotifier`.
5. **AC-5:** `topgun-server` exports exactly these public items: `ServerStorage`, `MapProvider`, `SchemaProvider`.
6. **AC-6:** All traits requiring `async` methods use `#[async_trait]` attribute.
7. **AC-7:** `ServerStorage`, `MapProvider`, `SchemaProvider`, `QueryNotifier` all require `Send + Sync` bounds.
8. **AC-8:** `Processor` requires `Send` (not `Sync`, because `&mut self` methods preclude shared access).
9. **AC-9:** No `impl` blocks exist for any of the 6 traits. No runtime logic. No `unsafe` code.
10. **AC-10:** `cargo doc --workspace --no-deps` generates documentation without warnings. Each trait and public type has a doc comment.

## Constraints

- DO NOT implement any trait (no `impl ServerStorage for ...` blocks).
- DO NOT add runtime dependencies beyond `async-trait`, `serde`, and `anyhow`. No `tokio`, `bytes`, or other crates.
- DO NOT create `main.rs` or binary targets -- both crates remain libraries.
- DO NOT remove existing `crate_loads` tests.
- DO NOT use `unsafe` code (workspace lint: `forbid`).
- DO NOT use concrete error types -- use `anyhow::Result` for all fallible methods. Custom error types are a Phase 2 concern.
- DO NOT add `#[derive(PartialEq)]` to types containing `Vec<u8>` unless needed for tests -- it will be added when actual CRDT types replace placeholders.

## Assumptions

- `anyhow::Result` is acceptable for trait error types at this stage. Custom error enums will be introduced when implementations are built (Phase 2+).
- `StorageValue` and `Value` use `Vec<u8>` as opaque byte containers. These will become proper typed enums when CRDTs and message schemas are ported to Rust.
- `FieldDef` is a minimal placeholder for schema field definitions. The real schema type will be derived from Zod schemas via codegen.
- `Predicate` uses a `String` expression placeholder. The real predicate will be an expression tree when query filtering is implemented.
- `Inbox` is a minimal placeholder. The real inbox will carry typed messages with backpressure semantics.
- `CrdtMap` is an opaque placeholder struct. The real implementation will be an enum over `LWWMap` and `ORMap`.
- No `Default` derives are needed at this stage -- construction will be explicit.
- `ValidationResult` does not need `Serialize`/`Deserialize` because it is a server-internal return type, not a wire type.
- `RequestContext` does not derive `Serialize`/`Deserialize` because it is a server-internal type. If distributed tracing or cluster forwarding requires serializing request context across nodes in future phases, these derives should be added at that time.

## Reference

- **TypeScript IServerStorage:** `packages/server/src/storage/IServerStorage.ts` (9 methods, maps 1:1 to `ServerStorage` trait)
- **Research:** `.specflow/reference/RUST_SERVER_MIGRATION_RESEARCH.md` Section 7
- **Product positioning:** `.specflow/reference/PRODUCT_POSITIONING_RESEARCH.md` Section 7.5
- **TODO:** `.specflow/todos/TODO.md` (TODO-060)

## Audit History

### Audit v1 (2026-02-13)
**Status:** APPROVED

**Context Estimate:** ~18% total (PEAK range)

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~18% | <=50% | OK |
| Largest file group | ~13% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:** PEAK range (0-30%)

**Strategic fit:** Aligned with project goals (directly implements TODO-060, Phase 1 Bridge)

**Project compliance:** Honors PROJECT.md decisions

**Language profile:** File count (9) exceeds Rust profile limit (5) -- justified deviation noted in spec (see Recommendation 2)

**Goal-backward validation:** All observable truths have artifact coverage; all artifacts serve a truth; all key links identified

**Recommendations:**
1. Required Artifacts table (row for topgun-server Cargo.toml) incorrectly lists `serde` as a dependency, but R7 correctly omits it for topgun-server. The table was corrected during this audit.
2. File count (9) exceeds Language Profile limit (5). The spec's justification is sound -- borrow checker cascade risk does not apply to pure trait definitions, and splitting would create artificial boundaries. Consider adding a "trait-only exception" to the Language Profile for future clarity.
3. `schema.rs` imports `use crate::types::Value` but `Value` is not used in any struct field or method signature within schema.rs. If clippy pedantic flags unused imports, the implementor should remove this import. (Note: `SyncShape` does not reference `Value`; `Value` is used in `SchemaProvider.validate` in server-rust, not in core schema types.)
4. `RequestContext` does not derive `Serialize`/`Deserialize`. This is presumably intentional (server-internal type), but worth noting in Assumptions if it might need wire serialization for distributed tracing in future phases.

**Comment:** Exceptionally well-crafted specification. Every type and trait is provided as literal Rust code with doc comments. Acceptance criteria are concrete and verifiable via cargo commands. Cross-crate dependencies are explicitly mapped. Placeholder rationale is documented in Assumptions. The spec is implementation-ready.

### Response v1 (2026-02-13)
**Applied:** All 4 recommendations from Audit v1

**Changes:**
1. [✓] Required Artifacts table — Already corrected during audit (topgun-server Cargo.toml row shows `async-trait, anyhow`)
2. [✓] File count deviation — Acknowledged; spec justification preserved. Language Profile "trait-only exception" to be added separately to PROJECT.md
3. [✓] Unused import in schema.rs — Removed `use crate::types::Value;` from R3 code snippet (Value not referenced in schema types)
4. [✓] RequestContext serialization — Added assumption noting Serialize/Deserialize may be needed for distributed tracing in future phases

### Audit v2 (2026-02-13)
**Status:** APPROVED

**Context Estimate:** ~20% total (PEAK range)

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~20% | <=50% | OK |
| Largest file group | ~13% | <=30% | OK |
| Worker overhead | ~5% | <=10% | OK |

**Quality Projection:** PEAK range (0-30%)

**Revision verification:** All 4 recommendations from Audit v1 confirmed addressed:
- R1: Required Artifacts table shows correct dependencies (async-trait, anyhow for topgun-server)
- R2: File count deviation justification preserved in spec
- R3: Unused `use crate::types::Value` import removed from R3 schema.rs code snippet
- R4: Assumption about RequestContext serialization added as final assumption

**Dimension scores:**
- Clarity: Excellent -- literal Rust code for every type and trait, no ambiguity
- Completeness: Excellent -- all files, imports, derives, and doc comments specified
- Testability: Excellent -- 10 acceptance criteria, all verifiable via cargo commands or code inspection
- Scope: Well-bounded -- 7 explicit "DO NOT" constraints
- Feasibility: Sound -- standard Rust trait patterns, minimal dependencies
- Architecture fit: Aligned -- uses SPEC-049 workspace structure, follows Language Profile trait-first pattern
- Non-duplication: Appropriate -- intentional Rust port of existing TS abstractions (IServerStorage 1:1 mapping verified)
- Cognitive load: Low -- placeholder types use simple fields, no unnecessary abstractions
- Strategic fit: Aligned with project goals (directly implements TODO-060, Phase 1 Bridge)
- Project compliance: Honors PROJECT.md decisions (6 traits, no extra deps, unsafe forbidden)

**Language profile:** File count (9) exceeds Rust profile limit (5) -- justified deviation accepted (trait-only work, 4 of 9 are trivial modifications)

**Goal-backward validation:** All 7 observable truths have artifact coverage; all 9 artifacts serve at least one truth; all 4 key links identified and verifiable

**ServerStorage 1:1 mapping verified** against `packages/server/src/storage/IServerStorage.ts`: 9 methods match exactly (initialize, close, load, loadAll, loadAllKeys, store, storeAll, delete, deleteAll) with idiomatic Rust translations (Map -> Vec of tuples, Promise -> async)

**Comment:** Spec is implementation-ready. All Audit v1 recommendations have been properly addressed. No new issues found. The specification provides complete, copy-pasteable Rust code for every file, making implementation straightforward.

---

## Execution Summary

**Executed:** 2026-02-13
**Commits:** 3

### Files Created
- `packages/core-rust/src/types.rs` -- StorageValue, Value, MapType, CrdtMap, Principal
- `packages/core-rust/src/context.rs` -- RequestContext struct
- `packages/core-rust/src/schema.rs` -- MapSchema, FieldDef, ValidationResult, Predicate, SyncShape
- `packages/core-rust/src/traits.rs` -- Processor, ProcessorContext, Inbox, QueryNotifier
- `packages/server-rust/src/traits.rs` -- ServerStorage, MapProvider, SchemaProvider

### Files Modified
- `packages/core-rust/Cargo.toml` -- Added async-trait, serde, anyhow dependencies
- `packages/core-rust/src/lib.rs` -- Added module declarations and re-exports for all 15 public items
- `packages/server-rust/Cargo.toml` -- Added async-trait, anyhow dependencies
- `packages/server-rust/src/lib.rs` -- Added module declaration and re-exports for 3 server traits

### Files Deleted
(none)

### Acceptance Criteria Status
- [x] AC-1: `cargo build --workspace` succeeds with zero errors
- [x] AC-2: `cargo clippy --workspace -- -D warnings` passes with zero warnings
- [x] AC-3: `cargo test --workspace` passes (2 crate_loads tests)
- [x] AC-4: topgun-core exports all 15 specified public items
- [x] AC-5: topgun-server exports ServerStorage, MapProvider, SchemaProvider
- [x] AC-6: All async traits use `#[async_trait]` attribute (Processor, ServerStorage, MapProvider, SchemaProvider)
- [x] AC-7: ServerStorage, MapProvider, SchemaProvider, QueryNotifier all require Send + Sync bounds
- [x] AC-8: Processor requires Send (not Sync)
- [x] AC-9: No impl blocks for any trait. No runtime logic. No unsafe code. Zero impl blocks verified via grep.
- [x] AC-10: `cargo doc --workspace --no-deps` generates without warnings. All types and traits have doc comments.

### Deviations
1. [Rule 1 - Bug] Fixed clippy::doc_markdown warning in `packages/server-rust/src/traits.rs`: backtick-escaped `PostgreSQL` and `SQLite` in ServerStorage doc comment to satisfy pedantic lint.

### Notes
- All code matches the specification's literal Rust snippets with added per-field doc comments (required by clippy pedantic `missing_docs_in_private_items` if enabled, and good practice for AC-10).
- The `schema.rs` file correctly omits the `use crate::types::Value` import per Audit v1 Recommendation 3 (Value not referenced in schema types).
- Cargo.lock updated with 9 new transitive dependencies (async-trait, serde, serde_derive, serde_core, anyhow, proc-macro2, quote, syn, unicode-ident).

---

## Review History

### Review v1 (2026-02-13)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [x] AC-1: `cargo build --workspace` succeeds with zero errors -- verified, clean build
- [x] AC-2: `cargo clippy --workspace -- -D warnings` passes with zero warnings -- verified, zero warnings
- [x] AC-3: `cargo test --workspace` passes -- verified, 2 tests (1 crate_loads per crate), 0 failures
- [x] AC-4: topgun-core exports exactly 15 public items -- verified: `StorageValue`, `Value`, `MapType`, `CrdtMap`, `Principal`, `RequestContext`, `MapSchema`, `FieldDef`, `ValidationResult`, `Predicate`, `SyncShape`, `Processor`, `ProcessorContext`, `Inbox`, `QueryNotifier`
- [x] AC-5: topgun-server exports exactly 3 public items -- verified: `ServerStorage`, `MapProvider`, `SchemaProvider`
- [x] AC-6: All async traits use `#[async_trait]` -- verified: 4 traits (`Processor`, `ServerStorage`, `MapProvider`, `SchemaProvider`); `QueryNotifier` correctly omits it (no async methods)
- [x] AC-7: Send + Sync bounds -- verified: `ServerStorage: Send + Sync`, `MapProvider: Send + Sync`, `SchemaProvider: Send + Sync`, `QueryNotifier: Send + Sync`
- [x] AC-8: Processor bounds -- verified: `Processor: Send` (no `Sync`, correct for `&mut self` methods)
- [x] AC-9: No impl blocks, no runtime logic, no unsafe -- verified via grep: zero `impl` blocks for any of the 6 traits; zero `unsafe` occurrences in both crates
- [x] AC-10: `cargo doc --workspace --no-deps` generates without warnings -- verified; all public types and traits have doc comments, including per-field docs
- [x] R1 compliance: `types.rs` matches spec exactly -- all 5 types with correct derives and field types
- [x] R2 compliance: `context.rs` matches spec exactly -- `RequestContext` with correct 4 fields, no `Serialize`/`Deserialize`
- [x] R3 compliance: `schema.rs` matches spec exactly -- all 5 types, `ValidationResult` correctly omits `Serialize`/`Deserialize`, unused `Value` import correctly omitted per Audit v1 Recommendation 3
- [x] R4 compliance: `traits.rs` (core) matches spec exactly -- `ProcessorContext`, `Inbox`, `Processor`, `QueryNotifier` with correct method signatures
- [x] R5 compliance: `traits.rs` (server) matches spec exactly -- `ServerStorage` (9 methods), `MapProvider` (3 methods), `SchemaProvider` (4 methods) with correct cross-crate imports
- [x] R6 compliance: Module structure and re-exports correct in both `lib.rs` files
- [x] R7 compliance: Dependencies correct -- core has `async-trait`, `serde`, `anyhow`; server has `async-trait`, `anyhow`, `topgun-core` (path dep)
- [x] R8 compliance: No implementation code, no runtime logic
- [x] Constraint: No `main.rs` or binary targets exist
- [x] Constraint: No `PartialEq` on types containing `Vec<u8>`
- [x] Constraint: Only `async-trait`, `serde`, `anyhow` as runtime deps (no `tokio`, `bytes`, etc.)
- [x] Constraint: Existing `crate_loads` tests preserved in both crates
- [x] Constraint: `unsafe_code = "forbid"` workspace lint active
- [x] Cross-crate wiring: `topgun-server::traits.rs` imports 8 types from `topgun_core` -- compiles correctly
- [x] Observable Truth 7: `SchemaProvider.get_shape` returns `SyncShape` with `RequestContext` parameter -- cross-type wiring compiles
- [x] No files to delete (spec specifies none)
- [x] Deviation documented: backtick-escaping `PostgreSQL` and `SQLite` for clippy doc_markdown lint -- appropriate fix
- [x] No security issues: no secrets, no unsafe, no user input handling, no network code
- [x] Architecture: follows Language Profile trait-first pattern, workspace structure from SPEC-049
- [x] Non-duplication: intentional Rust port of TS abstractions, no copy-paste within Rust crates
- [x] Cognitive load: minimal -- placeholder structs with simple fields, standard Rust trait patterns, clear doc comments

**Rust Idiom Check:**
- [x] No unnecessary `.clone()` calls (no runtime code at all)
- [x] Error handling uses `anyhow::Result` appropriately for trait definitions
- [x] No `.unwrap()` or `.expect()` in production code
- [x] No `unsafe` blocks (workspace-level `forbid`)
- [x] Proper `Send + Sync` bounds on all concurrency-relevant traits
- [x] No `Box<dyn Any>` type erasure

**Implementation Reality Check:**
- No strategic concerns. The implementation is a faithful, near-verbatim translation of the specification's literal Rust code snippets. The only deviation (backtick-escaping in doc comments) is a necessary fix for clippy pedantic compliance. The scope is well-bounded to trait/type definitions with no runtime logic, exactly as specified.

**Summary:** Exemplary implementation. Every type, trait, derive, bound, method signature, doc comment, dependency, and re-export matches the specification exactly. All 10 acceptance criteria pass. All 7 constraints are respected. The single deviation (doc comment escaping for clippy) is a correct and necessary fix. Build, clippy, test, and doc generation all succeed with zero errors and zero warnings. No issues found.

---

## Completion

**Completed:** 2026-02-13
**Total Commits:** 3
**Audit Cycles:** 2
**Review Cycles:** 1
