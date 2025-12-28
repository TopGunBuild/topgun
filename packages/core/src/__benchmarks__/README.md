# TopGun Query Engine Benchmarks

This directory contains comprehensive benchmarks for the Phase 7 Query Engine implementation.

## Overview

The benchmark suite measures:
- **Index performance** (HashIndex, NavigableIndex)
- **Query execution** (indexed vs full-scan comparison)
- **Live queries** (StandingQueryIndex performance)
- **Memory overhead** (index memory consumption)

## Directory Structure

```
__benchmarks__/
├── indexes/
│   ├── HashIndex.bench.ts          # O(1) equality lookups
│   └── NavigableIndex.bench.ts     # O(log N) range queries
├── query/
│   ├── IndexedQuery.bench.ts       # Indexed vs full-scan comparison
│   └── StandingQuery.bench.ts      # Live query performance
├── memory/
│   └── IndexMemory.bench.ts        # Memory overhead measurement
└── README.md                        # This file
```

## Running Benchmarks

### Run All Benchmarks

```bash
# From project root
pnpm --filter @topgunbuild/core bench

# Or from packages/core
cd packages/core
pnpm bench
```

### Run Specific Benchmarks

```bash
# HashIndex benchmarks only
pnpm --filter @topgunbuild/core bench -- HashIndex

# NavigableIndex benchmarks only
pnpm --filter @topgunbuild/core bench -- NavigableIndex

# IndexedQuery benchmarks only
pnpm --filter @topgunbuild/core bench -- IndexedQuery

# StandingQuery benchmarks only
pnpm --filter @topgunbuild/core bench -- StandingQuery
```

### Memory Benchmarks

Memory benchmarks require the `--expose-gc` flag for accurate measurements:

```bash
# Run memory benchmarks
node --expose-gc node_modules/.bin/vitest bench -- IndexMemory

# Or run the script directly
node --expose-gc src/__benchmarks__/memory/IndexMemory.bench.ts
```

## Benchmark Categories

### 1. HashIndex Benchmarks

**File:** `indexes/HashIndex.bench.ts`

**Tests:**
- Add operations
- Retrieve equal (existing/non-existing)
- Retrieve in (multiple values)
- Retrieve has (all keys)
- Update operations
- Remove operations
- Hash collision handling

**Scales:** 1K, 10K, 100K, 1M records

### 2. NavigableIndex Benchmarks

**File:** `indexes/NavigableIndex.bench.ts`

**Tests:**
- Add operations
- Retrieve equal
- Range queries (gt, gte, lt, lte, between)
- Different selectivity levels (50%, 10%, 1%, 0.1%)
- String vs numeric attributes
- Collision handling

**Scales:** 1K, 10K, 100K, 1M records

### 3. IndexedQuery Benchmarks

**File:** `query/IndexedQuery.bench.ts`

**Tests:**
- Equality queries (indexed vs full-scan)
- Range queries (indexed vs full-scan)
- Compound queries (AND/OR)
- Complex nested queries
- Non-indexed field queries
- Count operations
- Selectivity impact

**Scales:** 10K, 100K, 1M records

**Key Comparisons:**
- `[INDEXED]` - Using indexes
- `[FULL SCAN]` - Without indexes (baseline)

### 4. StandingQuery Benchmarks

**File:** `query/StandingQuery.bench.ts`

**Tests:**
- Standing query vs regular query
- Multiple live queries
- Callback overhead (1, 10, 100 callbacks)
- Update performance with live queries
- Scaling across different dataset sizes

**Scales:** 1K, 10K, 100K records

### 5. Memory Benchmarks

**File:** `memory/IndexMemory.bench.ts`

**Tests:**
- Full index suite memory overhead
- Index type comparison (Hash vs Navigable)
- Per-record overhead calculation
- Data size impact
- Scaling analysis

**Output:** Console report with formatted memory statistics

## Expected Results

Based on CQEngine benchmarks and Phase 7 targets:

| Metric | Expected Performance |
|--------|---------------------|
| HashIndex equal | < 1 μs (O(1)) |
| NavigableIndex range | < 10 μs + iteration (O(log N + K)) |
| Full scan 1M records | ~50-100 ms (O(N)) |
| **Improvement** | **100-1000× for indexed queries** |
| Memory overhead | 20-30% |

