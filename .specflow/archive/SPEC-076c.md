---
id: SPEC-076c
type: feature
status: done
priority: P1
complexity: medium
created: 2026-03-04
todo: TODO-093
parent: SPEC-076
depends_on: [SPEC-076b]
---

# Admin Dashboard v1.0 -- React Dashboard Adaptation

## Context

TopGun has an existing React 19 + Vite admin dashboard (`apps/admin-dashboard/`) that is ~85% functional. It currently expects a TS server admin API on port 9091 that does not exist in the Rust server. SPEC-076a created the Rust admin types and auth middleware, and SPEC-076b implemented the admin API handlers and wired them into the axum router. This sub-spec adapts the React dashboard to consume the Rust admin API.

**Current state of the dashboard:**
- Pages: Dashboard (system overview via `$sys/stats` + `$sys/cluster`), Maps list, Data Explorer (CRUD + Monaco JSON editor), Query Playground, Cluster Topology (SVG partition ring), Settings (tabbed config editor), Login
- Data fetching: manual `fetch` + `useState` + `setInterval` polling via `adminFetch()` helper to `http://localhost:9091`
- TopGun client: `@topgunbuild/react` hooks (`useQuery`, `useMap`) for live data via `$sys/*` system maps
- Auth: JWT token in localStorage, `Authorization: Bearer` header on admin API calls
- UI: Tailwind CSS, Radix UI primitives, lucide-react icons, cmdk command palette, Monaco editor, dark mode

**What needs to happen:**
1. Dashboard must talk to the Rust server (same port as WS, not separate port 9091)
2. SWR migration for live data fetching (replace manual fetch+useState+polling)
3. CRDT Debug panel placeholder UI in Data Explorer
4. Token key unification in `App.tsx`, `Layout.tsx`, and `client.ts` (already correct in `api.ts`)
5. Setup Wizard hidden from routing (deferred to v1.1)
6. Type interfaces hand-typed to match Rust admin response structs
7. Settings page restructured from 7-section/5-tab layout to flat 2-tab layout matching Rust `SettingsResponse`

**Parent spec:** SPEC-076 (Admin Dashboard v1.0 -- Rust Server Adaptation)
**Depends on:** SPEC-076b (admin handlers + wiring must be available for integration verification)
**Sibling specs:** SPEC-076a (types + auth), SPEC-076b (handlers + wiring)

### Inherited Decisions

- System maps (`$sys/*`) are NOT populated; Dashboard page (`pages/Dashboard.tsx`) and Cluster page (`pages/Cluster.tsx`) will show placeholder/empty states via their `useQuery`/`useMap` hooks. They are separate from the admin-API-backed `features/cluster/ClusterTopology.tsx`
- CRDT Debug panel renders placeholder UI with empty state in v1.0; data population deferred until system maps are implemented in a follow-up spec
- Hand-typed TypeScript interfaces in `admin-api-types.ts` (no OpenAPI codegen for v1.0)
- SWR with `refreshInterval: 5000` (cluster) and `refreshInterval: 1000` (dashboard stats) replaces manual `setInterval` polling
- Bootstrap mode and Setup Wizard deferred to v1.1
- Vector search toggle in Settings preserved as read-only display with hardcoded `false` default (no backend field exists)
- Token key unified to `topgun_admin_token` across all files (`api.ts` already uses this; changes needed only in `App.tsx`, `Layout.tsx`, `client.ts`)

### Available from SPEC-076a/076b

- Rust admin API endpoints:
  - `GET /api/status` -- public, returns `{ configured, version, mode }`
  - `POST /api/auth/login` -- public, returns `{ token }`
  - `GET /api/admin/cluster/status` -- admin auth required, returns `ClusterStatusResponse`
  - `GET /api/admin/maps` -- admin auth required, returns `MapsListResponse`
  - `GET /api/admin/settings` -- admin auth required, returns `SettingsResponse`
  - `PUT /api/admin/settings` -- admin auth required, accepts `SettingsUpdateRequest`, returns `SettingsResponse` (full settings after update)
- `GET /api/openapi.json` and `GET /api/docs` (Swagger UI)
- `GET /admin/` serves the SPA via `ServeDir`

## Task

Adapt the existing React admin dashboard to consume the Rust admin API endpoints. Migrate data fetching to SWR, create hand-typed TypeScript interfaces matching Rust response structs, add a CRDT Debug panel placeholder, unify token keys, restructure Settings to match the flat Rust response, and hide the Setup Wizard.

## Requirements

### New files

1. `apps/admin-dashboard/src/lib/swr-config.ts` -- SWR global configuration:
   - Default fetcher using `adminFetch()` from `api.ts`
   - `refreshInterval` configuration
   - Error retry settings
   - `revalidateOnFocus: true`

