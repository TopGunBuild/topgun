## Current Position

- **Active Specification:** none
- **Status:** idle
- **Next Step:** /sf:new or /sf:next

## Queue

| ID | Title | Status | Priority | Complexity |
|----|-------|--------|----------|------------|

## Decisions

- SPEC-136a added shape wire messages (5 payload structs, 5 Message variants), upgraded SyncShape (shape_id, map_name, filter, fields, limit), removed Predicate placeholder, added Operation::ShapeSubscribe/Unsubscribe/SyncInit + service_names::SHAPE routing. Shape Merkle sync reuses existing protocol with shape_id prefix. 494 core + 565 server tests pass, clippy-clean.
- SPEC-136b added ShapeEvaluator module (matches/project/apply_shape free functions) and ShapeRegistry (DashMap-based concurrent registry with ActiveShape, ShapeRegistryError). 18 new unit tests, 582 total server tests pass, clippy-clean.
- SPEC-136c added ShapeService Tower service (subscribe/unsubscribe handlers), value_to_rmpv consolidated in predicate.rs, CRDT broadcast filtering (ENTER/UPDATE/LEAVE), AppState shape_registry field, websocket disconnect cleanup. Option<Arc<ShapeRegistry>> pattern for optional service dependencies. 586 tests pass, clippy-clean.
- SPEC-136d added ShapeMerkleSyncManager (per-shape Merkle trees), handle_shape_sync_init in ShapeService, shape-prefixed bucket traversal in SyncService::handle_merkle_req_bucket, new_basic convenience constructor. 593 tests pass, clippy-clean.
- SPEC-136e added TS client shape API: shape Zod schemas in @topgunbuild/core, ShapeHandle + ShapeManager in @topgunbuild/client, SHAPE_RESP/SHAPE_UPDATE handlers in ClientMessageHandlers, subscribeShape() on SyncEngine, re-exports from index.ts. 6 integration tests. Fixed test_server.rs to share shape_registry between CrdtService and ShapeService. 61 integration tests pass, 461 client tests pass, clippy-clean.
- SPEC-137 fixed 5 auth vulnerabilities: re-enabled JWT exp validation with configurable leeway, JWT_SECRET read from env, cors_origins default changed to empty vec, missing sub claim rejected with AUTH_FAIL, TLS warning when jwt+no-TLS. 603 server tests pass, clippy-clean.
