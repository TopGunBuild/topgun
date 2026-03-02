---
id: SPEC-071
type: feature
status: done
priority: P0
complexity: medium
created: 2026-02-28
todo: TODO-097
---

# SPEC-071: Server-Side Write Validation and HLC Sanitization

## Context

Without server-side validation, any client can overwrite any key in any map and set HLC timestamps arbitrarily far in the future to "win" all LWW conflicts permanently. This makes the system unusable in production.

The security layer sits BEFORE the CRDT merge in the pipeline (same approach as Ditto and Firebase):

```
Client write -> Auth check -> Map ACL check -> HLC sanitization -> CRDT merge -> Persist
```

CRDTs handle conflict resolution; the security layer handles authorization and timestamp integrity.

**Current state:**
- `CrdtService.apply_single_op()` directly trusts client-provided `LWWRecord.timestamp` and `ORMapRecord.timestamp` fields
- `OperationContext.timestamp` is generated server-side by `OperationService.classify()`, but is NOT used for CRDT records -- it is metadata only
- `ConnectionMetadata` already has `authenticated: bool` and `principal: Option<Principal>`, but these are never checked before writes
- No map-level access control exists
- No value size limits exist

**Reference:** STRATEGIC_RECOMMENDATIONS.md Section 5 (Security Model)

## Goal Statement

After this spec, the server rejects unauthorized writes and replaces client-provided HLC timestamps with server-generated ones, preventing timestamp manipulation and unauthorized data access.

### Observable Truths

1. An unauthenticated connection that sends a `ClientOp` receives an `OpRejected` error, and no data is written to the `RecordStore`
2. An authenticated connection without write permission for the target map receives an `OpRejected` error, and no data is written
3. An authenticated connection with write permission successfully writes, and the stored `LWWRecord.timestamp` / `ORMapRecord.timestamp` uses the server's HLC, not the client's
4. A `ClientOp` with a serialized value exceeding the configured maximum size is rejected with an `OpRejected` error
5. `OpBatch` operations apply the same validation per-op: one rejected op rejects the entire batch
6. Operations from `CallerOrigin::Forwarded`, `Backup`, `Wan`, or `System` bypass auth and ACL checks (trusted server-to-server traffic)
7. Map permissions are stored per-connection in `ConnectionMetadata.map_permissions` and are readable by the validation layer; this spec reads them but does not define a setter API (setting is the responsibility of the auth adapter)

### Required Artifacts

| Artifact | Purpose |
|----------|---------|
| `packages/server-rust/src/service/security.rs` (NEW) | `SecurityConfig`, `WriteValidator` with auth check, ACL check, size check, HLC sanitization |
| `packages/server-rust/src/service/domain/crdt.rs` (MOD) | Call `WriteValidator` before `apply_single_op`; use sanitized timestamps |
| `packages/server-rust/src/service/config.rs` (MOD) | Add `SecurityConfig` fields to `ServerConfig` |
| `packages/server-rust/src/service/operation.rs` (MOD) | Add `Unauthorized`, `Forbidden`, and `ValueTooLarge` variants to `OperationError` |
| `packages/server-rust/src/network/connection.rs` (MOD) | Add `MapPermissions` struct, `map_permissions: HashMap<String, MapPermissions>` field, and `Clone` derive to `ConnectionMetadata` |
| `packages/server-rust/src/service/mod.rs` (MOD) | Add `pub mod security;` module declaration and re-export `SecurityConfig`, `WriteValidator` |
| `packages/server-rust/src/lib.rs` (MOD) | Wire `WriteValidator` into `CrdtService::new()` call sites |

**Note on file count:** This spec touches 7 files. The split is unavoidable: `lib.rs` must be modified because `CrdtService::new()` gains a required parameter; `MapPermissions` must live in `network/connection.rs` to avoid a circular module dependency (see Requirement 3); `service/mod.rs` must be updated to declare the new `security` module (see Requirement 7). Consolidating further would require moving `WriteValidator` out of `service/`, which would obscure its domain ownership.

## Task

Create a write validation layer that enforces authentication, map-level ACL, value size limits, and HLC sanitization on all client write operations before they reach CRDT merge.

## Requirements

### 1. New File: `packages/server-rust/src/service/security.rs`

#### 1.1 `SecurityConfig` struct

```rust
#[derive(Debug, Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityConfig {
    /// When true, all write operations require `ConnectionMetadata.authenticated == true`.
    /// When false, unauthenticated connections are allowed to write (development mode).
    pub require_auth: bool,
    /// Maximum serialized value size in bytes for a single ClientOp record.
    /// 0 means unlimited. Uses u64 (not usize) so this value is stable across
    /// 32-bit and 64-bit platforms and can be safely stored in config files.
    pub max_value_bytes: u64,
    /// Default permissions for maps not explicitly configured.
    pub default_permissions: MapPermissions,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            require_auth: false,
            max_value_bytes: 0,
            default_permissions: MapPermissions::default(),
        }
    }
}
```

Default is permissive: `require_auth: false`, unlimited size, default read+write. This ensures backward compatibility -- existing deployments without security config continue working.

#### 1.2 `WriteValidator` struct

```rust
pub struct WriteValidator {
    config: Arc<SecurityConfig>,
    hlc: Arc<parking_lot::Mutex<HLC>>,
}
```

Methods:

- `new(config: Arc<SecurityConfig>, hlc: Arc<parking_lot::Mutex<HLC>>) -> Self`
- `validate_write(&self, ctx: &OperationContext, metadata: &ConnectionMetadata, map_name: &str, value_size: u64) -> Result<(), OperationError>`:
  - If `ctx.caller_origin` is NOT `CallerOrigin::Client`, return `Ok(())` (trusted traffic bypasses all checks)
  - If `config.require_auth` is true AND `metadata.authenticated` is false, return `Err(OperationError::Unauthorized)`
  - Look up `map_name` in `metadata.map_permissions`; fall back to `config.default_permissions`. If `write` is false, return `Err(OperationError::Forbidden { map_name })`
  - If `config.max_value_bytes > 0` AND `value_size > config.max_value_bytes`, return `Err(OperationError::ValueTooLarge { size: value_size, max: config.max_value_bytes })`
  - Return `Ok(())`
- `sanitize_hlc(&self) -> Timestamp`: generates a fresh server HLC timestamp via `self.hlc.lock().now()`

**Note on `value_size` type:** `value_size` is `u64` rather than `usize`. `Vec<u8>::len()` returns `usize`; callers must cast with `as u64`. This is intentional: `u64` is stable across 32-bit and 64-bit platforms and matches `SecurityConfig.max_value_bytes`.

### 2. Modify: `packages/server-rust/src/service/operation.rs`

Add three new variants to `OperationError`:

```rust
#[error("authentication required")]
Unauthorized,

#[error("write access denied for map: {map_name}")]
Forbidden { map_name: String },

#[error("value size {size} bytes exceeds maximum {max} bytes")]
ValueTooLarge { size: u64, max: u64 },
```

### 3. Modify: `packages/server-rust/src/network/connection.rs`

`MapPermissions` is defined here -- not in `service/security.rs` -- to avoid a circular module dependency. Currently the dependency is one-way: `service` imports from `network` (e.g., `ConnectionRegistry`, `ConnectionId`). Placing `MapPermissions` in `service/security.rs` would require `network` to import from `service`, creating a cycle. As a plain data struct with no service logic, `MapPermissions` belongs in `network/connection.rs`.

