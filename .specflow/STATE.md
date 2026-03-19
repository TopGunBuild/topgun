## Current Position

- **Active Specification:** none
- **Status:** idle
- **Next Step:** /sf:new or /sf:next

## Queue

| ID | Title | Status | Priority | Complexity |
|----|-------|--------|----------|------------|

## Decisions

- SPEC-120 replaced blocking send().await with non-blocking try_send() in PartitionDispatcher::dispatch(). Full channels now return OperationError::Overloaded immediately. Reduced buffer from 1024 to 256. WebSocket handler sends 429 to client on overload; dispatch_op_batch preserves OperationError type for 429-vs-500 differentiation. 540 Rust tests passing.
- SPEC-126 replaced BATCH_FLUSH_THRESHOLD const with configurable SearchConfig (default 100ms/500), added conditional indexing in SearchMutationObserver (skip when no subscriptions, set needs_population flag), added lazy index population in SearchService::ensure_index_populated via populate_index_from_store (for_each_boxed on partition 0). Shared Arc<DashMap<String, AtomicBool>> wires observer-to-service flag. Established conditional observer pattern + lazy population pattern. 540 Rust tests passing, clippy-clean.
- SPEC-127 expanded MapSchema/FieldDef with FieldType enum (9 variants, Default=Any) and FieldConstraint struct (6 optional fields, camelCase serde). Added validate_value (pure fn, regex compiled per call) and validate_schema (registration-time pattern check). Created SchemaService implementing SchemaProvider (optional mode: no schema = valid) and ManagedService (no-ops). Established pattern: pure validation in core-rust, stateful service wrapper in server-rust. 551 server + 440 core tests passing, clippy-clean.
- SPEC-128 wired SchemaService into CrdtService write path. Added From<rmpv::Value> for Value (all 10 variants) in core-rust. Added OperationError::SchemaInvalid. Added validate_schema_for_op helper in CrdtService with REMOVE/OR_REMOVE/internal-call bypass. Wired schema validation after WriteValidator checks in both handle_client_op and handle_op_batch (atomic batch rejection). Updated all 6 external CrdtService::new() sites. 559 server + 454 core tests passing, clippy-clean.
- SPEC-129 delivered @topgunbuild/schema package: DSL builder (defineMap, t.* factory functions, SchemaRegistry), codegen (JSON + TypeScript output, Rust serde shape fidelity), and topgun codegen CLI command. Established TypeScript-first schema DSL pattern with SchemaRegistry.global singleton. 59 unit tests passing.
