# SPEC-046: Replace WebSocket Return Type in IConnectionProvider with IConnection Interface

---
id: SPEC-046
type: refactor
status: done
priority: high
complexity: small
created: 2026-02-10
todo: TODO-050
---

## Context

`IConnectionProvider.getConnection()` and `getAnyConnection()` return bare `WebSocket` types (`packages/client/src/types.ts`, lines 46 and 54). This creates a type-safety hole:

- `HttpSyncProvider` implements `IConnectionProvider` but throws at runtime on both methods because it cannot return a `WebSocket`.
- `AutoConnectionProvider` delegates to its active provider, so it throws at runtime when in HTTP mode.
- Any future transport (SSE, QUIC, cluster HTTP) will hit the same problem.
- All actual callers of these methods only need `send()` and readyState checks -- they never use WebSocket-specific APIs like `binaryType`, `protocol`, `bufferedAmount`, `onopen`, etc.

This refactoring replaces the concrete `WebSocket` return type with a minimal `IConnection` interface that captures the actual usage pattern, allowing `HttpSyncProvider` to return a valid object instead of throwing.

## Task

1. Define a new `IConnection` interface in `packages/client/src/types.ts`.
2. Change `IConnectionProvider.getConnection()` and `getAnyConnection()` return types from `WebSocket` to `IConnection`.
3. Wrap raw `WebSocket` instances in `IConnection` adapters in `SingleServerProvider` and `ConnectionPool`.
4. Update `ClusterClient` to use `IConnection` instead of `WebSocket`.
5. Have `HttpSyncProvider` return a no-op `IConnection` (that queues via `send()`) instead of throwing.
6. Update `AutoConnectionProvider` type signatures to match.
7. Update tests to reflect the new interface: `HttpSyncProvider.test.ts`, `IConnectionProvider.test.ts`, `PartitionRouter.test.ts`, and `AutoConnectionProvider.test.ts`.

## Requirements

### New Interface

**File:** `packages/client/src/types.ts`

Add `IConnection` interface:

```typescript
export interface IConnection {
  send(data: ArrayBuffer | Uint8Array | string): void;
  close(): void;
  readonly readyState: number;
}
```

Three methods/properties, matching what callers actually use:
- `send()` -- every caller calls `.send()` on the returned connection
- `close()` -- used in some shutdown paths
- `readyState` -- used in `ConnectionPool.flushPendingMessages()` (line 479: `connection.socket.readyState === WebSocket.OPEN`) and `ConnectionPool.send()` (line 229) and `ConnectionPool.performHealthCheck()` (line 538)

### Updated IConnectionProvider

**File:** `packages/client/src/types.ts`

Change:
- `getConnection(key: string): WebSocket` to `getConnection(key: string): IConnection`
- `getAnyConnection(): WebSocket` to `getAnyConnection(): IConnection`

### WebSocketConnection Adapter

**File:** `packages/client/src/connection/WebSocketConnection.ts` (new file)

Create a thin adapter class that wraps a `WebSocket` and implements `IConnection`:

```typescript
export class WebSocketConnection implements IConnection {
  constructor(private readonly ws: WebSocket) {}
  send(data: ArrayBuffer | Uint8Array | string): void { this.ws.send(data); }
  close(): void { this.ws.close(); }
  get readyState(): number { return this.ws.readyState; }
}
```

Export `READY_STATE` constants (or re-export WebSocket constants) so callers can compare `readyState` without referencing `WebSocket.OPEN` directly:

```typescript
export const ConnectionReadyState = {
  CONNECTING: 0,
  OPEN: 1,
  CLOSING: 2,
  CLOSED: 3,
} as const;
```

### SingleServerProvider Changes

**File:** `packages/client/src/connection/SingleServerProvider.ts`

- `getConnection()` and `getAnyConnection()` return `IConnection` (wrap `this.ws` in `WebSocketConnection`).
- Internal `this.ws` field remains `WebSocket | null` (no change to internal WebSocket management).

