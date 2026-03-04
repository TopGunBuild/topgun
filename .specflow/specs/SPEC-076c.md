---
id: SPEC-076c
type: feature
status: draft
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
3. CRDT Debug tab placeholder UI in Data Explorer
4. Token key unification across all files
5. Setup Wizard hidden from navigation (deferred to v1.1)
6. Type interfaces hand-typed to match Rust admin response structs

**Parent spec:** SPEC-076 (Admin Dashboard v1.0 -- Rust Server Adaptation)
**Depends on:** SPEC-076b (admin handlers + wiring must be available for integration verification)
**Sibling specs:** SPEC-076a (types + auth), SPEC-076b (handlers + wiring)

### Inherited Decisions

- System maps (`$sys/*`) are NOT populated; Dashboard page (`pages/Dashboard.tsx`) and Cluster page (`pages/Cluster.tsx`) will show placeholder/empty states via their `useQuery`/`useMap` hooks. They are separate from the admin-API-backed `features/cluster/ClusterTopology.tsx`
- CRDT Debug tab renders placeholder UI with empty state in v1.0; data population deferred until system maps are implemented in a follow-up spec
- Hand-typed TypeScript interfaces in `admin-api-types.ts` (no OpenAPI codegen for v1.0)
- SWR with `refreshInterval: 5000` (cluster) and `refreshInterval: 1000` (dashboard stats) replaces manual `setInterval` polling
- Bootstrap mode and Setup Wizard deferred to v1.1
- Vector search toggle in Settings preserved as read-only display (no backend wiring)
- Token key unified to `topgun_admin_token` across all files

### Available from SPEC-076a/076b

- Rust admin API endpoints:
  - `GET /api/status` -- public, returns `{ configured, version, mode }`
  - `POST /api/auth/login` -- public, returns `{ token }`
  - `GET /api/admin/cluster/status` -- admin auth required, returns `ClusterStatusResponse`
  - `GET /api/admin/maps` -- admin auth required, returns `MapsListResponse`
  - `GET /api/admin/settings` -- admin auth required, returns `SettingsResponse`
  - `PUT /api/admin/settings` -- admin auth required, accepts `SettingsUpdateRequest`
- `GET /api/openapi.json` and `GET /api/docs` (Swagger UI)
- `GET /admin/` serves the SPA via `ServeDir`

## Task

Adapt the existing React admin dashboard to consume the Rust admin API endpoints. Migrate data fetching to SWR, create hand-typed TypeScript interfaces matching Rust response structs, add a CRDT Debug tab placeholder, unify token keys, and hide the Setup Wizard.

## Requirements

### New files

1. `apps/admin-dashboard/src/lib/swr-config.ts` -- SWR global configuration:
   - Default fetcher using `adminFetch()` from `api.ts`
   - `refreshInterval` configuration
   - Error retry settings
   - `revalidateOnFocus: true`

2. `apps/admin-dashboard/src/features/explorer/CrdtDebug.tsx` -- CRDT Debug tab component (placeholder UI for v1.0):
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

4. `apps/admin-dashboard/src/lib/api.ts` -- Change `API_BASE` default from `http://localhost:9091` to same-origin (`''` or `window.location.origin`); add SWR configuration; unify localStorage token key to `topgun_admin_token`. Update the `login()` return type from `Promise<{ token: string; user: { id: string; username: string; role: string } }>` to `Promise<{ token: string }>` to match the Rust `LoginResponse` shape (no `user` object)

5. `apps/admin-dashboard/src/hooks/useServerStatus.ts` -- Replace manual fetch+polling with SWR hook. Consolidate the duplicate `API_BASE` constant (currently `import.meta.env.VITE_API_URL || 'http://localhost:9091'`) to use the shared `API_BASE` from `api.ts` or the SWR fetcher in `swr-config.ts` rather than maintaining a second hardcoded URL

6. `apps/admin-dashboard/src/features/explorer/DataExplorer.tsx` -- Add CRDT Debug tab (placeholder UI showing empty state with explanatory message; populated when system maps are wired in a follow-up spec)

