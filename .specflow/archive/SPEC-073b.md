---
id: SPEC-073b
parent: SPEC-073
type: feature
status: done
priority: P0
complexity: small
depends_on: [SPEC-073a]
created: 2026-03-01
todo_ref: TODO-068
---

# TS Test Harness: Spawn Rust Server, Client Factory, Cleanup

## Context

SPEC-073a wires the Rust server's WebSocket handler and creates a test server binary (`packages/server-rust/src/bin/test_server.rs`) that starts on port 0 and prints `PORT=<number>` to stdout. This spec creates the TypeScript test harness that spawns that binary, captures the port, and provides client factories for integration tests.

### Existing Infrastructure

The existing TS e2e test helpers (`tests/e2e/helpers/index.ts`) provide:
- `createTestClient(serverUrl, options)` -- connects via raw WebSocket with MsgPack serialization, auto-authenticates, returns `TestClient` interface
- `createTestContext(numClients, serverConfig)` -- creates TS server + N clients with cleanup
- `TestClient` interface with `send()`, `waitForMessage()`, `close()`, `startHeartbeat()`, `stopHeartbeat()`
- JWT token generation with `test-e2e-secret` secret
- BATCH message parsing (CoalescingWriter format: 4-byte LE count + [4-byte LE length + MsgPack payload]*) -- NOTE: this TS BATCH format is NOT compatible with the Rust server (see Message Framing below)

The integration test harness copies and adapts the `TestClient` interface and client creation logic from the e2e helpers. It MUST NOT import from `tests/e2e/helpers/index.ts` because that module imports `@topgunbuild/server` (ServerCoordinator, ServerFactory), which would violate the no-TS-server-dependency constraint. Instead, `test-client.ts` contains a standalone copy of the TestClient interface, WebSocket connection logic, auto-auth handling, and utility functions, adapted for Rust server compatibility.

### Test Server Protocol

The Rust test server binary prints exactly:
```
PORT=12345
```
The harness reads this line from stdout with a 30-second timeout (includes potential cargo build time). Only stdout is parsed for the PORT protocol; stderr is inherited to the parent process for build progress and debug output but is NOT parsed.

### Message Framing

**Outbound (Rust server to client):** The Rust server sends each response as an individual MsgPack-encoded WebSocket binary frame. There is no BATCH wrapping on outbound messages. The `outbound_task()` coalesces at the socket flush level (multiple `send()` calls before a single `flush()`), but each message is a separate WebSocket frame. The test client does NOT need BATCH parsing for server responses.

**Inbound (client to Rust server):** The Rust server accepts both individual MsgPack messages and BATCH messages. The Rust BATCH format uses 4-byte **big-endian** u32 length-prefixed inner messages with NO count header (just consecutive `[4-byte BE length][MsgPack payload]` pairs until the data is exhausted). This is NOT compatible with the TS CoalescingWriter format (which uses a 4-byte LE count header followed by LE length-prefixed payloads). The test client MUST send individual messages (not BATCH) unless it implements the Rust-compatible BE format.

## Task

Create the TypeScript test harness for Rust server integration tests.

### Files to Create

1. **`tests/integration-rust/helpers/index.ts`** -- Main harness module
   - `spawnRustServer(options?)`: spawns `cargo run --bin test-server --release` (or `RUST_SERVER_BINARY` env var for pre-built binary), captures `PORT=\d+` from stdout, returns `{ port, process, cleanup }`
   - `createRustTestClient(port, options?)`: creates a WebSocket test client connecting to `ws://localhost:${port}/ws` (the Rust test server mounts its WebSocket handler at the `/ws` path)
   - `createRustTestContext(numClients, options?)`: spawns server + N clients, waits for all AUTH_ACK, returns `{ port, clients, cleanup }`
   - Timeout: 30s for server startup (configurable via `RUST_SERVER_TIMEOUT` env var)
   - Cleanup: kills Rust process (SIGTERM first, SIGKILL after 5s) and closes all client WebSockets
   - stderr from the spawned process MUST be inherited (piped to parent stderr) for build progress and debug logging, but MUST NOT be parsed for the PORT protocol
   - Process cleanup MUST handle `cargo run` process trees: use process group kill (`process.kill(-pid)`) when the process was spawned with `detached: true` and `process.unref()`, or prefer `RUST_SERVER_BINARY` env var (direct binary, no cargo wrapper) in CI for reliable single-process cleanup

