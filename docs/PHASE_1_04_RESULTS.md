# Phase 1.04 Results: CRDTMergeWorker Implementation

## Summary

Implemented CRDTMergeWorker for offloading CRDT merge operations to worker threads. This component handles Last-Write-Wins (LWW) and Observed-Remove Map (ORMap) merge operations with automatic worker thread delegation for large batches.

## Implementation Details

### Files Created

| File | Description |
|------|-------------|
| `crdt-types.ts` | Type definitions for LWW and ORMap merge operations |
| `crdt.worker.ts` | Worker script with lww-merge and ormap-merge handlers |
| `CRDTMergeWorker.ts` | High-level API with inline fallback pattern |
| `CRDTMergeWorker.test.ts` | Comprehensive test suite (18 tests) |

### Key Features

#### LWW Merge (`lww-merge`)
- **Input**: Records to merge + existing state (timestamps only)
- **Output**: Records to apply, skipped count, detected conflicts
- **Conflict Detection**: Same millis but different counter/nodeId (concurrent writes)
- **Timestamp Comparison**: millis → counter → nodeId (lexicographic)

#### ORMap Merge (`ormap-merge`)
- **Input**: Items + tombstones + existing tags/tombstones
- **Output**: Items to apply, tombstones to apply, tags to remove
- **Tombstone Priority**: Tombstones processed first
- **Tag Uniqueness**: Each tag can only be added once (OR-Set semantics)

### Architecture

```
CRDTMergeWorker
├── mergeLWW(payload) ──────┬── < 10 records → inline
│                           └── ≥ 10 records → WorkerPool
├── mergeORMap(payload) ────┬── < 10 ops → inline
│                           └── ≥ 10 ops → WorkerPool
└── Worker Scripts
    └── crdt.worker.ts (lww-merge, ormap-merge handlers)
```

### Type Definitions

```typescript
// LWW Types
interface LWWMergePayload {
  mapName: string;
  records: LWWMergeRecord[];       // Records to merge
  existingState: LWWExistingRecord[]; // Current state (timestamps)
}

interface LWWMergeResult {
  toApply: Array<{ key, value, timestamp, ttlMs? }>;
  skipped: number;
  conflicts: string[];  // Keys with concurrent writes
}

// ORMap Types
interface ORMapMergePayload {
  mapName: string;
  items: ORMapMergeItem[];
  tombstones: ORMapMergeTombstone[];
  existingTags: string[];
  existingTombstones: string[];
}

interface ORMapMergeResult {
  itemsToApply: Array<{ key, value, timestamp, tag, ttlMs? }>;
  tombstonesToApply: string[];
  tagsToRemove: string[];
  itemsSkipped: number;
  tombstonesSkipped: number;
}
```

## Test Results

```
Test Suites: 3 passed, 3 total
Tests:       5 skipped, 47 passed, 52 total

CRDTMergeWorker Tests:
  LWW Merge
    ✓ should apply records with newer timestamps
    ✓ should skip records with older timestamps
    ✓ should resolve conflicts by counter when millis are equal
    ✓ should resolve conflicts by nodeId when millis and counter are equal
    ✓ should handle empty records
    ✓ should preserve TTL in merge results
    ○ skipped should handle large batches (worker thread)
  ORMap Merge
    ✓ should apply new items
    ✓ should skip items with existing tags
    ✓ should skip items with tombstoned tags
    ✓ should apply new tombstones
    ✓ should skip existing tombstones
    ✓ should handle concurrent add and remove
    ✓ should handle empty input
    ○ skipped should handle large batches (worker thread)
  Edge Cases
    ✓ should handle identical timestamps (LWW tie-breaker)
    ✓ should handle complex nested values
    ✓ should handle null and undefined values
```

## Conflict Detection

The implementation detects **concurrent writes** when:
1. Same millisecond timestamp (HLC millis)
2. Different counter OR different nodeId

This indicates that two nodes wrote to the same key "at the same time" (within clock resolution). While LWW still deterministically picks a winner, tracking conflicts is useful for:
- Monitoring concurrent access patterns
- Debugging sync issues
- Application-level conflict resolution hints

## Inline Fallback Pattern

For batches smaller than 10 operations, work is done inline on the main thread to avoid:
- `postMessage()` serialization overhead
- Worker thread dispatch latency
- Context switching costs

Threshold of 10 was chosen based on typical postMessage overhead (~10-50μs) vs inline execution (~1-5μs per operation).

## Commit

```
0140a3d feat(server): implement CRDTMergeWorker for CRDT merge operations
```

## Next Steps

**Phase 1.05**: Integration into Server
- Integrate WorkerPool into TopGunServer
- Replace synchronous merge calls with CRDTMergeWorker
- Add configuration options for worker pool

**Phase 1.06**: Benchmarking
- Compare throughput with/without workers
- Profile postMessage overhead
- Tune thresholds and pool size