Add `MapPermissions` struct and extend `ConnectionMetadata`. Also add `Clone` to `ConnectionMetadata`'s derive list so the metadata snapshot pattern in Requirement 5 (`handle.metadata.read().await.clone()`) compiles. All fields on `ConnectionMetadata` are individually `Clone`-eligible (`bool`, `Option<Principal>` where `Principal: Clone`, `HashSet<String>`, `Instant`, `Option<Timestamp>`, `Option<String>`, `HashMap<String, MapPermissions>`), so adding the derive is safe.

```rust
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapPermissions {
    pub read: bool,
    pub write: bool,
}

impl Default for MapPermissions {
    fn default() -> Self {
        Self { read: true, write: true }
    }
}

#[derive(Debug, Clone)]   // Clone added: required for metadata snapshot pattern
pub struct ConnectionMetadata {
    // ... existing fields ...
    /// Per-map access permissions for this connection.
    /// Maps not present here use the server's SecurityConfig.default_permissions.
    pub map_permissions: HashMap<String, MapPermissions>,
}
```

Update `Default` impl to include `map_permissions: HashMap::new()`.

In `service/security.rs`, import `MapPermissions` from `network`:

```rust
use crate::network::connection::MapPermissions;
```

### 4. Modify: `packages/server-rust/src/service/config.rs`

Add `security` field to `ServerConfig`:

```rust
use super::security::SecurityConfig;

pub struct ServerConfig {
    // ... existing fields ...
    /// Security configuration for write validation.
    pub security: SecurityConfig,
}
```

Update `Default` impl to include `security: SecurityConfig::default()`.

### 5. Modify: `packages/server-rust/src/service/domain/crdt.rs`

Add `WriteValidator` as a dependency of `CrdtService`:

```rust
pub struct CrdtService {
    record_store_factory: Arc<RecordStoreFactory>,
    connection_registry: Arc<ConnectionRegistry>,
    write_validator: Arc<WriteValidator>,
}
```

Modify `CrdtService::new()` to accept `write_validator: Arc<WriteValidator>`.

#### Metadata snapshot pattern

Both `handle_client_op` and `handle_op_batch` must read `ConnectionMetadata` before validation. The metadata is behind `Arc<tokio::sync::RwLock<ConnectionMetadata>>` on the `ConnectionHandle`. Use the following pattern:

```rust
// Clone the metadata snapshot once; do not hold the read guard across async calls.
let metadata_snapshot: ConnectionMetadata = {
    let handle = connection_registry.get(ctx.connection_id)?;
    handle.metadata.read().await.clone()
};
```

Pass `&metadata_snapshot` to `validate_write`. This avoids holding the `RwLock` read guard across async storage operations, which would cause contention if the auth adapter updates metadata concurrently. `ConnectionMetadata` derives `Clone` (added in Requirement 3) so the `.clone()` call compiles.

**If `connection_id` is `Some(id)` and the connection is not found in the registry** (e.g., the connection disconnected between routing and handling), treat the request as unauthorized: return `Err(OperationError::Unauthorized)`. This is the safe default -- a missing connection context cannot be trusted.

#### Modify `handle_client_op`

1. If `ctx.connection_id` is `None`, skip validation (internal/system call -- maintains test compatibility)
2. If `ctx.connection_id` is `Some(id)`, acquire metadata snapshot as described above; if connection not found, return `Err(OperationError::Unauthorized)`
3. Estimate value size from the `ClientOp` payload (see Value size estimation below)
4. Call `write_validator.validate_write(ctx, &metadata_snapshot, &op.map_name, value_size)?`
5. Generate sanitized timestamp via `write_validator.sanitize_hlc()`
6. Replace `op.record.timestamp` / `op.or_record.timestamp` with the sanitized timestamp before passing to `apply_single_op`

#### Modify `handle_op_batch`

1. If `ctx.connection_id` is `None`, skip validation for all ops (internal/system call)
2. If `ctx.connection_id` is `Some(id)`, acquire **one** metadata snapshot at batch start; if connection not found, return `Err(OperationError::Unauthorized)` immediately
3. Validate ALL ops against the snapshot before applying ANY op (atomic batch rejection)
4. If any op fails validation, return error immediately -- no ops are applied
5. After all ops pass validation, apply all ops sequentially; each op gets its own sanitized HLC timestamp (monotonically increasing via successive `sanitize_hlc()` calls)

**Locking strategy for OpBatch:** Clone the metadata snapshot once at the start of batch processing. Validate all ops against this single snapshot. This avoids re-acquiring the lock per-op and is architecturally sound: metadata permissions do not change mid-batch in any defined usage path. The snapshot approach matches Hazelcast's transactional batch semantics.

#### Timestamp replacement in `apply_single_op`

The method currently takes `&ClientOp` (borrowed). Create a sanitized copy before calling `apply_single_op`:

- For LWW PUT: clone the `LWWRecord`, replace `.timestamp` with the sanitized timestamp
- For OR_ADD: clone the `ORMapRecord`, replace `.timestamp` with the sanitized timestamp, regenerate `.tag` using the format `"{millis}:{counter}:{node_id}"` where millis, counter, and node_id are taken from the sanitized `Timestamp`. For example: `format!("{}:{}:{}", ts.millis, ts.counter, ts.node_id)`. This matches the existing tag generation convention used in the codebase.
- For REMOVE and OR_REMOVE: no timestamp sanitization needed (removes are idempotent and tag-based)

#### Value size estimation

Use `rmp_serde::to_vec_named()` on the `ClientOp.record` or `ClientOp.or_record` to get the serialized byte length. Cast the `usize` result to `u64` before passing to `validate_write`. This reuses the existing serialization codec and provides an accurate size. If serialization fails, treat the value size as exceeding the limit (return `Err(OperationError::ValueTooLarge { size: u64::MAX, max: config.max_value_bytes })`).

For REMOVE and OR_REMOVE operations, pass `value_size = 0` to `validate_write`. Since no data payload is being written, removes always pass the size check. This avoids the need to serialize a None/absent record field for size estimation.

### 6. Modify: `packages/server-rust/src/lib.rs`

Update all `CrdtService::new()` call sites to pass an `Arc<WriteValidator>`. There are at least two call sites (production wiring and integration tests). For integration tests, construct `WriteValidator` with `Arc::new(SecurityConfig::default())` and the existing server HLC -- this produces a permissive validator that preserves existing test behavior.

### 7. Modify: `packages/server-rust/src/service/mod.rs`

Add the module declaration for `security.rs` and re-export the key types:

```rust
pub mod security;

pub use security::{SecurityConfig, WriteValidator};
```

This declaration must be added as part of G1 (Wave 1) since it gates compilation of all code that references `crate::service::security`. The re-exports of `SecurityConfig` and `WriteValidator` follow the existing `service/mod.rs` convention of re-exporting key types (`ServerConfig`, `OperationError`, `CallerOrigin`) for use by integration tests and future modules.

## Acceptance Criteria

