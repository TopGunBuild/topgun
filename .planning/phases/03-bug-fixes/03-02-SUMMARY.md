---
phase: 03-bug-fixes
plan: 02
subsystem: client-sync
tags: [sync-engine, topic-queue, offline-first, reconnection]

dependency_graph:
  requires: []
  provides:
    - Topic offline queue implementation
    - Configurable queue size and eviction strategy
    - Automatic flush on authentication
  affects:
    - Any client application using topics while offline

tech_stack:
  added: []
  patterns:
    - Bounded queue with configurable eviction strategy

key_files:
  created: []
  modified:
    - packages/client/src/SyncEngine.ts
    - packages/client/src/__tests__/SyncEngine.test.ts

decisions:
  - id: "03-02-01"
    choice: "Queue topic messages instead of dropping when offline"
    reason: "Better UX - messages sent once online instead of silently lost"
  - id: "03-02-02"
    choice: "Configurable maxSize with default 100"
    reason: "Bounded memory usage while allowing reasonable offline activity"
  - id: "03-02-03"
    choice: "drop-oldest as default eviction strategy"
    reason: "Preserves most recent messages which are typically more relevant"
  - id: "03-02-04"
    choice: "Flush queue immediately on AUTH_ACK"
    reason: "Ensures queued messages are sent as soon as connection is authenticated"

metrics:
  duration: 4min
  completed: 2026-01-18
---

# Phase 03 Plan 02: Topic Offline Queue Summary

**One-liner:** Queued topic messages during offline periods with configurable bounded buffer and automatic flush on reconnect.

## What Was Built

### TopicQueueConfig Interface
- `maxSize`: Maximum queued messages (default: 100)
- `strategy`: Eviction strategy when full ('drop-oldest' | 'drop-newest', default: 'drop-oldest')

### Queue Implementation in SyncEngine
- `topicQueue`: Private array storing `QueuedTopicMessage` objects
- `queueTopicMessage()`: Queues message with timestamp, applies eviction if full
- `flushTopicQueue()`: Sends all queued messages and clears queue
- `getTopicQueueStatus()`: Returns current queue size and max size for monitoring

### Integration Points
- `publishTopic()`: Now queues when `isAuthenticated()` returns false instead of dropping
- `AUTH_ACK` handler: Calls `flushTopicQueue()` after successful authentication

## Technical Details

### Queue Entry Structure
```typescript
interface QueuedTopicMessage {
  topic: string;
  data: any;
  timestamp: number;  // When message was queued
}
```

### Eviction Behavior
- **drop-oldest**: Removes oldest message when queue is full, then adds new message
- **drop-newest**: Rejects new message when queue is full, preserves existing messages

### Flush Behavior
- Iterates through queue and sends each message as TOPIC_PUB
- Clears queue after all messages sent
- Logs count of flushed messages at INFO level

## Verification

- [x] All 46 SyncEngine tests pass
- [x] TypeScript compiles without errors
- [x] Topic queue tests verify:
  - Queueing when offline
  - drop-oldest eviction respects maxSize
  - drop-newest eviction drops new messages
  - Default config returns maxSize 100
  - AUTH_ACK flushes queue and sends messages

## Deviations from Plan

None - plan executed exactly as written.

## Files Changed

| File | Change |
|------|--------|
| `packages/client/src/SyncEngine.ts` | +77 lines: TopicQueueConfig interface, queue fields, queueing/flushing methods |
| `packages/client/src/__tests__/SyncEngine.test.ts` | +99 lines: 5 new tests for topic offline queue |

## Next Phase Readiness

- [x] BUG-06 (Topic messages lost when offline) is now fixed
- [x] Ready for BUG-07 (Heartbeat not starting after reconnection) if that's in the phase
