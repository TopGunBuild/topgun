---
id: SPEC-077
type: feature
status: done
priority: P1
complexity: medium
created: 2026-03-05
todo: TODO-096
---

# SPEC-077: Create Tier 1 Adoption Example App and Integration Guides

## Context

TopGun's PRODUCT_CAPABILITIES.md already documents the three adoption tiers and security model in text form. What is missing is a **working Tier 1 example app** that demonstrates the primary adoption story: "add TopGun for one collaborative feature while keeping your existing Express + Postgres stack." This is the most important adoption artifact because it proves the "works alongside existing DB: Yes" claim with running code.

Additionally, two integration guides are needed: one for using PostgresDataStore with existing Postgres tables (Tier 2 support), and one for integrating TopGun auth with existing JWT-based auth systems.

### Goal Statement

A developer evaluating TopGun can clone a working example, run `pnpm install && pnpm dev`, and see a hybrid app where REST handles CRUD and TopGun handles real-time collaboration -- proving they do not need to replace their existing stack.

### Observable Truths

1. A developer runs `pnpm install && pnpm dev` in `examples/collaborative-tasks/` and sees a working app at `http://localhost:3000`
2. The app has REST API endpoints (`GET /api/tasks`, `POST /api/tasks`, `PUT /api/tasks/:id`, `DELETE /api/tasks/:id`) that read/write tasks to Postgres
3. Opening two browser tabs shows real-time collaborative presence (who is viewing which task) and live task status updates via TopGun WebSocket
4. The Express server code clearly separates REST routes from TopGun setup (no interleaving)
5. A `docs/guides/` directory contains a PostgresDataStore integration guide with code examples
6. A `docs/guides/` directory contains a JWT auth integration guide with code examples
7. The example app's README explains the hybrid architecture pattern with a diagram

### Required Artifacts

| Artifact | Purpose |
|----------|---------|
| `examples/collaborative-tasks/package.json` | App manifest with express, pg, @topgunbuild/client, esbuild deps |
| `examples/collaborative-tasks/src/server.ts` | Express server with REST routes + TopGun Rust server spawn |
| `examples/collaborative-tasks/src/routes/tasks.ts` | REST CRUD routes using pg directly |
| `examples/collaborative-tasks/src/topgun-setup.ts` | TopGun client initialization, map subscriptions |
| `examples/collaborative-tasks/src/public/index.html` | Minimal frontend (vanilla JS, no framework) |
| `examples/collaborative-tasks/src/public/app.js` | Client-side TopGun source (bundled by esbuild before serving) |
| `examples/collaborative-tasks/README.md` | Setup instructions, architecture diagram, explanation |
| `docs/guides/postgres-integration.md` | PostgresDataStore guide for existing tables |
| `docs/guides/auth-integration.md` | JWT auth integration guide |

## Task

### 1. Create Tier 1 Example App (`examples/collaborative-tasks/`)

Build a "Collaborative Task Board" example that demonstrates the hybrid pattern:

**Backend (Express + Postgres):**
- Express server on port 3000 serving static files and REST API
- REST routes: `GET /api/tasks`, `POST /api/tasks`, `PUT /api/tasks/:id`, `DELETE /api/tasks/:id`
- Uses `pg` (node-postgres) directly for all CRUD -- no ORM, no TopGun for persistence
- SQL schema: single `tasks` table (`id`, `title`, `status`, `assignee`, `created_at`)
- Includes a `schema.sql` file for table creation

**Real-Time Layer (TopGun):**
- TopGun client connects to a Rust TopGun server (separate process or spawned)
- Uses a `task-presence` map for tracking which users are viewing which tasks
- Uses a `task-updates` topic for broadcasting live status changes
- When a REST endpoint mutates a task, it also publishes to the TopGun topic so other clients see changes instantly without polling

**Frontend (Vanilla JS + esbuild bundle):**
- Single-page HTML with vanilla JavaScript (no React, no framework)
- Fetches tasks via REST API on load
- Connects to TopGun via WebSocket for:
  - Presence indicators (colored dots showing who else is viewing)
  - Live task status updates (card moves between columns without page refresh)
