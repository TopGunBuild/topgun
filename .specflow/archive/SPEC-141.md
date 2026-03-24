---
id: SPEC-141
type: docs
status: done
priority: P1
complexity: small
created: 2026-03-24
source: TODO-173
---

# Add Shapes (Partial Replication) Guide Page

## Context

Shapes are a key v2.0 feature enabling partial replication — clients subscribe to filtered subsets of a map rather than receiving all records. The feature is fully implemented (SPEC-136a through SPEC-136e) with 6 passing integration tests, but has zero user-facing documentation. Without a guide page, the feature is invisible to users.

## Task

Create `apps/docs-astro/src/content/docs/guides/shapes.mdx` documenting the Shapes API with working code examples derived from the actual client SDK and integration tests.

## Requirements

### File: `apps/docs-astro/src/content/docs/guides/shapes.mdx`

**Frontmatter:**
- `title: Shapes (Partial Replication)`
- `description:` one-liner about subscribing to filtered subsets of maps
- `order:` pick a number after live-queries (12) that is not already taken. Order 13 is already used by both `pub-sub.mdx` and `indexing.mdx`. Inspect the current guide pages and select an unused integer; if the Astro framework supports decimal order values, 12.5 is a safe choice.

**Sections (in order):**

1. **What are Shapes** — Concept introduction: partial replication means clients receive only the records matching their criteria instead of entire maps. Contrast with full-map subscription (`client.map()`). Mention server-side evaluation (filter runs on the server, not client).

2. **Basic Usage** — Show `SyncEngine.subscribeShape()` as the current access pattern. `TopGunClient` does not expose `subscribeShape()` directly — the `syncEngine` field is private with no public accessor. Construct a `SyncEngine` directly (exported from `@topgunbuild/client`) and call `subscribeShape()` on it:
   - `subscribeShape(mapName, { filter, fields, limit })` call
   - Returns a `ShapeHandle`
   - Show the full lifecycle: construct SyncEngine, subscribe, read records, listen for updates, unsubscribe
   - Add a note that `TopGunClient` integration (convenience wrapper) is planned for a future release

3. **ShapeHandle API Reference** — Document each property/method:
   - `shapeId: string` (readonly, auto-generated UUID)
   - `mapName: string` (readonly)
   - `filter: PredicateNode | undefined` (readonly) — the filter this shape was created with; useful for inspection and reconnect
   - `fields: string[] | undefined` (readonly) — the field projection this shape was created with
   - `limit: number | undefined` (readonly) — the record limit this shape was created with
   - `records: Map<string, any>` — current matching records, populated from server snapshot
   - `merkleRootHash: number` — used internally for delta sync
   - `onUpdate(callback): () => void` — register listener, returns unsubscribe function
   - `unsubscribe(): void` — stops updates, sends SHAPE_UNSUBSCRIBE to server, clears local state
   - `isUnsubscribed(): boolean`

4. **ShapeUpdate Events** — Document the `ShapeUpdate` interface:
   - `key: string`
   - `value: any | undefined` (present for ENTER/UPDATE, absent for LEAVE)
   - `changeType: 'ENTER' | 'UPDATE' | 'LEAVE'`
   - Explain when each fires: ENTER = new record matches filter, UPDATE = existing match modified, LEAVE = record no longer matches or deleted

5. **Filter Syntax** — Document `PredicateNode` structure used in `filter`:
   - Comparison operators: `eq`, `neq`, `gt`, `gte`, `lt`, `lte`, `like`, `regex`
   - Logical operators: `and`, `or`, `not` (use `children` array)
   - Show examples: simple equality, compound filter with `and`
   - Structure: `{ op, attribute, value }` for comparisons, `{ op, children }` for logical

6. **Field Projection** — Document `fields: string[]`:
   - Server returns only the listed fields in each record's value
   - Reduces bandwidth for wide records
   - Show example selecting only `name` and `email` from a contacts map

7. **Limits** — Document `limit: number`:
   - Server returns at most `limit` matching records in the initial snapshot
   - `SHAPE_RESP` includes `hasMore: boolean` (optional) indicating more records exist
   - Note: live updates (SHAPE_UPDATE) still arrive for all matching mutations regardless of limit

8. **Offline and Reconnection** — Explain:
   - ShapeHandle survives disconnection (client holds shape definition locally)
   - On reconnect, `ShapeManager.resubscribeAll()` automatically re-sends SHAPE_SUBSCRIBE + SHAPE_SYNC_INIT for every active shape
   - SHAPE_SYNC_INIT carries the stored `merkleRootHash` so the server sends only the delta since last sync
   - No user action required — reconnection is transparent

