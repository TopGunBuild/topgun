# SPEC-037: Document HTTP Sync Protocol and Serverless Deployment

---
id: SPEC-037
type: docs
status: done
priority: medium
complexity: medium
created: 2026-02-07
source: TODO-046
---

## Context

SPEC-036 (completed 2026-02-06) added HTTP sync protocol support for serverless environments: `HttpSyncProvider`, `AutoConnectionProvider`, `POST /sync` endpoint, and HTTP sync Zod schemas. These are major user-facing features with zero documentation. Users deploying TopGun on Vercel Edge, AWS Lambda, or Cloudflare Workers have no guidance on how to use the HTTP sync transport or configure serverless deployments.

The existing documentation covers only WebSocket-based sync (concepts/sync-protocol.mdx), traditional Docker/Kubernetes deployment (guides/deployment.mdx), WebSocket connection providers (reference/client.mdx), and server configuration without the `/sync` endpoint (reference/server.mdx).

### Prior Discussion

From TODO-046: Documents SPEC-036 implementation. Priority medium. Effort 1-2 days.

## Goal Statement

After this documentation is complete, developers can find, understand, and use TopGun's HTTP sync protocol for serverless environments by reading the official documentation site, without needing to read source code.

### Observable Truths

1. The sync-protocol concepts page explains HTTP sync alongside WebSocket/Merkle sync, including when to use each
2. The deployment guide contains serverless deployment examples for Vercel Edge, AWS Lambda, and Cloudflare Workers
3. The client API reference documents `HttpSyncProvider` and `AutoConnectionProvider` with all config options
4. The server API reference documents the `POST /sync` endpoint with request/response schemas and HTTP status codes
5. A decision guide helps users choose between WebSocket, HTTP sync, and `AutoConnectionProvider`

### Required Artifacts

| Observable Truth | Required Artifacts |
|---|---|
| OT1 (Sync concepts) | Updated `apps/docs-astro/src/content/docs/concepts/sync-protocol.mdx` |
| OT2 (Serverless deploy) | Updated `apps/docs-astro/src/content/docs/guides/deployment.mdx` |
| OT3 (Client API ref) | Updated `apps/docs-astro/src/content/docs/reference/client.mdx` |
| OT4 (Server API ref) | Updated `apps/docs-astro/src/content/docs/reference/server.mdx` |
| OT5 (Decision guide) | Embedded within sync-protocol.mdx as a decision table |

### Key Links

1. **HttpSyncProviderConfig accuracy:** All config parameter names, types, and defaults must match the actual implementation in `packages/client/src/connection/HttpSyncProvider.ts` (e.g., `pollIntervalMs` default 5000, `requestTimeoutMs` default 30000).
2. **AutoConnectionProviderConfig accuracy:** Config must match `packages/client/src/connection/AutoConnectionProvider.ts` (e.g., `maxWsAttempts` default 3, `httpOnly` boolean).
3. **HttpSyncRequest/Response schema accuracy:** Field names and types must match `packages/core/src/schemas/http-sync-schemas.ts`.
4. **POST /sync endpoint behavior:** Status codes (200, 400, 401, 403, 500), content types (msgpackr default, JSON fallback), and auth header format must match `ServerFactory.handleHttpSync()`.

## Task

Add HTTP sync protocol documentation to four existing documentation pages:

1. **Concepts: sync-protocol.mdx** -- Add "HTTP Sync" section explaining the polling-based sync mechanism, its stateless design, and a decision guide (WebSocket vs HTTP vs Auto)
2. **Guides: deployment.mdx** -- Add "Serverless Deployment" section with code examples for Vercel Edge Function, AWS Lambda, and Cloudflare Worker
3. **Reference: client.mdx** -- Add `HttpSyncProvider` and `AutoConnectionProvider` API reference sections with all config parameters
4. **Reference: server.mdx** -- Add `POST /sync` endpoint section with request/response schemas, HTTP status codes, and content type negotiation

## Requirements

### 1. File to Modify: `apps/docs-astro/src/content/docs/concepts/sync-protocol.mdx`

Add the following content after the existing "Merkle Tree Synchronization" section and before the "Server Architecture" section:

#### 1.1 "HTTP Sync" section

Explain the HTTP sync protocol:
- Stateless request-response model: client sends `POST /sync` with operations and sync timestamps, server returns acknowledgments and deltas
- Polling-based: client polls at a configurable interval (default 5s) to send queued operations and receive deltas
- Delta computation: server iterates in-memory LWWMap, filters records newer than client's `lastSyncTimestamp` using HLC comparison
- Each request carries full client context (clientId, clientHlc, syncMaps with timestamps) so any server node behind a load balancer can respond
- Authentication via `Authorization: Bearer <token>` header on every request

Include a visual step-by-step flow (matching the existing numbered list style for Merkle Tree exchange):
1. Client accumulates operations locally
2. At poll interval, client sends POST /sync with queued operations + syncMap timestamps
3. Server applies operations, computes deltas from in-memory maps
4. Server returns acknowledgments + delta records + query results in single response
5. Client applies deltas to local state, updates lastSyncTimestamp per map

#### 1.2 "When to Use Which Transport" decision guide

Add a comparison table/section after the HTTP Sync section:

| Criterion | WebSocket | HTTP Sync | AutoConnectionProvider |
|---|---|---|---|
| Real-time updates | Pushed instantly | Polled (configurable interval) | WS when available, HTTP fallback |
| Serverless compatible | No (needs persistent connection) | Yes (stateless requests) | Yes (auto-detects) |
| Live query subscriptions | Yes | No (one-shot queries only) | Depends on active transport |
| Bandwidth efficiency | Merkle tree delta sync | Timestamp-based delta sync | Best available |
| Connection cost | Per-connection billing | Per-request billing | Adapts to environment |
| Recommended for | Real-time apps, VPS/container deployments | Serverless functions, edge functions | Unknown deployment target |

### 2. File to Modify: `apps/docs-astro/src/content/docs/guides/deployment.mdx`

Add a "Serverless Deployment" section after the "TLS Configuration" section and before the footer navigation. Update the page description frontmatter to mention serverless.

#### 2.1 Section introduction

Brief explanation that TopGun supports serverless deployment via HTTP sync protocol (`POST /sync` endpoint). The server exposes this endpoint automatically when using `ServerFactory` or `ServerCoordinator`. For documentation of the endpoint's request/response format, link to the Server API reference.