- `src/public/app.js` imports from `@topgunbuild/client` using standard ESM imports
- A `build:client` script in `package.json` runs `esbuild src/public/app.js --bundle --outfile=src/public/app.bundle.js --format=iife --platform=browser` to produce a single browser-ready file
- `index.html` loads `app.bundle.js` via a `<script>` tag
- The `dev` script runs `build:client` before starting the Express server

**Developer Experience:**
- `pnpm install` installs deps
- `pnpm dev` bundles the frontend JS and starts Express server (assumes Postgres is running and TopGun Rust server is running separately)
- `.env.example` with `DATABASE_URL`, `TOPGUN_SERVER_URL` defaults
- README includes "Prerequisites" section listing Postgres and TopGun server requirements, including the command to start the Rust server: `cargo run --release --bin test-server` (from the repository root)

### 2. Create PostgresDataStore Integration Guide (`docs/guides/postgres-integration.md`)

Document Tier 2 adoption -- using TopGun as an in-memory cache over existing Postgres:

- How PostgresDataStore connects to existing tables (no migration required)
- Configuration: connection string, table mapping
- Read path: data loaded into memory on startup, reads are 0ms
- Write path: write-through to Postgres
- Code examples showing configuration
- Caveats: schema expectations, column naming conventions
- When to use Tier 2 vs Tier 1
- Reference PRODUCT_CAPABILITIES.md for tier definitions and capability comparisons rather than duplicating that content

### 3. Create Auth Integration Guide (`docs/guides/auth-integration.md`)

Document how to integrate TopGun auth with existing JWT systems:

- TopGun's auth model: JWT with `sub` claim (standard RFC 7519)
- Sharing JWT secret between existing auth system and TopGun server
- Pattern: existing auth issues JWT, TopGun server validates same JWT
- Code example: Express middleware that issues JWT accepted by both REST routes and TopGun
- Security pipeline overview: reference PRODUCT_CAPABILITIES.md for the full pipeline description (Auth check -> Map ACL -> HLC sanitization -> CRDT merge) rather than duplicating it
- Map-level ACL configuration examples
- Common patterns: role-based access, user-scoped maps (`notes:{userId}`)

## Requirements

### Files to Create

| File | Description |
|------|-------------|
| `examples/collaborative-tasks/package.json` | Dependencies: express, pg, @topgunbuild/client, dotenv (prod); tsx, esbuild (dev) |
| `examples/collaborative-tasks/tsconfig.json` | TypeScript config for Node.js |
| `examples/collaborative-tasks/.env.example` | DATABASE_URL, TOPGUN_SERVER_URL defaults |
| `examples/collaborative-tasks/schema.sql` | CREATE TABLE tasks DDL |
| `examples/collaborative-tasks/src/server.ts` | Express app entry point |
| `examples/collaborative-tasks/src/routes/tasks.ts` | REST CRUD handlers |
| `examples/collaborative-tasks/src/topgun-setup.ts` | TopGun client init + bridge logic |
| `examples/collaborative-tasks/src/public/index.html` | Frontend HTML (loads app.bundle.js) |
| `examples/collaborative-tasks/src/public/app.js` | Frontend JS source with TopGun client imports |
| `examples/collaborative-tasks/README.md` | Setup guide with architecture explanation |
| `docs/guides/postgres-integration.md` | Tier 2 integration guide |
| `docs/guides/auth-integration.md` | JWT auth integration guide |

### Files to Modify

None.

## Acceptance Criteria

