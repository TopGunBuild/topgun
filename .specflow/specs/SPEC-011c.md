# SPEC-011c: Network Module (Deferred Startup)

---
id: SPEC-011c
parent: SPEC-011
type: refactor
status: draft
priority: high
complexity: small
depends_on: [SPEC-011a]
created: 2026-01-30
---

> Part 3 of 5 from SPEC-011 (ServerFactory Modularization for Rust-Portability)

## Context

**HIGH RISK** — This sub-specification changes server startup timing.

Currently, `ServerFactory.create()` starts HTTP/WebSocket servers **BEFORE** ServerCoordinator is created (line 278). This is problematic:
- Resources are bound before the coordinator can handle requests
- On initialization failure, ports remain bound (resource leak)
- Error recovery is complex

### Current State (Lines to Extract)

```
Lines 211-280: HTTP/WebSocket servers (STARTS LISTENING at line 278!)
  - httpServer.listen() called at line 278
  - metricsServer.listen() called at line 280
```

This is the **only behavioral change** in the entire SPEC-011 series.

## Task

Extract network creation with **DEFERRED STARTUP**:
1. Create `modules/network-module.ts` with HTTP, WSS, rate limiting
2. The factory creates servers but does NOT call `.listen()`
3. Return a `start()` function that begins listening
4. ServerFactory calls `network.start()` AFTER ServerCoordinator assembly

## Requirements

### R1: Network Module Types (update `modules/types.ts`)

```typescript
export interface NetworkModuleConfig {
  port: number;
  metricsPort?: number;
  tls?: TLSConfig;
  wsBacklog?: number;
  wsCompression?: boolean;
  wsMaxPayload?: number;
  maxConnections?: number;
  serverTimeout?: number;
  keepAliveTimeout?: number;
  headersTimeout?: number;
  maxConnectionsPerSecond?: number;
  maxPendingConnections?: number;
  debugEnabled?: boolean;
  jwtSecret: string;
}

export interface NetworkModuleDeps {
  metricsService: MetricsService;
  storageManager: StorageManager;
  cluster: ClusterManager;
  partitionService: PartitionService;
  connectionManager: ConnectionManager;
}

export interface NetworkModule {
  httpServer: HttpServer | HttpsServer;
  wss: WebSocketServer;
  metricsServer: HttpServer;
  rateLimiter: ConnectionRateLimiter;
  rateLimitedLogger: RateLimitedLogger;
  start: () => void;  // DEFERRED startup - call AFTER assembly
}
```

### R2: Network Module Factory (`modules/network-module.ts`)

```typescript
export function createNetworkModule(
  config: NetworkModuleConfig,
  deps: NetworkModuleDeps
): NetworkModule {
  // Create HTTP server (NOT listening yet)
  let httpServer: HttpServer | HttpsServer;
  if (config.tls?.enabled) {
    httpServer = createHttpsServer(buildTLSOptions(config.tls), defaultHandler);
  } else {
    httpServer = createHttpServer(defaultHandler);
  }

  // Configure server limits
  httpServer.maxConnections = config.maxConnections ?? 10000;
  httpServer.timeout = config.serverTimeout ?? 120000;
  httpServer.keepAliveTimeout = config.keepAliveTimeout ?? 5000;
  httpServer.headersTimeout = config.headersTimeout ?? 60000;

  // Create WebSocket server (attached to httpServer, NOT listening)
  const wss = new WebSocketServer({
    server: httpServer,
    backlog: config.wsBacklog ?? 511,
    perMessageDeflate: config.wsCompression ?? false,
    maxPayload: config.wsMaxPayload ?? 1024 * 1024,
  });

  // Create rate limiter
  const rateLimiter = new ConnectionRateLimiter({
    maxConnectionsPerSecond: config.maxConnectionsPerSecond ?? 100,
    maxPendingConnections: config.maxPendingConnections ?? 1000,
    cooldownMs: 1000,
  });

  // Create metrics server (NOT listening)
  const metricsServer = createMetricsServer(config, deps);

  // Create rate-limited logger
  const rateLimitedLogger = new RateLimitedLogger({
    windowMs: 10000,
    maxPerWindow: 5
  });

  return {
    httpServer,
    wss,
    metricsServer,
    rateLimiter,
    rateLimitedLogger,
    // DEFERRED STARTUP - call this AFTER ServerCoordinator assembly
    start: () => {
      httpServer.listen(config.port, () => {
        logger.info({ port: config.port }, 'Server Coordinator listening');
      });
      if (config.metricsPort) {
        metricsServer.listen(config.metricsPort, () => {
          logger.info({ port: config.metricsPort }, 'Metrics server listening');
        });
      }
    }
  };
}
```