Note: For serverless environments where users want a lightweight handler without the full `ServerCoordinator`, `HttpSyncHandler` can be used directly. However, `HttpSyncHandler` is not currently exported from the `@topgunbuild/server` package -- neither from the main entry point (`@topgunbuild/server`) nor as a sub-path export (`@topgunbuild/server/coordinator`). The server package's `package.json` exports map only contains `"."` and the tsup build only produces `index.js`/`index.mjs` entry points. A follow-up code change would be needed to make `HttpSyncHandler` independently importable. Until then, serverless examples should use `ServerFactory` or note this limitation.

#### 2.2 Vercel Edge Function example

Show a complete Vercel Edge Function that:
- Instantiates `HttpSyncHandler` directly (not the full `ServerCoordinator`, which is too heavy for serverless cold starts since it initializes WebSocket servers, cluster managers, and worker pools)
- Uses a module-scoped singleton pattern to reuse the handler across warm invocations
- Handles `POST /sync` by parsing the request body, calling `HttpSyncHandler.handleSyncRequest()`, and returning the response
- Demonstrates the stateless nature (any invocation can handle any request)
- Includes a prominent note/callout that `HttpSyncHandler` is not yet publicly exported, and that users must either: (a) add `"./coordinator"` to the server package's `exports` map and tsup entry points in their fork, or (b) wait for the public export to be added in a future release

#### 2.3 AWS Lambda handler example

Show a Lambda handler that processes POST /sync requests, including:
- Extracting Authorization header
- Parsing request body (msgpackr or JSON)
- Calling `HttpSyncHandler.handleSyncRequest()` as the core integration point
- Returning proper HTTP response
- Same note about import path limitation as 2.2

#### 2.4 Cloudflare Worker example

Show a Cloudflare Worker handler for POST /sync, using `HttpSyncHandler.handleSyncRequest()` as the core integration point and same note about import path limitation as 2.2.

#### 2.5 Client-side configuration

Show how to configure the client to connect via HTTP sync for serverless. Code examples must show the correct public API: `HttpSyncProvider` and `AutoConnectionProvider` are `IConnectionProvider` implementations that are instantiated directly and passed to `SyncEngine` (the low-level sync orchestrator), not to `TopGunClient` (which does not accept a `connectionProvider` option). Examples should show:

```typescript
import { HttpSyncProvider } from '@topgunbuild/client';
import { SyncEngine } from '@topgunbuild/client';
import { HLC } from '@topgunbuild/core';

const hlc = new HLC('client-1');
const provider = new HttpSyncProvider({
  url: 'https://your-api.vercel.app',
  clientId: 'client-1',
  hlc,
  authToken: 'your-jwt-token',
  syncMaps: ['todos'],
});

const engine = new SyncEngine({
  nodeId: 'client-1',
  connectionProvider: provider,
  storageAdapter: myStorageAdapter, // IStorageAdapter instance
  // ... other SyncEngine config
});
```

And similarly for `AutoConnectionProvider`.

### 3. File to Modify: `apps/docs-astro/src/content/docs/reference/client.mdx`

Add two new sections after the "Cluster Mode" subsection and before "Core Methods":

#### 3.1 "HTTP Sync Mode" section

Brief description: for serverless environments where WebSocket connections are unavailable. Note that `HttpSyncProvider` implements the `IConnectionProvider` interface and is used with `SyncEngine` directly (the low-level sync orchestrator), not with `TopGunClient` (the high-level convenience wrapper that only accepts `serverUrl` and `cluster` config options).

Code example showing `HttpSyncProvider` usage with `SyncEngine` (export as a const string variable at the top of the file, following the existing pattern). Example must include all required `SyncEngineConfig` fields: `nodeId`, `connectionProvider`, and `storageAdapter`.

#### 3.2 `HttpSyncProviderConfig` API reference

Document all config options using the `ApiConstructor` + `ApiParam` component pattern (matching existing `ClusterConfig` section):

| Parameter | Type | Description |
|---|---|---|
| `url` | `string` | HTTP URL of the TopGun server (e.g., `https://api.example.com`) |
| `clientId` | `string` | Client identifier for request construction |
| `hlc` | `HLC` | Hybrid Logical Clock instance for causality tracking |
| `authToken?` | `string` | JWT auth token for Authorization header |
| `pollIntervalMs?` | `number` | Polling interval in ms. Default: 5000 |
| `requestTimeoutMs?` | `number` | HTTP request timeout in ms. Default: 30000 |
| `syncMaps?` | `string[]` | Map names to sync deltas for on each poll |
| `fetchImpl?` | `typeof fetch` | Custom fetch implementation for testing or platform compatibility |

#### 3.3 "Auto Connection Mode" section

Brief description: automatically tries WebSocket, falls back to HTTP. Note that `AutoConnectionProvider` implements the `IConnectionProvider` interface and is used with `SyncEngine` directly, not with `TopGunClient`.

Code example showing `AutoConnectionProvider` usage with `SyncEngine`. Example must include all required `SyncEngineConfig` fields: `nodeId`, `connectionProvider`, and `storageAdapter`.

#### 3.4 `AutoConnectionProviderConfig` API reference

Document all config options:

| Parameter | Type | Description |
|---|---|---|
| `url` | `string` | Server URL (ws:// or http://) |
| `clientId` | `string` | Client identifier |
| `hlc` | `HLC` | Hybrid Logical Clock instance |
| `maxWsAttempts?` | `number` | Max WebSocket attempts before HTTP fallback. Default: 3 |
| `authToken?` | `string` | JWT auth token |
| `httpOnly?` | `boolean` | Skip WebSocket, go HTTP-only. Default: false |
| `httpPollIntervalMs?` | `number` | HTTP polling interval in ms. Default: 5000 |
| `syncMaps?` | `string[]` | Map names to sync via HTTP |
| `fetchImpl?` | `typeof fetch` | Custom fetch implementation for HTTP mode |

#### 3.5 Info callout

Add an info box (matching existing style) explaining that HTTP mode supports batch operations and one-shot queries only -- no live subscriptions, no Merkle sync, no real-time topic messages.

### 4. File to Modify: `apps/docs-astro/src/content/docs/reference/server.mdx`

Add a "POST /sync Endpoint" section after the "Methods" section and before the footer navigation.

#### 4.1 Endpoint overview

- URL: `POST /sync`
- Description: Stateless HTTP sync endpoint for serverless environments
- Authentication: `Authorization: Bearer <token>` header (required)
- Content types: `application/x-msgpack` (default), `application/json` (fallback)
- Response content type matches request content type

#### 4.2 Request schema (`HttpSyncRequest`)

Document all fields using a table or the `ApiConstructor`/`ApiParam` pattern:

| Field | Type | Required | Description |
|---|---|---|---|
| `clientId` | `string` | Yes | Client identifier |
| `clientHlc` | `Timestamp` | Yes | Client's current HLC (object: `{millis, counter, nodeId}`) |
| `operations` | `ClientOp[]` | No | Batch of operations to push |
| `syncMaps` | `SyncMapEntry[]` | No | Maps to pull deltas for, each with `mapName` and `lastSyncTimestamp` |
| `queries` | `HttpQueryRequest[]` | No | One-shot queries to execute |
| `searches` | `HttpSearchRequest[]` | No | One-shot search requests |

Include sub-schema details for `SyncMapEntry` (`mapName: string`, `lastSyncTimestamp: Timestamp`) and `HttpQueryRequest` (`queryId`, `mapName`, `filter`, `limit?`, `offset?`).

#### 4.3 Response schema (`HttpSyncResponse`)

| Field | Type | Description |
|---|---|---|
| `serverHlc` | `Timestamp` | Server's current HLC timestamp |
| `ack` | `{lastId, results?}` | Acknowledgment of processed operations |
| `deltas` | `MapDelta[]` | Delta records for requested maps (records newer than client's lastSyncTimestamp) |
| `queryResults` | `HttpQueryResult[]` | Results for one-shot queries |
| `searchResults` | `HttpSearchResult[]` | Results for one-shot searches |
| `errors` | `HttpSyncError[]` | Errors for individual operations (code, message, context) |

Include sub-schema details for `MapDelta` (`mapName`, `records: DeltaRecord[]`, `serverSyncTimestamp`) and `DeltaRecord` (`key`, `record: LWWRecord`, `eventType: 'PUT' | 'REMOVE'`).

#### 4.4 HTTP status codes

| Status | Meaning |
|---|---|
| 200 | Success -- response body contains `HttpSyncResponse` |
| 400 | Invalid request body (Zod validation failure) |
| 401 | Missing or invalid `Authorization` header / JWT verification failed |
| 403 | Permission denied for specific operations (individual errors in response `errors` array) |
| 500 | Internal server error |

#### 4.5 Example request/response

Show a JSON example of a POST /sync request and response for clarity (JSON is easier to read in docs than msgpackr binary):

```json
// Request
{
  "clientId": "client-1",
  "clientHlc": { "millis": 1706000000000, "counter": 0, "nodeId": "client-1" },
  "operations": [
    { "mapName": "todos", "key": "t1", "record": { "value": { "text": "Buy milk" }, "timestamp": {...} } }
  ],
  "syncMaps": [
    { "mapName": "todos", "lastSyncTimestamp": { "millis": 1705999000000, "counter": 0, "nodeId": "" } }
  ]
}

// Response
{
  "serverHlc": { "millis": 1706000001000, "counter": 1, "nodeId": "server-1" },
  "ack": { "lastId": "http-op-0", "results": [{ "opId": "http-op-0", "success": true, "achievedLevel": "MEMORY" }] },
  "deltas": [{
    "mapName": "todos",
    "records": [{ "key": "t2", "record": {...}, "eventType": "PUT" }],
    "serverSyncTimestamp": { "millis": 1706000001000, "counter": 2, "nodeId": "server-1" }
  }]
}
```

## Acceptance Criteria

1. **AC1:** `apps/docs-astro/src/content/docs/concepts/sync-protocol.mdx` contains an "HTTP Sync" section that explains the polling-based stateless sync mechanism with a step-by-step flow
2. **AC2:** `sync-protocol.mdx` contains a "When to Use Which Transport" decision table comparing WebSocket, HTTP Sync, and AutoConnectionProvider across at least 5 criteria
3. **AC3:** `apps/docs-astro/src/content/docs/guides/deployment.mdx` contains a "Serverless Deployment" section with code examples for Vercel Edge Function, AWS Lambda handler, and Cloudflare Worker
4. **AC4:** `deployment.mdx` shows client-side configuration examples using both `HttpSyncProvider` and `AutoConnectionProvider` with `SyncEngine` (not `TopGunClient`)
5. **AC5:** `apps/docs-astro/src/content/docs/reference/client.mdx` documents `HttpSyncProvider` with all 8 config parameters (`url`, `clientId`, `hlc`, `authToken`, `pollIntervalMs`, `requestTimeoutMs`, `syncMaps`, `fetchImpl`) using the existing `ApiConstructor`/`ApiParam` component pattern
6. **AC6:** `reference/client.mdx` documents `AutoConnectionProvider` with all 9 config parameters (`url`, `clientId`, `hlc`, `maxWsAttempts`, `authToken`, `httpOnly`, `httpPollIntervalMs`, `syncMaps`, `fetchImpl`) using the same component pattern
7. **AC7:** `apps/docs-astro/src/content/docs/reference/server.mdx` documents the `POST /sync` endpoint with request schema fields, response schema fields, and HTTP status codes (200, 400, 401, 403, 500)
8. **AC8:** `reference/server.mdx` includes a JSON example showing a complete request/response pair for `POST /sync`
9. **AC9:** All config parameter names, types, and default values in docs match the actual implementation source code exactly
10. **AC10:** All documentation follows the existing MDX patterns: `CodeBlock` for code, `ApiConstructor`/`ApiParam` for API references, lucide-react icons for section headers, consistent CSS classes
11. **AC11:** An info callout in `reference/client.mdx` explains that HTTP mode does not support live subscriptions, Merkle sync, or real-time topic messages
12. **AC12:** All code examples in sections 2.5, 3.1, and 3.3 show providers used with `SyncEngine`, not `TopGunClient`, reflecting the actual public API; examples include all required `SyncEngineConfig` fields (`nodeId`, `connectionProvider`, `storageAdapter`)
13. **AC13:** Serverless deployment examples (sections 2.2-2.4) use `HttpSyncHandler.handleSyncRequest()` directly rather than the full `ServerCoordinator`, and include a prominent note that `HttpSyncHandler` is not yet publicly exported from the `@topgunbuild/server` package
14. **AC14:** Section 2.1 accurately describes the current export limitation: `HttpSyncHandler` is not available via `@topgunbuild/server` or any sub-path export, and a code change is needed to make it independently importable

## Constraints

1. **DO NOT** modify any TypeScript source code files -- this spec is documentation only
2. **DO NOT** create new documentation pages -- only modify the four existing files listed in Requirements. Since only existing pages are modified, no sidebar configuration or next/prev navigation link changes are needed.
3. **DO** follow the existing MDX component patterns used in each file (CodeBlock, ApiParam, ApiConstructor, lucide-react icons, styled divs with Tailwind classes)
4. **DO** export code example strings as `const` variables at the top of MDX files, following the existing pattern in each file
5. **DO** use the same breadcrumb navigation component pattern already present in each file
6. **DO NOT** change existing content or section ordering except where sections need to be inserted (e.g., before footer navigation)
7. **DO** keep the existing footer navigation links intact in all four files
8. **DO** use accurate default values from the implementation: `pollIntervalMs: 5000`, `requestTimeoutMs: 30000`, `maxWsAttempts: 3`
9. **DO** follow the commit message format: `docs(scope): description`
10. **DO NOT** add emoji to documentation content

## Assumptions

1. **Serverless code examples are illustrative:** The Vercel/Lambda/Cloudflare examples show the general pattern for wiring `HttpSyncHandler` into a serverless function. They do not need to be production-ready or tested -- they demonstrate the integration approach.
2. **JSON examples for readability:** The `POST /sync` request/response example in server docs uses JSON format for readability, even though the default wire format is msgpackr. A note clarifies this.
3. **No new Astro components needed:** All documentation can be written using existing MDX components (`CodeBlock`, `ApiConstructor`, `ApiParam`, styled divs). No new React components are required.
4. **Providers are used with SyncEngine, not TopGunClient:** `HttpSyncProvider` and `AutoConnectionProvider` implement the `IConnectionProvider` interface and are publicly exported from `@topgunbuild/client`. They are instantiated directly and passed to `SyncEngine` (the low-level sync orchestrator) via its `connectionProvider` config option. `TopGunClient` (the high-level convenience wrapper) does not accept a `connectionProvider` option -- its `TopGunClientConfig` only has `serverUrl?: string` and `cluster?: TopGunClusterConfig`. Documentation must show `SyncEngine` usage for these providers, and clearly note this distinction.
5. **Page description updates are acceptable:** Frontmatter `description` fields can be updated to mention HTTP sync/serverless where appropriate (e.g., deployment.mdx adding "serverless" to its description).
6. **Docs site builds with MDX:** The docs site at `apps/docs-astro` renders `.mdx` files. No special build steps are needed beyond what already exists.
7. **HttpSyncHandler is the serverless integration point:** Serverless deployment examples use `HttpSyncHandler` directly (with `HttpSyncHandler.handleSyncRequest()`) rather than the full `ServerCoordinator`, because `ServerCoordinator` is a heavy object that initializes WebSocket servers, cluster managers, worker pools, and PostgreSQL connections -- inappropriate for cold-start-sensitive serverless functions. `HttpSyncHandler` is the lightweight, stateless handler designed for this use case.
8. **HttpSyncHandler is not publicly importable (current state):** `HttpSyncHandler` is not exported from the `@topgunbuild/server` package's public API. The main entry point (`packages/server/src/index.ts`) does not re-export from the coordinator barrel. The `package.json` exports map only contains `"."` (no `"./coordinator"` sub-path). The tsup build (`tsup.config.ts`) only has `src/index.ts` and `src/start-server.ts` as entry points, producing `dist/index.js` and `dist/start-server.js` -- no `dist/coordinator.*` files exist. The import path `@topgunbuild/server/coordinator` will fail at module resolution. Serverless deployment examples must include a prominent note about this limitation, explaining that a code change (adding the coordinator as a package sub-path export and tsup entry point) is needed before `HttpSyncHandler` can be imported independently. A follow-up TODO should be created to add this export.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Update `concepts/sync-protocol.mdx`: Add HTTP Sync section and decision guide table | -- | ~20% |
| G2 | 1 | Update `reference/server.mdx`: Add POST /sync endpoint section with schemas, status codes, and JSON example | -- | ~20% |
| G3 | 2 | Update `reference/client.mdx`: Add HttpSyncProvider and AutoConnectionProvider API reference sections (showing SyncEngine usage with all required config fields) | G1 | ~25% |
| G4 | 2 | Update `guides/deployment.mdx`: Add Serverless Deployment section with HttpSyncHandler-based Vercel/Lambda/Cloudflare examples (with import limitation note) and SyncEngine client config with all required fields | G2 | ~25% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3, G4 | Yes | 2 |

**Total workers needed:** 2 (max in any wave)

Note: G3 depends on G1 because the client reference may cross-link to the sync-protocol concepts page. G4 depends on G2 because the deployment guide may reference the server API endpoint documentation. These are soft dependencies (cross-links) rather than hard code dependencies, so wave 2 could start in parallel with wave 1 if workers insert placeholder links.

## Audit History

### Audit v1 (2026-02-07)
**Status:** NEEDS_REVISION

**Context Estimate:** ~90% total (across all workers), ~25% max per worker (within target for parallel execution)

**Critical:**
1. **Assumption 4 is incorrect -- `TopGunClient` does not accept a `connectionProvider` option.** The `TopGunClientConfig` interface (in `packages/client/src/TopGunClient.ts`, lines 70-88) only accepts `serverUrl?: string` or `cluster?: TopGunClusterConfig`. There is no `connectionProvider` property. The code examples in section 2.5 (`new TopGunClient({ connectionProvider: provider })`) and implicitly in sections 3.1/3.3 would produce non-functional documentation. Users following these examples would get TypeScript compilation errors. The spec must either: (a) update the code examples to show the correct API for using `HttpSyncProvider`/`AutoConnectionProvider` with `TopGunClient`, or (b) document these as standalone `IConnectionProvider` implementations used with `SyncEngine` directly, or (c) note that the client API needs to be extended first and add a prerequisite task.

**Recommendations:**
2. **Key Link 4 status code list is incomplete.** The Key Link 4 description says "Status codes (200, 400, 401)" but the actual implementation in `ServerFactory.handleHttpSync()` also returns 403 (line 459) and 500 (line 464). The Requirements section 4.4 and AC7 correctly list all 5 codes, so this is just an inconsistency in the Key Link description text. Update Key Link 4 to say "Status codes (200, 400, 401, 403, 500)" for consistency.
3. **[Strategic] Serverless deployment examples reference `ServerCoordinator` but serverless environments may not support the full coordinator.** The Vercel/Lambda/Cloudflare examples (sections 2.2-2.4) suggest creating a `ServerCoordinator` instance inside serverless functions. The `ServerCoordinator` is a heavy object that initializes WebSocket servers, cluster managers, worker pools, and PostgreSQL connections. It may not be appropriate for cold-start-sensitive serverless functions. Consider whether the examples should use `HttpSyncHandler` directly (lighter weight) rather than the full `ServerCoordinator`, and clarify the expected initialization pattern (global singleton vs per-request).

### Response v1 (2026-02-07)
**Applied:** All 3 audit items plus sidebar/navigation clarification

**Changes:**
1. [✓] **Assumption 4 rewritten** -- Replaced incorrect "connectionProvider constructor option exists" with accurate description: providers implement `IConnectionProvider` and are used with `SyncEngine` directly, not `TopGunClient`. Documented that `TopGunClientConfig` only accepts `serverUrl` and `cluster`.
2. [✓] **Section 2.5 code examples fixed** -- Replaced `TopGunClient({ connectionProvider })` pattern with correct `SyncEngine({ connectionProvider })` usage, including a complete code snippet showing `HttpSyncProvider` and `AutoConnectionProvider` instantiation with `SyncEngine`.
3. [✓] **Sections 3.1 and 3.3 descriptions fixed** -- Updated both sections to explicitly state providers are used with `SyncEngine` (not `TopGunClient`) and that code examples must show `SyncEngine` usage.
4. [✓] **Key Link 4 updated** -- Changed status code list from "(200, 400, 401)" to "(200, 400, 401, 403, 500)" for consistency with section 4.4 and AC7.
5. [✓] **Sections 2.2-2.4 updated for HttpSyncHandler** -- Replaced `ServerCoordinator` references with `HttpSyncHandler` as the lightweight serverless integration point. Added Assumption 7 documenting why `HttpSyncHandler.handleRequest()` is preferred over full `ServerCoordinator` for serverless.
6. [✓] **AC4 updated** -- Added "with `SyncEngine` (not `TopGunClient`)" clarification.
7. [✓] **AC12 added** -- New acceptance criterion: all code examples in sections 2.5, 3.1, and 3.3 show providers used with `SyncEngine`, not `TopGunClient`.
8. [✓] **AC13 added** -- New acceptance criterion: serverless deployment examples use `HttpSyncHandler.handleRequest()` directly rather than full `ServerCoordinator`.
9. [✓] **Constraint 2 clarified** -- Added note that since only existing pages are modified, no sidebar or navigation link changes are needed.
10. [✓] **G4 task description updated** -- Reflects HttpSyncHandler-based examples and SyncEngine client config.

### Audit v2 (2026-02-07)
**Status:** APPROVED

**Context Estimate:** ~90% total (across all workers), ~25% max per worker (within target for parallel execution)

**Inline Fix Applied:**
1. **Method name corrected: `handleRequest()` changed to `handleSyncRequest()` throughout.** The spec previously referenced `HttpSyncHandler.handleRequest()` in sections 2.1-2.4, Assumption 7, and AC13, but the actual method in `packages/server/src/coordinator/http-sync-handler.ts` (line 48) is `handleSyncRequest(request, authToken)`. All occurrences in the spec body have been corrected to `handleSyncRequest()` as part of this audit. Historical entries in Response v1 are left unchanged as they are audit history.

**Recommendations (Optional):**
2. **`HttpSyncHandler` is not publicly exported from `@topgunbuild/server`.** The server package's `index.ts` (`packages/server/src/index.ts`) does not export from the coordinator module. `HttpSyncHandler` is only exported from `packages/server/src/coordinator/index.ts` (an internal barrel file). Serverless deployment examples showing `import { HttpSyncHandler } from '@topgunbuild/server'` would fail at import time. Since Assumption 1 acknowledges examples are illustrative and Constraint 1 forbids source code changes, consider either: (a) noting in the docs that a deep import path is needed (e.g., `@topgunbuild/server/coordinator`), or (b) adding a follow-up TODO to export `HttpSyncHandler` from the package's public API.
3. **`SyncEngine` requires `nodeId` and `storageAdapter` in addition to `connectionProvider`.** The `SyncEngineConfig` interface requires `nodeId: string`, `connectionProvider: IConnectionProvider`, and `storageAdapter: IStorageAdapter` as non-optional fields. The code example in section 2.5 shows `// ... other SyncEngine config` which is acceptable, but sections 3.1 and 3.3 should ensure their code examples also include or acknowledge these required fields so implementers produce complete, functional examples.

**Comment:** Well-structured documentation spec with clear per-file requirements, accurate config parameter tables verified against source code, and thorough acceptance criteria. All 10 audit dimensions pass. Goal analysis is complete with full truth-artifact coverage. The method name was the only accuracy issue and has been corrected inline.

**Verification Summary:**
- `HttpSyncProviderConfig`: 8 parameters verified against `packages/client/src/connection/HttpSyncProvider.ts` -- all names, types, and defaults match
- `AutoConnectionProviderConfig`: 9 parameters verified against `packages/client/src/connection/AutoConnectionProvider.ts` -- all names, types, and defaults match
- `HttpSyncRequest/Response` schemas: All fields verified against `packages/core/src/schemas/http-sync-schemas.ts` -- match
- `SyncEngine.connectionProvider`: Verified in `packages/client/src/SyncEngine.ts` (line 85) -- correct
- `TopGunClientConfig`: Verified only has `serverUrl` and `cluster` (no `connectionProvider`) -- spec is correct
- `HttpSyncHandler.handleSyncRequest()`: Verified in `packages/server/src/coordinator/http-sync-handler.ts` (line 48) -- corrected in spec
- Existing MDX file structure: All 4 target files verified for insertion points and component patterns

### Response v2 (2026-02-07)
**Applied:** Both recommendations (2 and 3) from Audit v2

**Changes:**
1. [✓] **Recommendation 2: HttpSyncHandler deep import path** -- Added note to section 2.1 explaining that `HttpSyncHandler` is not exported from the main `@topgunbuild/server` entry point and requires the deep import path `@topgunbuild/server/coordinator`. Updated sections 2.2-2.4 examples to use the deep import path. Added Assumption 8 documenting the current export state and the required deep import path. Added AC14 to verify section 2.1 includes this note.
2. [✓] **Recommendation 3: SyncEngine required fields** -- Updated section 3.1 requirement to specify that code examples must include all required `SyncEngineConfig` fields (`nodeId`, `connectionProvider`, `storageAdapter`). Updated section 3.3 requirement identically. Updated AC12 to verify examples include all required fields. Updated G3 task description to note "showing SyncEngine usage with all required config fields". Updated G4 task description to note "SyncEngine client config with all required fields".

**Skipped:** None -- both recommendations applied.

### Audit v3 (2026-02-07)
**Status:** NEEDS_REVISION

**Context Estimate:** ~90% total (across all workers), ~25% max per worker (within target for parallel execution)

**Critical:**
1. **Assumption 8 is incorrect -- the deep import path `@topgunbuild/server/coordinator` does not exist and will not work.** The previous audit (v2) recommended noting a deep import path, and Response v2 added Assumption 8 stating `import { HttpSyncHandler } from '@topgunbuild/server/coordinator'` as the required import. However, this import path is not resolvable:

   - **tsup.config.ts** (`packages/server/tsup.config.ts`, line 6): Entry points are only `['src/index.ts', 'src/start-server.ts']`. There is no `src/coordinator/index.ts` entry point.
   - **package.json** (`packages/server/package.json`, lines 7-13): The `exports` field only maps `"."` -- there is no `"./coordinator"` sub-path export.
   - **dist/ directory**: Contains only `index.js`, `index.mjs`, `start-server.js`, `start-server.mjs`, and opaque chunk files. There is no `dist/coordinator.js`, `dist/coordinator.mjs`, or `dist/coordinator/` directory.

   Any user writing `import { HttpSyncHandler } from '@topgunbuild/server/coordinator'` will get a "Module not found" error. The serverless deployment examples in sections 2.1-2.4, AC13, and AC14 all depend on this non-existent import path.

   **Resolution options:**
   - **(a) Recommended:** Update Assumption 8 to accurately describe the limitation: `HttpSyncHandler` cannot currently be imported at all by external consumers. Update sections 2.1-2.4 to include a prominent note/callout that the import shown is aspirational and requires a code change (adding `"./coordinator"` to the package's `exports` map in `package.json` and `src/coordinator/index.ts` to tsup entry points). Update AC13 and AC14 to reflect this accurate description. Optionally create a follow-up TODO for the code change.
   - **(b) Alternative:** Remove Constraint 1 and add a prerequisite task to make the export work before documenting it. This changes the spec from docs-only to docs+code.

**Recommendations:**
2. **[Compliance] Constraint 1 ("DO NOT modify any TypeScript source code files") creates a contradiction.** The spec instructs implementers to write documentation showing `import { HttpSyncHandler } from '@topgunbuild/server/coordinator'`, but Constraint 1 prevents the code change that would make this import work. The documentation would be technically inaccurate. Consider either: (a) relaxing Constraint 1 to allow the minimal code change (add exports map entry + tsup entry point), or (b) explicitly acknowledging in the spec and documentation that the serverless examples are forward-looking and require a code change that is tracked separately.
3. **AC14 text should be updated.** Currently says "requires the deep import path `@topgunbuild/server/coordinator`". Should instead say something like "is not yet publicly exported from `@topgunbuild/server` and a code change is needed to make it independently importable."

**Verification (re-confirmed from Audit v2):**
- `HttpSyncProviderConfig`: 8 parameters match source -- correct
- `AutoConnectionProviderConfig`: 9 parameters match source -- correct
- `HttpSyncRequest/Response` schemas: All fields match source -- correct
- `HttpSyncHandler.handleSyncRequest()`: Method name correct
- `SyncEngine.connectionProvider`: Correct (line 85 of SyncEngine.ts)
- Client-side code examples (sections 2.5, 3.1, 3.3): Correctly use `SyncEngine` not `TopGunClient`

**All other dimensions pass:** Clarity, completeness (aside from the import issue), testability, scope, architecture fit, non-duplication, cognitive load, strategic fit, and project compliance (aside from the Constraint 1 tension) are all satisfactory.

### Response v3 (2026-02-07)
**Applied:** All 3 audit items from Audit v3

**Changes:**
1. [✓] **Critical item 1: Assumption 8 rewritten** -- Updated Assumption 8 (line 325) to accurately describe that the deep import path `@topgunbuild/server/coordinator` does not exist. Now correctly states that the `package.json` exports map only contains `"."`, the tsup build only produces `index.js`/`index.mjs` entry points, and no `dist/coordinator.*` files exist. The import path will fail at module resolution. Noted that a code change (adding the coordinator as a package sub-path export and tsup entry point) is needed before `HttpSyncHandler` can be imported independently.
2. [✓] **Recommendation 2: Constraint 1 contradiction addressed** -- Chose option (b): kept Constraint 1 as docs-only, but updated sections 2.1-2.4 to explicitly acknowledge the limitation. Section 2.1 (lines 102-104) now includes a prominent note explaining that `HttpSyncHandler` is not currently exported from `@topgunbuild/server` (neither main entry point nor sub-path export), and that a code change would be needed to make it independently importable. Sections 2.2-2.4 (lines 113, 122, 126) each include notes about the import limitation. AC13 (line 300) updated to require the note. This approach allows the spec to remain docs-only while accurately describing the current state.
3. [✓] **Recommendation 3: AC14 text updated** -- Updated AC14 (line 301) from "requires the deep import path `@topgunbuild/server/coordinator`" to "accurately describes the current export limitation: `HttpSyncHandler` is not available via `@topgunbuild/server` or any sub-path export, and a code change is needed to make it independently importable."

**Additional changes:**
4. [✓] **Section 2.1 updated** -- Lines 102-104 rewritten to mention `ServerFactory` as the default serverless integration point, with a note explaining the `HttpSyncHandler` limitation.
5. [✓] **Sections 2.2-2.4 updated** -- Each section now includes a prominent note/callout that `HttpSyncHandler` is not yet publicly exported.
6. [✓] **G4 task description updated** -- Line 336 now notes "(with import limitation note)" to reflect the requirement.

**Skipped:** None -- all items applied.

**Note:** The spec now accurately reflects the current state: `HttpSyncHandler` cannot be independently imported by external consumers. The documentation will guide users to use `ServerFactory` by default, or note the limitation if they want to use the lighter-weight `HttpSyncHandler` pattern. A follow-up TODO should be created to add the public export once this spec is implemented.

### Audit v4 (2026-02-07)
**Status:** APPROVED

**Context Estimate:** ~90% total (across all workers), ~25% max per worker (within target for parallel execution)

**Quality Projection:**

| Context Range | Expected Quality | Status |
|---------------|------------------|--------|
| 0-30% | PEAK | -- |
| 30-50% | GOOD | -- |
| 50-70% | DEGRADING | -- |
| 70%+ | POOR | -- |

Per-worker context is ~20-25%, well within the PEAK range for each individual worker.

**Per-Group Breakdown:**

| Group | Wave | Tasks | Est. Context | Status |
|-------|------|-------|--------------|--------|
| G1 | 1 | sync-protocol.mdx: HTTP Sync + decision guide | ~20% | OK |
| G2 | 1 | server.mdx: POST /sync endpoint docs | ~20% | OK |
| G3 | 2 | client.mdx: HttpSyncProvider + AutoConnectionProvider API ref | ~25% | OK |
| G4 | 2 | deployment.mdx: Serverless examples + client config | ~25% | OK |

**Source Code Verification (independently re-confirmed):**
- `HttpSyncProviderConfig`: 8 parameters at `packages/client/src/connection/HttpSyncProvider.ts` lines 13-30 -- all names, types, defaults match spec (pollIntervalMs=5000 at line 75, requestTimeoutMs=30000 at line 76)
- `AutoConnectionProviderConfig`: 9 parameters at `packages/client/src/connection/AutoConnectionProvider.ts` lines 15-34 -- all match (maxWsAttempts=3 at line 56, httpOnly=false at line 57)
- `HttpSyncRequest` schema: 6 top-level fields at `packages/core/src/schemas/http-sync-schemas.ts` lines 48-61 -- match
- `HttpSyncResponse` schema: 6 top-level fields at same file lines 119-135 -- match
- `HttpSyncHandler.handleSyncRequest()`: Confirmed at `packages/server/src/coordinator/http-sync-handler.ts` line 48
- `ServerFactory.handleHttpSync()`: Status codes 200 (lines 446, 450), 400 (line 433), 401 (lines 408, 457), 403 (line 460), 500 (line 464) -- all 5 match spec
- `SyncEngineConfig`: `nodeId`, `connectionProvider`, `storageAdapter` required at `packages/client/src/SyncEngine.ts` lines 82-86
- `TopGunClientConfig`: No `connectionProvider` field (lines 70-88 of TopGunClient.ts)
- `HttpSyncHandler` export: NOT in `packages/server/src/index.ts`; only in internal barrel `coordinator/index.ts` line 42; `package.json` exports only `"."`, `tsup.config.ts` entries only `src/index.ts` and `src/start-server.ts` -- Assumption 8 is now accurate

**MDX File Insertion Points Verified:**
- `sync-protocol.mdx`: "Merkle Tree Synchronization" ends at line 79, "Server Architecture" begins at line 81 -- correct insertion point
- `deployment.mdx`: "TLS Configuration" section starts at line 313, footer nav follows -- correct
- `client.mdx`: "Cluster Mode" at line 193, "Core Methods" at line 250 -- correct insertion point between them
- `server.mdx`: "Methods" section starts at line 292, with `shutdown()` as last method -- footer follows

**All 10 Audit Dimensions Pass:**
1. Clarity: Requirements are specific per-file with exact section names, content, and component patterns
2. Completeness: All 4 files identified, all config parameters enumerated, all schemas documented
3. Testability: 14 acceptance criteria are concrete and measurable
4. Scope: Bounded to 4 files, docs-only, no new pages
5. Feasibility: Updates to existing MDX files using existing components
6. Architecture fit: Uses established MDX patterns (CodeBlock, ApiConstructor, ApiParam)
7. Non-duplication: Fills documentation gap, does not duplicate
8. Cognitive load: Logical 4-file structure matching existing doc organization
9. Strategic fit: Documents major SPEC-036 features, essential for adoption
10. Project compliance: Honors all PROJECT.md constraints, follows commit format

**Goal Analysis Validation:**
- All 5 observable truths have corresponding artifacts -- complete coverage
- All artifacts map to at least one truth -- no orphans
- Key links 1-4 all verified against source code -- accurate
- Wiring: OT5 embedded in OT1's artifact -- reasonable

**Comment:** This specification has been through 3 prior audit cycles and all critical issues have been resolved. Assumption 8 now accurately describes the `HttpSyncHandler` export limitation. Sections 2.1-2.4 require prominent notes about this limitation. AC13 and AC14 enforce documentation of the limitation. The Constraint 1 tension is resolved by documenting the current state honestly rather than providing a non-functional import path. All config parameters, schema fields, method names, and status codes verified against source code. Ready for implementation.

## Execution Summary

**Executed:** 2026-02-07
**Mode:** orchestrated (sequential fallback -- subagent spawning unavailable)
**Commits:** 4

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2 | complete |
| 2 | G3, G4 | complete |

### Files Modified
- `apps/docs-astro/src/content/docs/concepts/sync-protocol.mdx` (G1)
- `apps/docs-astro/src/content/docs/reference/server.mdx` (G2)
- `apps/docs-astro/src/content/docs/reference/client.mdx` (G3)
- `apps/docs-astro/src/content/docs/guides/deployment.mdx` (G4)

### Commits
- `b6fe77c` docs(sync-protocol): add HTTP Sync section and transport decision guide
- `4c6bfbc` docs(server): add POST /sync endpoint reference with schemas and examples
- `8b115ae` docs(client): add HttpSyncProvider and AutoConnectionProvider API reference
- `c4f36b6` docs(deployment): add serverless deployment section with platform examples

### Acceptance Criteria Status
- [x] AC1: sync-protocol.mdx contains HTTP Sync section with step-by-step flow
- [x] AC2: sync-protocol.mdx contains decision table with 6 criteria
- [x] AC3: deployment.mdx contains Serverless Deployment section with Vercel/Lambda/Cloudflare examples
- [x] AC4: deployment.mdx shows client config with HttpSyncProvider and AutoConnectionProvider via SyncEngine
- [x] AC5: client.mdx documents HttpSyncProviderConfig with all 8 parameters
- [x] AC6: client.mdx documents AutoConnectionProviderConfig with all 9 parameters
- [x] AC7: server.mdx documents POST /sync with request/response schemas and status codes
- [x] AC8: server.mdx includes complete JSON request/response example
- [x] AC9: All config parameter names, types, and defaults verified against source
- [x] AC10: All documentation follows existing MDX patterns
- [x] AC11: Info callout in client.mdx explains HTTP mode limitations
- [x] AC12: All code examples show providers with SyncEngine (not TopGunClient) with required fields
- [x] AC13: Serverless examples use HttpSyncHandler.handleSyncRequest() with import limitation note
- [x] AC14: Section 2.1 accurately describes HttpSyncHandler export limitation

### Deviations
None.

### Self-Check
- All 4 modified files verified to exist
- All 4 commit hashes verified in git log
- No uncommitted changes to implementation files
- Key content patterns verified via grep across all files

---

## Review History

### Review v1 (2026-02-07)
**Result:** CHANGES_REQUESTED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Major:**
1. **HttpSyncError field types in server.mdx do not match source schema (AC9 violation)**
   - File: `apps/docs-astro/src/content/docs/reference/server.mdx:501`
   - Issue: The `errors` field description says "Each error contains code (string), message (string), and context (object)" but the actual Zod schema in `packages/core/src/schemas/http-sync-schemas.ts` (lines 108-112) defines `HttpSyncErrorSchema` as `code: z.number()`, `message: z.string()`, `context: z.string().optional()`. Two type mismatches: `code` is `number` not `string`, and `context` is `string` (optional) not `object`.
   - Fix: Change the `errors` ApiParam description from "Each error contains code (string), message (string), and context (object)" to "Each error contains code (number), message (string), and context (string, optional)".

**Minor:**
2. **HttpSyncResponse optional fields not marked with `?` in docs**
   - File: `apps/docs-astro/src/content/docs/reference/server.mdx:479-502`
   - Issue: In the `HttpSyncResponse` schema documentation, the fields `ack`, `deltas`, `queryResults`, `searchResults`, and `errors` are all optional in the Zod schema (`.optional()`) but are documented without the `?` suffix on their parameter names. Only `serverHlc` is required. While the spec's section 4.3 table also omits optionality markers, users reading the API reference may assume all fields are always present.

3. **Serverless examples show simplified HttpSyncHandler constructor**
   - File: `apps/docs-astro/src/content/docs/guides/deployment.mdx:217` (and similar in Lambda/Cloudflare examples)
   - Issue: The serverless examples show `new HttpSyncHandler(hlc)` but the actual constructor at `packages/server/src/coordinator/http-sync-handler.ts:35` accepts `HttpSyncHandlerConfig` with 7 dependency properties (`authHandler`, `operationHandler`, `storageManager`, `queryConversionHandler`, `searchCoordinator`, `hlc`, `securityManager`). This is mitigated by Assumption 1 (examples are illustrative) and the prominent warning that `HttpSyncHandler` is not yet publicly exported, but the constructor signature mismatch could confuse users who inspect the source.

**Passed:**
- [x] AC1-AC8, AC10-AC14 all passed (13/14 criteria)

**Summary:** The implementation is comprehensive and well-structured. One major issue found (AC9 violation for HttpSyncError types). Two minor issues identified.

### Fix Response v1 (2026-02-07)
**Applied:** All 3 issues (1 major + 2 minor)

**Fixes:**
1. [✓] HttpSyncError field types corrected -- changed `code (string)` to `code (number)` and `context (object)` to `context (string, optional)` in server.mdx errors ApiParam description
   - Commit: `c43e0c3`
2. [✓] HttpSyncResponse optional fields marked -- added `?` suffix to `ack`, `deltas`, `queryResults`, `searchResults`, and `errors` parameter names in server.mdx
   - Commit: `c43e0c3`
3. [✓] HttpSyncHandler constructor updated -- replaced `new HttpSyncHandler(hlc)` with `new HttpSyncHandler({ hlc, authHandler, operationHandler, storageManager, queryConversionHandler, searchCoordinator, securityManager })` in all 3 serverless examples (Vercel, Lambda, Cloudflare)
   - Commit: `69ced3e`

**Skipped:** None

### Review v2 (2026-02-07)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Fix Verification:**

All 3 fixes from Review v1 verified as correctly applied:

1. **HttpSyncError field types (Fix #1):** `apps/docs-astro/src/content/docs/reference/server.mdx` line 501 now reads "Each error contains code (number), message (string), and context (string, optional)" -- matches `HttpSyncErrorSchema` in `packages/core/src/schemas/http-sync-schemas.ts` lines 108-112 exactly (`code: z.number()`, `message: z.string()`, `context: z.string().optional()`). VERIFIED.

2. **HttpSyncResponse optional fields (Fix #2):** `server.mdx` lines 479-499 now show `ack?`, `deltas?`, `queryResults?`, `searchResults?`, `errors?` with `?` suffixes, matching the `.optional()` markers on all five fields in the Zod schema (lines 123-134). Only `serverHlc` (line 473) remains without `?`, correctly reflecting it is the sole required field. VERIFIED.

3. **HttpSyncHandler constructor (Fix #3):** All three serverless examples in `apps/docs-astro/src/content/docs/guides/deployment.mdx` now show `new HttpSyncHandler({ hlc, authHandler, operationHandler, storageManager, queryConversionHandler, searchCoordinator, securityManager })` (Vercel at line 219, Lambda at line 253, Cloudflare at line 283), matching the `HttpSyncHandlerConfig` interface at `packages/server/src/coordinator/http-sync-handler.ts` lines 10-22 which requires all 7 properties. VERIFIED.

**Full Review Pass:**

**Passed:**
- [x] AC1: sync-protocol.mdx contains HTTP Sync section with 5-step exchange flow (lines 81-123)
- [x] AC2: Decision table with 6 criteria present (lines 125-176)
- [x] AC3: Serverless Deployment section with Vercel/Lambda/Cloudflare examples (lines 513-567)
- [x] AC4: Client config examples use SyncEngine with HttpSyncProvider and AutoConnectionProvider (lines 551-567)
- [x] AC5: HttpSyncProviderConfig documents all 8 parameters with ApiConstructor/ApiParam (client.mdx lines 301-343)
- [x] AC6: AutoConnectionProviderConfig documents all 9 parameters with same pattern (client.mdx lines 356-403)
- [x] AC7: POST /sync endpoint with request schema, response schema, and 5 status codes (server.mdx lines 404-539)
- [x] AC8: Complete JSON request/response example (server.mdx lines 15-98, rendered at 541-555)
- [x] AC9: All parameter names, types, and defaults match source code -- independently verified against HttpSyncProvider.ts (8 params), AutoConnectionProvider.ts (9 params), http-sync-schemas.ts (request/response fields), http-sync-handler.ts (constructor)
- [x] AC10: MDX patterns followed consistently -- CodeBlock, ApiConstructor/ApiParam, lucide-react icons, Tailwind classes, const exports at file top
- [x] AC11: Info callout at client.mdx lines 405-416 explains HTTP mode limitations (no live subscriptions, no Merkle sync, no real-time topics)
- [x] AC12: All 4 provider code examples use SyncEngine with nodeId, connectionProvider, storageAdapter (client.mdx lines 137-175, deployment.mdx lines 309-346)
- [x] AC13: All 3 serverless examples use HttpSyncHandler.handleSyncRequest() with import limitation warning (deployment.mdx lines 524-531)
- [x] AC14: Section 2.1 accurately describes export limitation -- not available via main entry or sub-path, code change needed (deployment.mdx lines 520-531)
- [x] Constraint 1: No TypeScript source files modified (git diff confirms only 4 MDX files changed)
- [x] Constraint 6: No existing content removed or reordered (diffs show only insertions)
- [x] Constraint 7: Footer navigation preserved in all 4 files
- [x] Constraint 10: No emoji in documentation content
- [x] No lingering references to incorrect APIs (no TopGunClient with connectionProvider, no handleRequest() without Sync)
- [x] Frontmatter description updated in deployment.mdx to include "serverless platforms"

**Summary:** All 14 acceptance criteria fully satisfied. All 3 fixes from Review v1 verified as correctly applied with exact source code matching. No new issues found. The documentation is comprehensive, accurate, well-structured, and follows existing patterns consistently across all four modified files.

---

## Completion

**Completed:** 2026-02-07
**Total Commits:** 6 (4 implementation + 2 fixes)
**Audit Cycles:** 4
**Review Cycles:** 2