2. `apps/admin-dashboard/src/features/explorer/CrdtDebug.tsx` -- CRDT Debug panel component (placeholder UI for v1.0):
   - Renders an empty state with explanatory text: "CRDT debugging data will be available when system maps are implemented"
   - UI layout prepared for future data: HLC clock state per map, recent merge history, Merkle tree summary, conflict inspector
   - No data fetching in v1.0 (system maps are not populated)

3. `apps/admin-dashboard/src/lib/admin-api-types.ts` -- Hand-typed TypeScript interfaces matching Rust admin response structs:
   - `ServerMode` (`"normal"` | `"bootstrap"`)
   - `NodeStatus` (`"healthy"` | `"suspect"` | `"dead"`)
   - `ServerStatusResponse` (`configured: boolean`, `version: string`, `mode: ServerMode`)
   - `NodeInfo` (`nodeId: string`, `address: string`, `status: NodeStatus`, `partitionCount: number`, `connections: number`, `memory: number`, `uptime: number`)
   - `PartitionInfo` (`id: number`, `ownerNodeId: string`)
   - `ClusterStatusResponse` (`nodes: NodeInfo[]`, `partitions: PartitionInfo[]`, `totalPartitions: number`, `isRebalancing: boolean`)
   - `MapInfo` (`name: string`, `entryCount: number`)
   - `MapsListResponse` (`maps: MapInfo[]`)
   - `SettingsResponse` (`nodeId`, `defaultOperationTimeoutMs`, `maxConcurrentOperations`, `gcIntervalMs`, `partitionCount`, `host`, `port`, `requireAuth`, `maxValueBytes`, `logLevel?`)
   - `SettingsUpdateRequest` (`logLevel?`, `gcIntervalMs?`, `maxConcurrentOperations?`)
   - `LoginRequest` (`username`, `password`)
   - `LoginResponse` (`token`)
   - `ErrorResponse` (`error`, `field?`)

### Modified files

4. `apps/admin-dashboard/src/lib/api.ts` -- Change `API_BASE` default from `http://localhost:9091` to same-origin (`''` or `window.location.origin`); export `API_BASE` so `swr-config.ts` can use it in its fetcher. Remove the `http://localhost:9091` comment on line 5. Update the `login()` return type from `Promise<{ token: string; user: { id: string; username: string; role: string } }>` to `Promise<{ token: string }>` to match the Rust `LoginResponse` shape (no `user` object). **Note:** `api.ts` already uses `topgun_admin_token` as the token key -- no token key change is needed in this file

5. `apps/admin-dashboard/src/hooks/useServerStatus.ts` -- Replace manual fetch+polling with SWR hook. Consolidate the duplicate `API_BASE` constant (currently `import.meta.env.VITE_API_URL || 'http://localhost:9091'`) to use the shared `API_BASE` from `api.ts` or the SWR fetcher in `swr-config.ts` rather than maintaining a second hardcoded URL

6. `apps/admin-dashboard/src/features/explorer/DataExplorer.tsx` -- Add a "CRDT Debug" panel. The existing layout is a sidebar (map list) + main panel (data table). Add a toggle/button in the main panel header area (e.g., a segmented control or button group with "Data" and "CRDT Debug" options) that switches the main panel content between the existing data table view and the `CrdtDebug` placeholder component. The sidebar remains visible in both views. This is NOT a browser-style tab bar -- it is a view switcher within the existing main panel

7. `apps/admin-dashboard/src/features/cluster/ClusterTopology.tsx` -- Replace manual polling with SWR; adapt component to match Rust response shapes:
   - `NodeInfo`: `partitionCount: u32` instead of `partitions: number[]`, `memory: u64` (bytes) instead of `{ used, total }`, `address: String` added, `status: NodeStatus` enum
   - `PartitionInfo`: `ownerNodeId` instead of `owner`, no `replicas` field
   - Update partition ring visualization: `p.owner` becomes `p.ownerNodeId`; remove `replicas` rendering logic
   - **Memory display:** Replace the progress bar (`<Progress>`) and "used / total" text (lines 317-329) with a single text line showing `formatBytes(node.memory)` (e.g., "Memory: 256.0 MB"). The progress bar is removed because Rust `NodeInfo.memory` is a single `u64` value (bytes used) with no `total` field. The existing `formatBytes()` helper already handles the conversion

