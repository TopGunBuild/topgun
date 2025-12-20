# TopGun Native Benchmark Harness

Accurate performance benchmarking using native MessagePack protocol.

## Quick Start

```bash
# Start server
pnpm start:server

# Quick smoke test (10 sec)
pnpm benchmark:smoke

# Full throughput test (60 sec)
pnpm benchmark:throughput
```

## Scenarios

| Scenario | Duration | Connections | Purpose |
|----------|----------|-------------|---------|
| smoke | 10 sec | 10 | CI sanity check |
| throughput | 60 sec | 100 | Max ops/sec |
| latency | 30 sec | 20 | Latency percentiles |
| stress | until fail | increasing | Breaking point |

## Architecture

```
tests/benchmark/
├── config.ts        # Scenario configurations
├── harness.ts       # Main benchmark runner
├── index.ts         # CLI entry point
├── metrics.ts       # HDR histogram wrapper
├── reporter.ts      # Console output + JSON export
├── types.ts         # TypeScript interfaces
└── scenarios/
    ├── smoke.ts
    ├── throughput.ts
    ├── latency.ts
    └── stress.ts
```

## Results

JSON output: `tests/benchmark/results/`

Each benchmark run produces a timestamped JSON file with:
- ops/sec (operations per second)
- Latency percentiles (p50, p95, p99, p99.9)
- Error counts
- Connection statistics

## CI Integration

- `benchmark:smoke` runs on every PR
- `benchmark:throughput` runs on push to main
- Results uploaded as GitHub Actions artifacts

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | All passed |
| 1 | Warning (errors < 1%) |
| 2 | Failed (errors >= 1%) |
| 3 | Critical (errors >= 5%) |
| 4 | Fatal (benchmark crash) |

## Environment Variables

- `JWT_TOKEN` - Authentication token for WebSocket connections
- `BENCHMARK_URL` - Server URL (default: ws://localhost:8080)