1. `examples/collaborative-tasks/` directory exists with all listed files
2. `package.json` lists express, pg, @topgunbuild/client, dotenv as production dependencies and tsx, esbuild as dev dependencies (no React, no Vite, no webpack)
3. `schema.sql` contains valid PostgreSQL DDL for a `tasks` table
4. `src/server.ts` starts an Express server that serves static files from `src/public/` and mounts REST routes from `src/routes/tasks.ts`
5. `src/routes/tasks.ts` implements GET/POST/PUT/DELETE for tasks using `pg` Pool directly
6. `src/topgun-setup.ts` creates a TopGunClient, connects to a configurable server URL, and exports functions for publishing task updates and managing presence
7. `src/server.ts` calls topgun-setup functions after REST mutations to bridge REST writes to TopGun topics
8. `src/public/index.html` loads `app.bundle.js` (the esbuild output) via a `<script>` tag and renders a task board UI
9. `src/public/app.js` imports from `@topgunbuild/client`, connects to TopGun WebSocket, and updates the DOM when presence or task updates arrive; esbuild bundles this into `app.bundle.js`
10. `package.json` contains a `build:client` script that runs esbuild to bundle `app.js` into `app.bundle.js` (IIFE format, browser platform), and the `dev` script runs `build:client` before starting the server
11. `README.md` contains: prerequisites (including the command `cargo run --release --bin test-server` to start the Rust server), setup steps, architecture diagram (ASCII), explanation of the hybrid pattern, and "what to try" section
12. `docs/guides/postgres-integration.md` documents PostgresDataStore configuration, read/write paths, references PRODUCT_CAPABILITIES.md for tier definitions, and includes at least 2 code examples
13. `docs/guides/auth-integration.md` documents JWT integration pattern, map ACL configuration, references PRODUCT_CAPABILITIES.md for the security pipeline, and includes at least 2 code examples showing shared JWT validation
14. No file imports from `@topgunbuild/server` (this is a client-side integration example; server is the Rust binary)
15. `.env.example` contains `DATABASE_URL=postgres://postgres:postgres@localhost:5432/collaborative_tasks` and `TOPGUN_SERVER_URL=ws://localhost:8080`

## Constraints

- Do NOT use React, Vue, or any frontend framework -- vanilla HTML/JS only to minimize cognitive overhead
- Frontend bundling uses a single esbuild command (one line in `package.json` scripts) -- no Vite, no webpack, no complex build configuration
- Do NOT import from `@topgunbuild/server` -- the example uses the Rust server binary
- Do NOT add TopGun as a persistence layer in this example -- Postgres is the single source of truth; TopGun handles only real-time collaboration features
- Do NOT create a custom auth system -- the auth guide documents integration patterns, the example app uses a simple shared secret for demo purposes
- Keep the example under 500 lines total (all source files combined) -- this is a teaching tool, not a production app

## Assumptions

- The Rust TopGun server binary is available and running separately (the example does not start it)
- Postgres is running locally with default credentials for development
- The `@topgunbuild/client` package is bundled for the browser using esbuild at dev time; no pre-built UMD/IIFE bundle is required to exist in the package
- Vanilla JS is preferable over React for a Tier 1 example because it shows TopGun works with any stack, not just React
- ASCII art is sufficient for architecture diagrams in the README (no image files)
- The `docs/guides/` directory is a new top-level directory (no existing docs structure)

## Goal Analysis

### Key Links

| From | To | Connection | Risk |
|------|-----|-----------|------|
| `src/server.ts` | `src/topgun-setup.ts` | Calls topgun bridge after REST mutations | Low -- simple function calls |
| `src/routes/tasks.ts` | `pg` Pool | Direct SQL queries | Low -- standard pattern |
| `src/public/app.js` | TopGun WS | Client subscribes to presence + topics via @topgunbuild/client | Low -- esbuild handles bundling |
| `README.md` | All source files | Documents architecture accurately | Low -- review-only |

### Critical Path

The frontend uses `@topgunbuild/client` via standard ESM imports in `app.js`, which esbuild bundles into a single `app.bundle.js` (IIFE format). This is the same approach used by the existing `notes-app` and `todo-app` examples (which use Vite), but simplified to a single esbuild command with no config file. Risk is low since esbuild handles `@topgunbuild/client` and its transitive dependencies (`@topgunbuild/core`, `idb`, `pino`) automatically.

## Implementation Tasks

### Task Groups

