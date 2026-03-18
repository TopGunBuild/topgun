# Flamegraph Analysis

## Environment

- Date: [FILL IN]
- Hardware: [FILL IN]
- OS: [FILL IN]
- Rust version: [FILL IN]
- Commit hash: [FILL IN]

## Baselines

- Fire-and-forget: [measured] ops/sec
- Fire-and-wait: [measured] ops/sec

## Fire-and-Forget Hot Path Analysis

Use `flamegraph-fire-and-forget-server-only.svg` (filtered to `server-rt` threads) to identify
server-side hot paths separately from client-side load harness overhead.

| Rank | Function | Cumulative % | Category |
|------|----------|-------------|----------|
| 1    | [name]   | [X]%        | CPU/IO/lock/alloc |
| 2    | [name]   | [X]%        | CPU/IO/lock/alloc |
| 3    | [name]   | [X]%        | CPU/IO/lock/alloc |
| 4    | [name]   | [X]%        | CPU/IO/lock/alloc |
| 5    | [name]   | [X]%        | CPU/IO/lock/alloc |

### Observations

[Analysis of what the hot path reveals]

## Fire-and-Wait Hot Path Analysis

Use `flamegraph-fire-and-wait-server-only.svg` (filtered to `server-rt` threads).

| Rank | Function | Cumulative % | Category |
|------|----------|-------------|----------|
| 1    | [name]   | [X]%        | CPU/IO/lock/alloc |
| 2    | [name]   | [X]%        | CPU/IO/lock/alloc |
| 3    | [name]   | [X]%        | CPU/IO/lock/alloc |
| 4    | [name]   | [X]%        | CPU/IO/lock/alloc |
| 5    | [name]   | [X]%        | CPU/IO/lock/alloc |

### Observations

[Analysis of what the hot path reveals]

## Optimization Plan

| Priority | Target Function | Current % | Expected Improvement | Approach |
|----------|----------------|-----------|---------------------|----------|
| 1        | [name]         | [X]%      | [estimate]          | [brief]  |
| 2        | [name]         | [X]%      | [estimate]          | [brief]  |

## Proposed TODOs

- TODO-NNN: [optimization based on finding 1]
- TODO-NNN: [optimization based on finding 2]