7. `apps/admin-dashboard/src/features/cluster/ClusterTopology.tsx` -- Replace manual polling with SWR; adapt component to match Rust response shapes:
   - `NodeInfo`: `partitionCount: u32` instead of `partitions: number[]`, `memory: u64` (bytes) instead of `{ used, total }`, `address: String` added, `status: NodeStatus` enum
   - `PartitionInfo`: `ownerNodeId` instead of `owner`, no `replicas` field
   - Update partition ring visualization: `p.owner` becomes `p.ownerNodeId`; remove `replicas` rendering logic

8. `apps/admin-dashboard/src/features/settings/Settings.tsx` -- Replace manual polling with SWR; align settings structure with Rust `SettingsResponse`:
   - Remove `// Phase 14D-3: Settings Page` comment (violates code comment convention)
   - Preserve existing vector search toggle as read-only display (no backend wiring)
   - Change `PATCH /api/admin/settings` to `PUT /api/admin/settings`
   - Remove `POST /api/admin/settings/validate` call (endpoint does not exist)
   - Restructure save flow for `PUT` response handling

9. `apps/admin-dashboard/src/features/setup/SetupWizard.tsx` -- **Defer to v1.1.** Remove Setup Wizard from navigation/routing for v1.0 (the setup endpoint and bootstrap mode are deferred). The file itself is not deleted, only hidden from the UI

