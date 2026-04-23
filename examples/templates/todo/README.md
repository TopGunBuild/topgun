# TopGun Collaborative Todo

A real-time collaborative todo list that works offline. Open two browser tabs, go offline in one, make edits, come back online — your changes merge automatically using CRDT conflict resolution without losing any work.

## Quick start

```bash
# 1. Start the TopGun backend (single-node Docker compose)
#    See packages/server-rust/README.md or the root docs for docker-compose.yml.
#    The backend must expose a WebSocket at ws://localhost:8080/ws.

# 2. Copy the env file and (optionally) edit the WS URL
cp .env.example .env

# 3. Install and run
pnpm install && pnpm dev
# Opens at http://localhost:5174
```

> **No prior `pnpm build` at the repo root is required.** This app uses pnpm
> workspace resolution — `@topgunbuild/*` packages are resolved directly from
> source. Run `pnpm install` inside this directory (or at the repo root) and
> `pnpm dev` works immediately.

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `VITE_TOPGUN_URL` | `ws://localhost:8080/ws` | WebSocket URL of the TopGun server |

## Showcased TopGun features

### Primary differentiator: `<SyncStatus>` banner

The headline moment: go offline (DevTools → Network → Offline), edit a todo, go back online. The `<SyncStatus>` banner transitions visibly through:

```
Offline · writes queued locally  →  Syncing…  →  Synced · merged N pending writes
```

The client stores writes in IndexedDB (`topgun-template-todo`) while offline and replays them through the SyncEngine on reconnect. The banner reads `client.onConnectionStateChange()` — no polling, no custom hooks.

### Collections without write loss: `useORMap`

`TodoList` uses `useORMap('todos')` — an Observed-Remove Map. When two tabs both click "Add" at the same moment (even while one is offline), both todos appear after sync. Standard LWW would silently drop one. OR-Set semantics prevent that.

### Field-level conflict resolution: `useMap` + server-side resolver

Each todo stores its fields (`title`, `completed`) in a per-todo `useMap('todo:<id>')`. The app registers a server-side `ConflictResolverDef` at startup (in `src/lib/conflictResolver.ts`, called once from `src/main.tsx` before React mounts to avoid React 18 strict-mode double-registration):

```ts
client.getConflictResolvers().register('todos', {
  name: 'reject-stale',
  priority: 100,
  code: `/* server-evaluated JS: return { action: 'reject', reason: 'lower HLC' } */`,
});
```

The `code` string is evaluated server-side in a sandbox. When the losing write is rejected, the server emits a `MERGE_REJECTED` event. The `<ConflictLog>` panel subscribes via `useMergeRejections({ mapName: 'todos', maxHistory: 20 })` and shows the rejected write's key and reason.

**Acceptance criteria verified without a live backend (offline checks):**
- AC #1: `pnpm install` at repo root succeeds with both templates as workspace packages.
- AC #2: `pnpm --filter @topgun-examples/todo build` succeeds (TypeScript clean).
- AC #4: `pnpm --filter @topgun-examples/chat build` succeeds.

**Acceptance criteria requiring a live backend (deferred to human verification):**
- AC #5–#9: require `docker-compose up` per TODO-194. Verify with a running backend using the Validation Checklist in SPEC-223.md.
