# Phase 1.06 Results: Performance Benchmarking

## Summary

Implemented benchmark tests for measuring worker operation performance. Benchmarks measure inline execution (batches < 10 items) since worker thread tests are skipped due to ts-node limitations.

## Benchmark Results

### Throughput Summary (batch size = 5)

| Operation | Ops/sec | Items/sec | Avg Latency |
|-----------|---------|-----------|-------------|
| MerkleHash | ~49,000 | ~245,000 | 20 μs |
| LWWMerge | ~1.8M | ~9.2M | 1 μs |
| ORMapMerge | ~1.5M | ~7.6M | 1 μs |

### MerkleHash Scaling

| Entries | Ops/sec | Items/sec |
|---------|---------|-----------|
| 1 | 137,359 | 137,359 |
| 3 | 48,918 | 146,754 |
| 5 | 44,377 | 221,885 |
| 7 | 32,395 | 226,763 |
| 9 | 27,132 | 244,191 |

**Observations:**
- MerkleHash has O(n) complexity due to tree building
- Items/sec increases with batch size (amortized overhead)
- Time per item: ~4-8 μs

### LWW Merge Scaling

| Records | Ops/sec | Items/sec |
|---------|---------|-----------|
| 1 | 1,141,011 | 1,141,011 |
| 3 | 982,880 | 2,948,641 |
| 5 | 406,600 | 2,033,001 |
| 7 | 547,771 | 3,834,394 |
| 9 | 465,243 | 4,187,184 |

**Observations:**
- LWW merge is extremely fast (~1μs per operation)
- Pure timestamp comparison with Map lookup
- No tree building overhead

### ORMap Merge Scaling

| Items | Ops/sec | Items/sec |
|-------|---------|-----------|
| 1 | 1,100,212 | 1,100,212 |
| 3 | 890,935 | 2,672,806 |
| 5 | 796,337 | 3,981,684 |
| 7 | 618,238 | 4,327,666 |
| 9 | 564,627 | 5,081,639 |

**Observations:**
- ORMap merge is also very fast
- Set operations with tag lookups
- Linear scaling with items

## Analysis

### When Workers Provide Benefit

Based on benchmarks:

1. **MerkleHash**: Worker beneficial at 50+ entries
   - Inline: ~20μs for 5 entries = 4μs/entry
   - postMessage overhead: ~10-50μs
   - Break-even: ~12-50 entries

2. **LWW/ORMap Merge**: Worker beneficial at 100+ records
   - Inline: ~1μs per record
   - postMessage overhead dominates for small batches
   - Break-even: ~50-100 records

### Current Threshold (10) Analysis

The current threshold of 10 is **conservative** for:
- **LWW/ORMap Merge**: Could be higher (50+)
- **MerkleHash**: Appropriate for light workloads

### Recommendations

1. **Keep threshold at 10** for MerkleHash operations
2. **Consider raising to 50** for CRDT merge operations
3. **Profile with worker threads** after compilation for accurate crossover point

## Test Coverage

```
Worker Performance Benchmarks
  MerkleWorker Benchmarks (Inline)
    ✓ should benchmark computeHashes at inline batch sizes
    ✓ should benchmark diff operation (inline)
    ✓ should benchmark rebuild operation (inline)
  CRDTMergeWorker Benchmarks (Inline)
    ✓ should benchmark LWW merge at inline batch sizes
    ✓ should benchmark ORMap merge at inline batch sizes
  Scaling Analysis (Inline)
    ✓ should show linear scaling for inline operations
    ✓ should show LWW merge scaling
  Throughput Summary
    ✓ should summarize overall throughput metrics

Tests: 8 passed
```

## Commit

```
1f92834 test(server): add worker benchmark tests
```

## Limitations

1. **Worker thread tests skipped**: Jest with ts-node cannot load .ts files in worker_threads
2. **Inline-only benchmarks**: Only measures < 10 item batches
3. **No concurrent benchmarks**: Would require compiled .js files

## Future Work

1. Create compiled benchmark runner for full worker thread testing
2. Profile real-world workloads in production
3. Add metrics to ServerCoordinator for runtime threshold tuning
4. Consider adaptive thresholds based on load
