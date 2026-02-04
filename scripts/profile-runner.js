/**
 * Profile Runner - запускает сервер для профилирования
 * Использование: node --prof scripts/profile-runner.js
 */

const path = require('path');

// Устанавливаем пути для резолва модулей
const serverPath = path.join(__dirname, '../packages/server/dist/index.js');

// Импортируем сервер
const { ServerFactory, MemoryServerAdapter } = require(serverPath);

console.log('Starting TopGun server for profiling...');

const adapter = new MemoryServerAdapter();

const server = ServerFactory.create({
  port: parseInt(process.env.PORT || '8080'),
  clusterPort: parseInt(process.env.CLUSTER_PORT || '9080'),
  metricsPort: parseInt(process.env.METRICS_PORT || '9091'),
  storage: adapter,
  jwtSecret: process.env.JWT_SECRET || 'benchmark-secret-key-for-testing',
  heartbeatConfig: {
    interval: 30000,
    timeout: 90000,
  },
});

server.ready().then(() => {
  console.log('TopGun server ready for profiling on ws://localhost:8080');
  console.log('Run benchmark, then Ctrl+C to stop and generate profile');
});

// Graceful shutdown
process.on('SIGINT', () => {
  console.log('\nShutting down...');
  server.shutdown().then(() => {
    console.log('Server stopped');
    process.exit(0);
  });
});

process.on('SIGTERM', () => {
  server.shutdown().then(() => process.exit(0));
});