### R3: Update ServerFactory for Deferred Startup

**CRITICAL CHANGE** — Move `.listen()` calls to AFTER assembly:

```typescript
// Create network module (does NOT start listening)
const network = createNetworkModule(
  { port: config.port, metricsPort: config.metricsPort, ... },
  { metricsService: core.metricsService, storageManager: storage.storageManager, ... }
);

// ... create all other modules ...

// Assemble ServerCoordinator
const coordinator = new ServerCoordinator(config, {
  ...flattenModules({ core, network, cluster, storage, workers, handlers, search, lifecycle }),
  connectionManager,
  jwtSecret,
});

// DEFERRED STARTUP - now safe to listen
network.start();

return coordinator;
```

### R4: Error Recovery Test

Verify that if an error occurs during module creation (after network module, before coordinator):
- No ports should be bound
- All resources should be cleanable

```typescript
// Test: Error during handler creation should not leave ports bound
test('initialization failure does not bind ports', async () => {
  const config = { port: 19999, ... };

  // Mock a failure in handler creation
  jest.spyOn(OperationHandler, 'create').mockImplementation(() => {
    throw new Error('Simulated failure');
  });

  await expect(() => ServerFactory.create(config)).toThrow('Simulated failure');

  // Verify port 19999 is NOT bound
  const server = createServer();
  await new Promise((resolve, reject) => {
    server.listen(19999, resolve);
    server.on('error', reject);
  });
  server.close();
});
```

## Files

### Files to Create

| File | Purpose |
|------|---------|
| `packages/server/src/modules/network-module.ts` | HTTP, WSS, RateLimiter (deferred start) |

### Files to Modify

| File | Change |
|------|--------|
| `packages/server/src/modules/types.ts` | Add NetworkModule interface with start() |
| `packages/server/src/modules/index.ts` | Export network-module |
| `packages/server/src/ServerFactory.ts` | Move .listen() to after coordinator assembly |

## Acceptance Criteria

1. [ ] `modules/types.ts` exports NetworkModule, NetworkModuleConfig, NetworkModuleDeps interfaces
2. [ ] NetworkModule interface includes `start: () => void` method
3. [ ] `modules/network-module.ts` exports `createNetworkModule(config, deps)` function
4. [ ] `createNetworkModule()` does NOT call `.listen()` on any server
5. [ ] `network.start()` calls `.listen()` on httpServer and metricsServer
6. [ ] ServerFactory calls `network.start()` AFTER ServerCoordinator construction
7. [ ] On initialization failure before coordinator, no server ports are bound
8. [ ] TLS configuration is correctly handled (HTTP vs HTTPS)
9. [ ] WebSocket server is attached to httpServer but not listening
10. [ ] All 203+ existing tests pass
11. [ ] Build passes (`pnpm build`)
12. [ ] No circular dependencies
13. [ ] TypeScript strict mode passes

## Constraints

- **Behavioral Change**: This is the ONLY behavioral change — deferred startup
- **Test Carefully**: Error recovery must be tested
- **Port Binding**: Must verify ports are not bound on failure

## Assumptions

1. ConnectionManager is created early in ServerFactory (before network module)
2. Debug endpoints (if enabled) are created inside createNetworkModule
3. Metrics server port defaults to undefined (not started unless configured)

## Risk Mitigation

**This is the highest-risk sub-specification.** Test thoroughly:

1. **Happy path**: Normal startup works as before
2. **Error path**: Failure during init does not leave ports bound
3. **Timing**: Requests during startup are handled correctly (or rejected)
4. **TLS**: Both HTTP and HTTPS modes work
5. **Metrics**: Optional metrics server starts only when configured

---
*Created by SpecFlow split from SPEC-011 on 2026-01-30*
