# Phase 1.05 Results: ServerCoordinator Integration

## Summary

Integrated WorkerPool, MerkleWorker, and CRDTMergeWorker into ServerCoordinator. The worker pool is opt-in (disabled by default) for backward compatibility.

## Implementation Details

### Configuration Changes

Added to `ServerCoordinatorConfig`:

```typescript
// === Worker Pool Options ===
/** Enable worker pool for CPU-bound operations (default: false) */
workerPoolEnabled?: boolean;
/** Worker pool configuration */
workerPoolConfig?: Partial<WorkerPoolConfig>;
```

### Default Configuration

| Option | Default | Description |
|--------|---------|-------------|
| `workerPoolEnabled` | `false` | Opt-in for backward compatibility |
| `minWorkers` | `2` | Minimum workers in pool |
| `maxWorkers` | `cpus - 1` | Auto-scales based on CPU count |
| `taskTimeout` | `5000ms` | Task execution timeout |
| `idleTimeout` | `30000ms` | Worker idle timeout |
| `autoRestart` | `true` | Restart crashed workers |

### Usage Example

```typescript
const server = new ServerCoordinator({
  port: 8080,
  nodeId: 'node-1',

  // Enable worker pool
  workerPoolEnabled: true,
  workerPoolConfig: {
    minWorkers: 2,
    maxWorkers: 8,
    taskTimeout: 10000,
  },
});

await server.ready();

// Check worker pool stats
if (server.workerPoolEnabled) {
  const stats = server.getWorkerPoolStats();
  console.log('Worker pool stats:', stats);
}
```

### Lifecycle Integration

1. **Initialization**: Worker pool created in constructor if enabled
2. **Ready**: Pool starts initializing workers immediately
3. **Shutdown**: Pool gracefully shuts down with 5s timeout

### New Public API

```typescript
class ServerCoordinator {
  /** Check if worker pool is enabled */
  get workerPoolEnabled(): boolean;

  /** Get worker pool statistics for monitoring */
  getWorkerPoolStats(): WorkerPoolStats | null;
}
```

### Internal Integration Points

Workers are accessible internally for future use:

```typescript
// In ServerCoordinator class (private)
private workerPool?: WorkerPool;
private merkleWorker?: MerkleWorker;
private crdtMergeWorker?: CRDTMergeWorker;
```

## Test Results

```
WorkerPool Integration with ServerCoordinator
  Configuration
    ✓ should create server without worker pool by default
    ✓ should create server with worker pool when enabled
    ✓ should use custom worker pool config
  Shutdown
    ✓ should gracefully shutdown worker pool
    ✓ should shutdown cleanly even with no worker pool

Test Suites: 1 passed, 1 total
Tests:       5 passed, 5 total
```

## Commit

```
a9bf9e2 feat(server): integrate WorkerPool into ServerCoordinator
```

## Architecture

```
ServerCoordinator
├── Constructor
│   ├── Config validation
│   ├── HLC, Storage, Security init
│   ├── Event executor init
│   ├── Backpressure regulator init
│   ├── Rate limiter init
│   └── WorkerPool init (if enabled) ← NEW
│       ├── WorkerPool
│       ├── MerkleWorker
│       └── CRDTMergeWorker
│
├── Monitoring
│   ├── getEventExecutorMetrics()
│   ├── getRateLimiterStats()
│   └── getWorkerPoolStats() ← NEW
│
└── Shutdown
    ├── Close HTTP/WS servers
    ├── Disconnect clients
    ├── Shutdown event executor
    ├── Shutdown WorkerPool ← NEW
    ├── Stop cluster
    └── Close storage
```

## Next Steps

**Phase 1.06**: Benchmarking
- Profile throughput with/without workers
- Compare sync vs async merge performance
- Tune thresholds based on real workload

**Future Phases**:
- PHASE_1_06: Actually use MerkleWorker in sync handlers
- PHASE_1_07: Use CRDTMergeWorker in batch processing
- PHASE_1_08: SerializationWorker for large payloads

## Notes

- Worker pool is **disabled by default** to ensure backward compatibility
- Worker thread tests are skipped in Jest (ts-node limitation)
- Workers use inline fallback for small batches (< 10 operations)
- Full worker thread support requires compiled .js files