2. **`tests/integration-rust/helpers/test-client.ts`** -- Standalone test client for Rust server
   - Contains a self-contained copy of the `TestClient` interface and `createTestClient()` function adapted from `tests/e2e/helpers/index.ts`, WITHOUT importing from the e2e helpers (to avoid transitive `@topgunbuild/server` dependency)
   - Does NOT include BATCH parsing for inbound messages (Rust server sends individual MsgPack frames)
   - Sends individual MsgPack messages (not BATCH) to the Rust server
   - Exports `createTestToken()`, `waitForSync()`, `waitUntil()` utilities

3. **`tests/integration-rust/tsconfig.json`** -- TypeScript configuration for ts-jest
   - Mirrors `tests/e2e/tsconfig.json` structure: strict mode, ES2020 target, commonjs module
   - Path mappings for `@topgunbuild/core` (required for `serialize`/`deserialize` imports)
   - MUST NOT include path mappings for `@topgunbuild/server` (not used)
   - `baseUrl` set to `../..` (repository root) to resolve package paths

4. **`tests/integration-rust/jest.config.ts`** (or `jest.config.js`) -- Jest configuration
   - Extends root Jest config
   - Sets `testTimeout` to 60000 (Rust server may need cargo build on first run)
   - Configures `testMatch` for `tests/integration-rust/**/*.test.ts`

### Optional Files

5. **`tests/integration-rust/helpers/setup.ts`** -- Global setup/teardown
   - Optional: pre-builds Rust binary once via `cargo build --bin test-server --release` in `globalSetup`
   - Sets `RUST_SERVER_BINARY` env var to the built binary path for faster individual test runs

## Requirements

- The harness MUST NOT depend on the TS server package (`@topgunbuild/server`) -- it only spawns the Rust binary. This means NO imports from `tests/e2e/helpers/index.ts` (which imports `@topgunbuild/server`); all shared logic must be copied into `test-client.ts`.
- The harness MUST reuse the `TestClient` interface shape from e2e helpers for test compatibility
- The `spawnRustServer()` function MUST handle the case where `cargo build` runs as part of `cargo run` (first invocation)
- Cleanup MUST be robust: SIGTERM -> wait 5s -> SIGKILL, and must work even if the test crashes
- Port capture MUST use line-buffered reading of stdout, matching the `PORT=<number>` protocol
- The harness MUST support running from the repository root (correct working directory for `cargo run`)

## Acceptance Criteria

- AC1: `spawnRustServer()` starts Rust binary, captures port within 30s, returns cleanup function
- AC2: `createRustTestClient(port)` connects via WebSocket to `ws://localhost:${port}/ws`, auto-authenticates, returns `TestClient` interface
- AC3: Server process is killed and cleaned up after each test suite
- AC4: `tests/integration-rust/tsconfig.json` exists with `@topgunbuild/core` path mapping and strict mode enabled
- AC5: `test-client.ts` has no import paths that resolve to `@topgunbuild/server`

## Constraints

- Tests MUST NOT call Rust server internals from TS -- all verification through message exchange
- Tests MUST NOT require PostgreSQL -- Rust server uses NullDataStore
- Tests MUST NOT use hardcoded ports -- Rust server uses port 0, TS reads actual port
- Existing TS e2e tests (`tests/e2e/`) MUST NOT be modified

## Assumptions

- The Rust test server binary is built by SPEC-073a and is available via `cargo run --bin test-server`
- The Rust server mounts its WebSocket handler at the `/ws` path (confirmed in `test_server.rs`)
- The Rust server sends individual MsgPack binary frames (no BATCH wrapping on outbound)
- The Rust server's inbound BATCH format uses 4-byte big-endian length prefixes with no count header
- Jest is the test runner (matches existing e2e tests)

---
*Child of SPEC-073. Created by SpecFlow spec-splitter on 2026-03-01.*

## Audit History

### Audit v1 (2026-03-01)
**Status:** NEEDS_REVISION

**Context Estimate:** ~15% total

