# TODO-105: Sync Showcase Demo — Design Recommendations

*Created: 2026-03-01*
*Context: Research for TODO-105 (Sync Showcase Demo App, P1)*

---

## Core Principle

The demo must show what is **impossible with competitors** (Firebase, Supabase, Convex).

All of them do realtime — that's commodity. TopGun's unique differentiators:

1. **Zero-latency local reads/writes** — data lives in memory, never waits for network
2. **Offline → reconnect → automatic CRDT merge** — no conflict-resolution code required
3. **Merkle tree delta sync** — reconnection in milliseconds, not a full refetch

The demo must hit all three points.

---

## Recommended Concept: "TopGun Sync Lab"

One application, three tabs/modes. Each tab demonstrates one superpower.

### Tab 1: "Conflict Arena" (hero screen, viral potential)

Split-screen: two "devices" editing a shared **to-do list** (simpler and more universally understood than Kanban).

**Mechanics:**
- Each device has an independent "Disconnect" / "Reconnect" button
- Both edit the same records while offline
- "Reconnect" triggers an animated merge with visual highlights:
  - Green — fields that matched (no conflict)
  - Yellow — fields where LWW resolved a conflict, showing which version won
  - Each field displays its HLC timestamp so the user sees WHY a particular version won

**Why this is viral:** Developers have suffered from sync conflicts for years. A video showing "it resolves conflicts automatically with zero code" is content that gets reposted on X/Reddit/HN because it solves a real pain point.

**Implementation notes:**
- Use a simple shared to-do list: `{ id, title, done, color }` fields per item
- Each "device" is a separate TopGun client instance with its own SyncEngine
- "Disconnect" = pause SyncEngine WebSocket, "Reconnect" = resume
- Conflict visualization: compare HLC timestamps after merge, highlight fields where remote timestamp > local pre-merge timestamp

### Tab 2: "Latency Race" (competitor killer)

Real-time benchmark visualization:

- Left: TopGun (`map.set()` → local write → instant render)
- Right: simulation of "traditional" approach (`await fetch()` → 50-200ms → render)
- User clicks a button — both perform 100 write operations
- Visualization: two progress bars, one instant, the other delayed
- Counter: "TopGun: 0.3ms avg / Traditional: 147ms avg"

**Why:** Numbers sell. A gif with two bars — one instant, one crawling — is the most repostable format for technical content.

**Implementation notes:**
- Right side is a mock that adds artificial `setTimeout` to simulate network round-trip
- Measure actual `performance.now()` around each `map.set()` call for the left side
- Show histogram of latency distribution, not just average

### Tab 3: "Network Chaos" (for hardcore audience)

One to-do list + network control panel:

- "Disconnect" / "Reconnect"
- "Slow 3G" (throttle to 2000ms latency)
- "Packet loss 50%"
- Under all conditions, UI works instantly. Sidebar shows:
  - Pending ops queue (grows while offline)
  - Merkle tree diff (how many nodes differ)
  - Sync throughput (ops/sec on reconnect)

**Why:** This turns the demo into a diagnostic tool. People evaluating infrastructure want to see behavior under degraded network, not ideal conditions.

**Implementation notes:**
- Network degradation can be simulated at the WebSocket wrapper level
- Merkle tree diff visualization: show tree nodes as colored blocks (green = matching, red = diverged)
- This tab is optional for v1 — can be added after Tabs 1 and 2

---

## Must-Have UX Elements (applies to all tabs)

These rules apply regardless of which concept is chosen:

1. **Zero friction — embedded in docs.** No `git clone` required. Demo must work directly on the homepage or docs site as an iframe. User must start interacting within the first second.

2. **"Magic Control Panel"** next to the demo:
   - Offline / Online toggle button
   - Counter: **"Read Latency: < 1ms"** (constantly flashing zero — this kills competitors)
   - Counter: **"Pending Operations: 0"** (grows when offline)