| Group | Wave | Tasks | Dependencies | Est. Context |
|-------|------|-------|--------------|--------------|
| G1 | 1 | Create example app skeleton: package.json, tsconfig.json, .env.example, schema.sql | -- | ~10% |
| G2 | 1 | Create integration guide docs: postgres-integration.md, auth-integration.md | -- | ~20% |
| G3 | 2 | Create backend: server.ts, routes/tasks.ts, topgun-setup.ts | G1 | ~25% |
| G4 | 2 | Create frontend: index.html, app.js (with esbuild bundling) | G1 | ~20% |
| G5 | 3 | Create README.md with architecture explanation and setup guide | G3, G4 | ~20% |

### Execution Plan

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3, G4 | Yes | 2 |
| 3 | G5 | No | 1 |

**Total workers needed:** 2 (max in any wave)

## Audit History

### Audit v1 (2026-03-05)
**Status:** NEEDS_REVISION

**Context Estimate:** ~90% total (12 new files, heavy text generation for guides and README)

**Critical:**
1. **Browser client import strategy unresolved.** The `@topgunbuild/client` package ships only CJS (`dist/index.js`) and ESM (`dist/index.mjs`) with dependencies on `@topgunbuild/core`, `idb`, and `pino`. It has no UMD/IIFE browser bundle. The spec's constraint "no build step for frontend" directly conflicts with AC #9 (`app.js` connects to TopGun WebSocket). The assumption "The `@topgunbuild/client` package can be used standalone in a browser via a bundled UMD/ESM file" is false -- no such bundle exists today. **Resolution required:** Either (a) add a `pnpm build:browser` step to create an IIFE bundle served as a static asset, (b) relax the "no build step" constraint to allow a single esbuild command in the `dev` script that bundles `app.js` with its TopGun imports, or (c) use raw WebSocket in `app.js` (bypassing `@topgunbuild/client` entirely) with manual MsgPack encoding. The spec must pick one approach and update constraints, AC #8/#9, and the frontend task description accordingly.

2. **Contradiction between constraint and AC.** Constraint says "Do NOT require a build step for the frontend" but AC #2 lists `@topgunbuild/client` as a production dependency in `package.json`, and AC #9 expects `app.js` to use TopGun client functionality. These are incompatible without a bundling step (see issue 1). The spec must resolve this contradiction explicitly.

**Recommendations:**
3. [Strategic] The "no build step" purity may not serve the adoption goal. The existing `notes-app` and `todo-app` examples both use Vite. A single esbuild command (one line in `package.json` scripts) would solve the browser import problem while keeping the example simple. Consider relaxing this constraint.
4. The `dev` script should document how to start the Rust TopGun server. Currently the spec says "assumes TopGun Rust server is running separately" but does not specify the command. Adding `cargo run --release --bin test-server` (or the built binary path) to the README prerequisites would help developers.
5. G2 (integration guides) at ~25% context is heavy for pure text generation. Consider whether the implementer should reference PRODUCT_CAPABILITIES.md content rather than rewriting it, to keep guides concise and maintainable.
6. Observable Truth #2 mentions only GET/POST but the task and AC #5 specify GET/POST/PUT/DELETE. Truth #2 should list all four methods for consistency.

### Response v1 (2026-03-05)
**Applied:** All critical issues (1-2) and all recommendations (3-6)

**Changes:**
1. [✓] Browser client import strategy resolved -- adopted esbuild bundling approach (option b from audit). Updated Frontend section to describe `app.js` as ESM source bundled by esbuild into `app.bundle.js` (IIFE). Updated AC #8 (loads `app.bundle.js`), AC #9 (imports from `@topgunbuild/client`, esbuild bundles), added AC #10 (`build:client` script). Renumbered subsequent ACs. Updated Required Artifacts, Files to Create, Goal Analysis (risk lowered to Low), and Critical Path sections.
2. [✓] Constraint/AC contradiction resolved -- replaced "no build step" constraint with "single esbuild command" constraint. Updated AC #2 to include esbuild as dev dependency. Updated Assumptions to reflect esbuild bundling instead of pre-built UMD.
3. [✓] Relaxed "no build step" constraint per recommendation -- aligned with existing notes-app/todo-app precedent. Single esbuild command, no config file required.
4. [✓] Added Rust server start command -- README prerequisites now require documenting `cargo run --release --bin test-server`. Added to Developer Experience section and AC #11.
5. [✓] Added PRODUCT_CAPABILITIES.md reference guidance -- both guide task descriptions now instruct implementer to reference PRODUCT_CAPABILITIES.md rather than duplicating content. ACs #12 and #13 updated to require these references.
6. [✓] Fixed Observable Truth #2 -- now lists all four HTTP methods: GET, POST, PUT, DELETE.