**Critical:**
1. **WebSocket URL path mismatch.** The spec says `createRustTestClient(port)` connects to `ws://localhost:${port}`, but the Rust test server (`test_server.rs` line 71) mounts the WebSocket handler at the `/ws` path: `.route("/ws", get(ws_upgrade_handler))`. The correct URL must be `ws://localhost:${port}/ws`. Without this, every connection attempt will get a 404 and no test will work.

2. **Incorrect BATCH framing assumption.** The spec's Assumptions section states: "The CoalescingWriter BATCH framing format in the Rust outbound task matches what the TS test client parses." This is wrong in two ways:
   - The Rust server does NOT send BATCH-wrapped outbound messages. The `outbound_task()` in `websocket.rs` sends each message as an individual WebSocket binary frame (it coalesces at the socket flush level, not at the protocol level). The TS e2e helper's BATCH parsing code (4-byte LE count + LE length-prefixed payloads) will never be triggered by Rust server responses.
   - The Rust server's INBOUND BATCH parser uses **big-endian** 4-byte length prefixes with NO count header, while the TS CoalescingWriter format uses **little-endian** with a 4-byte LE count prefix. These are incompatible formats.
   The spec must explicitly state: (a) the Rust server sends individual MsgPack messages per WebSocket frame (no BATCH wrapping), so the test client does NOT need BATCH parsing for responses; (b) if the test client sends BATCH messages to the Rust server, it must use 4-byte BE length-prefixed format (no count header), not the TS CoalescingWriter LE format.

3. **Missing `tsconfig.json` file.** The existing `tests/e2e/` directory has a `tsconfig.json` (with path mappings to `@topgunbuild/core`, strict mode, etc.) that `ts-jest` requires. The spec lists 3-4 files but omits `tests/integration-rust/tsconfig.json`. Without it, `ts-jest` will either fail to resolve `@topgunbuild/core` imports or use incorrect compiler settings.

**Recommendations:**
4. **Clarify code reuse strategy for `test-client.ts`.** The spec says "Reuses the `TestClient` interface from `tests/e2e/helpers/index.ts`" but also says "The harness MUST NOT depend on the TS server package (`@topgunbuild/server`)." The e2e `index.ts` file imports `ServerCoordinator` and `ServerFactory` from `@topgunbuild/server`. The spec should clarify whether `test-client.ts` imports from the e2e helpers (which would pull in `@topgunbuild/server` as a transitive type dependency) or copies/adapts the relevant code. Given the constraint, copying is the correct approach, and the spec should state this explicitly.

5. **AC numbering starts at 8.** The acceptance criteria are numbered AC8-AC10, presumably continuing from SPEC-073a. This is unusual for a standalone spec and could confuse implementers. Consider renumbering to AC1-AC3 or explicitly noting the continuation scheme.

6. **`cargo run` stderr filtering.** When using `cargo run` (not a pre-built binary), Cargo prints build progress to stderr. The spec should mention that stderr should be inherited or logged for debugging but NOT parsed for the PORT protocol (only stdout).

7. **Process tree cleanup.** On some systems, `cargo run` spawns a child process (the actual binary). SIGTERM to the `cargo` process may not propagate to the child. Consider documenting that `spawnRustServer` should use `{ detached: false }` or `process.kill(-pid)` (process group kill) for reliable cleanup, or recommend always using `RUST_SERVER_BINARY` in CI.

### Response v1 (2026-03-01)
**Applied:** All critical issues (1-3) and all recommendations (4-7)