### ConnectionPool Changes

**File:** `packages/client/src/cluster/ConnectionPool.ts`

- `NodeConnection.socket` field type stays `WebSocket | null` internally.
- `getConnection(nodeId)` return type changes from `WebSocket | null` to `IConnection | null` (wrap socket in `WebSocketConnection` before returning).
- `getAnyHealthyConnection()` return type changes from `{ nodeId: string; socket: WebSocket } | null` to `{ nodeId: string; connection: IConnection } | null` (rename field from `socket` to `connection` for clarity).
- `getPrimaryConnection()` (line 200) delegates to `getConnection()`, so its return type will implicitly change from `WebSocket | null` to `IConnection | null`. No external callers, but be aware of this cascading change.
- Internal methods (`flushPendingMessages`, `performHealthCheck`, `sendAuth`, `send`) continue using the raw `WebSocket` internally since they manage the socket lifecycle.

### ClusterClient Changes

**File:** `packages/client/src/cluster/ClusterClient.ts`

- `getConnection(key)` return type changes to `IConnection`.
- `getAnyConnection()` return type changes to `IConnection`.
- `getAnyConnectionOrNull()` return type changes to `IConnection | null`.
- `getFallbackConnection()` return type changes to `IConnection`.
- `send()` method (line 273): `socket.send(data)` -- already works since `IConnection` has `send()`.
- `sendWithRetry()` (line 318): `socket.send(data)` -- already works.
- `sendDirect()` (line 498): Update from `connection.socket.send()` to `connection.connection.send()` to match the renamed `getAnyHealthyConnection()` return field.
- `getFallbackConnection()` (line 224): Update `conn?.socket` to `conn?.connection` to match renamed field.
- `getAnyConnection()` (line 658): Update `conn?.socket` to `conn?.connection` to match renamed field.
- `getAnyConnectionOrNull()` (line 670): Update `conn?.socket` to `conn?.connection` to match renamed field.
- `requestPartitionMapFromNode()` (line 245): Uses `connectionPool.getConnection()` then calls `.send()` -- already works with `IConnection`.

### PartitionRouter Changes

**File:** `packages/client/src/cluster/PartitionRouter.ts`

- `routeToConnection()` return type changes from `{ nodeId: string; socket: WebSocket } | null` to `{ nodeId: string; connection: IConnection } | null`.
- All internal references update from `.socket` to `.connection` in this return value.

### HttpSyncProvider Changes

**File:** `packages/client/src/connection/HttpSyncProvider.ts`

- `getConnection()` and `getAnyConnection()` return an `HttpConnection` (a no-op `IConnection`) instead of throwing.
- The `HttpConnection` implementation:
  - `send()`: delegates to the provider's `send()` method (queues the operation for next HTTP poll)
  - `close()`: no-op (HTTP connections are stateless)
  - `readyState`: returns `ConnectionReadyState.OPEN` when connected, `ConnectionReadyState.CLOSED` otherwise

### AutoConnectionProvider Changes

**File:** `packages/client/src/connection/AutoConnectionProvider.ts`

- `getConnection()` and `getAnyConnection()` return types change to `IConnection`.
- Delegation logic unchanged.

### Public API Exports

**File:** `packages/client/src/index.ts`

- Export `IConnection` type from `./types`.
- Export `WebSocketConnection` class and `ConnectionReadyState` from `./connection/WebSocketConnection`.

**File:** `packages/client/src/connection/index.ts`

- Re-export `WebSocketConnection` and `ConnectionReadyState` from the connection barrel file for consistency with other connection exports.

### Files to Delete

None.

## Acceptance Criteria