10. `apps/admin-dashboard/src/App.tsx` -- Remove the `status?.mode === 'bootstrap'` conditional redirect block and the `/setup` route entry. Update the `ServerUnavailable` component text to remove the hardcoded "port 9091" reference (replace with a generic message referencing the server's configured address). Update `ProtectedRoute` to use `topgun_admin_token` as the localStorage key (unifying with `api.ts`)

11. `apps/admin-dashboard/src/pages/Login.tsx` -- Adapt to the simpler Rust `LoginResponse` shape (`{ token }` only, no `user` object). The existing `Login.tsx` does NOT use the `user` object from the login response -- it calls `await login(username, password)` and navigates on success. The primary change is ensuring the component works with the updated `login()` return type in `api.ts` (which drops the `user` field). No JWT decoding is needed for v1.0

12. `apps/admin-dashboard/src/components/Layout.tsx` -- Change `localStorage.removeItem('topgun_token')` in the logout handler to `localStorage.removeItem('topgun_admin_token')`, unifying with the token key used by `api.ts` and `App.tsx`. Without this change, the logout button would remove the wrong key and leave the actual session token intact

13. `apps/admin-dashboard/src/lib/client.ts` -- Change `localStorage.getItem('topgun_token')` to `localStorage.getItem('topgun_admin_token')` so the TopGun WebSocket client reads the admin JWT from the same key used by `api.ts`, `App.tsx`, and `Layout.tsx`

14. `apps/admin-dashboard/package.json` -- Add `swr` dependency

15. `apps/admin-dashboard/vite.config.ts` -- Add dev proxy for `/api/*` to Rust server; configure static asset serving path

## Acceptance Criteria

1. Dashboard loads from Rust server (same origin, no port 9091 dependency)
2. Cluster Topology page renders nodes and partition ring from Rust `/api/admin/cluster/status` endpoint, adapted to the `NodeInfo` response shape (`partitionCount` instead of `partitions[]`, `memory` as `u64` bytes instead of `{ used, total }`, `address` as `String`) and the `PartitionInfo` response shape (`ownerNodeId` instead of `owner`, no `replicas` field)
3. Data Explorer lists maps from Rust `/api/admin/maps` endpoint with entry counts
4. CRDT Debug tab renders a placeholder UI with empty state message indicating system map data is not yet available; UI layout is prepared for future HLC, merge history, and Merkle tree data
5. Settings page loads configuration from Rust `/api/admin/settings` and saves changes via `PUT` (not `PATCH`); existing vector search toggle is preserved as read-only display
6. Login page authenticates via Rust `/api/auth/login`, handles the `{ token }` response (no `user` object), and stores JWT in localStorage under the key `topgun_admin_token`
7. SWR replaces manual `fetch`+`useState`+`setInterval` in at least ClusterTopology, Settings, and useServerStatus
8. All existing dashboard pages render without JavaScript errors when connected to Rust server
9. `Settings.tsx` does not contain phase/spec/bug reference comments
10. Setup Wizard is hidden from navigation (not accessible in v1.0); file is preserved for v1.1
11. `App.tsx` does not contain hardcoded "port 9091" references; `ServerUnavailable` component uses a generic server address message
12. `App.tsx`, `api.ts`, `Layout.tsx`, and `client.ts` all use the same localStorage token key (`topgun_admin_token`). Specifically, `Layout.tsx` logout handler removes `topgun_admin_token` (not `topgun_token`), and `client.ts` reads auth token from `topgun_admin_token`
13. `useServerStatus.ts` does not have its own `API_BASE` constant; it uses the shared base URL from `api.ts` or the SWR fetcher

## Constraints

- Do NOT modify Rust server files (those are in SPEC-076a and SPEC-076b)
- Do NOT add v2.0 features (pipeline visualization, connector wizard, Arroyo-style DAG editor)
- Do NOT add v3.0 features (tenant admin, S3 storage config, vector search config beyond toggle)
- Do NOT remove the TS server admin API compatibility (it will be removed in TODO-103)
- CRDT Debug tab renders placeholder UI only; it will read live data from system maps once they are populated in a follow-up spec (system maps are NOT populated in this spec)

## Assumptions

1. The Rust admin API (from SPEC-076a/076b) is operational and accessible on the same port as WebSocket/health/metrics.
2. `pages/Dashboard.tsx` and `pages/Cluster.tsx` (which use `useQuery('$sys/stats')` and `useQuery('$sys/cluster')` via the TopGun WebSocket client) will show placeholder/empty states or "No Cluster Data" messages in v1.0 since system maps are not populated. These pages are separate from the admin-API-backed `features/cluster/ClusterTopology.tsx` and will render without JavaScript errors (satisfying AC 8) but with degraded UX until system maps are implemented in a follow-up spec.
3. The existing `Login.tsx` does NOT use the `user` object from the login response. The simpler `{ token }` response from the Rust server is sufficient.
4. SWR with `refreshInterval` is an adequate replacement for manual `setInterval` polling.
5. Vector search toggle in Settings is preserved as-is (read-only display, no backend wiring).

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create `admin-api-types.ts` with hand-typed TypeScript interfaces matching Rust admin response structs (including `NodeStatus`, `ServerMode`) | -- | ~5% |
| G2 | 1 | Create `swr-config.ts` with SWR global configuration (fetcher, refreshInterval, error retry) | -- | ~3% |
| G3 | 1 | Create `CrdtDebug.tsx` placeholder UI component | -- | ~5% |
| G4 | 2 | Update `api.ts` (same-origin base URL, SWR config, `topgun_admin_token` key, simplified `login()` return type); update `useServerStatus.ts` (SWR hook, remove duplicate `API_BASE`) | G1, G2 | ~10% |
| G5 | 2 | Update `ClusterTopology.tsx` (SWR, Rust response shapes for NodeInfo and PartitionInfo); update `Settings.tsx` (SWR, PUT instead of PATCH, remove phase comment, preserve vector search toggle); integrate `CrdtDebug.tsx` into `DataExplorer.tsx` | G1, G2, G3 | ~15% |
| G6 | 2 | Update `App.tsx` (remove bootstrap redirect, `/setup` route, port 9091 reference, unify token key); update `Login.tsx` (simpler response shape); update `Layout.tsx` (unify logout token key); update `client.ts` (unify auth token key) | G1 | ~10% |
| G7 | 3 | Update `package.json` (add `swr`); update `vite.config.ts` (dev proxy for `/api/*`); hide Setup Wizard from navigation | G4, G5, G6 | ~5% |
| G8 | 3 | Integration verification: all dashboard pages work against Rust server without JS errors | G4, G5, G6 | ~5% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2, G3 | Yes | 3 |
| 2 | G4, G5, G6 | Yes | 3 |
| 3 | G7, G8 | Yes | 2 |

**Total workers needed:** 3 (max in any wave)