3. **Multi-tab awareness.** Show a banner: *"Open this link on your phone or in another tab"* with a QR code. The moment someone sees their action from their phone instantly appear on their laptop — that's the "Aha!" moment.

4. **Under-the-hood transparency.** Add a tab or toggle "Show State / Network" with a running log:
   ```
   [Local Write] → { id: 'todo1', title: 'Buy milk', hlc: '2026-03-01T12:00:01.000Z_0_node1' }
   [Remote Merge] → { id: 'todo1', title: 'Buy oat milk', hlc: '...002Z_0_node2' } — WINS (newer HLC)
   ```
   Developers love seeing *how* it works, not just a pretty UI.

5. **"How it's built" code snippets.** Directly below the demo, show 5 lines of code:
   *"No Redux, no `await fetch`, no WebSocket listeners. Just `map.set()` and `useQuery()`."*

---

## Rejected Alternatives (with reasoning)

| Idea | Why rejected |
|------|-------------|
| **r/place Canvas (1M pixels)** | Empty canvas without traffic = anti-advertisement. r/place works because Reddit has millions of users. Also, pixel LWW is trivial — no interesting conflicts to demonstrate. This is a WebSocket broadcast demo, not a CRDT demo. |
| **Collaborative Drum Machine** | Music synchronization is latency-sensitive, which contradicts offline-first model. An offline beat merged with an online beat = cacophony, not a demonstration. |
| **Node Editor (React Flow style)** | Shows speed but doesn't show the killer feature: conflicts and automatic resolution. Two people dragging the same node — LWW just picks the last one. Boring. |
| **Full Kanban board** | Too complex for 1-week budget. A simple to-do list demonstrates the same CRDT properties with 1/3 the UI work. Kanban adds drag-and-drop ordering complexity (needs sequence CRDTs or fractional indexing) that distracts from the core message. |

---

## Review Notes (2026-03-01)

### Tab 2 "Latency Race" — переработать подход

**Проблема:** Правая сторона — mock с искусственным `setTimeout`. Техническая аудитория (HN, Reddit) немедленно это заметит. Сравнение с фейковой задержкой — антиреклама.

**Решение:** Сравнивать TopGun **с самим собой** — online vs offline:
- Пользователь нажимает "Go Offline" → пишет 100 записей → видит тот же 0.3ms
- Сообщение: "нам не нужна сеть для записи" (доказуемо и честно)
- Показать гистограмму latency distribution, не только среднее

### Tab 3 "Network Chaos" — отложить

**Проблема:** Merkle tree visualization и "Packet loss 50%" — нишевый инструмент (~5% аудитории). Сложная реализация (WebSocket wrapper hacks) при низкой отдаче.

**Решение:** Убрать из скоупа. Вместо этого усилить Tab 1:
- Третье "устройство" — 3-way merge
- Или таймлайн: визуальная история всех операций с HLC-порядком

### Инфраструктура

Для multi-tab/multi-device демо нужен работающий TopGun-сервер. Решение: деплой на имеющийся VPS.

---

## Phasing Suggestion

| Phase | Scope | Effort |
|-------|-------|--------|
| **v1** | Tab 1 (Conflict Arena) + UX elements 1-5 + VPS deploy | ~3-4 days |
| **v2** | Tab 2 (Latency Race — online vs offline, без mock-конкурента) | ~1-2 days |
| **v3** | Polish: QR code, mobile-responsive, embed as iframe in docs | ~1 day |
| **v4** *(optional)* | Tab 1 enhancement: 3-way merge или HLC-таймлайн | ~2 days |

v1 alone is sufficient for a "hero demo" on the homepage. Tab 3 (Network Chaos) исключён из скоупа — см. Review Notes.

---

## Related Files

- [TODO-105 in TODO.md](../todos/TODO.md) — task definition
- [STRATEGIC_RECOMMENDATIONS.md](STRATEGIC_RECOMMENDATIONS.md) — Section 12.4 (Sync Showcase spec)
- [PRODUCT_CAPABILITIES.md](PRODUCT_CAPABILITIES.md) — product positioning context