9. **Performance Considerations** — Cover:
   - Each shape subscription creates a per-shape Merkle tree on the server
   - Many shapes on one connection = more server memory
   - Use field projection to reduce payload size
   - Prefer specific filters over broad ones
   - Unsubscribe when shapes are no longer needed

**Code example requirements:**
- All code examples use the Astro `CodeBlock` component pattern (export const + CodeBlock) matching existing guides
- Import lucide-react icons and FeatureCard component matching existing guide pattern
- Examples must be consistent with actual API: construct `SyncEngine` from `@topgunbuild/client`, then call `subscribeShape(mapName, options)` which returns `ShapeHandle`
- Filter examples must use the actual `PredicateNode` structure (`{ op, attribute, value }`)
- No `useShape` React hook exists — do not document one. Mention that React bindings are planned if desired, but the current API is `subscribeShape()` on the sync engine.

## Acceptance Criteria

1. File `apps/docs-astro/src/content/docs/guides/shapes.mdx` exists with valid Astro frontmatter
2. All 9 sections listed above are present with content
3. Code examples use actual API signatures: construct `SyncEngine` directly, call `subscribeShape(mapName, { filter?, fields?, limit? })` returning `ShapeHandle`
4. Filter examples use correct `PredicateNode` structure with operators from the schema (`eq`, `neq`, `gt`, `gte`, `lt`, `lte`, `like`, `regex`, `and`, `or`, `not`)
5. ShapeHandle API section documents `filter`, `fields`, `limit`, `records`, `onUpdate()`, `unsubscribe()`, `isUnsubscribed()`
6. ShapeUpdate events section documents all three change types: ENTER, UPDATE, LEAVE
7. Reconnection section explains automatic resubscription with SHAPE_SYNC_INIT and merkleRootHash
8. No reference to `useShape` hook (does not exist)
9. Page follows the same Astro component pattern as existing guides (CodeBlock, FeatureCard, lucide-react imports)
10. `order` value in frontmatter does not collide with an existing guide page

## Constraints

- Do not create a `useShape` React hook or document one as existing
- Do not modify any source code files — this is a docs-only spec
- Follow the existing guide page format (see `live-queries.mdx` for reference)
- Do not add phase/spec references in the documentation content

## Assumptions

- `SyncEngine` is exported from `@topgunbuild/client` and `subscribeShape()` is a public method on it (confirmed in `SyncEngine.ts` line 654)
- `TopGunClient` does not expose `subscribeShape()` — `syncEngine` is a private field with no public accessor
- Order 13 is already taken by `pub-sub.mdx` and `indexing.mdx`; the implementer must choose an unused order number
- No React `useShape` hook exists (confirmed: grep found no matches in `packages/react/`)
- The Astro docs site builds and renders `.mdx` files from `apps/docs-astro/src/content/docs/guides/` automatically
- The `CodeBlock` component import path follows the pattern in existing guides (`../../../components/docs/CodeBlock.astro`)

## Audit History

### Audit v1 (2026-03-24)
**Status:** APPROVED

**Context Estimate:** ~8% total (1 new MDX file, docs-only, no logic)

**Quality Dimensions:**
- Clarity: Strong. Task, context, and sections are well-defined.
- Completeness: All 9 sections specified with detailed content guidance.
- Testability: All 9 acceptance criteria are concrete and verifiable.
- Scope: Clean boundary -- single file, docs only, no source changes.
- Feasibility: Straightforward documentation task.
- Architecture fit: Follows existing guide page patterns (CodeBlock, FeatureCard, lucide-react).
- Non-duplication: No existing shapes documentation exists.
- Cognitive load: Low. Single file creation with clear template.
- Strategic fit: Aligned -- documenting an implemented but invisible feature.
- Project compliance: Honors "no phase/spec references in code comments" constraint.

**Recommendations:**

1. **Order number 13 is already taken.** The assumption "order number 13 is available" is incorrect. Both `pub-sub.mdx` (order: 13) and `indexing.mdx` (order: 13) already use order 13. Suggest using 12.5 or renumbering -- or simply pick an unused number like 12 (but that is live-queries). A safe choice would be any number between 12 and 13 if the framework supports decimals, otherwise the implementer should inspect the actual ordering behavior and pick accordingly. This is non-critical since order collisions may just produce arbitrary sort order, but worth noting.