8. `apps/admin-dashboard/src/features/settings/Settings.tsx` -- Replace manual polling with SWR; restructure from 7-section nested `SettingsData` to flat `SettingsResponse` matching the Rust API. Specific changes:
   - Remove `// Phase 14D-3: Settings Page` comment (violates code comment convention)
   - **Tab restructuring:** Replace the existing 5-tab layout (General, Storage, Integrations, Cluster, Limits) with 2 tabs:
     - **"Server" tab:** Shows all fields from Rust `SettingsResponse` -- read-only fields (`nodeId`, `host`, `port`, `partitionCount`, `requireAuth`, `maxValueBytes`, `defaultOperationTimeoutMs`) displayed as disabled inputs with "Restart required" badges; editable fields (`logLevel` as select, `gcIntervalMs` as number input, `maxConcurrentOperations` as number input) with "Hot-reloadable" badges
     - **"Integrations" tab:** Contains the Vector Search toggle as a read-only display. Since `SettingsResponse` has no vector search field, the toggle reads from a hardcoded default (`false`) and is rendered as disabled (`<Switch disabled checked={false} />`). The MCP card is also preserved as read-only/disabled with placeholder values. Both are marked with a "Coming soon" badge indicating these features are not yet available in the Rust server
   - **Removed tabs:** Storage (no `storage` section in Rust response), Cluster (info now shown on Server tab via `nodeId`/`partitionCount`), Limits (no `rateLimits` section in Rust response)
   - **Replace `SettingsData` interface** with imported `SettingsResponse` from `admin-api-types.ts`
   - **Remove `unflattenObject` and `getNestedValue` helpers** -- no longer needed since the data structure is flat
   - **Save flow:** Remove `POST /api/admin/settings/validate` call (endpoint does not exist). Change `PATCH /api/admin/settings` to `PUT /api/admin/settings`. The `PUT` response is a `SettingsResponse` (the full settings after update) -- on success, update the SWR cache with the returned data and show a success toast. On error (4xx), the response is `ErrorResponse` (`{ error: string, field?: string }`) -- show the error message in the toast
   - **`handleChange` simplification:** Since the data is flat, change tracking uses plain field names (e.g., `'logLevel'`, `'gcIntervalMs'`) instead of dot-notation paths

9. `apps/admin-dashboard/src/features/setup/SetupWizard.tsx` -- **No changes to this file.** The Setup Wizard is not in the `navItems` array in `Layout.tsx` and is only accessible via routes in `App.tsx`. Hiding it is handled entirely by removing the `/setup` route and bootstrap redirect in `App.tsx` (requirement 10)

10. `apps/admin-dashboard/src/App.tsx` -- Remove the `status?.mode === 'bootstrap'` conditional redirect block (lines 92-101) and the `/setup` route entry (line 108). Remove the `SetupWizard` import. Update the `ServerUnavailable` component text to remove both the hardcoded "port 8080" and "port 9091" references (line 47: "Make sure the server is running on port 8080 with the admin API on port 9091"); replace with a generic message: "Cannot connect to TopGun server. Make sure the server is running." Update `ProtectedRoute` to use `topgun_admin_token` as the localStorage key (lines 22, 29: change `'topgun_token'` to `'topgun_admin_token'`)

11. `apps/admin-dashboard/src/pages/Login.tsx` -- Adapt to the simpler Rust `LoginResponse` shape (`{ token }` only, no `user` object). The existing `Login.tsx` does NOT use the `user` object from the login response -- it calls `await login(username, password)` and navigates on success. The primary change is ensuring the component works with the updated `login()` return type in `api.ts` (which drops the `user` field). No JWT decoding is needed for v1.0

12. `apps/admin-dashboard/src/components/Layout.tsx` -- Change `localStorage.removeItem('topgun_token')` in the logout handler (line 54) to `localStorage.removeItem('topgun_admin_token')`, unifying with the token key used by `api.ts` and `App.tsx`. Without this change, the logout button would remove the wrong key and leave the actual session token intact

13. `apps/admin-dashboard/src/lib/client.ts` -- Change `localStorage.getItem('topgun_token')` (line 17) to `localStorage.getItem('topgun_admin_token')` so the TopGun WebSocket client reads the admin JWT from the same key used by `api.ts`, `App.tsx`, and `Layout.tsx`

14. `apps/admin-dashboard/package.json` -- Add `swr` dependency

15. `apps/admin-dashboard/vite.config.ts` -- Add dev proxy for `/api/*` to Rust server; configure static asset serving path

## Acceptance Criteria