### Audit v2 (2026-03-05)
**Status:** NEEDS_DECOMPOSITION

**Context Estimate:** ~95% total (12 new files, heavy text generation for guides and README)

**Scope:** Large (~95% estimated, exceeds 50% target)

**Per-Group Breakdown:**

| Group | Wave | Tasks | Est. Context | Cumulative | Status |
|-------|------|-------|--------------|------------|--------|
| G1 | 1 | Skeleton: package.json, tsconfig, .env.example, schema.sql | ~10% | 10% | OK |
| G2 | 1 | Docs: postgres-integration.md, auth-integration.md | ~20% | 30% | OK |
| G3 | 2 | Backend: server.ts, routes/tasks.ts, topgun-setup.ts | ~25% | 55% | OK |
| G4 | 2 | Frontend: index.html, app.js | ~20% | 75% | OK |
| G5 | 3 | README.md | ~20% | 95% | OK |

**Quality Projection:** POOR range (95% total), but each individual group is within acceptable bounds (all under 30%). With orchestrated parallel execution, each worker stays in PEAK-to-GOOD range.

**Execution Plan:**

| Wave | Groups | Parallel? | Workers |
|------|--------|-----------|---------|
| 1 | G1, G2 | Yes | 2 |
| 2 | G3, G4 | Yes | 2 |
| 3 | G5 | No | 1 |

**Audit Dimensions:**
- Clarity: PASS -- all tasks, ACs, and constraints are specific and unambiguous
- Completeness: PASS -- all 12 files listed with descriptions, no missing artifacts
- Testability: PASS -- all 15 ACs are measurable and concrete
- Scope: PASS -- constraints and 500-line limit prevent creep
- Feasibility: PASS -- esbuild bundling verified (pino has browser field, idb is browser-native, TopGunClient API has .topic() and .getMap() methods as spec assumes)
- Architecture fit: PASS -- follows existing examples/ pattern, workspace deps
- Non-duplication: PASS -- unique example, guides reference rather than duplicate PRODUCT_CAPABILITIES.md
- Cognitive load: PASS -- vanilla JS, clear separation, teaching-focused
- Strategic fit: PASS -- aligned with v1.0 adoption goals
- Project compliance: PASS -- Language Profile does not apply (TypeScript/JS/docs in examples/ and docs/, not core-rust or server-rust)

**Goal Analysis Validation:** All 7 observable truths have corresponding artifacts. All artifacts have purpose. All 4 key links identified. No orphans, no missing coverage.

**Assumptions Validated:**
- A1: @topgunbuild/client can be bundled by esbuild for browser -- CONFIRMED (pino has "browser" field in package.json, idb is browser-native, core is pure logic)
- A2: Rust server supports topics and maps -- CONFIRMED (messaging service and topic handlers exist in server-rust)

**Recommendations:**
1. The README prerequisites should mention running `pnpm build` at the monorepo root before `pnpm dev` in the example. The existing Vite examples resolve workspace packages via `resolve.alias` pointing to source TS files, but esbuild CLI without a config file relies on node_modules resolution, which requires built dist files to exist. Add this to AC #11 or the Developer Experience section.
2. The `@topgunbuild/client` dependency in package.json should use `"workspace:*"` to match the pattern used by existing examples (notes-app, todo-app). The spec's AC #2 does not specify the version format.

**Recommendation:** Use `/sf:run --parallel` for orchestrated execution across 3 waves.

## Execution Summary

**Executed:** 2026-03-05
**Mode:** orchestrated
**Commits:** 5

### Execution Waves

| Wave | Groups | Status |
|------|--------|--------|
| 1 | G1, G2 | complete |
| 2 | G3, G4 | complete |
| 3 | G5 | complete |

### Files Created