1. **AC1:** When `SecurityConfig.require_auth` is true, a `ClientOp` from an unauthenticated connection returns `OperationError::Unauthorized` and writes nothing to the `RecordStore`
2. **AC2:** When `SecurityConfig.require_auth` is true, a `ClientOp` from an authenticated connection with `map_permissions[map_name].write == false` returns `OperationError::Forbidden` and writes nothing
3. **AC3:** When `SecurityConfig.require_auth` is true, a `ClientOp` from an authenticated connection with write permission succeeds and returns `OpAck`
4. **AC4:** After a successful LWW PUT, the `RecordValue::Lww.timestamp.node_id` in the `RecordStore` matches the server's `node_id`, NOT the client's original `node_id`
5. **AC5:** After a successful OR_ADD, the `OrMapEntry.timestamp.node_id` in the `RecordStore` matches the server's `node_id`
6. **AC6:** When `max_value_bytes` is set to N, a `ClientOp` with a serialized record larger than N bytes returns `OperationError::ValueTooLarge`
7. **AC7:** When `max_value_bytes` is set to N, a `ClientOp` with a serialized record of exactly N bytes succeeds
8. **AC8:** An `OpBatch` where the 2nd of 3 ops fails validation returns an error, and the 1st op's data is NOT present in the `RecordStore` (atomic batch rejection)
9. **AC9:** A `ClientOp` with `CallerOrigin::Forwarded` bypasses all auth/ACL/size checks and succeeds regardless of authentication state
10. **AC10:** A `ClientOp` with `CallerOrigin::System` bypasses all auth/ACL/size checks
11. **AC11:** When `SecurityConfig.require_auth` is false (default), unauthenticated writes succeed (backward compatibility)
12. **AC12:** When `SecurityConfig.max_value_bytes` is 0 (default), no size limit is enforced
13. **AC13:** `MapPermissions` implements `Default` with `read: true, write: true`
14. **AC14:** `ConnectionMetadata.map_permissions` defaults to empty `HashMap` (falls back to `SecurityConfig.default_permissions`)
15. **AC15:** `WriteValidator::sanitize_hlc()` returns a `Timestamp` with `node_id` matching the server's HLC node
16. **AC16:** Each op in an `OpBatch` receives a distinct sanitized HLC timestamp (monotonically increasing)
17. **AC17:** `ManagedService::name()` for `CrdtService` still returns `"crdt"`
18. **AC18:** Existing `CrdtService` tests continue to pass with default (permissive) `SecurityConfig`
19. **AC19:** When `ctx.connection_id` is `Some(id)` and the connection is not found in the registry, `handle_client_op` returns `OperationError::Unauthorized` and writes nothing
20. **AC20:** A REMOVE or OR_REMOVE `ClientOp` is never rejected due to value size (size is treated as 0 for removal ops)

## Constraints

- Do NOT add field-level or row-level ACL -- map-level only
- Do NOT modify the `Message` wire format -- security is server-internal
- Do NOT add JWT verification or token parsing -- that is a separate concern (auth adapter). This spec checks `ConnectionMetadata.authenticated` which is set by the auth layer
- Do NOT block reads based on ACL in this spec -- read ACL enforcement is separate (queries, sync, subscriptions each need their own check points)
- Do NOT add an `OpRejected` wire message in this spec -- return `OperationError` variants that the caller (WebSocket handler) will translate to the appropriate wire format
- Validation runs synchronously before any async storage calls -- a rejected write never touches the `RecordStore`

## Assumptions

- The WebSocket handler (or future dispatch layer) is responsible for looking up `ConnectionHandle` by `connection_id` and passing metadata to the service. If `connection_id` is `None` on the `OperationContext` (which happens in current tests), validation is skipped (treated as internal/system). This maintains test compatibility.
- The `OpBatch` atomic rejection means "all or nothing" -- if validation fails for any op, the entire batch fails before any op is applied. This is simpler than partial application and matches Hazelcast's transactional batch semantics.
- The `ConnectionMetadata.map_permissions` HashMap is populated by an external auth adapter (e.g., BetterAuth integration, JWT claims) that is outside the scope of this spec. This spec only reads the permissions; it does not set them.
- OR_REMOVE operations do not require timestamp sanitization because they are tag-based removals, not timestamp-based wins.
- Value size is measured as the MsgPack-serialized byte length of the `record` or `or_record` field, not the entire `ClientOp` message. For REMOVE and OR_REMOVE ops, value size is defined as 0.

## Goal Analysis

### Key Links (fragile connections)

1. **CrdtService -> WriteValidator**: The `CrdtService` must call `validate_write` BEFORE `apply_single_op`. If this ordering is violated, unauthorized writes reach storage.
2. **WriteValidator -> ConnectionMetadata**: The validator reads `map_permissions` and `authenticated` from a cloned metadata snapshot. The snapshot is taken once per operation (or once per batch) by acquiring and immediately releasing the `RwLock` read guard.
3. **HLC sanitization -> RecordStore**: The sanitized timestamp must be the one stored, not the original. If `apply_single_op` accidentally uses the original `ClientOp.record.timestamp`, the entire security model is bypassed.
4. **OpBatch atomicity**: The batch loop must validate ALL ops before applying ANY. If validation and application are interleaved, partial writes can escape on batch failure. The metadata snapshot is cloned once at batch start.

### Implementation Tasks

#### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Types and traits: `MapPermissions` (in connection.rs), `SecurityConfig`, new `OperationError` variants, `ConnectionMetadata.map_permissions` field + `Clone` derive, `pub mod security;` in `service/mod.rs` | -- | ~20% |
| G2 | 2 | `WriteValidator` implementation: auth check, ACL check, size check, HLC sanitization | G1 | ~25% |
| G3a | 3 | `CrdtService` integration: inject `WriteValidator`, metadata snapshot pattern, validate before merge, replace timestamps in `handle_client_op` | G1, G2 | ~25% |
| G3b | 4 | Atomic batch rejection: validate-all-then-apply-all in `handle_op_batch`, locking strategy | G1, G2, G3a | ~15% |
| G4 | 4 | Tests: unit tests for `WriteValidator`, integration tests for `CrdtService` with security enabled, `lib.rs` wiring | G1, G2, G3a | ~15% |

#### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1 | No | 1 |
| 2 | G2 | No | 1 |
| 3 | G3a | No | 1 |
| 4 | G3b, G4 | Yes | 2 |

**Note on G3a/G3b sequencing:** G3b (atomic batch rejection in `handle_op_batch`) is in Wave 4, not Wave 3, because both G3a and G3b modify `crdt.rs`. Running them in parallel would cause merge conflicts. G3b depends on G3a to establish the `WriteValidator` injection and metadata snapshot pattern that batch handling reuses.

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-02-28)
**Status:** NEEDS_REVISION

**Context Estimate:** ~100% total (sum of group estimates)

**Critical:**

1. **Circular module dependency: `network` -> `service`**. The spec adds `use crate::service::security::MapPermissions` to `packages/server-rust/src/network/connection.rs`. Currently the dependency is one-way: `service` imports from `network` (e.g., `ConnectionRegistry`, `ConnectionId`). Adding the reverse direction creates a circular module dependency (`network` <-> `service`). While Rust allows intra-crate circular `use` statements, this violates the established architectural layering where `network` is a lower-level module than `service`. **Fix:** Move `MapPermissions` to a location accessible to both modules without circular dependency. Options: (a) Define `MapPermissions` directly in `network/connection.rs` (simplest -- it is a plain data struct with no service logic); (b) Create a shared `types` module; (c) Move it to `core-rust` since it is a pure data type with serde derives.

