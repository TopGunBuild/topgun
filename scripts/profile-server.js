#!/usr/bin/env node
/**
 * Profile TopGun Server with Node.js built-in profiler
 *
 * Usage:
 *   node --prof scripts/profile-server.js
 *   # Run k6 tests
 *   # Stop server with Ctrl+C
 *   node --prof-process isolate-*.log > profile.txt
 *
 * Or with 0x (flamegraph):
 *   npx 0x scripts/profile-server.js
 */

const { ServerCoordinator, MemoryServerAdapter } = require('../packages/server/dist');

console.log('Starting TopGun Server with profiling...');
console.log('Press Ctrl+C to stop and generate profile data');
console.log('');

const adapter = new MemoryServerAdapter();

const server = new ServerCoordinator({
  port: parseInt(process.env.PORT || '8080'),
  clusterPort: parseInt(process.env.CLUSTER_PORT || '9080'),
  metricsPort: parseInt(process.env.METRICS_PORT || '9091'),
  nodeId: process.env.NODE_ID || 'profile-node-1',
  storage: adapter,
  jwtSecret: 'topgun-secret-dev',
  // Disable worker pool to profile main thread
  workerPoolEnabled: false,
});

console.log('TopGun Server running on ws://localhost:8080');
console.log('');
console.log('Run load test:');
console.log('  pnpm test:k6:write');
console.log('');

// Graceful Shutdown
const shutdown = async (signal) => {
  console.log(`\nReceived ${signal}. Stopping server...`);

  const forceKillTimer = setTimeout(() => {
    console.error('Shutdown timed out. Forcing exit.');
    process.exit(1);
  }, 10000);

  try {
    await server.shutdown();
    console.log('Server stopped.');
    console.log('');
    console.log('To analyze profile:');
    console.log('  node --prof-process isolate-*.log > profile.txt');
    console.log('  cat profile.txt | head -100');
    clearTimeout(forceKillTimer);
    process.exit(0);
  } catch (err) {
    console.error('Error during shutdown:', err);
    clearTimeout(forceKillTimer);
    process.exit(1);
  }
};

process.on('SIGTERM', () => shutdown('SIGTERM'));
process.on('SIGINT', () => shutdown('SIGINT'));