1. `IConnection` interface exists in `packages/client/src/types.ts` with `send()`, `close()`, and `readyState` members.
2. `IConnectionProvider.getConnection()` returns `IConnection` (not `WebSocket`).
3. `IConnectionProvider.getAnyConnection()` returns `IConnection` (not `WebSocket`).
4. `HttpSyncProvider.getConnection()` returns a valid `IConnection` object instead of throwing.
5. `HttpSyncProvider.getAnyConnection()` returns a valid `IConnection` object instead of throwing.
6. `SingleServerProvider.getConnection()` returns an `IConnection` wrapping the internal WebSocket.
7. `ConnectionPool.getConnection()` returns `IConnection | null`.
8. `ClusterClient.getConnection()` returns `IConnection`.
9. `ClusterClient.getAnyConnection()` returns `IConnection`.
10. `pnpm --filter @topgunbuild/client test` passes with zero failures.
11. `pnpm build` succeeds with no TypeScript errors.
12. No `WebSocket` type appears in the signatures of `IConnectionProvider`, `getConnection`, or `getAnyConnection` across the client package public API.

## Constraints

- Do NOT change the internal WebSocket management in `SingleServerProvider`, `ConnectionPool`, or `ClusterClient`. Only the public return types change.
- Do NOT change the `IConnectionProvider.send()` method signature -- it already provides transport-agnostic sending.
- Do NOT modify any server package code -- this is a client-only refactoring.
- Do NOT break the existing `WebSocketManager` or `SyncEngine` -- they do not call `getConnection()`/`getAnyConnection()` and should be unaffected.
- Keep `WebSocket` as the internal socket type in `NodeConnection` (ConnectionPool) and `SingleServerProvider.ws` -- the adapter wraps on return, not at storage.

## Assumptions

- `readyState` numeric constants (0-3) match the WebSocket spec values. Using a `ConnectionReadyState` constant object avoids depending on the `WebSocket` global for comparisons.
- The `send()` method on `IConnection` accepts `ArrayBuffer | Uint8Array | string` to match the existing WebSocket send() usage patterns in the codebase.
- The `HttpConnection` class can be defined as a private class inside `HttpSyncProvider.ts` since it is tightly coupled to the HTTP provider's internal queue mechanism.
- Renaming `socket` to `connection` in `ConnectionPool.getAnyHealthyConnection()` return type is acceptable since the old field name leaked the implementation. If this causes too many cascading changes, keeping `socket` as the field name (typed as `IConnection`) is an acceptable alternative.
- The existing test in `HttpSyncProvider.test.ts` that asserts `getConnection() throws` will be updated to assert it returns a valid `IConnection` instead.

## Audit History

### Audit v1 (2026-02-10 15:00)
**Status:** APPROVED

**Context Estimate:** ~40% total

**Quality Dimensions:**
- Clarity: PASS -- Context explains WHY, task explains WHAT, requirements explain HOW. No vague terms.
- Completeness: PASS -- All source files listed with specific changes. Test files covered via acceptance criterion 10.
- Testability: PASS -- All 12 acceptance criteria are measurable and verifiable with concrete commands.
- Scope: PASS -- Boundaries explicit (client package only, no server changes). Constraints well-defined.
- Feasibility: PASS -- Verified against source code. Adapter pattern is straightforward. Claims about caller usage validated.
- Architecture Fit: PASS -- Adapter pattern consistent with existing codebase. TypeScript strict mode honored.
- Non-Duplication: PASS -- No existing abstraction for this; novel interface extraction.
- Cognitive Load: PASS -- Minimal interface (3 members), simple adapter, clear naming.
- Strategic Fit: PASS -- Aligns with future transport plans (HTTP sync, Rust migration). Solves real type-safety issue.
- Project Compliance: PASS -- No violations of PROJECT.md constraints. Client-only, TypeScript, no new deps.

**Assumptions Validated:**
| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | readyState constants (0-3) match WebSocket spec | Incorrect comparisons; low risk, constants are standard |
| A2 | Callers only use send(), close(), readyState | IConnection interface would be insufficient; validated via grep |
| A3 | `socket` to `connection` rename is manageable | Extra cascading changes; mitigated by fallback option in spec |
| A4 | HttpConnection can be private to HttpSyncProvider | Works since no external consumers need it |

