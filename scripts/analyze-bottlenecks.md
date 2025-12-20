# TopGun Server Performance Analysis Guide

## Current Performance Baseline

- **Throughput**: ~16,000 ops/sec (single node)
- **Latency**: p99 ~12ms
- **Tested with**: Native Benchmark Harness

## Quick Start Profiling

```bash
# 1. Start profiling server
./scripts/profile-server.sh flame

# 2. In another terminal, run benchmark
JWT_TOKEN=your_token pnpm benchmark:throughput

# 3. Stop server (Ctrl+C) - flame graph opens automatically
```

## Profiling Modes

| Mode | Use Case | Output |
|------|----------|--------|
| `cpu` | General CPU analysis | Text report |
| `flame` | Hot path visualization | Interactive HTML |
| `bubble` | Async operation analysis | Interactive HTML |
| `doctor` | Overall health check | Recommendations |
| `inspect` | Live debugging | Chrome DevTools |
| `heap` | Memory analysis | Heap snapshots |

## Potential Bottlenecks to Investigate

### 1. WebSocket Library (`ws`)

**Check**: Look for `ws` or `WebSocket` in flame graph
**Alternative**: µWebSockets.js (10-100x faster)

```bash
# Compare ws vs uWebSockets
pnpm add uWebSockets.js
```

### 2. MessagePack Serialization

**Check**: Look for `serialize`/`deserialize` or `msgpack` in profile
**Alternative**: `msgpackr` (faster than `@msgpack/msgpack`)

```bash
# Benchmark current serialization
node -e "
const { serialize, deserialize } = require('./packages/core/dist');
const data = { type: 'OP_BATCH', payload: { ops: [...Array(100)].map((_, i) => ({ id: i })) } };

console.time('serialize 10000x');
for (let i = 0; i < 10000; i++) serialize(data);
console.timeEnd('serialize 10000x');
"
```

### 3. CRDT Operations

**Check**: Look for `merge`, `ORMap`, `LWWMap` in profile
**Potential**: Batch processing, reduce allocations

### 4. Storage I/O

**Check**: Look for `write`, `read`, `storage` operations
**Potential**: Batch writes, async flush, write-behind cache

### 5. Event Loop Blocking

**Check**: Use `clinic doctor` - shows event loop delays
**Potential**: Move heavy work to worker threads

## Optimization Roadmap

### Phase 1: Measure (Current)
- [x] Establish baseline with Native Harness
- [ ] Run flame graph profiling
- [ ] Identify top 3 hotspots

### Phase 2: Quick Wins
- [ ] Optimize hottest code paths
- [ ] Consider `msgpackr` if serialization is hot
- [ ] Batch database writes if storage is hot

### Phase 3: Architecture Changes (if needed)
- [ ] Evaluate µWebSockets.js migration
- [ ] Consider worker threads for CPU-bound work
- [ ] Implement connection pooling improvements

## Commands Reference

```bash
# Full profiling session
./scripts/profile-server.sh flame

# CPU text profile
./scripts/profile-server.sh cpu

# Chrome DevTools
./scripts/profile-server.sh inspect

# Memory analysis
./scripts/profile-server.sh heap
```

## Expected Results After Optimization

| Metric | Current | Target |
|--------|---------|--------|
| Throughput | 16K ops/sec | 50K+ ops/sec |
| p99 Latency | 12ms | <5ms |
| Memory | TBD | Stable |