**Changes:**
1. [x] **WebSocket URL path mismatch** -- Changed `ws://localhost:${port}` to `ws://localhost:${port}/ws` in Files to Create item 1 (`createRustTestClient`), in AC2, and in the Assumptions section. Added note that the Rust test server mounts its handler at `/ws`.
2. [x] **Incorrect BATCH framing assumption** -- Removed the incorrect CoalescingWriter assumption. Added new "Message Framing" subsection to Context explaining: (a) outbound is individual MsgPack frames per WebSocket message, no BATCH; (b) inbound BATCH uses 4-byte BE length prefixes with no count header; (c) test client should send individual messages. Updated test-client.ts description to explicitly state no BATCH parsing needed. Updated Assumptions to list correct framing facts. Added NOTE to Existing Infrastructure referencing the incompatibility.
3. [x] **Missing tsconfig.json** -- Added `tests/integration-rust/tsconfig.json` as file 3 in Files to Create (renumbered jest.config to 4, setup.ts to 5). Specified strict mode, ES2020, commonjs, @topgunbuild/core path mapping, no @topgunbuild/server mapping. Added AC4 requiring its existence.
4. [x] **Clarify code reuse strategy** -- Rewrote Existing Infrastructure paragraph and test-client.ts description to explicitly state: code is copied/adapted (not imported) from e2e helpers to avoid transitive @topgunbuild/server dependency. Expanded the first Requirement to explain why no e2e imports. Added AC5 verifying no @topgunbuild/server import paths.
5. [x] **AC numbering** -- Renumbered AC8/AC9/AC10 to AC1/AC2/AC3. Added AC4 (tsconfig) and AC5 (no server imports).
6. [x] **cargo run stderr filtering** -- Added stderr handling guidance to Test Server Protocol subsection and to spawnRustServer description in Files to Create. Stderr is inherited for debug output but not parsed.
7. [x] **Process tree cleanup** -- Added process group kill guidance to spawnRustServer description in Files to Create. Documented `detached: true` + `process.kill(-pid)` pattern and recommendation to use `RUST_SERVER_BINARY` in CI.

### Audit v2 (2026-03-01)
**Status:** APPROVED

**Context Estimate:** ~17% total

**Comment:** Spec is well-structured and complete after v1 revision. All 3 critical issues from Audit v1 (WebSocket path, BATCH framing, missing tsconfig) have been thoroughly addressed. The Message Framing section provides precise documentation of the Rust server's wire format. The code reuse strategy (copy, not import) is now clearly stated with rationale. All 5 acceptance criteria are concrete and verifiable. Assumptions have been validated against the Rust source code (`test_server.rs`, `websocket.rs`). Process cleanup guidance (SIGTERM/SIGKILL, process group kill) is thorough. The spec is ready for implementation.

Strategic fit: Aligned with project goals. Project compliance: Honors PROJECT.md decisions. Language profile: N/A (TypeScript spec, explicitly exempted from Rust profile).

---

## Execution Summary

**Executed:** 2026-03-01
**Commits:** 4

### Files Created
- `tests/integration-rust/tsconfig.json` — TypeScript config: strict, ES2020, commonjs, DOM lib, @topgunbuild/core path mapping only
- `tests/integration-rust/jest.config.js` — Jest config: 60s timeout, ts-jest preset, @topgunbuild/core moduleNameMapper, setupFilesAfterEnv
- `tests/integration-rust/helpers/test-client.ts` — Standalone TestClient interface and createTestClient(); copied/adapted from e2e helpers; no BATCH parsing; no @topgunbuild/server dependency
- `tests/integration-rust/helpers/index.ts` — Main harness: spawnRustServer(), createRustTestClient(), createRustTestContext(), makeCleanup() with SIGTERM/SIGKILL/process-group kill
- `tests/integration-rust/helpers/setup.ts` — Per-test-file setup: 60s jest.setTimeout, optional DEBUG console suppression

### Files Modified
(none)

### Files Deleted
(none)

### Acceptance Criteria Status
- [x] AC1: spawnRustServer() starts Rust binary, captures port within 30s, returns cleanup function
- [x] AC2: createRustTestClient(port) connects via WebSocket to ws://localhost:${port}/ws, auto-authenticates, returns TestClient interface
- [x] AC3: Server process is killed and cleaned up after each test suite (SIGTERM -> 5s -> SIGKILL, process group)
- [x] AC4: tests/integration-rust/tsconfig.json exists with @topgunbuild/core path mapping and strict mode enabled
- [x] AC5: test-client.ts has no import paths that resolve to @topgunbuild/server

### Deviations
1. [Rule 1 - Bug] Removed unused `serverProcess` variable from `createRustTestContext` destructuring to prevent TypeScript strict-mode error.
2. [Rule 2 - Missing Critical] Added `"DOM"` to tsconfig lib array to match e2e tsconfig and resolve pre-existing `window` reference in @topgunbuild/core/utils/logger.ts under strict type-checking.

### Notes
- The setup.ts is simplified compared to the spec's optional "global setup" example — it does not call cargo build at setup time. Instead, it documents the recommended CI pattern (set RUST_SERVER_BINARY). The build-at-setup-time pattern would block Jest's setupFilesAfterEnv phase for every test file. Tests use spawnRustServer() with RUST_SERVER_BINARY for fast per-test spawning or fall back to cargo run.
- TypeScript compilation verified cleanly (0 errors in integration-rust files).