## Performance Targets

| Metric | Target | Phase 7 Goal |
|--------|--------|--------------|
| Equality query (1M) | < 1ms | ✓ Sub-millisecond |
| Range query (1M) | < 5ms | ✓ Sub-5ms |
| Memory overhead | < 30% | ✓ Acceptable |
| Query parity | 100% | ✓ Full compatibility |

## Interpreting Results

### Operations/Second (ops/sec)

Vitest bench reports operations per second. Higher is better.

```
✓ HashIndex Performance > 1,000,000 records > retrieve equal (existing)
  1,234,567 ops/sec ±0.42%

This means ~0.8 microseconds per operation (1/1,234,567)
```

### Comparison Metrics

Look for the speedup ratio between `[INDEXED]` and `[FULL SCAN]`:

```
[INDEXED] equality query: 1,000,000 ops/sec (~1 μs)
[FULL SCAN] equality query: 20 ops/sec (~50 ms)
Speedup: 50,000×
```

### Memory Overhead

Memory benchmarks output formatted reports:

```
100,000 records:
  Base memory:    15.42 MB
  Indexed memory: 19.87 MB
  Overhead:       4.45 MB (+28.9%)
  Per-record overhead: 46.5 bytes
```

## Troubleshooting

### Benchmarks Running Slow

Large dataset benchmarks (1M records) can take several minutes. Consider:

```bash
# Run only smaller scales for quick feedback
pnpm bench -- "HashIndex.*1,000 records"

# Skip 1M benchmarks
pnpm bench -- --exclude "1,000,000"
```

### Memory Benchmarks Inaccurate

Ensure `--expose-gc` flag is used:

```bash
# Without flag
❌ node vitest bench -- IndexMemory

# With flag (correct)
✅ node --expose-gc node_modules/.bin/vitest bench -- IndexMemory
```

### Out of Memory Errors

Reduce benchmark scales in the test files or increase Node.js memory:

```bash
node --max-old-space-size=8192 --expose-gc node_modules/.bin/vitest bench
```

## Adding New Benchmarks

### Template

```typescript
import { bench, describe } from 'vitest';
import { IndexedLWWMap } from '../../IndexedLWWMap';
import { HLC } from '../../HLC';
import { simpleAttribute } from '../../query/Attribute';

describe('My Benchmark', () => {
  const sizes = [1_000, 10_000];

  for (const size of sizes) {
    describe(`${size.toLocaleString()} records`, () => {
      // Setup
      const hlc = new HLC('bench');
      const map = new IndexedLWWMap<string, any>(hlc);

      // Populate data
      for (let i = 0; i < size; i++) {
        map.set(`${i}`, { id: i, value: i });
      }

      bench('my operation', () => {
        // Benchmark code
      });
    });
  }
});
```

## Reporting Results

After running benchmarks, document results in:

```
/Users/koristuvac/Downloads/topgun/PHASE_7_BENCHMARK_REPORT.md
```

Use the provided template to fill in:
- Performance metrics
- Comparison tables
- Analysis and conclusions

## CI/CD Integration

To run benchmarks in CI:

```yaml
# .github/workflows/benchmark.yml
- name: Run benchmarks
  run: pnpm --filter @topgunbuild/core bench

- name: Run memory benchmarks
  run: node --expose-gc node_modules/.bin/vitest bench -- IndexMemory
```

Consider using:
- [hyperfine](https://github.com/sharkdp/hyperfine) for more detailed timing
- [benchmark.js](https://benchmarkjs.com/) for statistical analysis
- Continuous benchmarking services (e.g., Bencher.dev)

## References

- [CQEngine Benchmarks](https://github.com/npgall/cqengine#benchmark)
- [Vitest Benchmark API](https://vitest.dev/guide/features.html#benchmarking)
- [Phase 7 Specification](../../../../../../PROMPTS/PHASE_7_QUERY_ENGINE_SPEC.md)

---

**Last Updated:** 2025-12-28
**Phase:** 7.08 (Query Engine Benchmarks)
