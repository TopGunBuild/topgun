---
id: SPEC-120
type: perf
status: done
priority: high
complexity: small
created: 2026-03-17
---

# SPEC-120: Replace blocking dispatch with try_send for immediate backpressure

## Context

`PartitionDispatcher::dispatch()` uses `sender.send(request).await`, which blocks the calling Tokio task when the channel is full. With 10 workers x 1024 buffer = 10,240 buffered items, up to ~3.2 seconds of queue accumulation can occur before the caller experiences any backpressure. This delays rejection signals, wastes memory on doomed-to-timeout operations, and starves other Tokio tasks sharing the same runtime.

The `OperationError::Overloaded` variant already exists (used by `LoadShedLayer`), and the WebSocket `dispatch_message()` error path already handles dispatch errors -- but currently only logs them without sending an error response to the client.

## Task

1. In `dispatch()`, replace `sender.send(request).await` with `sender.try_send(request)`, mapping `TrySendError::Full` to `OperationError::Overloaded` and `TrySendError::Closed` to `OperationError::Internal` (preserving existing closed-channel behavior).
2. Reduce `DispatchConfig::default()` `channel_buffer_size` from 1024 to 256.
3. In `dispatch_message()`, when `dispatcher.dispatch(op).await` returns `Err(OperationError::Overloaded)`, send a `TopGunMessage::Error` with code 429 and message "server overloaded, try again later" to the client. Other errors retain the existing debug-log-only behavior.
4. In `dispatch_op_batch()`, when any sub-batch dispatch returns `OperationError::Overloaded`, use code 429 (not 500) in the error response sent to the client.
5. Update the existing test `dispatch_config_default_has_sensible_values` to assert `channel_buffer_size == 256`.
6. Add a test that fills a channel to capacity and verifies the next dispatch returns `OperationError::Overloaded` immediately (not after blocking).

## Requirements

### Files to Modify

1. **`packages/server-rust/src/service/dispatch.rs`**
   - Change `sender.send(request).await` to `sender.try_send(request)` with error mapping
   - Change `channel_buffer_size` default from 1024 to 256
   - Update existing default-values test assertion
   - Add test: create dispatcher with `channel_buffer_size: 1`, fill the channel with a slow-processing operation, then call `dispatch()` and assert it returns `Err(OperationError::Overloaded)` without blocking

2. **`packages/server-rust/src/network/handlers/websocket.rs`**
   - In `dispatch_message()`: match on `OperationError::Overloaded` in the `Err(e)` arm and send `TopGunMessage::Error { payload: ErrorPayload { code: 429, message: "server overloaded, try again later", details: None } }` to the client via `tx`
   - In `dispatch_op_batch()`: when `dispatch_error` originates from an `Overloaded` error, use code 429 instead of 500. **Important:** The current error collection in `dispatch_op_batch()` stores errors as `Option<String>` via `format!("{e}")`, which loses type information. To distinguish 429 from 500, the implementer must change this collection to preserve the `OperationError` type (e.g. use `Option<OperationError>` instead of `Option<String>`) so the error variant can be matched directly when building the response code. Do not distinguish error types by inspecting string content.

## Acceptance Criteria

1. `PartitionDispatcher::dispatch()` calls `try_send()`, never `send().await`
2. When a worker channel is full, `dispatch()` returns `Err(OperationError::Overloaded)` immediately (non-blocking)
3. When a worker channel is closed, `dispatch()` returns `Err(OperationError::Internal(_))` (unchanged behavior)
4. `DispatchConfig::default().channel_buffer_size` equals 256
5. Clients receive a 429-coded `Error` message when their operation is rejected due to overload
6. `dispatch_op_batch()` sends 429 (not 500) for overload rejections
7. All existing tests pass (dispatch routing, closed channel, etc.)
8. New test verifies immediate `Overloaded` rejection on full channel

## Constraints

- Do NOT change the `OperationError` enum (the `Overloaded` variant already exists)
- Do NOT modify `LoadShedLayer` or its permit-based backpressure (orthogonal mechanism)
- Do NOT add retry logic in the dispatcher -- retries are the client's responsibility
- The `dispatch()` method signature remains `pub async fn dispatch(&self, operation: Operation) -> Result<OperationResponse, OperationError>` (the async is still needed for `response_rx.await`)
- No phase/spec references in code comments

