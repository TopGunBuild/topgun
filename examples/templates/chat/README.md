# TopGun Real-Time Chat

A multi-user chat room where messages are delivered and displayed in HLC-causal order, not arrival order. Open two browser tabs, join the same room via URL hash (`#room-name`), and send messages from both — they appear in the same deterministic order in both tabs regardless of network timing.

## Quick start

```bash
# 1. Start the TopGun backend (single-node Docker compose)
#    See packages/server-rust/README.md or the root docs for docker-compose.yml.
#    The backend must expose a WebSocket at ws://localhost:8080/ws.

# 2. Copy the env file and (optionally) edit the WS URL
cp .env.example .env

# 3. Install and run
pnpm install && pnpm dev
# Opens at http://localhost:5175
```

> **No prior `pnpm build` at the repo root is required.** This app uses pnpm
> workspace resolution — `@topgunbuild/*` packages are resolved directly from
> source. Run `pnpm install` inside this directory (or at the repo root) and
> `pnpm dev` works immediately.

## Sharing a room

Room names live in the URL hash. Open `http://localhost:5175/#my-room` in two tabs to join the same room instantly — no sign-in required.

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `VITE_TOPGUN_URL` | `ws://localhost:8080/ws` | WebSocket URL of the TopGun server |

## Showcased TopGun features

### HLC-causal ordering: `useTopic`

`useRoom` subscribes to `chat:<room>` via `useTopic`. The message list sorts by `hlcTimestamp` (the sender's HLC wall clock at publish time), not by the order messages arrive. This means a message published 2 seconds ago but delivered late slots into its correct causal position when it arrives.

### Demo-only incoming-message delay: `SkewClockPanel`

The `<SkewClockPanelUI>` toggle buffers each *incoming* message by 5 seconds before passing it to the message list. This is a purely UI-side simulation — it does not skew the client's outgoing HLC (no such API exists on `TopGunClient`). The panel always displays the label:

> **Demo-only: simulates incoming message delay — not real HLC skew.**

To demonstrate:
1. Enable the +5s toggle on tab A.
2. Send messages from both tabs.
3. Watch tab A hold incoming messages for 5s, then insert them into correct HLC order on delivery.

A real `TopGunClient.setClockOffset()` API is tracked as TODO-287.

### Guest identity

Each browser tab gets a stable random display name (e.g. `SwiftFalcon42`) stored in `localStorage` via `examples/templates/_shared/guestIdentity.ts`. No authentication is required.

### Topics accept "from now forward" semantics

`useTopic` does not replay past messages on join. If you open a new tab after messages have been sent, you will only see messages sent after joining. This is documented as a known limitation of the current topic API — a "topic with history" feature is a future enhancement.

**Acceptance criteria verified without a live backend (offline checks):**
- AC #3: `pnpm --filter @topgun-examples/chat build` succeeds (TypeScript clean).

**Acceptance criteria requiring a live backend (deferred to human verification):**
- AC #8, #9: require `docker-compose up` per TODO-194. Verify with a running backend using the Validation Checklist in SPEC-223.md.
