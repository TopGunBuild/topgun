/**
 * Write-Heavy Scenario for TopGun Server
 *
 * Intensive write load test to measure PUT operation throughput.
 * - 100 VUs for 5 minutes
 * - Each VU performs ~10 PUT operations per second
 * - Tests server's write throughput and latency under load
 *
 * Run:
 *   k6 run tests/k6/scenarios/write-heavy.js -e JWT_TOKEN=<token>
 *
 * Debug mode:
 *   k6 run tests/k6/scenarios/write-heavy.js -e JWT_TOKEN=<token> --vus 10 --duration 30s
 */

import ws from 'k6/ws';
import { check, sleep } from 'k6';
import { Counter, Trend, Rate } from 'k6/metrics';
import {
  TopGunClient,
  createMessageHandler,
  connectionTime,
  authTime,
  authSuccess,
  errors,
} from '../lib/topgun-client.js';

// Test configuration
export const options = {
  vus: 100,
  duration: '5m',
  thresholds: {
    // p99 write latency < 100ms
    write_latency: ['p(99)<100'],
    // Error rate < 1%
    write_error_rate: ['rate<0.01'],
    // Ops per second > 1000
    write_ops_total: ['count>300000'], // 100 VUs * 10 ops/s * 300s = 300,000
    // Auth should succeed
    topgun_auth_success: ['rate>0.99'],
  },
};

// Custom metrics for this scenario
const writeLatency = new Trend('write_latency', true);
const writeOpsTotal = new Counter('write_ops_total');
const writeOpsAcked = new Counter('write_ops_acked');
const writeErrorRate = new Rate('write_error_rate');
const opsPerSecond = new Trend('ops_per_second', true);

// Configuration from environment
const WS_URL = __ENV.WS_URL || 'ws://localhost:8080';
const JWT_TOKEN = __ENV.JWT_TOKEN || null;
const OPS_PER_SECOND = parseInt(__ENV.OPS_PER_SECOND || '10');
const BATCH_SIZE = parseInt(__ENV.BATCH_SIZE || '5'); // Operations per batch

/**
 * Generate auth token for VU
 */
function getAuthToken(vuId) {
  if (JWT_TOKEN) {
    return JWT_TOKEN;
  }

  const header = JSON.stringify({ alg: 'HS256', typ: 'JWT' });
  const payload = JSON.stringify({
    userId: `k6-writer-${vuId}`,
    roles: ['USER', 'ADMIN'],
    sub: `k6-writer-${vuId}`,
    iat: Math.floor(Date.now() / 1000),
    exp: Math.floor(Date.now() / 1000) + 3600,
  });

  const b64 = (s) => {
    const chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';
    let result = '';
    const bytes = [];
    for (let i = 0; i < s.length; i++) {
      bytes.push(s.charCodeAt(i));
    }
    for (let i = 0; i < bytes.length; i += 3) {
      const b1 = bytes[i];
      const b2 = bytes[i + 1] || 0;
      const b3 = bytes[i + 2] || 0;
      result += chars[b1 >> 2];
      result += chars[((b1 & 3) << 4) | (b2 >> 4)];
      result += i + 1 < bytes.length ? chars[((b2 & 15) << 2) | (b3 >> 6)] : '';
      result += i + 2 < bytes.length ? chars[b3 & 63] : '';
    }
    return result;
  };

  return `${b64(header)}.${b64(payload)}.mock-signature`;
}

/**
 * Generate random data payload
 */
function generatePayload(vuId, opNum) {
  return {
    vuId: vuId,
    opNum: opNum,
    timestamp: Date.now(),
    data: `write-heavy-test-${Math.random().toString(36).substring(7)}`,
    nested: {
      field1: Math.random() * 1000,
      field2: `value-${opNum}`,
      array: [1, 2, 3, 4, 5],
    },
  };
}

/**
 * Main test function - runs for each VU
 */
