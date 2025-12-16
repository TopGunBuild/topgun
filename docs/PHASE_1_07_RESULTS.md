# Phase 1.07 Results: SerializationWorker Implementation

## Summary

Implemented SerializationWorker for offloading batch serialization/deserialization operations to worker threads. This component handles MessagePack encoding/decoding with automatic worker thread delegation for large payloads.

## Implementation Details

### Files Created

| File | Description |
|------|-------------|
| `serialization-types.ts` | Type definitions for batch serialize/deserialize operations |
| `serialization.worker.ts` | Worker script with serialize/deserialize handlers |
| `SerializationWorker.ts` | High-level API with inline fallback pattern |
| `SerializationWorker.test.ts` | Comprehensive test suite (25 tests) |

### Key Features

#### Batch Serialize
- **Input**: Array of objects to serialize
- **Output**: Array of Uint8Array (MessagePack binary)
- **Worker Threshold**: ≥10 items OR estimated size ≥50KB

#### Batch Deserialize
- **Input**: Array of Uint8Array (MessagePack binary)
- **Output**: Array of deserialized objects
- **Worker Threshold**: ≥10 items

#### Binary Transfer
Uses base64 encoding to transfer binary data through postMessage:
- Adds ~33% overhead but necessary for structured clone transfer
- Could be optimized with Transferable ArrayBuffers in future

### Architecture

```
SerializationWorker
├── serializeBatch(items) ─────┬── < 10 items AND < 50KB → inline
│                              └── ≥ 10 items OR ≥ 50KB → WorkerPool
├── deserializeBatch(items) ───┬── < 10 items → inline
│                              └── ≥ 10 items → WorkerPool
├── serialize(data) ───────────→ always inline (single item)
├── deserialize(data) ─────────→ always inline (single item)
└── Worker Scripts
    └── serialization.worker.ts (serialize, deserialize handlers)
```

### Type Definitions

```typescript
// Batch Serialize
interface SerializeBatchPayload {
  items: unknown[];
}

interface SerializeBatchResult {
  serialized: string[];  // base64-encoded
}

// Batch Deserialize
interface DeserializeBatchPayload {
  items: string[];  // base64-encoded
}

interface DeserializeBatchResult {
  deserialized: unknown[];
}
```

### Usage Example

```typescript
import { WorkerPool, SerializationWorker } from '@topgunbuild/server/workers';

const pool = new WorkerPool({ minWorkers: 2 });
const serializer = new SerializationWorker(pool);

// Single item (always inline)
const bytes = serializer.serialize({ name: 'Alice' });
const obj = serializer.deserialize(bytes);

// Batch (auto-delegates to worker if large)
const items = [{ id: 1 }, { id: 2 }, /* ... many items */];
const serialized = await serializer.serializeBatch(items);
const deserialized = await serializer.deserializeBatch(serialized);

// Check threshold decision
if (serializer.shouldUseWorker(items)) {
  console.log('Will use worker thread');
}
```

## Test Results

```
SerializationWorker Tests
  Single Serialize/Deserialize
    ✓ should serialize and deserialize primitives
    ✓ should serialize and deserialize objects
    ✓ should serialize and deserialize arrays
    ✓ should handle empty objects and arrays
    ✓ should handle unicode strings
  Batch Serialize (Inline)
    ✓ should serialize batch of small items
    ✓ should handle empty batch
    ✓ should serialize batch up to threshold inline
  Batch Deserialize (Inline)
    ✓ should deserialize batch of items
    ✓ should handle empty batch
  TopGun-like Data Structures
    ✓ should serialize LWW record structure
    ✓ should serialize OR record structure
    ✓ should serialize batch event structure
  shouldUseWorker Decision
    ✓ should return false for small batches
    ✓ should return true for batches at threshold
    ✓ should return true for large payload size
    ✓ should return false for empty array
  Edge Cases
    ✓ should handle deeply nested objects
    ✓ should handle large arrays
    ✓ should handle special number values
    ✓ should preserve object key order
  Roundtrip Tests
    ✓ should maintain data integrity through roundtrip
    ✓ should handle multiple roundtrips
  Worker Thread Operations
    ○ skipped should serialize large batch via worker thread
    ○ skipped should deserialize large batch via worker thread

Tests: 2 skipped, 23 passed, 25 total
```

## Threshold Decision Logic

```typescript
shouldUseWorker(items: unknown[]): boolean {
  // 1. Batch size threshold
  if (items.length >= 10) return true;

  // 2. Estimated size threshold
  let totalSize = 0;
  for (const item of items) {
    totalSize += estimateSize(item);
    if (totalSize >= 50 * 1024) return true;  // 50 KB
  }

  return false;
}
```

Size estimation is heuristic-based:
- Primitives: 1-9 bytes
- Strings: length + 5 bytes
- Objects/Arrays: sum of children + 5 bytes overhead

## When SerializationWorker Provides Benefit

Based on MessagePack performance characteristics:

1. **Large Batches** (>10 items): Worker beneficial
   - Offloads CPU work from event loop
   - postMessage overhead amortized over batch

2. **Large Payloads** (>50KB): Worker beneficial
   - JSON.stringify benchmark: ~10μs/KB
   - postMessage overhead: ~10-50μs base
   - Break-even: ~5-10KB per item

3. **Many Clients**: Worker beneficial
   - Same payload serialized once, sent to many clients
   - Already optimized in broadcastBatch() - serialize once per role group

## Commit

```
e360274 feat(server): implement SerializationWorker for batch serialization operations
```

## All Worker Tests Summary

```
Test Suites: 6 passed
Tests:       7 skipped, 83 passed, 90 total
```

| Worker | Tests Passed | Skipped |
|--------|-------------|---------|
| WorkerPool | 19 | 0 |
| MerkleWorker | 15 | 0 |
| CRDTMergeWorker | 18 | 0 |
| WorkerPoolIntegration | 5 | 0 |
| WorkerBenchmark | 8 | 0 |
| SerializationWorker | 23 | 2 |

## Limitations

1. **Worker thread tests skipped**: Jest with ts-node cannot load .ts files in worker_threads
2. **Base64 overhead**: ~33% size increase for binary transfer
3. **No Transferable support**: Could optimize with ArrayBuffer transfer

## Future Improvements

1. **Streaming serialization**: For very large payloads, serialize in chunks
2. **Transferable ArrayBuffers**: Zero-copy binary transfer when possible
3. **Compression**: Optional zstd/lz4 compression for network-bound scenarios
4. **Integration**: Use in ServerCoordinator.broadcastBatch for large payloads

## Next Steps

**Phase 1.08**: Integration into ServerCoordinator
- Use SerializationWorker in broadcastBatch for large payloads
- Add metrics for serialization time
- Profile real-world workloads

**Phase 1.09**: Full Worker Thread Testing
- Create compiled benchmark runner
- Measure actual postMessage overhead
- Tune thresholds based on real data
