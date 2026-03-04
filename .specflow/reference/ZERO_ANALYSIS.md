# Zero/RociCorp Analysis — Lessons for TopGun

*Created: 2026-03-03*
*Context: Competitive analysis for TODO-105 (Sync Showcase Demo) and marketing strategy*

---

## What Is Zero

Zero (by RociCorp) is a **sync layer over PostgreSQL** — not a standalone data platform. It provides instant client-side queries via local cache (IndexedDB/SQLite) with server-authoritative conflict resolution.

- **Repo analyzed:** `/Users/koristuvac/Projects/dev/mono` (open-source monorepo)
- **Demo app:** zbugs — bug tracker with 240K issues, 2.5M rows
- **Website:** https://zero.rocicorp.dev/
- **Live demo:** https://bugs.rocicorp.dev/p/roci

## Key Differences: Zero vs TopGun

| | Zero | TopGun |
|---|---|---|
| **Architecture** | Sync layer over PostgreSQL | Self-contained data platform |
| **Conflicts** | Server-authoritative (server decides) | CRDT (automatic merge, no server needed) |
| **Storage** | PostgreSQL = primary, client = cache | In-memory = primary, PostgreSQL = persist |
| **Read API** | Async (ZQL queries) | Synchronous (`map.get()` — no await) |
| **Offline writes** | Queued, server validates on reconnect | Applied locally via CRDT, merge on sync |
| **Server compute** | None (read-only cache) | Entry processors, DAG, SQL, pub/sub |

**Conclusion:** Not a direct competitor. Different architectural category. Zero = "make Postgres instant". TopGun = "replace the backend stack".

---

## Borrowable: Marketing & UX Patterns

### 1. `?demo` URL Param (applied to TODO-105)

zbugs uses URL params to switch demo modes:
- `?demo` — shows "Loaded in 1.5s" performance badge
- `?demovideo` — compact load indicator for screen recordings
- `?spinnerstay` — keeps spinner visible for performance comparison screenshots

**Applied:** Added to TODO-105 scope. Show read latency and pending ops count in badge.

### 2. "Video is not sped up" Disclaimer

Zero's demo video includes overlay text: "Video is not sped up. All transitions are realtime." Simple but highly effective trust signal. Use in TopGun Remotion video.

### 3. Social Proof Section (deferred)

Zero has 10 user testimonials on frontpage. TopGun has 1 GitHub issue. **Do not add social proof section until real users exist.** One quote looks worse than no section.

**Alternative for now:** "Built with" section showing own demo apps, or objective metrics (test count, uptime).

### 4. Performance Badges in Demo

zbugs shows live metrics:
- "Loaded in X seconds"
- Issue count ("100 of ~240k")
- Connection state

TopGun's TODO-105 "Magic Control Panel" already plans this. Confirmed as correct approach.

### 5. Synthetic Data Generation (v2.0)

zbugs generates 240K realistic issues via Claude API templates → CSV → PostgreSQL seed. Useful pattern for future TopGun full-app demo (Real-time Dashboard, v2.0).

---

## Borrowable: Technical Patterns

### Virtual Scrolling (@tanstack/react-virtual)

zbugs uses `@tanstack/react-virtual` for 2.5M row lists. **Not needed for TODO-105** (simple to-do list), but relevant for v2.0 full-app demo.

### Soft Navigation (wouter)

zbugs uses `wouter` (lightweight router) + custom `useSoftNav()` hook for SPA navigation without page reload. Consider for TODO-105 if multi-page.

### Cursor-Based Pagination

```typescript
start: { id: '123', modified: 1700000000 },
dir: 'forward',
inclusive: false
```
Relevant pattern for TopGun live queries with large result sets.

---

## NOT Borrowable

| Zero Pattern | Why Not for TopGun |
|---|---|
| **ZQL query DSL** | TopGun uses `map.get()/set()` + predicate queries. Different paradigm |
| **Replicache sync engine** | TopGun has its own SyncEngine with Merkle trees |
| **Server-side mutation validation** | TopGun uses CRDTs — mutations are local-first, not server-validated |
| **PostgreSQL CDC** | TopGun IS the primary store, not a Postgres replica |
| **zero-cache architecture** | Read-only cache layer — TopGun is read-write everywhere |

---

## Demo Strategy (confirmed by analysis)

**TODO-105 Conflict Arena is the right choice.** Reasons:

1. Zero already occupies "fast bug tracker" niche. Don't compete on their turf
2. TopGun's killer feature (automatic CRDT merge) is **invisible in Zero** (server-authoritative)
3. Split-screen conflict demo is something Zero **cannot show** — their conflicts are hidden
4. Conflict Arena = 3-4 days. Bug tracker = months. ROI is clear

**Future (v2.0):** Real-time monitoring dashboard. Uses maps, queries, pub/sub, counters, entry processors. Doesn't compete with zbugs (different domain). Appeals to Hazelcast audience.

---

## Remotion Video (future TODO)

Existing Remotion project at `/Users/koristuvac/Projects/dev/remotion-test/my-video` has reusable components:
- **Terminal** (typewriter effect) — show `map.set()`, sync log
- **CodeBlock** (syntax highlight) — show CRDT schema, 5-line code example
- **StepColumn + ConnectorArrow** — architecture flow diagram
- **StatCard** (counting animation) — `< 0.3ms`, `271 partitions`, `0 conflict code`
- **Audio system** — whoosh, success chime, typing sounds

New components needed: SplitScreen (two devices), HLC Timeline visualization.

**Estimated effort:** 8-12 hours after TODO-105 is deployed. Add as separate TODO when ready.

---

## Related Files

- [SYNC_DEMO_RECOMMENDATIONS.md](SYNC_DEMO_RECOMMENDATIONS.md) — detailed UX spec for TODO-105
- [PRODUCT_CAPABILITIES.md](PRODUCT_CAPABILITIES.md) — product positioning
- [TODO.md](../todos/TODO.md) — TODO-105 definition (updated with `?demo` param)