export default function () {
  const vuId = __VU;
  const iterationId = __ITER;
  const nodeId = `k6-writer-vu${vuId}-iter${iterationId}`;

  let authenticated = false;
  let client = null;
  let opCounter = 0;
  let pendingOps = new Map(); // Track pending operations for latency
  let opsThisSecond = 0;
  let secondStart = Date.now();

  const connectStart = Date.now();

  const res = ws.connect(WS_URL, {}, function (socket) {
    client = new TopGunClient(socket, nodeId);

    const handleMessage = createMessageHandler(client, {
      onAuthRequired: () => {
        const token = getAuthToken(vuId);
        client.authenticate(token);
      },

      onAuthAck: () => {
        authenticated = true;
        connectionTime.add(Date.now() - connectStart);

        // Start write loop
        scheduleWrites();
      },

      onAuthError: () => {
        errors.add(1);
        socket.close();
      },

      onOpAck: (msg) => {
        // Calculate latency for acknowledged operations
        const lastId = msg.payload?.lastId;
        if (lastId && pendingOps.has(lastId)) {
          const sendTime = pendingOps.get(lastId);
          const latency = Date.now() - sendTime;
          writeLatency.add(latency);
          writeOpsAcked.add(BATCH_SIZE);
          writeErrorRate.add(0);
          pendingOps.delete(lastId);
        }
      },
    });

    socket.on('binaryMessage', handleMessage);

    socket.on('error', (e) => {
      errors.add(1);
      writeErrorRate.add(1);
    });

    socket.on('close', () => {
      // Track any unacked ops as potential errors
      pendingOps.forEach(() => {
        writeErrorRate.add(1);
      });
    });

    /**
     * Schedule periodic write operations
     */
    function scheduleWrites() {
      const intervalMs = 1000 / (OPS_PER_SECOND / BATCH_SIZE);

      function doWrite() {
        if (!authenticated) return;

        // Track ops per second
        const now = Date.now();
        if (now - secondStart >= 1000) {
          if (opsThisSecond > 0) {
            opsPerSecond.add(opsThisSecond);
          }
          opsThisSecond = 0;
          secondStart = now;
        }

        // Create batch of operations
        const operations = [];
        for (let i = 0; i < BATCH_SIZE; i++) {
          opCounter++;
          operations.push({
            mapName: `k6-write-heavy-${vuId % 10}`, // Distribute across 10 maps
            key: `key-${vuId}-${opCounter}`,
            value: generatePayload(vuId, opCounter),
          });
        }

        // Send batch and track for latency measurement
        const sendTime = Date.now();
        const lastOpId = client.putBatch(operations);
        pendingOps.set(lastOpId, sendTime);
        writeOpsTotal.add(BATCH_SIZE);
        opsThisSecond += BATCH_SIZE;

        // Clean up old pending ops (timeout after 5s)
        const timeout = 5000;
        pendingOps.forEach((time, id) => {
          if (now - time > timeout) {
            writeErrorRate.add(1);
            pendingOps.delete(id);
          }
        });

        // Schedule next write
        socket.setTimeout(doWrite, intervalMs);
      }

      // Start writing
      doWrite();
    }

    // Run for test duration, then close
    // k6 will handle the duration, but we add a safety timeout
    socket.setTimeout(function () {
      socket.close();
    }, 6 * 60 * 1000); // 6 minutes safety timeout
  });

  // Check connection was successful
  check(res, {
    'WebSocket connected': (r) => r && r.status === 101,
  });

  // Keep iteration alive for the test duration
  // The socket callbacks handle everything
  sleep(300); // 5 minutes
}

/**
 * Setup function
 */
export function setup() {
  console.log('='.repeat(60));
  console.log('Write-Heavy Test');
  console.log('='.repeat(60));
  console.log(`Target: ${WS_URL}`);
  console.log(`VUs: ${options.vus}`);
  console.log(`Duration: ${options.duration}`);
  console.log(`Ops per second per VU: ${OPS_PER_SECOND}`);
  console.log(`Batch size: ${BATCH_SIZE}`);
  console.log(`Expected total ops: ${options.vus * OPS_PER_SECOND * 300}`);
  console.log('');

  if (!JWT_TOKEN) {
    console.warn('WARNING: No JWT_TOKEN provided. Using mock tokens.');
    console.warn('Run: pnpm test:k6:token');
  }

  return { startTime: Date.now() };
}

/**
 * Teardown function
 */
export function teardown(data) {
  const duration = (Date.now() - data.startTime) / 1000;
  console.log('');
  console.log('='.repeat(60));
  console.log(`Test completed in ${duration.toFixed(2)}s`);
  console.log('='.repeat(60));
}

/**
 * Handle summary
 */
export function handleSummary(data) {
  const duration = 300; // 5 minutes in seconds
  const totalOps = data.metrics.write_ops_total?.values?.count || 0;
  const actualOpsPerSecond = totalOps / duration;

  const summary = {
    timestamp: new Date().toISOString(),
    scenario: 'write-heavy',
    wsUrl: WS_URL,
    config: {
      vus: options.vus,
      duration: options.duration,
      targetOpsPerSecond: options.vus * OPS_PER_SECOND,
    },
    metrics: {
      writeLatency: {
        avg: data.metrics.write_latency?.values?.avg || 0,
        p95: data.metrics.write_latency?.values['p(95)'] || 0,
        p99: data.metrics.write_latency?.values['p(99)'] || 0,
        max: data.metrics.write_latency?.values?.max || 0,
      },
      throughput: {
        totalOps: totalOps,
        ackedOps: data.metrics.write_ops_acked?.values?.count || 0,
        opsPerSecond: actualOpsPerSecond,
      },
      errorRate: data.metrics.write_error_rate?.values?.rate || 0,
      authSuccessRate: data.metrics.topgun_auth_success?.values?.rate || 0,
    },
    thresholds: data.thresholds || {},
  };

  return {
    stdout: textSummary(data, { indent: ' ', enableColors: true }),
    'tests/k6/results/write-heavy-summary.json': JSON.stringify(summary, null, 2),
  };
}

import { textSummary } from 'https://jslib.k6.io/k6-summary/0.0.1/index.js';