**Project Compliance:** Honors PROJECT.md decisions. No new dependencies, client-only scope, TypeScript strict mode.

**Strategic Fit:** Aligned with project goals -- enables transport-agnostic connection provider pattern.

**Recommendations:**
1. The ClusterClient section lists `sendDirect()` as needing `.socket` to `.connection` rename but does not explicitly list `getFallbackConnection()` (line 224: `conn?.socket`), `getAnyConnection()` (line 658: `conn?.socket`), and `getAnyConnectionOrNull()` (line 670: `conn?.socket`) which also access the `.socket` field on `getAnyHealthyConnection()` results. These will need updating too. The implementer will discover this via TypeScript errors, but listing them would improve completeness.
2. `ConnectionPool.getPrimaryConnection()` (line 200) returns `WebSocket | null` and delegates to `getConnection()`. Its return type will implicitly change to `IConnection | null`. Consider mentioning this for completeness, though it has no external callers.
3. The spec lists ~13 files to modify/create but test files are only mentioned generically ("Update tests to reflect the new interface"). For implementer clarity, explicitly listing the affected test files would help: `HttpSyncProvider.test.ts`, `IConnectionProvider.test.ts`, `PartitionRouter.test.ts`, and potentially `AutoConnectionProvider.test.ts`.
4. Consider whether `ConnectionReadyState` should also be exported from `packages/client/src/connection/index.ts` barrel file for consistency with other connection exports.

**Comment:** Well-structured specification with clear motivation, detailed per-file instructions, and pragmatic assumptions with fallback options. The interface design is minimal and appropriate. Source code verification confirms all claims about caller patterns are accurate.

### Response v1 (2026-02-10)
**Applied:** All 4 recommendations from Audit v1

**Changes:**
1. [✓] Added `getFallbackConnection()`, `getAnyConnection()`, and `getAnyConnectionOrNull()` to ClusterClient section — these also access `.socket` on `getAnyHealthyConnection()` results and need `.connection` rename
2. [✓] Added `getPrimaryConnection()` note to ConnectionPool section — its return type implicitly changes from `WebSocket | null` to `IConnection | null`
3. [✓] Added explicit test file list to Task step 7 — `HttpSyncProvider.test.ts`, `IConnectionProvider.test.ts`, `PartitionRouter.test.ts`, `AutoConnectionProvider.test.ts`
4. [✓] Added `packages/client/src/connection/index.ts` barrel file re-export for `WebSocketConnection` and `ConnectionReadyState`

### Audit v2 (2026-02-10 16:30)
**Status:** APPROVED

**Context Estimate:** ~44% total

**Quality Dimensions:**
- Clarity: PASS -- All sections well-structured. Context, Task, Requirements, and Constraints are unambiguous.
- Completeness: PASS -- All 4 recommendations from Audit v1 incorporated. Every source file, test file, and cascading change explicitly listed.
- Testability: PASS -- 12 acceptance criteria, each verifiable. Includes concrete build/test commands.
- Scope: PASS -- Client-only boundary clear. Constraints prevent scope creep.
- Feasibility: PASS -- Source code verified. Adapter pattern straightforward. All caller usage patterns confirmed via grep.
- Architecture Fit: PASS -- Adapter pattern consistent with existing codebase conventions.
- Non-Duplication: PASS -- No existing abstraction for this purpose.
- Cognitive Load: PASS -- 3-member interface, thin adapter, clear naming conventions.
- Strategic Fit: PASS -- Solves root cause (concrete type in interface), enables future transports.
- Project Compliance: PASS -- No PROJECT.md violations. No new dependencies, TypeScript strict, client-only.

**Assumptions Validated:**
| # | Assumption | If wrong, impact |
|---|------------|------------------|
| A1 | readyState constants (0-3) match WebSocket spec | Low risk; standard values |
| A2 | Callers only use send(), close(), readyState | Validated by grep across client package |
| A3 | `socket` to `connection` rename manageable | Spec lists all 5 affected sites; fallback option included |
| A4 | HttpConnection can be private to HttpSyncProvider | No external consumers verified |