1. Dashboard loads from Rust server (same origin, no port 9091 dependency)
2. Cluster Topology page renders nodes and partition ring from Rust `/api/admin/cluster/status` endpoint, adapted to the `NodeInfo` response shape (`partitionCount` instead of `partitions[]`, `memory` as `u64` bytes displayed as a single formatted value instead of progress bar, `address` as `String`) and the `PartitionInfo` response shape (`ownerNodeId` instead of `owner`, no `replicas` field)
3. Data Explorer lists maps from Rust `/api/admin/maps` endpoint with entry counts
4. CRDT Debug panel renders a placeholder UI with empty state message indicating system map data is not yet available; accessible via a view switcher in the Data Explorer main panel; UI layout is prepared for future HLC, merge history, and Merkle tree data
5. Settings page has 2 tabs (Server, Integrations); loads configuration from Rust `/api/admin/settings` and saves editable fields (`logLevel`, `gcIntervalMs`, `maxConcurrentOperations`) via `PUT` (not `PATCH`); vector search toggle is disabled with hardcoded `false`; `PUT` success updates SWR cache with the returned `SettingsResponse`
6. Login page authenticates via Rust `/api/auth/login`, handles the `{ token }` response (no `user` object), and stores JWT in localStorage under the key `topgun_admin_token`
7. SWR replaces manual `fetch`+`useState`+`setInterval` in at least ClusterTopology, Settings, and useServerStatus
8. All existing dashboard pages render without JavaScript errors when connected to Rust server
9. `Settings.tsx` does not contain phase/spec/bug reference comments
10. Setup Wizard is not accessible in v1.0 (no `/setup` route, no bootstrap redirect); `SetupWizard.tsx` file is preserved unchanged for v1.1
11. `App.tsx` does not contain hardcoded "port 8080" or "port 9091" references; `ServerUnavailable` component uses a generic server message
12. `App.tsx`, `api.ts`, `Layout.tsx`, and `client.ts` all use the same localStorage token key (`topgun_admin_token`). Specifically: `api.ts` already uses `topgun_admin_token` (no change needed); `App.tsx` `ProtectedRoute` uses `topgun_admin_token`; `Layout.tsx` logout handler removes `topgun_admin_token` (not `topgun_token`); `client.ts` reads auth token from `topgun_admin_token`
13. `useServerStatus.ts` does not have its own `API_BASE` constant; it uses the shared base URL from `api.ts` or the SWR fetcher

## Constraints

- Do NOT modify Rust server files (those are in SPEC-076a and SPEC-076b)
- Do NOT add v2.0 features (pipeline visualization, connector wizard, Arroyo-style DAG editor)
- Do NOT add v3.0 features (tenant admin, S3 storage config, vector search config beyond toggle)
- Do NOT remove the TS server admin API compatibility (it will be removed in TODO-103)
- CRDT Debug panel renders placeholder UI only; it will read live data from system maps once they are populated in a follow-up spec (system maps are NOT populated in this spec)

## Assumptions

1. The Rust admin API (from SPEC-076a/076b) is operational and accessible on the same port as WebSocket/health/metrics.
2. `pages/Dashboard.tsx` and `pages/Cluster.tsx` (which use `useQuery('$sys/stats')` and `useQuery('$sys/cluster')` via the TopGun WebSocket client) will show placeholder/empty states or "No Cluster Data" messages in v1.0 since system maps are not populated. These pages are separate from the admin-API-backed `features/cluster/ClusterTopology.tsx` and will render without JavaScript errors (satisfying AC 8) but with degraded UX until system maps are implemented in a follow-up spec.
3. The existing `Login.tsx` does NOT use the `user` object from the login response. The simpler `{ token }` response from the Rust server is sufficient.
4. SWR with `refreshInterval` is an adequate replacement for manual `setInterval` polling.
5. Vector search toggle in Settings is preserved as disabled with hardcoded `false` since `SettingsResponse` has no vector search field.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create `admin-api-types.ts` with hand-typed TypeScript interfaces matching Rust admin response structs (including `NodeStatus`, `ServerMode`) | -- | ~5% |
| G2 | 1 | Create `swr-config.ts` with SWR global configuration (fetcher, refreshInterval, error retry); update `package.json` to add `swr` dependency | -- | ~3% |
| G3 | 1 | Create `CrdtDebug.tsx` placeholder UI component | -- | ~5% |
| G4 | 2 | Update `api.ts` (same-origin base URL, export `API_BASE`, simplified `login()` return type); update `useServerStatus.ts` (SWR hook, remove duplicate `API_BASE`) | G1, G2 | ~10% |
| G5 | 2 | Update `ClusterTopology.tsx` (SWR, Rust response shapes for NodeInfo and PartitionInfo, single-value memory display); update `Settings.tsx` (SWR, 2-tab layout, PUT instead of PATCH, flat data structure, remove phase comment, disabled vector search toggle); integrate `CrdtDebug.tsx` view switcher into `DataExplorer.tsx` | G1, G2, G3 | ~15% |
| G6 | 2 | Update `App.tsx` (remove bootstrap redirect, `/setup` route, both port references, unify token key); update `Login.tsx` (simpler response shape); update `Layout.tsx` (unify logout token key); update `client.ts` (unify auth token key) | G1 | ~10% |
| G7 | 3 | Update `vite.config.ts` (dev proxy for `/api/*`) | G4, G5, G6 | ~3% |
| G8 | 3 | Integration verification: all dashboard pages work against Rust server without JS errors | G4, G5, G6 | ~5% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2, G3 | Yes | 3 |
| 2 | G4, G5, G6 | Yes | 3 |
| 3 | G7, G8 | Yes | 2 |