2. **`validate_write` takes `&ConnectionMetadata` but `ConnectionMetadata` is behind `Arc<RwLock<ConnectionMetadata>>`**. The spec says `handle_client_op` should "Read `metadata` from the handle" and call `validate_write(ctx, &metadata, ...)`. But `ConnectionHandle.metadata` is `Arc<RwLock<ConnectionMetadata>>`. This means the caller must `metadata.read().await` to get a read guard, and pass a reference from the guard. The spec should explicitly state this is an async read lock operation within `handle_client_op` and `handle_op_batch`. More importantly, `validate_write` is described as synchronous but it needs to hold a tokio `RwLock` read guard. The spec must clarify that validation occurs while holding the read guard, or that metadata is cloned out of the lock before validation. This is a correctness concern -- holding the read lock across the validation + apply sequence could cause contention or deadlock if metadata is updated concurrently.

3. **OpBatch "validate ALL then apply ALL" requires holding metadata read locks for all ops simultaneously or re-acquiring them**. The spec states: "The batch loop must validate ALL ops before applying ANY." For a batch, this means either: (a) read metadata once, validate all ops, then apply all ops (but metadata could change between validate and apply -- unlikely but architecturally unsound), or (b) clone metadata snapshot once, validate all against it, then apply. The spec must explicitly specify the locking strategy. Recommendation: clone the metadata snapshot once at batch start, validate all ops against the snapshot, then apply all ops sequentially.

4. **G3 estimated at ~35% exceeds the per-group 30% target**. The CrdtService integration group involves reading the existing 727-line `crdt.rs` file, understanding the complex `apply_single_op` branching logic (4 cases: REMOVE, OR_ADD, OR_REMOVE, LWW PUT), modifying `handle_client_op` and `handle_op_batch`, adding timestamp replacement logic, and updating the constructor signature plus all call sites in `lib.rs`. This is too large for a single group and risks quality degradation.

5. **Missing update for `lib.rs` (and integration tests in `lib.rs`)**. The spec modifies `CrdtService::new()` to require `write_validator: Arc<WriteValidator>`, but does not list `lib.rs` as a modified file. The `lib.rs` file contains at least 3 call sites of `CrdtService::new()` (lines 87, 320 in `integration_tests`). All of these will fail to compile after the constructor change. The spec must either: (a) add `lib.rs` to the artifacts list (but this would bring the file count to 6, exceeding the Language Profile max of 5), or (b) restructure to avoid changing the `CrdtService::new()` signature -- e.g., make `write_validator` optional with a default, or provide a builder.

**Recommendations:**