## Assumptions

- 256 is a sufficient burst buffer (provides ~0.8s at 300 ops/sec per worker, ample for transient spikes while still surfacing backpressure promptly)
- The `unpack_and_dispatch_batch()` function does not need 429 handling because it processes inner messages one-by-one and already silently drops errors (consistent with existing BATCH semantics where partial success is acceptable)
- HTTP 429 is the correct semantic code for "overloaded" even though this is a WebSocket message, not an HTTP response -- it maps to the standard "Too Many Requests" meaning

## Audit History

### Audit v1 (2026-03-17)
**Status:** APPROVED

**Context Estimate:** ~15% total

**Dimensions:**
- Clarity: PASS -- precise task descriptions with exact code locations
- Completeness: PASS -- all files listed, edge cases covered, explicit exclusion of unpack_and_dispatch_batch
- Testability: PASS -- all 8 acceptance criteria are concrete and verifiable
- Scope: PASS -- 2 files, well within Language Profile limit of 5
- Feasibility: PASS -- all referenced APIs, types, and code patterns verified in source
- Architecture fit: PASS -- reuses existing OperationError::Overloaded and TopGunMessage::Error patterns
- Non-duplication: PASS -- no new abstractions, reuses existing error variants
- Cognitive load: PASS -- straightforward, idiomatic Rust change (send -> try_send)
- Strategic fit: PASS -- continues SPEC-116/117/118/119 performance improvement trajectory
- Project compliance: PASS -- no new dependencies, follows WHY-comment convention, u32 error code correct
- Language profile: PASS -- 2 files modified, small complexity, no trait-first concerns

**Recommendations:**
1. In `dispatch_op_batch()`, the current code collects errors as `Option<String>` via `format!("{e}")`, losing type information. The implementer will need to either (a) change the collection to preserve `OperationError` type for the 429-vs-500 distinction, or (b) check the string content. Consider noting in the spec that the error collection pattern needs adjustment to support error-code differentiation. This is clear enough from context that an implementer will figure it out, but explicit guidance would reduce ambiguity.

**Comment:** Well-structured spec with precise code references verified against the actual source. All technical claims (existing Overloaded variant, current error handling behavior, ErrorPayload structure) are accurate. The scope is appropriately small and focused.

### Response v1 (2026-03-17)
**Applied:** Recommendation 1

**Changes:**
1. [✓] Error collection pattern guidance for `dispatch_op_batch()` — Added an explicit note to the `dispatch_op_batch()` bullet in Requirements specifying that the implementer must change the error collection from `Option<String>` to `Option<OperationError>` to preserve type information for 429-vs-500 differentiation, and must not distinguish error types by inspecting string content.

---

## Execution Summary

**Executed:** 2026-03-17
**Commits:** 2

### Files Created
None

### Files Modified
- `packages/server-rust/src/service/dispatch.rs` — replaced `sender.send().await` with `sender.try_send()`, mapped error variants, reduced buffer from 1024 to 256, updated existing test, added `full_channel_returns_overloaded_immediately` test
- `packages/server-rust/src/network/handlers/websocket.rs` — added `Overloaded` arm in `dispatch_message()` that sends 429 error to client; changed `dispatch_op_batch()` error collection from `Option<String>` to `Option<OperationError>` with 429-vs-500 match on variant

### Files Deleted
None

### Acceptance Criteria Status
- [x] `PartitionDispatcher::dispatch()` calls `try_send()`, never `send().await`
- [x] When a worker channel is full, `dispatch()` returns `Err(OperationError::Overloaded)` immediately (non-blocking)
- [x] When a worker channel is closed, `dispatch()` returns `Err(OperationError::Internal(_))` (unchanged behavior)
- [x] `DispatchConfig::default().channel_buffer_size` equals 256
- [x] Clients receive a 429-coded `Error` message when their operation is rejected due to overload
- [x] `dispatch_op_batch()` sends 429 (not 500) for overload rejections
- [x] All existing tests pass (540 passed, 0 failed)
- [x] New test verifies immediate `Overloaded` rejection on full channel

### Deviations
None