**Total workers needed:** 3 (max in any wave)

## Audit History

### Audit v1 (2026-03-04 20:15)
**Status:** NEEDS_REVISION

**Context Estimate:** ~58% total

**Critical:**
1. **Settings.tsx restructuring is under-specified.** The existing `SettingsData` interface has 7 nested sections (`general`, `storage`, `security`, `integrations`, `cluster`, `rateLimits`, `_meta`) with 5 tab pages. The Rust `SettingsResponse` is a flat struct with only 10 fields (nodeId, defaultOperationTimeoutMs, maxConcurrentOperations, gcIntervalMs, partitionCount, host, port, requireAuth, maxValueBytes, logLevel). The spec says "align settings structure with Rust SettingsResponse" and "preserve vector search toggle as read-only" but does not describe: (a) which existing tabs to keep/remove/restructure, (b) how to display settings that do not exist in the Rust response (storage status, MCP integration, rate limits, connection string), (c) what the vector search toggle should read from since `SettingsResponse` has no vector search field, (d) what the `handleSave` response shape should be (the Rust `PUT` handler's response format). Without this guidance, an implementer must guess the UI restructuring, risking inconsistent results.
2. **ClusterTopology memory display change is under-specified.** The existing UI displays memory as a progress bar with "used / total" format (lines 319-329 of ClusterTopology.tsx). The Rust `NodeInfo` has only `memory: u64` (a single value in bytes), not `{ used, total }`. The spec identifies the shape change but does not specify how the UI should adapt -- should it show a single "Memory: 256 MB" value, remove the progress bar entirely, or use some heuristic? This is a visible UI change that needs explicit direction.
3. **api.ts token key is already `topgun_admin_token`.** The spec (requirement 4) says to "unify localStorage token key to `topgun_admin_token`" in api.ts, but the existing `api.ts` already uses `const TOKEN_KEY = 'topgun_admin_token'` (line 7). The spec should not describe this as a change to api.ts since it is already correct. The real changes are only in `App.tsx` (line 22: `topgun_token`), `Layout.tsx` (line 54: `topgun_token`), and `client.ts` (line 17: `topgun_token`). This inaccuracy could confuse implementers.

**Recommendations:**
4. [Strategic] The spec says `api.ts` should "add SWR configuration" but SWR configuration is already covered by the new `swr-config.ts` file (requirement 1). Clarify what "SWR configuration" means in api.ts specifically -- likely just exporting `API_BASE` for SWR fetcher usage. Avoid duplicating the SWR config concern across two files.
5. G7 depends on G4, G5, G6 but `package.json` (add `swr`) should logically be in Wave 1 since SWR is needed by G4 and G5. The dependency graph has `swr` being added after it is already used. Move `package.json` update to G2 or make G7 part of Wave 1.
6. The spec says to hide SetupWizard from navigation (requirement 9), but SetupWizard is not in the `navItems` array in `Layout.tsx` -- it is only accessible via routes in `App.tsx`. The spec already covers removing it from `App.tsx` routes (requirement 10). Requirement 9 saying "remove from navigation/routing" is redundant with requirement 10. Consider whether any change to `SetupWizard.tsx` itself is needed (the answer appears to be no).
7. The `App.tsx` `ServerUnavailable` component references "port 8080" (line 47: "Make sure the server is running on port 8080 with the admin API on port 9091"), not just "port 9091". The spec only mentions removing the "port 9091" reference. Both port references should be removed and replaced with a generic message.
8. DataExplorer.tsx currently has no tab structure -- it is a sidebar+main panel layout. The spec says "Add CRDT Debug tab" but does not describe the UI mechanism (a Tabs component wrapping the explorer content? A new tab alongside the existing content?). Provide guidance on where and how the tab is integrated.

### Response v1 (2026-03-04 20:45)
**Applied:** All 3 critical issues and all 5 recommendations

**Changes:**
1. [x] Settings.tsx restructuring fully specified -- requirement 8 now describes: 2-tab layout (Server + Integrations), which tabs removed (Storage, Cluster, Limits), flat `SettingsResponse` replaces nested `SettingsData`, vector search toggle disabled with hardcoded `false`, MCP card preserved as disabled with "Coming soon" badge, `PUT` response is `SettingsResponse` with SWR cache update on success and `ErrorResponse` on error, `unflattenObject`/`getNestedValue` helpers removed, flat change tracking. AC 5 updated to match.
2. [x] ClusterTopology memory display specified -- requirement 7 now explicitly says: replace progress bar and "used / total" with single text line `formatBytes(node.memory)`, progress bar removed. AC 2 updated.
3. [x] api.ts token key inaccuracy fixed -- requirement 4 now notes `api.ts` already uses `topgun_admin_token` and no token key change is needed in that file. Inherited Decisions and AC 12 updated to clarify which files actually need changes.
4. [x] SWR config duplication clarified -- requirement 4 now says "export `API_BASE`" instead of "add SWR configuration", making clear that `swr-config.ts` owns SWR config and `api.ts` just exports the base URL for the fetcher.
5. [x] `package.json` `swr` dependency moved from G7/Wave 3 to G2/Wave 1 so SWR is available when G4/G5 need it.
6. [x] SetupWizard requirement 9 rewritten -- now explicitly states no changes to `SetupWizard.tsx` itself, and that hiding is handled entirely by `App.tsx` route/redirect removal in requirement 10. AC 10 updated.
7. [x] Both port references (8080 and 9091) now mentioned in requirement 10 and AC 11 for removal from `ServerUnavailable`.
8. [x] DataExplorer CRDT Debug integration mechanism specified -- requirement 6 now describes a view switcher (segmented control/button group) in the main panel header that toggles between "Data" and "CRDT Debug" views, with sidebar remaining visible. Terminology changed from "tab" to "panel" throughout to avoid confusion with browser tabs. AC 4 updated.

### Audit v2 (2026-03-04 21:15)
**Status:** APPROVED

**Context Estimate:** ~56% total

**Previous Issues Resolution:**
- Critical 1 (Settings restructuring): Fully resolved. Requirement 8 now specifies 2-tab layout, removed tabs, flat data structure, PUT response handling, helper removal, and flat change tracking. Verified against actual `Settings.tsx` source (581 lines, 5 tabs, nested `SettingsData`, `unflattenObject`/`getNestedValue` helpers).
- Critical 2 (ClusterTopology memory): Fully resolved. Requirement 7 now explicitly specifies replacing progress bar with `formatBytes(node.memory)` single text line. Verified against actual source lines 317-329.
- Critical 3 (api.ts token key): Fully resolved. Requirement 4 now correctly notes `api.ts` already uses `topgun_admin_token`. AC 12 clarifies per-file status.
- Recommendations 4-8: All resolved as described in Response v1.

**Source File Verification:**
All spec claims verified against actual source files in `apps/admin-dashboard/src/`:
- `api.ts`: `API_BASE` on line 6 is `http://localhost:9091`, `TOKEN_KEY` on line 7 is `topgun_admin_token` -- matches spec
- `App.tsx`: `topgun_token` on lines 22/29, bootstrap redirect on lines 92-101, `/setup` route on line 108, port references on line 47 -- all match spec
- `Layout.tsx`: `topgun_token` on line 54 -- matches spec
- `client.ts`: `topgun_token` on line 17 -- matches spec
- `useServerStatus.ts`: duplicate `API_BASE` on line 10 -- matches spec
- `Settings.tsx`: 5-tab layout, nested `SettingsData`, `unflattenObject`/`getNestedValue`, phase comment on line 2, `PATCH` method, validate endpoint -- all match spec
- `ClusterTopology.tsx`: memory progress bar on lines 317-329 -- matches spec
- `DataExplorer.tsx`: sidebar+main panel layout, no tab/view switcher -- matches spec
- `Login.tsx`: does not use `user` object -- matches spec (line 28: just `await login(...)`)
- `package.json`: no `swr` dependency -- matches spec
- `vite.config.ts`: no dev proxy -- matches spec

**Dimensions:**
- Clarity: Excellent. Every requirement specifies exact file, line numbers where applicable, and precise changes.
- Completeness: All files listed with specific changes. No deletion specs needed (files are modified, not deleted).
- Testability: All 13 acceptance criteria are measurable and verifiable.
- Scope: Clear boundaries (no Rust changes, no v2/v3 features, no TS server removal).
- Feasibility: Sound approach -- SWR migration is straightforward, type changes are well-defined.
- Architecture fit: Uses existing patterns (adminFetch, Radix UI, Tailwind).
- Non-duplication: SWR replaces manual polling (net simplification). Types are hand-typed to avoid OpenAPI codegen dependency.
- Cognitive load: Reasonable for a medium-complexity React adaptation spec.
- Strategic fit: Aligned with project goals (Rust migration, admin dashboard for v1.0).
- Project compliance: Honors PROJECT.md constraints. Language Profile does not apply (TypeScript, not Rust).

**Assumptions:**

| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | Rust admin API is operational on same port | Dashboard cannot fetch data; integration verification fails |
| A2 | System maps show empty states without JS errors | AC 8 fails for Dashboard/Cluster pages |
| A3 | Login.tsx does not use `user` object | Login page would break (verified: assumption correct) |
| A4 | SWR refreshInterval replaces setInterval adequately | Data may be stale or polling may not work |
| A5 | `client.ts` WS URL does not need changing | WebSocket client may connect to wrong port |

All assumptions are reasonable. A5 is notable -- the spec does not change `client.ts` WS URL (still `ws://localhost:8080`), but since the Rust server runs on the same port for WS and HTTP, this should work when using the vite dev proxy or same-origin deployment. The `VITE_WS_URL` env var provides override capability.

**Recommendations:**
1. The spec creates `swr-config.ts` (requirement 1) but does not specify where `<SWRConfig value={...}>` wraps the component tree. The implementer should add it in `App.tsx` or `main.tsx`. This is obvious to any React developer familiar with SWR but could be explicitly stated.
2. `client.ts` line 4 has a comment "WebSocket connects to main server port (8080), not admin API port (9091)" which should also be updated since the distinction no longer applies. This is a minor cleanup not covered by the spec.

**Comment:** Well-revised specification. All 3 previous critical issues and 5 recommendations have been thoroughly addressed. The spec is now highly detailed with verified line numbers, explicit UI restructuring guidance, and accurate per-file change descriptions. Ready for implementation.

## Execution Summary

**Executed:** 2026-03-04
**Mode:** orchestrated (sequential fallback)
**Commits:** 3

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2, G3 | complete |
| 2 | G4, G5, G6 | complete |
| 3 | G7, G8 | complete |

### Files Created
- `apps/admin-dashboard/src/lib/admin-api-types.ts` -- hand-typed TypeScript interfaces matching Rust admin response structs
- `apps/admin-dashboard/src/lib/swr-config.ts` -- SWR global configuration with adminFetch-based fetcher
- `apps/admin-dashboard/src/features/explorer/CrdtDebug.tsx` -- CRDT Debug placeholder UI component

### Files Modified
- `apps/admin-dashboard/package.json` -- added `swr` dependency
- `apps/admin-dashboard/src/lib/api.ts` -- same-origin API_BASE, exported, simplified login return type
- `apps/admin-dashboard/src/hooks/useServerStatus.ts` -- SWR hook replaces manual fetch+polling
- `apps/admin-dashboard/src/features/cluster/ClusterTopology.tsx` -- SWR, Rust NodeInfo/PartitionInfo shapes, single memory value display
- `apps/admin-dashboard/src/features/settings/Settings.tsx` -- 2-tab layout, flat SettingsResponse, PUT, removed phase comment and nested helpers
- `apps/admin-dashboard/src/features/explorer/DataExplorer.tsx` -- CRDT Debug view switcher in main panel
- `apps/admin-dashboard/src/App.tsx` -- removed bootstrap redirect, /setup route, port references; unified token key; SWRConfig wrapper
- `apps/admin-dashboard/src/components/Layout.tsx` -- unified logout token key
- `apps/admin-dashboard/src/lib/client.ts` -- unified auth token key, removed outdated port comment
- `apps/admin-dashboard/vite.config.ts` -- dev proxy for /api/*, base path /admin/

### Acceptance Criteria Status
- [x] AC1: Dashboard loads from Rust server (same origin, no port 9091 dependency)
- [x] AC2: Cluster Topology renders nodes and partition ring from Rust endpoint with adapted shapes
- [x] AC3: Data Explorer lists maps from Rust /api/admin/maps endpoint with entry counts
- [x] AC4: CRDT Debug panel renders placeholder UI with empty state, accessible via view switcher
- [x] AC5: Settings has 2 tabs (Server, Integrations); loads/saves via Rust API with PUT; vector search disabled
- [x] AC6: Login authenticates via Rust /api/auth/login, handles {token} response, stores under topgun_admin_token
- [x] AC7: SWR replaces manual fetch+useState+setInterval in ClusterTopology, Settings, and useServerStatus
- [x] AC8: All existing dashboard pages render without JavaScript errors (structural verification)
- [x] AC9: Settings.tsx does not contain phase/spec/bug reference comments
- [x] AC10: Setup Wizard not accessible (no /setup route, no bootstrap redirect); SetupWizard.tsx preserved unchanged
- [x] AC11: App.tsx does not contain hardcoded port references; ServerUnavailable uses generic message
- [x] AC12: App.tsx, api.ts, Layout.tsx, and client.ts all use topgun_admin_token
- [x] AC13: useServerStatus.ts uses shared API_BASE from api.ts, no duplicate constant

### Deviations
- Auditor recommendation 1 (SWRConfig wrapping): Addressed by wrapping `<SWRConfig value={swrConfig}>` in App.tsx around TopGunProvider
- Auditor recommendation 2 (client.ts port comment): Addressed by removing the outdated port distinction comment
- Login.tsx: No changes needed -- the existing code does not use the login return value, so the api.ts type change is sufficient

---

## Review History

### Review v1 (2026-03-04 22:30)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Minor:**
1. **Spec reference in code comment**
   - File: `apps/admin-dashboard/src/lib/admin-api-types.ts:3`
   - Issue: Comment contains "from SPEC-076a" which violates the project convention ("Do NOT add phase/spec/bug references in code comments"). Should use a WHY-comment instead.
   - Fix: Replace "These types mirror the Rust admin_types.rs definitions from SPEC-076a." with "These types mirror the Rust admin_types.rs definitions for the admin API."

**Passed:**
- [x] AC1: `api.ts` exports `API_BASE` as `import.meta.env.VITE_API_URL || ''` (same-origin). No `localhost:9091` references anywhere in dashboard source except the preserved-unchanged `SetupWizard.tsx`.
- [x] AC2: `ClusterTopology.tsx` uses SWR to fetch `/api/admin/cluster/status`, types imported from `admin-api-types.ts` (`NodeInfo` with `partitionCount`, `memory` as single number, `address`; `PartitionInfo` with `ownerNodeId`). Memory displayed as `formatBytes(node.memory)` single text line (line 245). No progress bar for memory. No `replicas` references. Partition ring uses `p.ownerNodeId` (line 154).
- [x] AC3: `DataExplorer.tsx` fetches `/api/admin/maps` via `adminFetch` (line 48), displays `entryCount` per map (line 241).
- [x] AC4: `CrdtDebug.tsx` renders placeholder with message "CRDT debugging data will be available when system maps are implemented" (line 38). Four prepared sections: HLC Clock State, Merge History, Merkle Tree Summary, Conflict Inspector. View switcher in `DataExplorer.tsx` with "Data" and "CRDT Debug" buttons (lines 251-276). Sidebar remains visible in both views.
- [x] AC5: `Settings.tsx` has 2 tabs: "Server" (line 186) and "Integrations" (line 191). Loads from `/api/admin/settings` via SWR (line 64). Saves via `PUT` (line 83). Vector search: `<Switch disabled checked={false} />` (line 313). On success, `mutate(updatedSettings, false)` updates SWR cache (line 89). On error, displays `ErrorResponse.error` in toast (lines 96-100).
- [x] AC6: `api.ts` `login()` posts to `/api/auth/login` (line 43), returns `Promise<{ token: string }>` (line 42), stores via `setAuthToken(data.token)` which uses `topgun_admin_token` key (line 56). `Login.tsx` calls `await login(username, password)` and navigates on success -- no `user` object usage.
- [x] AC7: SWR replaces manual polling in `ClusterTopology.tsx` (line 39, `useSWR` with `refreshInterval: 5000`), `Settings.tsx` (line 64, `useSWR` with `refreshInterval: 10000`), `useServerStatus.ts` (line 10, `useSWR` with `refreshInterval: 5000`). No `setInterval` in any of these files.
- [x] AC8: All pages wrapped in `<ErrorBoundary>` in `App.tsx`. Structural verification shows clean component trees with proper null/loading states.
- [x] AC9: `Settings.tsx` contains no `Phase`, `SPEC-`, or `BUG-` references (verified by grep).
- [x] AC10: `App.tsx` has no `/setup` route, no `SetupWizard` import, no `bootstrap` redirect. `SetupWizard.tsx` has zero git diff (preserved unchanged).
- [x] AC11: `App.tsx` `ServerUnavailable` message: "Cannot connect to TopGun server. Make sure the server is running." (line 48). No "port 8080" or "port 9091" anywhere in `App.tsx`.
- [x] AC12: Token key `topgun_admin_token` used in: `api.ts` line 6 (`TOKEN_KEY`), `App.tsx` lines 23/30 (`ProtectedRoute`), `Layout.tsx` line 54 (logout handler), `client.ts` line 16 (session restore). No `topgun_token` (without `_admin_`) in any file except `SetupWizard.tsx` (preserved, inaccessible).
- [x] AC13: `useServerStatus.ts` imports `API_BASE` from `@/lib/api` (line 2). No duplicate `API_BASE` constant defined locally.

**Summary:** All 13 acceptance criteria are met. The implementation is clean, well-structured, and faithfully follows the specification. The SWR migration is consistent across all three target files. Type interfaces match the spec exactly. Settings page restructuring is thorough with proper read-only/editable field separation, badges, and error handling. The only finding is a minor spec reference in a code comment that violates project convention.
