# To-Do List

---

## TODO-002 — 2026-01-29
**Description:** Implement OP_ACK response for OP_BATCH in SyncProtocol
**Priority:** low
**Notes:** 2 tests in SyncProtocol.test.ts fail (OP_BATCH/OP_ACK). OP_BATCH processing works correctly, but server doesn't send ACK response. Minor issue identified during SPEC-006 test harness work.

---

## TODO-001 — 2026-01-29
**Description:** Implement proper timer/resource cleanup in server shutdown to prevent hanging tests - TimerRegistry pattern, clear pendingClusterQueries timers, proper handler stop methods
**Priority:** medium
**Notes:** Tests hang due to unclosed async handles (setTimeout in query-handler.ts:138, intervals in HeartbeatHandler/RepairScheduler). Solution: centralized TimerRegistry, explicit cleanup in shutdown(), clear all pending queries with their timers.

---
*Last updated: 2026-01-29*