6. **[Compliance] File count is exactly 5, at the Language Profile limit.** Since the `lib.rs` wiring update is required (Critical #5), the actual file count is 6. Consider either: (a) combining `SecurityConfig` into `config.rs` instead of creating a separate `security.rs` file (putting `MapPermissions`, `SecurityConfig`, and `WriteValidator` all in the existing `config.rs` or `operation.rs`), or (b) splitting the spec so that G1 changes (types + error variants) are one spec and G2-G4 (validator + integration) are another.

7. **[Strategic] Observable Truth #7 ("Map permissions can be set and queried per connection via the ConnectionHandle") has no corresponding acceptance criterion.** The spec only reads permissions -- it never sets them. Truth #7 implies a setter API, but the Assumptions section says "This spec only reads the permissions; it does not set them." Remove Truth #7 or clarify it describes the existing `map_permissions` field accessibility, not a new API.

8. **`SecurityConfig` should have `#[serde(rename_all = "camelCase")]` for Rust auditor checklist compliance.** The struct is not currently annotated with serde derives, but if it is ever serialized (e.g., for config files or admin APIs), it should follow the convention. Since `MapPermissions` already has serde derives and `rename_all`, `SecurityConfig` should be consistent. At minimum, add a comment explaining why serde is omitted if intentional.

9. **The `handle_client_op` metadata lookup can fail (connection not found).** The spec says "If `connection_id` is `None`, validation is skipped." But what if `connection_id` is `Some(id)` and the connection is not found in the registry (e.g., disconnected between routing and handling)? The spec should define this behavior explicitly -- likely return an `OperationError::Internal` or treat as unauthorized.

10. **Tag regeneration for OR_ADD is underspecified.** The spec says "regenerate `.tag` from the new timestamp" but does not specify the tag format. The existing code uses tags like `"1700000000000:1:test-node"` (millis:counter:node_id). The spec should reference this format or point to the code that generates it so the implementer knows the exact algorithm.

11. **Consider using `usize` instead of `u64` for `max_value_bytes` and `value_size`.** While `u64` is fine, Rust serialization functions return `Vec<u8>` whose length is `usize`. Using `u64` requires casting from `usize`. Not critical, but `usize` is more idiomatic for in-process byte lengths. However, if this will ever be part of a config file that crosses 32-bit boundaries, `u64` is safer. Keep `u64` but add a note.

### Response v1 (2026-02-28)
**Applied:** All 5 critical issues and all 6 recommendations

**Changes:**
1. [✓] Circular module dependency -- Moved `MapPermissions` definition to `network/connection.rs`. `service/security.rs` now imports it via `use crate::network::connection::MapPermissions`. Dependency remains one-way: `service` imports from `network`. Requirement 3 updated with architectural rationale.
2. [✓] `validate_write` signature vs `RwLock<ConnectionMetadata>` mismatch -- Added explicit "Metadata snapshot pattern" section in Requirement 5 specifying `handle.metadata.read().await.clone()` to clone out of the lock immediately. Both `handle_client_op` and `handle_op_batch` documented to use this pattern.
3. [✓] OpBatch validate-then-apply locking strategy -- Added "Locking strategy for OpBatch" subsection: clone metadata snapshot once at batch start, validate all ops, then apply all ops sequentially. Rationale and Hazelcast reference included.
4. [✓] G3 at ~35% exceeds per-group 30% target -- Split G3 into G3a (~25%, CrdtService integration: inject, validate, replace timestamps) and G3b (~15%, atomic batch rejection and locking). Wave 3 now has G3a, G3b, G4 running in parallel with 3 workers.
5. [✓] Missing `lib.rs` from artifact list -- Added `lib.rs` (MOD) to Required Artifacts. Added Requirement 6 describing the wiring update. Added note in Required Artifacts explaining why 6 files is unavoidable.
6. [✓] File count at Language Profile limit -- Acknowledged in Required Artifacts note. Did not split the spec (splitting would separate tightly coupled G1 types from G2-G4 implementation, harming coherence). The 6-file justification is documented inline.
7. [✓] Observable Truth #7 contradicts Assumptions -- Revised Truth #7 to clarify it describes the existing `map_permissions` field accessibility, not a setter API. Now consistent with Assumptions section.
8. [✓] `SecurityConfig` missing serde derives -- Added `#[derive(serde::Serialize, serde::Deserialize)]` and `#[serde(rename_all = "camelCase")]` to `SecurityConfig` in Requirement 1.1.
9. [✓] Missing behavior for `connection_id = Some(id)` when connection not found -- Specified explicitly in `handle_client_op` steps and `handle_op_batch` steps: return `Err(OperationError::Unauthorized)`. Added AC19 covering this behavior.
10. [✓] OR_ADD tag regeneration format unspecified -- Added explicit format: `format!("{}:{}:{}", ts.millis, ts.counter, ts.node_id)`. References the existing convention used in the codebase.
11. [✓] `u64` vs `usize` for `max_value_bytes` -- Kept `u64`. Added inline note in Requirement 1.1 explaining the rationale (platform stability, config file safety). Added cast instruction in Value size estimation section: `as u64` from `Vec<u8>::len()`.

### Audit v2 (2026-02-28)
**Status:** NEEDS_REVISION

**Context Estimate:** ~100% total (sum of group estimates: 20+25+25+15+15)

**Audit Dimensions:**
- Clarity: Strong. Requirements are specific with code snippets.
- Completeness: One critical gap (see below).
- Testability: All 19 ACs are measurable and concrete.
- Scope: Well-bounded by constraints.
- Feasibility: Sound technical approach.
- Architecture fit: Aligns with existing service/domain patterns.
- Non-duplication: No reinvention; reuses existing HLC, ConnectionRegistry.
- Cognitive load: Reasonable for a security layer insertion.
- Strategic fit: P0 security for production readiness -- well justified.
- Project compliance: Compliant (see details below).

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields (`max_value_bytes: u64`, `value_size: u64`)
- [x] No `r#type: String` on message structs (N/A)
- [x] `Default` derived on payload structs with 2+ optional fields (N/A -- no optional fields)
- [x] Enums used for known value sets (N/A)
- [x] Wire compatibility: `rmp_serde::to_vec_named()` used for size estimation
- [x] `#[serde(rename_all = "camelCase")]` on `SecurityConfig` and `MapPermissions`
- [x] `#[serde(skip_serializing_if = "Option::is_none", default)]` on every `Option<T>` (N/A -- no `Option<T>` fields in new structs)

**Goal-Backward Validation:**
| Check | Status | Issue |
|-------|--------|-------|
| Truth 1 has artifacts | OK | `WriteValidator` + `OperationError::Unauthorized` |
| Truth 2 has artifacts | OK | `WriteValidator` + `MapPermissions` + `OperationError::Forbidden` |
| Truth 3 has artifacts | OK | `WriteValidator.sanitize_hlc()` + `CrdtService` integration |
| Truth 4 has artifacts | OK | `WriteValidator.validate_write()` size check |
| Truth 5 has artifacts | OK | `handle_op_batch` validate-all-then-apply-all |
| Truth 6 has artifacts | OK | `validate_write` caller_origin bypass |
| Truth 7 has artifacts | OK | `ConnectionMetadata.map_permissions` field |
| Key Link 1 (ordering) | OK | Spec explicitly states validate BEFORE apply |
| Key Link 2 (snapshot) | OK | Metadata snapshot pattern documented |
| Key Link 3 (timestamp) | OK | Explicit timestamp replacement per op type |
| Key Link 4 (atomicity) | OK | Validate-all-then-apply-all documented |

**Strategic Sanity Check:**
| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | Auth adapter sets `ConnectionMetadata.authenticated` before writes | Validation always passes unauth check if metadata never set |
| A2 | Auth adapter populates `map_permissions` | All maps fall back to default_permissions (permissive by default) |
| A3 | `connection_id` is `None` only for internal/test calls | Skipping validation for `None` could be a bypass if WebSocket handler has bugs |
| A4 | MsgPack serialization size is a good proxy for value size | Different from in-memory size, but consistent and deterministic |

Strategic fit: Aligned with project goals. Security is a P0 prerequisite for production readiness. The validate-before-merge pattern mirrors Ditto and Firebase -- proven architectures.

**Project Compliance:**
| Decision | Spec Compliance | Status |
|----------|-----------------|--------|
| MsgPack wire format | Spec uses `rmp_serde::to_vec_named()` | Compliant |
| `#[serde(rename_all = "camelCase")]` | Applied to both new structs | Compliant |
| No phase/spec references in code | No code comments with spec references | Compliant |
| Commit format | Not applicable at spec level | N/A |
| Trait-first ordering | G1 defines types, G2+ implements | Compliant |

**Language Profile Check:**
- File count: 6 files, exceeds max of 5 by 1. Acknowledged and justified in spec. The 7th file (`service/mod.rs`) is also needed (see Recommendation 1).
- Trait-first: G1 (Wave 1) contains only types/structs/error variants. Compliant.
- Compilation gate: No single group modifies more than 3 non-trivial files. OK.

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~100% | <=50% | Exceeded |
| Largest task group | ~25% (G2 or G3a) | <=30% | OK |
| Worker overhead | ~15% (3 waves) | <=10% | Warning |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | - |
| 30-50% | GOOD | - |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | -- Current estimate |

**Scope:** Large (~100% estimated, significantly exceeds 50% target). The spec NEEDS_DECOMPOSITION for execution, but the task groups are already defined and individually within bounds. The `/sf:run --parallel` mode with the existing wave structure is appropriate.

**Critical:**

1. **`ConnectionMetadata` does not derive `Clone` -- metadata snapshot pattern will not compile.** The spec's Requirement 5 uses `handle.metadata.read().await.clone()` to create a metadata snapshot. However, `ConnectionMetadata` at `/Users/koristuvac/Projects/topgun/topgun/packages/server-rust/src/network/connection.rs` line 112 currently derives only `Debug`, not `Clone`. All its fields are individually Clone-eligible (`bool`, `Option<Principal>` where `Principal: Clone`, `HashSet<String>`, `Instant`, `Option<Timestamp>`, `Option<String>`), but the struct itself lacks the derive. The spec must explicitly state that `#[derive(Debug, Clone)]` (or at minimum `Clone`) is added to `ConnectionMetadata` as part of the `connection.rs` modifications in Requirement 3. Without this, the code will fail to compile.

**Recommendations:**

2. **Missing `pub mod security;` in `service/mod.rs`.** Creating `packages/server-rust/src/service/security.rs` requires adding `pub mod security;` to `packages/server-rust/src/service/mod.rs`. This is a 7th file touched (though trivially -- a single line addition). The spec should mention this explicitly so the implementer does not overlook it. It can be handled as part of G1 since it is a one-line module declaration.

3. **[Compliance] Consider re-exporting `SecurityConfig` and `WriteValidator` from `service/mod.rs`.** The existing `service/mod.rs` re-exports key types like `ServerConfig`, `OperationError`, `CallerOrigin`. For consistency, `SecurityConfig` and `WriteValidator` should also be re-exported if they are intended for external use (e.g., by integration tests or future modules). At minimum, `SecurityConfig` should be re-exported since `ServerConfig.security` exposes it.

4. **Value size estimation for REMOVE operations.** The spec says to use `rmp_serde::to_vec_named()` on `ClientOp.record` or `ClientOp.or_record`. For REMOVE operations, both fields may be `None` or `Some(None)`. The spec should clarify that size validation is skipped for REMOVE and OR_REMOVE operations (since no data is being written), or specify what value size to use. Since `validate_write` is called before the op-type branching, the implementer needs guidance on what to pass as `value_size` when the op is a REMOVE. Suggested approach: pass 0 for REMOVE/OR_REMOVE ops, which always passes the size check.

5. **G3a and G3b share the same file (`crdt.rs`) but are marked as parallel in Wave 3.** Both G3a (inject WriteValidator, metadata snapshot, validate before merge, replace timestamps) and G3b (atomic batch rejection in `handle_op_batch`) modify `crdt.rs`. If two workers attempt to modify the same file in parallel, merge conflicts are inevitable. Either: (a) merge G3a and G3b into a single group (the combined ~40% is high but manageable if properly segmented), or (b) make G3b depend on G3a (sequential, not parallel). Option (b) is recommended since G3b's batch logic depends on the validation infrastructure G3a introduces.

### Response v2 (2026-02-28)
**Applied:** All 5 items (1 critical + 4 recommendations)

**Changes:**
1. [✓] `ConnectionMetadata` missing `Clone` derive -- Added `Clone` to the `#[derive(Debug, Clone)]` annotation on `ConnectionMetadata` in Requirement 3. Added explanatory note confirming all existing fields are `Clone`-eligible. Updated Requirement 5 metadata snapshot pattern section to reference Requirement 3 as the source of the `Clone` derive.
2. [✓] Missing `pub mod security;` in `service/mod.rs` -- Added `service/mod.rs` as a 7th required artifact in the Required Artifacts table. Added Requirement 7 specifying the `pub mod security;` declaration plus the `pub use security::{SecurityConfig, WriteValidator};` re-exports. Updated G1 task group description to include this as part of Wave 1 work.
3. [✓] Re-export `SecurityConfig` and `WriteValidator` from `service/mod.rs` -- Included in Requirement 7: `pub use security::{SecurityConfig, WriteValidator};` with rationale (follows existing re-export convention for `ServerConfig`, `OperationError`, `CallerOrigin`).
4. [✓] Value size for REMOVE/OR_REMOVE -- Added explicit guidance in Requirement 5 "Value size estimation" section: pass `value_size = 0` for REMOVE and OR_REMOVE operations. Updated Assumptions section to state REMOVE/OR_REMOVE value size is defined as 0. Added AC20 to verify REMOVE ops are never rejected due to size.
5. [✓] G3a and G3b parallel conflict in same file -- Moved G3b to Wave 4 (depends on G3a). Updated Task Groups table: G3b now depends on G1, G2, G3a. Updated Execution Plan: Wave 3 is G3a alone (1 worker), Wave 4 is G3b + G4 in parallel (2 workers). Added explanatory note on G3a/G3b sequencing. Total workers needed reduced from 3 to 2.

### Audit v3 (2026-02-28)
**Status:** NEEDS_DECOMPOSITION

**Context Estimate:** ~100% total (sum of group estimates: 20+25+25+15+15)

**Audit Dimensions:**
- Clarity: Excellent. Every requirement has code snippets, exact method signatures, step-by-step logic, and architectural rationale. No vague terms.
- Completeness: All 7 files listed with explicit modification instructions. Edge cases covered (REMOVE ops, empty batches, missing connections, serialization failures). Clone derive explicitly specified.
- Testability: All 20 ACs are concrete and measurable. Boundary condition tested (AC7: exactly N bytes succeeds).
- Scope: Well-bounded by 6 explicit constraints. Clear about what is NOT in scope.
- Feasibility: All technical assumptions verified against codebase: `HLC::now()` returns `Timestamp` (hlc.rs:330), `ConnectionRegistry.get()` returns `Option<Arc<ConnectionHandle>>` (connection.rs:196), `Principal` derives `Clone` (types.rs:117), `Timestamp` derives `Clone` (hlc.rs:159), `OperationError` uses `thiserror::Error` (operation.rs:379), CrdtService call sites in lib.rs at lines 87 and 320.
- Architecture fit: `Arc<WriteValidator>` injection follows existing domain service DI pattern. Validate-before-merge is a clean interception point.
- Non-duplication: Reuses existing `HLC`, `ConnectionRegistry`, `OperationError`, `CallerOrigin`. No reinvention.
- Cognitive load: Reasonable. `WriteValidator` is a simple struct with clear validation logic. Metadata snapshot pattern is well-documented.
- Strategic fit: P0 security for production readiness. Validate-before-merge pattern proven by Ditto and Firebase.
- Project compliance: Fully compliant (see below).

**Rust Auditor Checklist:**
- [x] No `f64` for integer-semantic fields (`max_value_bytes: u64`, `value_size: u64`)
- [x] No `r#type: String` on message structs (N/A -- no message structs created)
- [x] `Default` derived on payload structs with 2+ optional fields (N/A -- no optional fields)
- [x] Enums used for known value sets (N/A -- no enum-eligible string fields)
- [x] Wire compatibility: `rmp_serde::to_vec_named()` used for size estimation
- [x] `#[serde(rename_all = "camelCase")]` on `SecurityConfig` and `MapPermissions`
- [x] `#[serde(skip_serializing_if = "Option::is_none", default)]` on every `Option<T>` (N/A -- no `Option<T>` fields in new structs)

**Goal-Backward Validation:**

| Check | Status | Issue |
|-------|--------|-------|
| Truth 1 (unauth rejected) | OK | `WriteValidator` + `OperationError::Unauthorized` |
| Truth 2 (no write perm rejected) | OK | `WriteValidator` + `MapPermissions` + `OperationError::Forbidden` |
| Truth 3 (server HLC stored) | OK | `WriteValidator.sanitize_hlc()` + timestamp replacement in crdt.rs |
| Truth 4 (oversized rejected) | OK | `WriteValidator.validate_write()` size check |
| Truth 5 (batch atomic) | OK | `handle_op_batch` validate-all-then-apply-all |
| Truth 6 (server bypasses) | OK | `validate_write` caller_origin check |
| Truth 7 (map_permissions readable) | OK | `ConnectionMetadata.map_permissions` field |
| Key Link 1 (ordering) | OK | Spec states validate BEFORE apply |
| Key Link 2 (snapshot) | OK | Metadata snapshot pattern with clone |
| Key Link 3 (timestamp) | OK | Explicit replacement per op type |
| Key Link 4 (atomicity) | OK | Validate-all-then-apply-all |

**Strategic Sanity Check:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | Auth adapter sets `authenticated` before writes | Unauth check is a no-op; fail-open by default (`require_auth: false`) is safe |
| A2 | Auth adapter populates `map_permissions` | Falls back to `default_permissions` (permissive) -- graceful degradation |
| A3 | `connection_id` is `None` only for internal/test calls | WebSocket handler always sets it; `None` is safe default for tests |
| A4 | MsgPack serialization size is a good proxy for value size | Consistent, deterministic, reuses existing codec |

Strategic fit: Aligned with project goals. No concerns.

**Project Compliance:**

| Decision | Spec Compliance | Status |
|----------|-----------------|--------|
| MsgPack wire format | Uses `rmp_serde::to_vec_named()` | Compliant |
| `#[serde(rename_all = "camelCase")]` | Applied to both new structs | Compliant |
| No phase/spec references in code | No code comments with spec references | Compliant |
| Trait-first ordering | G1 defines types, G2+ implements | Compliant |
| Rust integer types, not f64 | `u64` for byte sizes and limits | Compliant |
| No new runtime deps | Uses existing crates only (parking_lot, rmp_serde, thiserror) | Compliant |

Project compliance: Honors PROJECT.md decisions.

**Language Profile Check:**
- File count: 7 files (1 NEW + 6 MOD). Exceeds max of 5 by 2. Acknowledged and justified in spec with detailed rationale. The excess is unavoidable given constructor signature change and module declaration requirements.
- Trait-first: G1 (Wave 1) contains only types, structs, error variants, and module declaration. No implementation logic. Compliant.
- Compilation gate: G1 touches 4 files but all are type-level changes. G2 is a single new file. G3a modifies 1 file. Manageable.

**Execution Scope:**

| Metric | Est. Context | Target | Status |
|--------|--------------|--------|--------|
| Total spec context | ~100% | <=50% | Exceeded |
| Largest task group | ~25% (G2 or G3a) | <=30% | OK |
| Worker overhead | ~20% (4 waves x ~5%) | <=10% | Warning |

**Per-Group Breakdown:**

| Group | Wave | Tasks | Est. Context | Cumulative |
|-------|------|-------|--------------|------------|
| G1 | 1 | Types: MapPermissions, SecurityConfig, error variants, Clone derive, mod.rs | ~20% | 20% |
| G2 | 2 | WriteValidator implementation | ~25% | 45% |
| G3a | 3 | CrdtService integration: inject, validate, replace timestamps | ~25% | 70% |
| G3b | 4 | Atomic batch rejection in handle_op_batch | ~15% | 85% |
| G4 | 4 | Tests and lib.rs wiring | ~15% | 100% |

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | -- Per-group G1, G3b, G4 |
| 30-50% | GOOD | -- Per-group G2, G3a (with overhead) |
| 50-70% | DEGRADING | - |
| 70%+ | POOR | - |

Note: In parallel execution mode, each wave runs in a fresh worker context. The ~100% total is the sum across all waves, not the load on any single worker. Per-wave context (largest wave = G3a at ~25% + ~5% overhead = ~30%) is within the GOOD range. The decomposition into task groups with the existing wave structure is appropriate for `/sf:run --parallel`.

**Scope:** Large (~100% cumulative, exceeds 50% target). Task groups are already defined and individually within bounds (<=25% each). Implementation Tasks section is complete with wave assignments and execution plan.

**Recommendation:** Use `/sf:run --parallel` with the existing 4-wave structure.

**Comment:** This is a thorough, well-structured specification that has been refined through two revision cycles. All previous critical issues have been fully addressed. The requirements are implementation-ready with exact code snippets, method signatures, and step-by-step logic. The 7-file count exceeds the Language Profile limit but is justified and unavoidable. The spec is approved for decomposed execution.

## Execution Summary

**Executed:** 2026-02-28
**Mode:** orchestrated
**Commits:** 4

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1 | complete |
| 2 | G2 | complete |
| 3 | G3a | complete |
| 4 | G3b, G4 | complete |

### Files Created

- `packages/server-rust/src/service/security.rs` — `SecurityConfig`, `WriteValidator` with validate_write() and sanitize_hlc(), unit tests

### Files Modified

- `packages/server-rust/src/network/connection.rs` — `MapPermissions` struct, `Clone` derive on `ConnectionMetadata`, `map_permissions` field
- `packages/server-rust/src/service/operation.rs` — `Unauthorized`, `Forbidden`, `ValueTooLarge` variants on `OperationError`
- `packages/server-rust/src/service/mod.rs` — `pub mod security;`, re-exports of `SecurityConfig` and `WriteValidator`
- `packages/server-rust/src/service/config.rs` — `security: SecurityConfig` field on `ServerConfig`
- `packages/server-rust/src/service/domain/crdt.rs` — `WriteValidator` injection, `snapshot_metadata()`, validation before merge, timestamp replacement, atomic batch rejection, security integration tests
- `packages/server-rust/src/lib.rs` — updated `CrdtService::new()` call sites with `Arc<WriteValidator>`

### Acceptance Criteria Status

- [x] AC1: Unauthenticated write rejected with Unauthorized when require_auth=true
- [x] AC2: Authenticated + no write perm returns Forbidden
- [x] AC3: Authenticated + write perm succeeds
- [x] AC4: LWW PUT stores server node_id in timestamp (sanitize_hlc replaces client ts)
- [x] AC5: OR_ADD stores server node_id in timestamp (tag regenerated from server ts)
- [x] AC6: Value exceeding max_value_bytes returns ValueTooLarge
- [x] AC7: Value at exactly max_value_bytes succeeds
- [x] AC8: OpBatch atomic rejection — 2nd failing op prevents 1st op from being applied
- [x] AC9: CallerOrigin::Forwarded bypasses all checks
- [x] AC10: CallerOrigin::System bypasses all checks
- [x] AC11: Default config (require_auth=false) allows unauthenticated writes
- [x] AC12: max_value_bytes=0 means unlimited
- [x] AC13: MapPermissions defaults to read=true, write=true
- [x] AC14: ConnectionMetadata.map_permissions defaults to empty HashMap
- [x] AC15: sanitize_hlc() returns Timestamp with server node_id
- [x] AC16: Successive sanitize_hlc() calls produce monotonically increasing timestamps
- [x] AC17: CrdtService ManagedService::name() still returns "crdt"
- [x] AC18: Existing CrdtService tests pass with default (permissive) SecurityConfig
- [x] AC19: connection_id=Some(id) with connection not in registry returns Unauthorized
- [x] AC20: REMOVE and OR_REMOVE ops never rejected due to value size

### Deviations

- G2 implemented in the same commit as G1 (security.rs created in G1 wave included WriteValidator implementation)
- G3b (handle_op_batch atomic rejection) implemented in the same commit as G3a since both modified crdt.rs sequentially without conflicts
- Clippy warnings addressed in a follow-up commit (fix commit after main implementation commit)


---

## Review History

### Review v1 (2026-02-28 12:00)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Passed:**
- [v] AC1: WriteValidator rejects unauthenticated writes when require_auth=true (unit test in security.rs + integration test in crdt.rs)
- [v] AC2: WriteValidator returns Forbidden when write permission is false (unit + integration tests)
- [v] AC3: Authenticated connection with write permission succeeds (integration test in crdt.rs)
- [v] AC4: LWW PUT stores sanitized server HLC timestamp -- apply_single_op replaces record timestamp before RecordStore.put()
- [v] AC5: OR_ADD stores sanitized timestamp and regenerates tag as millis:counter:node_id (crdt.rs lines 287-289)
- [v] AC6: ValueTooLarge returned when record exceeds max_value_bytes (unit test in security.rs)
- [v] AC7: Exactly N bytes succeeds (unit test exactly_max_value_bytes_succeeds in security.rs)
- [v] AC8: OpBatch atomic rejection -- validate-all-then-apply-all loop in handle_op_batch; integration test confirms Forbidden returned before any apply
- [v] AC9: CallerOrigin::Forwarded bypasses all checks via != Client guard (unit test in security.rs)
- [v] AC10: CallerOrigin::System bypasses all checks (unit test in security.rs)
- [v] AC11: Default config (require_auth=false) allows unauthenticated writes (unit test in security.rs)
- [v] AC12: max_value_bytes=0 means unlimited (unit test in security.rs)
- [v] AC13: MapPermissions defaults to read=true, write=true (unit test in security.rs)
- [v] AC14: ConnectionMetadata.map_permissions defaults to empty HashMap (unit test in security.rs)
- [v] AC15: sanitize_hlc() returns Timestamp with server node_id (unit test sanitize_hlc_returns_server_node_id)
- [v] AC16: Successive sanitize_hlc() calls are monotonically increasing (unit test in security.rs)
- [v] AC17: CrdtService ManagedService::name() still returns crdt (test in crdt.rs)
- [v] AC18: All pre-existing CrdtService tests pass with default SecurityConfig
- [v] AC19: connection_id=Some(id) with missing connection returns Unauthorized (integration test in crdt.rs)
- [v] AC20: REMOVE and OR_REMOVE ops pass value_size=0, never rejected on size (integration test in crdt.rs)
- [v] SecurityConfig and WriteValidator re-exported from service/mod.rs
- [v] MapPermissions placed in network/connection.rs -- no circular dependency
- [v] Clone derive added to ConnectionMetadata -- metadata snapshot pattern compiles
- [v] WriteValidator injected into all CrdtService::new() call sites in lib.rs
- [v] Build check passes (cargo check): no errors
- [v] Clippy passes with -D warnings: no warnings
- [v] No spec/phase/bug references in code comments -- WHY-comments used throughout
- [v] u64 for max_value_bytes -- platform-stable, matches SecurityConfig spec
- [v] serde(rename_all = camelCase) on SecurityConfig and MapPermissions
- [v] Metadata snapshot clones immediately after read lock -- no guard held across async ops
- [v] Value size estimation uses rmp_serde::to_vec_named(); serialization failure returns u64::MAX

**Minor:**
1. Test comment labels in crdt.rs are misleading: test commented as AC4 is or_remove_returns_op_ack and AC5 is op_batch_processes_all_ops (these are legacy numbering from crdt.rs internal ACs, not SPEC-071 ACs). No functional impact.
2. LWW PUT broadcasts the original client record (crdt.rs line 357) while OR_ADD broadcasts the sanitized record. Spec only requires correct RecordStore storage; broadcast content is out of scope. Worth noting for future correctness.
3. AC8 test verifies Forbidden is returned but cannot inspect RecordStore state because NullDataStore discards writes. Validate-all-then-apply-all code structure logically guarantees atomicity.

**Summary:** Implementation fully meets all 20 acceptance criteria. Code is clean, idiomatic Rust with correct Arc/RwLock/metadata-snapshot patterns, validate-before-merge ordering, and atomic batch rejection. Build check and clippy pass clean. Three minor observations are non-blocking.

### Fix Response v1 (2026-02-28)
**Applied:** Minor issues 1 and 2 (issue 3 skipped — test limitation, not a code fix)

**Fixes:**
1. [✓] Misleading test comment labels — Removed legacy `AC` numbering from 8 pre-existing CrdtService test comments. Labels now use descriptive names only (e.g., `// -- LWW PUT --` instead of `// -- AC1: LWW PUT --`).
   - Commit: e69f5de
2. [✓] LWW PUT broadcast timestamp inconsistency — Restructured LWW PUT branch to track the sanitized record and broadcast it instead of the original client record. Now consistent with OR_ADD path which already broadcasts the sanitized record.
   - Commit: e69f5de

**Skipped:**
3. [✗] AC8 test NullDataStore limitation — Not a code bug; the validate-all-then-apply-all structure logically guarantees atomicity. A more thorough test would require a different datastore, which is out of scope for this fix.

---

### Review v2 (2026-02-28 14:00)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: unauthenticated write rejected with Unauthorized when require_auth=true — both unit test (security.rs) and integration test (crdt.rs:unauthenticated_write_rejected_when_require_auth)
- [✓] AC2: authenticated + no write perm returns Forbidden — unit test and integration test both present
- [✓] AC3: authenticated + write perm succeeds — integration test crdt.rs:authenticated_write_succeeds
- [✓] AC4: LWW PUT stores server node_id in timestamp — sanitized_ts replaces record.timestamp before RecordStore.put(); broadcast_rec also uses sanitized record (fix from v1 applied)
- [✓] AC5: OR_ADD stores server node_id; tag regenerated as \"{millis}:{counter}:{node_id}\" — crdt.rs lines 287-289
- [✓] AC6: ValueTooLarge returned when value exceeds max_value_bytes — unit test in security.rs
- [✓] AC7: Exactly N bytes succeeds — unit test exactly_max_value_bytes_succeeds
- [✓] AC8: OpBatch atomic rejection — validate-all loop (lines 182-185) fully separates from apply-all loop (lines 187-194); integration test confirms Forbidden returned
- [✓] AC9: CallerOrigin::Forwarded bypasses all checks via \!= Client guard — unit test forwarded_origin_bypasses_all_checks
- [✓] AC10: CallerOrigin::System bypasses all checks — unit test system_origin_bypasses_all_checks
- [✓] AC11: Default config allows unauthenticated writes — unit test default_config_allows_unauthenticated_writes
- [✓] AC12: max_value_bytes=0 means unlimited — unit test zero_max_value_bytes_means_unlimited
- [✓] AC13: MapPermissions defaults to read=true, write=true — Default impl in connection.rs; unit test map_permissions_default_is_read_write
- [✓] AC14: ConnectionMetadata.map_permissions defaults to empty HashMap — Default impl includes HashMap::new(); unit test connection_metadata_map_permissions_defaults_to_empty
- [✓] AC15: sanitize_hlc() returns Timestamp with server node_id — unit test sanitize_hlc_returns_server_node_id
- [✓] AC16: Successive sanitize_hlc() calls are monotonically increasing — unit test successive_sanitize_hlc_calls_are_monotonic
- [✓] AC17: CrdtService ManagedService::name() still returns "crdt" — test managed_service_name
- [✓] AC18: Existing CrdtService tests pass with default (permissive) SecurityConfig — all pre-existing tests use make_service() with SecurityConfig::default()
- [✓] AC19: connection_id=Some(id) with connection not in registry returns Unauthorized — integration test missing_connection_returns_unauthorized
- [✓] AC20: REMOVE and OR_REMOVE ops are never rejected due to value size — estimate_value_size returns 0 for removes; integration test remove_op_not_rejected_by_size_limit
- [✓] Minor issue 1 fixed: pre-existing test comments no longer use legacy AC numbering (now descriptive: "-- LWW PUT --", "-- OR_ADD --", etc.)
- [✓] Minor issue 2 fixed: LWW PUT now broadcasts sanitized record (not original client record) — broadcast_rec tracks stored_rec with server timestamp (crdt.rs line 338-362)
- [✓] MapPermissions in network/connection.rs — no circular dependency; service imports from network (one-way)
- [✓] Clone derive on ConnectionMetadata — snapshot_metadata() compiles; no lock held across async ops
- [✓] WriteValidator injected at all CrdtService::new() call sites in lib.rs (lines 96-100, 330-333)
- [✓] SecurityConfig and WriteValidator re-exported from service/mod.rs
- [✓] SecurityConfig uses #[derive(Default)] — simpler than manual impl but semantically identical
- [✓] serde(rename_all = "camelCase") on SecurityConfig and MapPermissions
- [✓] u64 for max_value_bytes and value_size (platform-stable, matches spec)
- [✓] No phase/spec/bug references in code comments — WHY-comments used throughout
- [✓] Build check (cargo check): passes with no errors
- [✓] Clippy (-D warnings): passes with no warnings
- [✓] CallerOrigin::Backup and CallerOrigin::Wan also bypass all checks via the \!= Client guard (spec Observable Truth #6 fully covered)

**Summary:** All fixes from Review v1 are correctly applied. All 20 acceptance criteria remain satisfied. The implementation is clean, idiomatic Rust: validate-before-merge ordering enforced, atomic batch rejection implemented, metadata snapshot pattern correctly releases the RwLock before async storage calls, and HLC sanitization ensures client timestamps cannot pollute the RecordStore. Build and clippy pass cleanly. No issues found in this review.

---

## Completion

**Completed:** 2026-02-28
**Total Commits:** 4
**Audit Cycles:** 3
**Review Cycles:** 2 (+ 1 fix cycle)
