# Testing Guide

## Running Tests

### All TS Package Tests
From the project root:
```bash
pnpm test
```

This runs tests for: core, client, react, adapters, adapter-better-auth, mcp-server.

### Rust Server Tests
```bash
SDKROOT=$(/usr/bin/xcrun --sdk macosx --show-sdk-path) cargo test --release -p topgun-server
```

### Integration Tests (TS Client to Rust Server)
```bash
pnpm test:integration-rust
```

### Specific TS Package Tests
```bash
pnpm --filter @topgunbuild/core test
pnpm --filter @topgunbuild/client test
pnpm --filter @topgunbuild/react test
```

### k6 Load Tests
```bash
pnpm test:k6:smoke
pnpm test:k6:throughput
pnpm test:k6:write
pnpm test:k6:connections
```

### With Verbose Output
```bash
pnpm test -- --verbose
```

### Watch Mode
```bash
pnpm test -- --watch
```

## Test Coverage
```bash
pnpm test:coverage
pnpm --filter @topgunbuild/core test:coverage
```

## Test Suites

### Core Package (`packages/core`)
- `LWWMap.test.ts` - Last-Write-Wins Map implementation
- `MerkleTree.test.ts` - Merkle Tree synchronization
- Message schema tests, HLC tests, serialization tests

### Client Package (`packages/client`)
- `ClusterClient.integration.test.ts` - Client-cluster interactions
- SyncEngine tests, QueryHandle tests

### Server (Rust - `packages/server-rust`)
- 509+ unit tests covering all domain services
- CRDT, Sync, Query, Search, Messaging, Persistence, Coordination
- Run via `cargo test`

### Integration Tests (`tests/integration-rust`)
- 55 tests validating TS client against Rust server
- Covers CRDT operations, sync protocol, queries, search, auth

## Troubleshooting

### "Cannot find module '@jest/test-sequencer'" Error
1. Clean install: `pnpm install`
2. Full reinstall: `rm -rf node_modules packages/*/node_modules && pnpm install`

## CI/CD Considerations

1. Run TS tests sequentially to avoid port conflicts: `pnpm test -- --runInBand`
2. Rust tests: `cargo test --release` (uses parallel test harness by default)
3. Integration tests require the Rust server binary: `cargo build --release --bin test-server`