- `examples/collaborative-tasks/package.json`
- `examples/collaborative-tasks/tsconfig.json`
- `examples/collaborative-tasks/.env.example`
- `examples/collaborative-tasks/schema.sql`
- `examples/collaborative-tasks/src/server.ts`
- `examples/collaborative-tasks/src/routes/tasks.ts`
- `examples/collaborative-tasks/src/topgun-setup.ts`
- `examples/collaborative-tasks/src/public/index.html`
- `examples/collaborative-tasks/src/public/app.js`
- `examples/collaborative-tasks/README.md`
- `docs/guides/postgres-integration.md`
- `docs/guides/auth-integration.md`

### Files Modified

- `.gitignore` (added exception for `examples/**/src/**/*.js`)

### Acceptance Criteria Status

- [x] AC1: examples/collaborative-tasks/ directory exists with all listed files
- [x] AC2: package.json lists express, pg, @topgunbuild/client (workspace:*), dotenv as prod deps; tsx, esbuild as dev deps
- [x] AC3: schema.sql contains valid PostgreSQL DDL for tasks table
- [x] AC4: src/server.ts starts Express serving static files and REST routes
- [x] AC5: src/routes/tasks.ts implements GET/POST/PUT/DELETE using pg Pool
- [x] AC6: src/topgun-setup.ts creates TopGunClient, connects to configurable URL, exports bridge functions
- [x] AC7: src/server.ts calls topgun-setup functions after REST mutations
- [x] AC8: src/public/index.html loads app.bundle.js via script tag, renders task board UI
- [x] AC9: src/public/app.js imports from @topgunbuild/client, connects via WebSocket, updates DOM
- [x] AC10: package.json has build:client (esbuild IIFE) and dev (build:client + tsx) scripts
- [x] AC11: README.md has prerequisites (cargo run, pnpm build), setup, ASCII diagram, hybrid explanation, "what to try"
- [x] AC12: postgres-integration.md documents config, read/write paths, references PRODUCT_CAPABILITIES.md, 2+ code examples
- [x] AC13: auth-integration.md documents JWT pattern, map ACL, references PRODUCT_CAPABILITIES.md, 2+ code examples
- [x] AC14: No file imports from @topgunbuild/server
- [x] AC15: .env.example contains DATABASE_URL and TOPGUN_SERVER_URL with specified defaults

### Deviations

- Added `.gitignore` exception (`!examples/**/src/**/*.js`) because the existing `*.js` rule blocked committing `app.js` source file

---

## Review History

### Review v1 (2026-03-05)
**Result:** CHANGES_REQUESTED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Major:**
1. **Presence rendering uses IterableIterator as Array**
   - File: `examples/collaborative-tasks/src/public/app.js:133-138`
   - Issue: `presenceMap.entries()` returns an `IterableIterator<[K, V]>` (from `LWWMap.entries()`), not an Array. The code calls `.length` (undefined on iterators) and `.map()` (not available on iterators) on the result. At runtime, `.length` is `undefined`, the `=== 0` check passes through, but `.map()` on line 138 will throw `TypeError: entries.map is not a function`. This breaks the presence display feature entirely, which is part of Observable Truth #3 and AC #9 ("updates the DOM when presence or task updates arrive").
   - Fix: Convert the iterator to an array: `const entries = [...presenceMap.entries()];`

**Minor:**
2. `MemoryStorageAdapter.initialize()` omits the `dbName: string` parameter required by `IStorageAdapter`. Same for `markOpsSynced()` missing the `lastId: number` parameter. Both are harmless at runtime but would cause TypeScript compilation errors if type-checked strictly.
   - File: `examples/collaborative-tasks/src/topgun-setup.ts:21,47`
   - Fix: Add the parameters: `async initialize(_dbName: string)` and `async markOpsSynced(_lastId: number)`

3. The architecture diagram in README.md shows the TopGun Rust Server connecting to PostgreSQL, but in this Tier 1 example, the TopGun server does NOT connect to Postgres -- only the Express server does. The diagram may confuse developers about the data flow.
   - File: `examples/collaborative-tasks/README.md:37-38`