**Project Compliance:** Honors PROJECT.md decisions. No new dependencies, client-only scope, TypeScript strict mode.

**Strategic Fit:** Aligned with project goals -- enables transport-agnostic connection provider pattern.

**Comment:** Specification is complete and ready for implementation. All recommendations from Audit v1 have been properly incorporated. The revised spec now explicitly lists every method that accesses `getAnyHealthyConnection().socket`, the `getPrimaryConnection()` cascading change, all four affected test files, and the barrel file re-export. No further issues found.

---

## Execution Summary

**Executed:** 2026-02-10
**Commits:** 9

### Files Created
- `packages/client/src/connection/WebSocketConnection.ts` -- IConnection adapter wrapping raw WebSocket, plus ConnectionReadyState constants

### Files Modified
- `packages/client/src/types.ts` -- Added IConnection interface; changed getConnection/getAnyConnection return types from WebSocket to IConnection
- `packages/client/src/connection/SingleServerProvider.ts` -- getConnection/getAnyConnection wrap ws in WebSocketConnection
- `packages/client/src/cluster/ConnectionPool.ts` -- getConnection returns IConnection | null; getAnyHealthyConnection renamed socket field to connection
- `packages/client/src/cluster/PartitionRouter.ts` -- routeToConnection returns { nodeId, connection: IConnection }
- `packages/client/src/cluster/ClusterClient.ts` -- All public methods return IConnection; updated field access from .socket to .connection
- `packages/client/src/connection/HttpSyncProvider.ts` -- Added HttpConnection class; getConnection/getAnyConnection return valid IConnection instead of throwing
- `packages/client/src/connection/AutoConnectionProvider.ts` -- Updated return types to IConnection
- `packages/client/src/connection/index.ts` -- Re-export WebSocketConnection and ConnectionReadyState
- `packages/client/src/index.ts` -- Export IConnection type, WebSocketConnection class, ConnectionReadyState
- `packages/client/src/__tests__/HttpSyncProvider.test.ts` -- Updated tests: assert valid IConnection return instead of throws; added readyState and send delegation tests
- `packages/client/src/__tests__/PartitionRouter.test.ts` -- Updated mock to use { connection } field name

### Files Deleted
None.

### Acceptance Criteria Status
- [x] 1. IConnection interface exists in types.ts with send(), close(), and readyState
- [x] 2. IConnectionProvider.getConnection() returns IConnection
- [x] 3. IConnectionProvider.getAnyConnection() returns IConnection
- [x] 4. HttpSyncProvider.getConnection() returns valid IConnection instead of throwing
- [x] 5. HttpSyncProvider.getAnyConnection() returns valid IConnection instead of throwing
- [x] 6. SingleServerProvider.getConnection() returns IConnection wrapping internal WebSocket
- [x] 7. ConnectionPool.getConnection() returns IConnection | null
- [x] 8. ClusterClient.getConnection() returns IConnection
- [x] 9. ClusterClient.getAnyConnection() returns IConnection
- [x] 10. pnpm --filter @topgunbuild/client test passes with zero failures (497/497 tests, 27/27 suites)
- [x] 11. pnpm build succeeds with no TypeScript errors
- [x] 12. No WebSocket type appears in signatures of IConnectionProvider, getConnection, or getAnyConnection

### Deviations
None. All spec requirements implemented as specified.

### Notes
- IConnectionProvider.test.ts and AutoConnectionProvider.test.ts required no changes -- existing tests pass without modification since getConnection/getAnyConnection still throw when not connected (SingleServerProvider) and mock patterns are compatible
- ConnectionPool internal methods (flushPendingMessages, performHealthCheck, sendAuth, send) continue using raw WebSocket as specified in constraints
- The `socket` to `connection` field rename in getAnyHealthyConnection() cascaded cleanly through PartitionRouter and ClusterClient with no issues

---

## Review History

