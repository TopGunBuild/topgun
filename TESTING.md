# Testing Guide and Recent Fixes

## Summary of Recent Fixes

### Issue: Test Environment Failures
The system experienced test failures related to cluster coordination and live query updates across distributed nodes.

### Root Causes and Solutions

#### 1. **PartitionService Initialization Bug**
- **Problem**: When a node had no connected peers, `getMembers()` returned an empty array, causing partition ownership to be undefined
- **Solution**: Modified `PartitionService.rebalance()` to include the local node when no other members exist
- **File**: `packages/server/src/cluster/PartitionService.ts`

#### 2. **Cluster Event Broadcasting Issue**
- **Problem**: Forwarded operations weren't broadcasting CLUSTER_EVENT messages to other nodes, preventing live query updates from propagating
- **Solution**: Modified `processLocalOp()` to always broadcast events to all other nodes, ensuring clients with active subscriptions receive updates
- **File**: `packages/server/src/ServerCoordinator.ts`

#### 3. **Test Timing Issue**
- **Problem**: Query subscriptions were not fully registered before write operations in tests
- **Solution**: Added appropriate delays after subscription registration to ensure proper initialization
- **File**: `packages/server/src/__tests__/Cluster.test.ts`

## Running Tests

### All Tests
From the project root:
```bash
npm test
```

### Server Tests Only
From the project root:
```bash
cd packages/server && npm test
```

### Specific Test Files
```bash
# Run tests matching a specific pattern
npm test -- --testNamePattern="Cluster"

# Run tests in a specific file
npm test -- --testPathPattern="SyncProtocol"

# Run tests for live queries
npm test -- --testPathPattern="LiveQuery"
```

### Run Distributed Query Test
To verify distributed query functionality:
```bash
npx ts-node examples/distributed-query-test.ts
```

### With Verbose Output
```bash
npm test -- --verbose
```

### Watch Mode
To run tests in watch mode during development:
```bash
npm test -- --watch
```

## Current Test Status
- ✅ **All 31 tests passing**
- ✅ SyncProtocol tests: Working correctly
- ✅ LiveQuery tests: Functioning properly
- ✅ Cluster tests: Fixed and operational
- ✅ QueryRegistry tests: All passing
- ✅ Matcher tests: Working as expected

## Test Coverage
The project includes the following test suites:

### Core Package (`packages/core`)
- `LWWMap.test.ts` - Tests for Last-Write-Wins Map implementation
- `MerkleTree.test.ts` - Tests for Merkle Tree synchronization

### Server Package (`packages/server`)
- `Cluster.test.ts` - Cluster formation, replication, pub/sub, and partition service tests
- `LiveQuery.test.ts` - Live query subscription and update tests
- `SyncProtocol.test.ts` - Synchronization protocol tests
- `QueryRegistry.test.ts` - Query registry management tests
- `Matcher.test.ts` - Query matcher logic tests

## Verified Functionality
The system now correctly handles:
- **Distributed Query Execution**: Scatter-gather queries across multiple nodes
- **Live Updates**: Filter queries work correctly on spectator nodes
- **Partition Rebalancing**: Proper redistribution when nodes join/leave the cluster
- **Event Propagation**: Updates reach subscribed clients on any node in the cluster
- **Idempotent Operations**: Duplicate batches are handled correctly

## Known Limitations
- **Sort/Limit Queries**: Live updates for sort/limit queries on spectator nodes have limitations since spectator nodes don't store the full dataset
- **Memory Optimization**: Spectator nodes don't store data they don't own to save memory

## Troubleshooting

### "Cannot find module '@jest/test-sequencer'" Error

This error indicates that Jest dependencies are not properly installed. To fix:

1. **Clean install dependencies from project root:**
   ```bash
   npm ci
   ```

2. **If the problem persists, try a full reinstall:**
   ```bash
   rm -rf node_modules packages/*/node_modules
   npm install
   ```

3. **Verify Jest is installed:**
   ```bash
   npm ls @jest/test-sequencer
   ```
   You should see `@jest/test-sequencer@29.7.0` in the output.

### Configuration

The project uses a workspace structure with Jest configured at both root and package levels:

- Root `package.json` contains Jest dependencies
- Each package has its own `jest.config.js` with specific settings
- The server package uses `ts-jest` for TypeScript support

### If Cluster Tests Timeout
- Ensure no other processes are using ports 10001-10002, 11001-11002, 12000-13001
- Check firewall settings allow local WebSocket connections
- Increase timeout if needed: `npm test -- --testTimeout=10000`

### Debug Output
To see detailed test output:
```bash
npm test -- --verbose
```

## CI/CD Considerations

When running tests in CI/CD pipelines:
1. Ensure sufficient timeout for cluster formation (tests include delays for node synchronization)
2. Run tests sequentially to avoid port conflicts: `npm test -- --runInBand`
3. Consider increasing Jest timeout for slower environments: `npm test -- --testTimeout=10000`

### Important Notes

- Tests use specific ports (10000+ for servers, 11000+ for cluster nodes)
- Make sure these ports are available before running tests
- Tests include timeouts for cluster stabilization - don't interrupt them early
- Console logs during tests are normal and indicate proper cluster communication