### Notes
The initial `full_channel_returns_overloaded_immediately` test used `dispatch()` to fill the channel, which blocked forever waiting on the oneshot response (no real worker was consuming). Fixed by pre-filling the raw MPSC channel via `tx.try_send()` directly, bypassing `dispatch()` for the slot-filling step. This avoids the `response_rx.await` deadlock while still exercising the overload path correctly.

### Audit v2 (2026-03-17)
**Status:** APPROVED

**Context Estimate:** ~15% total

**Dimensions:**
- Clarity: PASS -- all 6 tasks precise with exact function names, error variants, and code patterns
- Completeness: PASS -- v1 recommendation addressed; error collection guidance now explicit in Requirements
- Testability: PASS -- all 8 acceptance criteria concrete and verifiable
- Scope: PASS -- 2 files, small complexity
- Feasibility: PASS -- all APIs verified in source (try_send, OperationError::Overloaded, ErrorPayload.code: u32)
- Architecture fit: PASS -- reuses existing error/message patterns established by LoadShedLayer and send_operation_response
- Non-duplication: PASS -- no new types or abstractions
- Cognitive load: PASS -- minimal, idiomatic Rust change
- Strategic fit: PASS -- aligned with SPEC-116-119 performance trajectory
- Project compliance: PASS -- no new dependencies, follows all conventions
- Language profile: PASS -- 2 files modified, well within 5-file limit

**Comment:** Clean re-audit after v1 revision. The added guidance for `dispatch_op_batch()` error collection (`Option<OperationError>` instead of `Option<String>`) eliminates the only ambiguity from v1. Spec is ready for implementation.

---

## Review History

### Review v1 (2026-03-17)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: `dispatch()` uses `try_send()` exclusively — no `send().await` remains anywhere in dispatch.rs
- [✓] AC2: `TrySendError::Full` maps to `OperationError::Overloaded`; verified in dispatch.rs:143-148
- [✓] AC3: `TrySendError::Closed` maps to `OperationError::Internal(_)` — closed-channel behavior preserved
- [✓] AC4: `DispatchConfig::default().channel_buffer_size` is 256; asserted by existing test at dispatch.rs:285
- [✓] AC5: `dispatch_message()` in websocket.rs:249-261 sends `TopGunMessage::Error { code: 429, message: "server overloaded, try again later" }` on `Overloaded`
- [✓] AC6: `dispatch_op_batch()` uses `Option<OperationError>` (not `Option<String>`) for error collection; matches on `OperationError::Overloaded` to emit code 429, all other variants emit code 500
- [✓] AC7: 540 tests pass, 0 failures, clippy-clean with `-D warnings`
- [✓] AC8: `full_channel_returns_overloaded_immediately` test pre-fills channel via raw `tx.try_send()` to avoid response-await deadlock, then calls `dispatcher.dispatch()` and asserts `Err(OperationError::Overloaded)`
- [✓] Constraints respected: `OperationError` enum unchanged, `LoadShedLayer` untouched, no retry logic added, `dispatch()` signature unchanged
- [✓] No spec/phase references in code comments — all comments are WHY-oriented
- [✓] No code duplication — 429 error message string reused consistently in both dispatch_message() and dispatch_op_batch()
- [✓] Build: `cargo check` and `cargo clippy -- -D warnings` both exit 0

**Summary:** Implementation is a clean, minimal, and correct execution of the spec. All 8 acceptance criteria are satisfied, all 540 tests pass, clippy is clean. The test approach for `full_channel_returns_overloaded_immediately` is well-reasoned — bypassing `dispatch()` for the fill step correctly avoids the oneshot deadlock without compromising test validity.

---

## Completion

**Completed:** 2026-03-17
**Total Commits:** 2
**Review Cycles:** 1

### Outcome

Replaced blocking `send().await` with non-blocking `try_send()` in PartitionDispatcher, enabling immediate backpressure via `OperationError::Overloaded`. Clients now receive 429 error responses when the server is overloaded instead of silently queuing operations.

### Key Files

- `packages/server-rust/src/service/dispatch.rs` — non-blocking dispatch with try_send and overload detection
- `packages/server-rust/src/network/handlers/websocket.rs` — 429 error responses to clients on overload

### Patterns Established

None — followed existing patterns.

### Deviations

None — implemented as specified.
