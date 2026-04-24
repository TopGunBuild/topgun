# Template QA Runner

Automated end-to-end smoke tests for `examples/templates/{todo,chat}` covering acceptance criteria **AC #5–#9** from SPEC-223.

Drives [gsd-browser](https://github.com/koristuvac/gsd-browser) through five real-user scenarios and stores before/after screenshots so a human can verify behaviour at a glance.

## Quick start

Open three terminals:

```bash
# Terminal 1 — Rust backend (auth-optional mode; see Hardening Notes)
TOPGUN_NO_AUTH=1 pnpm start:server             # ws://localhost:8080/ws

# Terminal 2 — todo dev server
pnpm --filter @topgun-examples/todo dev        # http://localhost:5174

# Terminal 3 — chat dev server
pnpm --filter @topgun-examples/chat dev        # http://localhost:5175
```

Then run all scenarios in a fourth terminal:

```bash
./examples/templates/qa/run-qa.sh
```

Or just one:

```bash
./examples/templates/qa/run-qa.sh ac5         # only AC #5
./examples/templates/qa/run-qa.sh ac8 ac9     # AC #8 + #9
```

Screenshots and logs land in `examples/templates/qa/results/<timestamp>/<scenario>/`.

## Scenarios

| ID | What it verifies | Mechanism |
|----|------------------|-----------|
| **AC #5** | Real-time todo sync between two distinct guests | Two isolated Chrome sessions; tab A adds a uniquely-marked todo; tab B (separate localStorage = different `guestId`) opens fresh and waits for it to appear |
| **AC #6** | Offline edit → reconnect → `<SyncStatus>` shows "merged N pending writes" | Single session; `block-urls 'ws://localhost:8080/*'` + reload → page can't connect → write while offline → `clear-routes` + reload → expect Connected/Synced/merged label |
| **AC #7** | `<ConflictLog>` panel is wired into the UI | **INFO verdict (DOWNGRADE).** The `todosConflictResolver` targets the `todos` ORMap, whose unique-tag add semantics cannot produce merge rejections — so no live conflict path exists to assert on. Scenario captures a single screenshot documenting the panel's presence; see `### AC #7 disposition` below for the full investigation. |
| **AC #8** | HLC-ordered chat across two distinct guests in the same room | Two sessions on `${CHAT_URL}/#room-name`; A sends, B sends, both should see both messages |
| **AC #9** | `<SkewClockPanel>` buffers incoming messages +5s, then delivers them in HLC order | Two sessions; B toggles skew checkbox; A sends two messages; B should NOT see them immediately, then both appear after ~5s drain |

## Session model

Each scenario uses one or two named gsd-browser sessions:

- `topgun-qa-tab-a`, `topgun-qa-tab-b` — for two-user scenarios. Each session is a separate Chrome process with its own user data dir, so they have **distinct `localStorage` and IndexedDB**, which means `guestIdentity` generates different `guestId` values per session.
- `topgun-qa-solo` — for single-user scenarios (AC #6).

Sessions are stopped automatically via a trap on script exit.

## Per-scenario outputs

Each scenario directory contains numbered PNG screenshots tracking the flow:

```
results/2026-04-23_18-42-17/
├── ac5-sync/
│   ├── 01-tab-a-connected.png
│   ├── 02-tab-a-after-add.png
│   └── 03-tab-b-received.png        # or 03-tab-b-MISSING.png on fail
├── ac6-offline/
│   ├── 01-connected.png
│   ├── 02-offline.png
│   ├── 03-queued-while-offline.png
│   └── 04-reconnected-merged.png    # or 04-NO-RECONNECT.png on fail
├── ac7-conflict/
│   └── 01-conflict-log-panel-presence.png   # INFO — see AC #7 disposition
├── ac8-chat-order/
│   ├── 01-tab-a-empty.png
│   ├── 01-tab-b-empty.png
│   ├── 02-tab-a-after-exchange.png
│   └── 02-tab-b-after-exchange.png
└── ac9-skew-buffer/
    ├── 01-tab-b-skew-on.png
    ├── 02-tab-b-buffering.png       # or 02-tab-b-immediately-after-send.png
    └── 03-tab-b-after-drain.png
```

## Pass / fail signal

The script exits 0 if all selected scenarios pass, non-zero otherwise. Per-scenario verdicts are colourised in the terminal:

- `✓ AC #N PASS` — assertion succeeded
- `✗ AC #N FAIL` — assertion failed; check the `*-MISSING` / `*-NO-RECONNECT` screenshot
- `ℹ AC #N INFO` — informational, non-fatal (AC #7 by design; see `### AC #7 disposition`)
- `⚠ AC #N PARTIAL` — yellow non-fatal note (currently only AC #6 partial-drain)

## Environment overrides

| Variable | Default | Notes |
|----------|---------|-------|
| `TODO_URL` | `http://localhost:5174` | Vite dev URL for todo app |
| `CHAT_URL` | `http://localhost:5175` | Vite dev URL for chat app |
| `SERVER_PORT` | `8080` | TopGun server port (used for WS-block URL pattern + preflight `nc` probe) |
| `GSD_BROWSER` | `/Users/koristuvac/.cargo/bin/gsd-browser` | gsd-browser binary path |

## Caveats

- **`block-urls` blocks new connections only**, so AC #6 / AC #7 use `reload` to drop the existing WS handshake before the block takes effect.
- **macOS-only paths** — script assumes `/usr/bin/curl`, `/usr/bin/nc`, `/bin/sleep` etc. work without further PATH munging. Adjust the shebang and `g`/`shot` helpers for Linux if needed.

## Hardening Notes

### TOPGUN_NO_AUTH env-flag

The Rust test server (`packages/server-rust/src/bin/test_server.rs`) now reads `TOPGUN_NO_AUTH`:

```bash
TOPGUN_NO_AUTH=1 pnpm start:server
```

When set to `1` or `true`, `jwt_secret` is `None` in `AppState`, making the server auth-optional. Without the flag, the server retains `jwt_secret: Some("test-e2e-secret")` so existing integration tests remain unaffected.

The templates (`todo`, `chat`) intentionally omit `setAuthToken`. With SPEC-224's `AUTH_REQUIRED` 500ms grace-timeout, the client drives `CONNECTING → AUTHENTICATING → SYNCING → CONNECTED` when the server is auth-optional, reaching `CONNECTED` within 8 seconds of page load.

**The preflight section of `run-qa.sh` always prints this advisory unconditionally** — no detection logic, always shown regardless of env state. This ensures the advisory is never silently dropped by a future edit.

### wait_text_visible pattern

`gsd-browser`'s `wait-for --condition text_visible` matches text anywhere in the DOM, including input field values and hidden elements. This causes false positives: the "Add a new todo" `<input>` placeholder and any pre-filled input value can match a todo marker string before it appears as a rendered list item.

The `wait_text_visible` helper (added in SPEC-225 G4) pairs the wait with a post-wait assertion:

1. **Phase 1:** `wait-for --condition text_visible` — waits until any DOM text matches (same as before).
2. **Phase 2:** `find --text --json` — checks the match is NOT inside a `tag=input` or `tag=textarea` element. If `find` returns only input matches, falls back to an `eval` call that searches non-input elements' `textContent`.

All `wait_text` call sites in scenarios are replaced with `wait_text_visible`. Use `wait_text_visible` for any assertion where the expected text could appear in an input value or hidden element.

### prewarm_daemon rationale

gsd-browser's Chrome daemon cold-starts take >10 seconds in some environments (see `reference_gsd_browser_limitations.md` in user memory). Without prewarming, the first `wait_text_visible "Connected" 8000` in two-session scenarios (AC #5, #7, #8, #9) races the daemon startup and produces false-negative timeouts.

`prewarm_daemon` runs `daemon start` for each session in `ALL_SESSIONS` in parallel, then polls with `daemon status` for up to 15 seconds. It is invoked from `main()` after `preflight` and before the scenarios loop, moving the cold-start cost out of the scenario timeout budget.

### Cleanup guarantees

The `cleanup` trap (registered via `trap cleanup EXIT`) runs on script exit, clean or error:

1. **`daemon stop`** — graceful daemon shutdown per session in `ALL_SESSIONS`.
2. **`pkill -f "Google Chrome.*--user-data-dir=.*topgun-qa"`** — kills any orphan Chrome processes. macOS-only pattern (`Google Chrome`); on Linux the process name is `chrome` or `chromium` (harness not expected on Linux CI).
3. **`rm -rf ~/.gsd-browser/sessions/topgun-qa-*`** — removes stale session profile directories. Guards against accidental removal by checking the path starts with `$HOME/` and contains `topgun-qa`.

**Manual recovery** (if trap fails after SIGKILL):
```bash
pkill -f "Google Chrome.*topgun-qa"
rm -rf ~/.gsd-browser/sessions/topgun-qa-*
```

Re-running `run-qa.sh` immediately after a previous run should not show a "Failed to open profile" Chrome dialog if cleanup ran successfully.

### AC #7 disposition

**Decision: DOWNGRADE (INFO verdict)**

Investigation (SPEC-225 G1) found that the `todosConflictResolver` is registered on `mapName: 'todos'` — the ORMap that holds the todo collection. ORMap's `add(id, tag)` generates a unique tag per call, so two concurrent adds to the `todos` map produce distinct tag entries that both survive merge. No merge rejection is emitted.

For a genuine LWW conflict, the resolver would need to target per-todo LWWMaps (key pattern `todo:{id}`) where two tabs editing the same todo's `title` offline produce competing HLC entries on the same key. However, the `register(mapName, def)` API takes a literal map name, not a glob, and per-todo LWWMaps use dynamic names (`todo:abc123`, `todo:def456`, …) that cannot be enumerated statically.

**Result:** AC #7 emits an `INFO` verdict instead of `PASS` or `FAIL`. The `<ConflictLog>` panel and `useMergeRejections` hook shipped in SPEC-223 are architecturally correct; they just require a LWWMap target scenario to fire. A follow-up TODO tracks a dedicated smoke-test using a `user:settings` LWWMap that two tabs can edit simultaneously to verify the end-to-end conflict surface.