2. **API access path is misleading.** Section 2 says "Show `SyncEngine.subscribeShape()` pattern via `TopGunClient`" but `TopGunClient` does not expose `subscribeShape()` -- the `syncEngine` field is private with no public accessor. The API is only accessible by constructing a `SyncEngine` directly (which is exported from `@topgunbuild/client`). The implementer should document the actual access pattern: either `new SyncEngine(config).subscribeShape()` or note that `TopGunClient` integration is pending. This avoids documenting an API path that does not work.

3. **ShapeHandle also has readonly `filter`, `fields`, and `limit` properties.** The spec's ShapeHandle API Reference section lists 7 members but omits `filter`, `fields`, and `limit` (all readonly on the class). These are useful for users to inspect what a shape is subscribed to. Consider documenting them for completeness.

**Comment:** Well-structured docs-only spec with thorough section-by-section requirements and accurate API cross-references. The recommendations above are minor accuracy improvements that the implementer can address during writing.

### Response v1 (2026-03-24)
**Applied:** All 3 recommendations from Audit v1

**Changes:**
1. [✓] Order number collision — Removed the incorrect assumption that order 13 is available. Updated the frontmatter requirement to direct the implementer to inspect existing pages and pick an unused number, with 12.5 noted as a safe decimal option. Added acceptance criterion 10 requiring no order collision. Updated Assumptions section to reflect the correct state.
2. [✓] API access path is misleading — Rewrote Section 2 (Basic Usage) to remove the incorrect "via TopGunClient" framing. Now directs the implementer to construct `SyncEngine` directly (exported from `@topgunbuild/client`) and call `subscribeShape()` on it. Added a note that `TopGunClient` convenience integration is planned. Updated code example requirements and acceptance criterion 3 to match. Updated Assumptions section to explicitly state `syncEngine` is private with no public accessor.
3. [✓] Missing ShapeHandle readonly properties — Added `filter`, `fields`, and `limit` (all readonly) to Section 3 (ShapeHandle API Reference) with descriptions of each. Updated acceptance criterion 5 to require these three properties be documented.

### Audit v2 (2026-03-24)
**Status:** APPROVED

**Context Estimate:** ~8% total (1 new MDX file, docs-only, no logic)

**Verification against source code:**
- `SyncEngine` exported from `@topgunbuild/client` index.ts line 37 -- confirmed
- `subscribeShape()` public on `SyncEngine` at line 654 -- confirmed
- `ShapeHandle` exported from index.ts line 65, all 10 members match spec -- confirmed
- `ShapeUpdate` interface (key, value, changeType) matches spec -- confirmed
- `PredicateNode` structure (`{ op, attribute, value, children }`) with all 11 operators matches spec -- confirmed
- `ShapeManager.resubscribeAll()` exists at ShapeManager.ts line 152 -- confirmed
- No `useShape` hook in `packages/react/` -- confirmed
- `SyncEngineConfig` requires `nodeId`, `connectionProvider`, `storageAdapter` -- noted (implementer should show realistic constructor args in examples)

**Strategic fit:** Aligned with project goals. Documenting an implemented but invisible v2.0 feature.
**Project compliance:** Honors PROJECT.md decisions. Docs-only, no source changes, no phase/spec references.
**Language profile:** Not applicable (TypeScript docs spec, not Rust packages).

**Comment:** All v1 recommendations were properly addressed in Response v1. The spec now accurately reflects the actual API surface. All assumptions verified against source code. Ready for implementation.

---

## Execution Summary

**Executed:** 2026-03-24
**Commits:** 1

### Files Created
- `apps/docs-astro/src/content/docs/guides/shapes.mdx` — Shapes (Partial Replication) guide page with all 9 sections, working code examples, and full API reference

### Files Modified
None

### Files Deleted
None

### Acceptance Criteria Status
- [x] File exists with valid Astro frontmatter (title, description, order: 12.5)
- [x] All 9 sections present: What Are Shapes, Basic Usage, ShapeHandle API Reference, ShapeUpdate Events, Filter Syntax, Field Projection, Limits, Offline and Reconnection, Performance Considerations
- [x] Code examples use actual API: `new SyncEngine({ nodeId, connectionProvider, storageAdapter })` then `subscribeShape(mapName, { filter, fields, limit })`
- [x] Filter examples use correct PredicateNode structure with all documented operators (eq, neq, gt, gte, lt, lte, like, regex, and, or, not)
- [x] ShapeHandle API section documents filter, fields, limit, records, onUpdate(), unsubscribe(), isUnsubscribed()
- [x] ShapeUpdate events section documents all three change types: ENTER, UPDATE, LEAVE
- [x] Reconnection section explains automatic resubscription with ShapeManager.resubscribeAll(), SHAPE_SYNC_INIT, and merkleRootHash
- [x] No reference to useShape hook
- [x] Page follows CodeBlock + FeatureCard + lucide-react component pattern matching existing guides
- [x] order: 12.5 — no collision with any existing guide page (12 is live-queries, 13 is pub-sub/indexing)