### Review v1 (2026-02-10 19:15)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [v] AC1: `IConnection` interface exists in `packages/client/src/types.ts` (lines 12-16) with `send()`, `close()`, and `readyState` members -- matches spec exactly
- [v] AC2: `IConnectionProvider.getConnection()` returns `IConnection` (line 58 of types.ts)
- [v] AC3: `IConnectionProvider.getAnyConnection()` returns `IConnection` (line 66 of types.ts)
- [v] AC4: `HttpSyncProvider.getConnection()` returns `new HttpConnection(this)` (line 133) -- valid IConnection, no longer throws
- [v] AC5: `HttpSyncProvider.getAnyConnection()` returns `new HttpConnection(this)` (line 141) -- valid IConnection, no longer throws
- [v] AC6: `SingleServerProvider.getConnection()` returns `new WebSocketConnection(this.ws)` (line 119) -- wraps internal WebSocket
- [v] AC7: `ConnectionPool.getConnection()` returns `IConnection | null` (line 191) -- wraps via `new WebSocketConnection(connection.socket)`
- [v] AC8: `ClusterClient.getConnection()` returns `IConnection` (line 172)
- [v] AC9: `ClusterClient.getAnyConnection()` returns `IConnection` (line 656)
- [v] AC10: `pnpm --filter @topgunbuild/client test` passes -- 497/497 tests, 27/27 suites, zero failures
- [v] AC11: `pnpm build` succeeds with no TypeScript errors (full monorepo build clean)
- [v] AC12: No `WebSocket` type in `getConnection`/`getAnyConnection` signatures -- verified via grep across client package (only comments mention "WebSocket" in types.ts, no type references)
- [v] Constraint: Internal `WebSocket` management preserved -- `SingleServerProvider.ws` is `WebSocket | null` (line 30), `NodeConnection.socket` is `WebSocket | null` (line 42 of ConnectionPool.ts)
- [v] Constraint: `IConnectionProvider.send()` signature unchanged -- `send(data: ArrayBuffer | Uint8Array, key?: string): void` (line 105)
- [v] Constraint: No server package files modified (verified via `git diff`)
- [v] Constraint: `SyncEngine` and `WebSocketManager` not modified (verified via `git diff`)
- [v] `WebSocketConnection` adapter is clean, thin, and implements `IConnection` correctly (33 lines)
- [v] `ConnectionReadyState` constants match WebSocket spec values (0-3)
- [v] `HttpConnection` class properly delegates `send()` to provider, handles string/ArrayBuffer/Uint8Array, returns correct `readyState` based on connection state
- [v] `.socket` to `.connection` field rename in `getAnyHealthyConnection()` cascaded cleanly -- zero `.socket` references remain in ClusterClient.ts and PartitionRouter.ts
- [v] Public API exports correct: `IConnection` type, `WebSocketConnection` class, `ConnectionReadyState` all exported from both `connection/index.ts` and `index.ts`
- [v] Test coverage: 4 new test cases added in HttpSyncProvider.test.ts (getConnection valid IConnection, getAnyConnection valid IConnection, readyState reflects state, send delegation)
- [v] PartitionRouter.test.ts mock updated to use `{ connection }` field name (line 164)
- [v] No code comments contain spec/phase/bug references (project convention honored)
- [v] Code quality: Clean, readable, follows existing patterns. No unnecessary complexity.
- [v] No security issues: No hardcoded secrets, no input validation gaps relevant to this change
- [v] No duplication: Reuses existing patterns, no copy-paste

**Summary:** Implementation is a clean, faithful execution of the specification. All 12 acceptance criteria verified. All 5 constraints respected. The adapter pattern is minimal and appropriate -- `WebSocketConnection` adds zero overhead, `HttpConnection` correctly delegates to the provider's queue. The `.socket` to `.connection` rename was applied consistently. Build passes, all 497 tests pass, no server code touched. No issues found.

---

## Completion

**Completed:** 2026-02-10
**Total Commits:** 9
**Audit Cycles:** 2
**Review Cycles:** 1