**Passed:**
- [x] AC1: All 12 specified files exist
- [x] AC2: package.json has correct prod deps (express, pg, @topgunbuild/client workspace:*, dotenv) and dev deps (tsx, esbuild) -- no React/Vite/webpack
- [x] AC3: schema.sql has valid CREATE TABLE DDL with correct columns (id SERIAL, title TEXT, status TEXT, assignee TEXT, created_at TIMESTAMPTZ)
- [x] AC4: server.ts creates Express app, serves static files from `src/public/`, mounts routes at `/api/tasks`
- [x] AC5: tasks.ts implements all four CRUD operations (GET, POST, PUT, DELETE) using pg Pool with proper error handling
- [x] AC6: topgun-setup.ts creates TopGunClient with configurable server URL, exports publishTaskUpdate, publishTaskDelete, setPresence, closeTopGun
- [x] AC7: tasks.ts routes call publishTaskUpdate/publishTaskDelete after mutations
- [x] AC8: index.html loads `app.bundle.js` via `<script>` tag, renders a kanban board UI with three columns
- [x] AC9 (partial): app.js imports from @topgunbuild/client, subscribes to task-updates topic, updates DOM on events -- but presence rendering has a runtime bug (Major #1)
- [x] AC10: package.json has `build:client` (esbuild IIFE) and `dev` (build:client && tsx) scripts
- [x] AC11: README has prerequisites (cargo run, pnpm build), setup steps, ASCII architecture diagram, hybrid pattern explanation, "What to Try" section
- [x] AC12: postgres-integration.md has config, read/write paths, 2 code examples, 2 PRODUCT_CAPABILITIES.md references
- [x] AC13: auth-integration.md has JWT pattern, map ACL examples, 4 code examples, 2 PRODUCT_CAPABILITIES.md references
- [x] AC14: No source file imports from @topgunbuild/server
- [x] AC15: .env.example has correct DATABASE_URL and TOPGUN_SERVER_URL values
- [x] Constraint: No frontend framework (vanilla JS)
- [x] Constraint: Single esbuild command for bundling
- [x] Constraint: Under 500 lines total (463 lines across 5 source files)
- [x] Constraint: Postgres is source of truth, TopGun is real-time only
- [x] Code quality: Clean separation of concerns, good error handling in REST routes, graceful shutdown
- [x] Security: No hardcoded secrets, XSS prevention via escapeHtml(), input validation on POST
- [x] Integration guides: Well-structured, reference PRODUCT_CAPABILITIES.md rather than duplicating, practical code examples

**Summary:** The implementation is high quality overall with excellent code organization, clean separation between REST and TopGun layers, comprehensive guides, and a well-written README. One major bug in the frontend presence rendering (`IterableIterator` used as `Array`) needs fixing before approval -- it would cause a runtime error when the presence map has entries, breaking Observable Truth #3.

### Fix Response v1 (2026-03-05)
**Applied:** all (1 major + 2 minor)

**Fixes:**
1. [✓] Presence rendering uses IterableIterator as Array -- converted `presenceMap.entries()` to array via spread: `[...presenceMap.entries()]`
   - File: `examples/collaborative-tasks/src/public/app.js:133`
   - Commit: 143dfff
2. [✓] MemoryStorageAdapter missing parameters -- added `_dbName: string` to `initialize()` and `_lastId: number` to `markOpsSynced()`
   - File: `examples/collaborative-tasks/src/topgun-setup.ts:21,47`
   - Commit: 143dfff
3. [✓] README architecture diagram incorrectly shows TopGun->Postgres -- removed the Postgres box from the TopGun server connection (only Express connects to Postgres in Tier 1)
   - File: `examples/collaborative-tasks/README.md:36-38`
   - Commit: 143dfff

### Review v2 (2026-03-05 re-review)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Fix Verification:**
1. [verified] Presence rendering -- `app.js:133` now uses `const entries = [...presenceMap.entries()];` which correctly converts the iterator to an Array. `.length` and `.map()` will work as expected.
2. [verified] MemoryStorageAdapter -- `topgun-setup.ts:21` has `async initialize(_dbName: string)` and line 47 has `async markOpsSynced(_lastId: number)`. Both match the `IStorageAdapter` interface.
3. [verified] README diagram -- Lines 36-39 show TopGun Rust Server as a standalone box with no Postgres connection, accurately reflecting Tier 1 architecture.

**Passed:**
- [x] AC1: All 12 specified files exist (verified via filesystem check)
- [x] AC2: package.json has express, pg, @topgunbuild/client (workspace:*), dotenv as prod deps; esbuild, tsx as dev deps
- [x] AC3: schema.sql has valid CREATE TABLE DDL (id SERIAL PRIMARY KEY, title TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'todo', assignee TEXT, created_at TIMESTAMPTZ)
- [x] AC4: server.ts creates Express app, serves static from `src/public/`, mounts `/api/tasks`
- [x] AC5: tasks.ts implements GET/POST/PUT/DELETE with pg Pool, parameterized queries, error handling, 404 responses
- [x] AC6: topgun-setup.ts creates TopGunClient with configurable URL, exports publishTaskUpdate, publishTaskDelete, setPresence, closeTopGun
- [x] AC7: tasks.ts calls publishTaskUpdate after POST/PUT and publishTaskDelete after DELETE
- [x] AC8: index.html loads `app.bundle.js` via `<script>` tag, renders kanban board with three columns
- [x] AC9: app.js imports from @topgunbuild/client, subscribes to task-updates topic and task-presence map, renders presence dots and live task updates
- [x] AC10: package.json has `build:client` (esbuild IIFE) and `dev` (build:client && tsx) scripts
- [x] AC11: README has prerequisites (cargo run --release --bin test-server, pnpm build), setup steps, ASCII diagram, hybrid pattern explanation, "What to Try" section
- [x] AC12: postgres-integration.md has config, read/write paths, 2 code examples, 2 PRODUCT_CAPABILITIES.md references
- [x] AC13: auth-integration.md has JWT pattern, map ACL, 4 code examples, 2 PRODUCT_CAPABILITIES.md references
- [x] AC14: No source file imports from @topgunbuild/server (only README mentions it descriptively)
- [x] AC15: .env.example has correct DATABASE_URL and TOPGUN_SERVER_URL values
- [x] Constraint: No frontend framework (vanilla HTML/JS)
- [x] Constraint: Single esbuild command for bundling
- [x] Constraint: Under 500 lines total (463 lines across 5 source files)
- [x] Constraint: Postgres is source of truth, TopGun handles only real-time
- [x] Constraint: No custom auth system in example app
- [x] Security: No hardcoded secrets, XSS prevention via escapeHtml(), parameterized SQL queries, input validation
- [x] Code quality: Clean separation of concerns, graceful shutdown, proper error handling
- [x] Cognitive load: Simple, teachable code; clear naming; no unnecessary abstractions
- [x] Non-duplication: Guides reference PRODUCT_CAPABILITIES.md rather than duplicating content
- [x] Integration: Follows monorepo patterns (workspace:* dependency, examples/ directory)

**Summary:** All three fixes from Review v1 have been correctly applied. The presence rendering bug is resolved, the MemoryStorageAdapter matches the IStorageAdapter interface, and the README diagram accurately represents the Tier 1 architecture. All 15 acceptance criteria are met and all constraints are satisfied. The implementation is clean, well-documented, and serves its purpose as a teaching tool for Tier 1 adoption.

---

## Completion

**Completed:** 2026-03-05
**Total Commits:** 5
**Review Cycles:** 2

### Outcome

Delivered a working Tier 1 adoption example app (Collaborative Task Board) demonstrating the hybrid pattern where Express+Postgres handles CRUD and TopGun handles real-time collaboration, plus two integration guides for PostgresDataStore and JWT auth.

### Key Files

- `examples/collaborative-tasks/` -- Complete working example app (12 files)
- `docs/guides/postgres-integration.md` -- Tier 2 PostgresDataStore integration guide
- `docs/guides/auth-integration.md` -- JWT auth integration guide

### Patterns Established

None -- followed existing patterns.

### Deviations

- Added `.gitignore` exception (`!examples/**/src/**/*.js`) to allow committing `app.js` source file blocked by existing `*.js` rule.