### Deviations
None

### Notes
- Order 12.5 was chosen as a safe decimal between live-queries (12) and the pub-sub/indexing collision at 13. The Astro framework uses numeric sort order so 12.5 slots cleanly between them.
- SyncEngineConfig constructor examples show all three required fields (nodeId, connectionProvider, storageAdapter) using SingleServerProvider and IDBAdapter as realistic examples.
- A note callout explains that TopGunClient does not expose subscribeShape() (syncEngine is private) and that a convenience wrapper is planned.

---

## Review History

### Review v1 (2026-03-24)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: File exists at `apps/docs-astro/src/content/docs/guides/shapes.mdx` with valid Astro frontmatter (title, description, order: 12.5)
- [✓] AC2: All 9 sections present — What Are Shapes, Basic Usage, ShapeHandle API Reference, ShapeUpdate Events, Filter Syntax, Field Projection, Limits, Offline and Reconnection, Performance Considerations
- [✓] AC3: Code examples construct `SyncEngine` directly from `@topgunbuild/client` and call `subscribeShape(mapName, { filter, fields, limit })` returning `ShapeHandle` — matches actual source at `SyncEngine.ts:654`
- [✓] AC4: Filter examples use correct `PredicateNode` structure; all 11 operators present (eq, neq, gt, gte, lt, lte, like, regex, and, or, not) matching `PredicateOpSchema` in `base-schemas.ts`
- [✓] AC5: ShapeHandle API table documents all required members: `filter`, `fields`, `limit`, `records`, `onUpdate()`, `unsubscribe()`, `isUnsubscribed()` — all verified against `ShapeHandle.ts`
- [✓] AC6: ShapeUpdate events section documents all three change types: ENTER, UPDATE, LEAVE — verified against `ChangeEventTypeSchema` in `base-schemas.ts`
- [✓] AC7: Reconnection section explains `ShapeManager.resubscribeAll()`, SHAPE_SYNC_INIT, and `merkleRootHash` delta sync
- [✓] AC8: No reference to `useShape` hook anywhere in the file
- [✓] AC9: Component pattern matches existing guides — same CodeBlock import path, FeatureCard import, lucide-react icon imports, breadcrumb nav, pagination footer
- [✓] AC10: `order: 12.5` — no collision; verified against all 25 guide pages (live-queries=12, pub-sub=13, indexing=13)
- [✓] Constraint: No phase/spec/bug references in documentation content
- [✓] Constraint: No source files modified — docs-only
- [✓] Import accuracy: `IDBAdapter` and `SingleServerProvider` both re-exported from `@topgunbuild/client` (confirmed via `packages/client/src/index.ts` lines 4 and 85)
- [✓] `SyncEngineConfig` constructor args (`nodeId`, `connectionProvider`, `storageAdapter`) match actual interface definition
- [✓] `TopGunClient` limitation callout present and accurate — blue info box explains `syncEngine` is private
- [✓] Planned React bindings callout present in Offline and Reconnection section (green info box, no fake `useShape` API documented)
- [✓] `hasMore: true` behavior in Limits section correctly described
- [✓] Code example lifecycle is complete: construct engine, subscribe, read records, listen, unsubscribe

**Summary:** The implementation fully satisfies all 10 acceptance criteria with no deviations. API signatures, import paths, operator lists, and component patterns are all verified against source. The page is well-structured with appropriate callouts for planned features and accurate constraint messaging.

---

## Completion

**Completed:** 2026-03-24
**Total Commits:** 1
**Review Cycles:** 1

### Outcome

Created the Shapes (Partial Replication) guide page documenting the full shape subscription API with 9 sections covering concepts, usage, API reference, filter syntax, field projection, limits, offline behavior, and performance considerations.

### Key Files

- `apps/docs-astro/src/content/docs/guides/shapes.mdx` — Shapes guide page with working code examples using actual SyncEngine + subscribeShape() API

### Patterns Established

None — followed existing guide page patterns (CodeBlock, FeatureCard, lucide-react).

### Deviations

None — implemented as specified.