---

## Review History

### Review v1 (2026-03-01)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: `spawnRustServer()` correctly spawns either `RUST_SERVER_BINARY` or `cargo run --bin test-server --release` from repo root, reads `PORT=\d+` from stdout via `readline` with 30s configurable timeout, returns `{ port, process, cleanup }` — fully implemented in `helpers/index.ts`
- [✓] AC2: `createRustTestClient(port)` constructs `ws://localhost:${port}/ws` and delegates to `createTestClient()` which auto-authenticates on `AUTH_REQUIRED` and resolves the promise on WebSocket `open` — correct `/ws` path confirmed against `test_server.rs` line 70
- [✓] AC3: `makeCleanup()` sends SIGTERM to process group (`process.kill(-pid, 'SIGTERM')`), waits up to 5s, then escalates to SIGKILL — handles `cargo run` process tree correctly; `createRustTestContext` closes all client WebSockets before killing server
- [✓] AC4: `tests/integration-rust/tsconfig.json` exists with `strict: true`, `target: ES2020`, `module: commonjs`, `@topgunbuild/core` path mapping, no `@topgunbuild/server` mapping — mirrors `tests/e2e/tsconfig.json` structure exactly
- [✓] AC5: `test-client.ts` imports only `@topgunbuild/core`, `jsonwebtoken`, and `ws` — no `@topgunbuild/server` import paths, no import from `tests/e2e/helpers/index.ts`
- [✓] No BATCH parsing in test-client.ts — individual MsgPack frames sent and received as required by Rust server protocol
- [✓] BATCH framing correctly avoided — individual `ws.send(serialize(message))` calls only, matching spec requirements
- [✓] stderr inherited via `stdio: ['ignore', 'pipe', 'inherit']` — cargo build progress visible in parent process, not parsed for PORT
- [✓] `process.unref()` called after spawn to prevent test process from being held alive by child
- [✓] `RUST_SERVER_TIMEOUT` env var respected for configurable timeout
- [✓] `createRustTestContext` has robust error handling — closes clients and kills server if any client fails to connect
- [✓] `waitForMessage` resolver correctly checks existing messages before registering a new waiter (no race condition for pre-arrived messages)
- [✓] `TestClient` interface shape matches e2e helpers for test compatibility: `ws`, `nodeId`, `messages`, `isAuthenticated`, `send`, `waitForMessage`, `close`, `startHeartbeat`, `stopHeartbeat`
- [✓] `jest.config.js` sets `testTimeout: 60000`, correct `moduleNameMapper` for `@topgunbuild/core`, `setupFilesAfterEnv` pointing to `helpers/setup.ts`, and `testMatch: ['<rootDir>/**/*.test.ts']`
- [✓] Existing e2e tests untouched — no modifications to `tests/e2e/`
- [✓] No hardcoded ports — `spawnRustServer` uses port 0 (OS-assigned) as specified in Rust binary
- [✓] No PostgreSQL dependency — Rust binary uses `NullDataStore` as confirmed in `test_server.rs`
- [✓] `createTestToken` uses same `test-e2e-secret` JWT secret as Rust test server

**Minor:**
1. Unused local variable `isAuthenticated` at `test-client.ts:66` — the `let isAuthenticated = false` variable is initialized but only serves as the initial value for `client.isAuthenticated` on the object literal. After that, `client.isAuthenticated = true` sets the object property directly. The local variable is never read again. This is a harmless code smell inherited from the e2e helper pattern, but could be removed to avoid confusion.

2. `setup.ts` suppresses `console.log/debug/info/warn` but not `console.error` — the comment says "Suppress console output during tests" which is slightly misleading since errors still pass through. This is actually the correct and desirable behavior, but the comment could be more precise ("non-error console output").

**Summary:** The implementation fully meets all 5 acceptance criteria and all functional requirements. The code is clean, well-documented, and follows established patterns from the e2e helpers. The minor issues are cosmetic and inherited from the reference implementation — no action required.

---

## Completion

**Completed:** 2026-03-01
**Total Commits:** 4
**Audit Cycles:** 2
**Review Cycles:** 1
