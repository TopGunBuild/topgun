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
  errors,
} from '../lib/topgun-client.js';
import {
  getWsUrl,
  getAuthToken,
  getConfig,
  logTestHeader,
  getResultsPath,
} from '../lib/config.js';

// Configuration
const WS_URL = getWsUrl();
const OPS_PER_SECOND = getConfig('OPS_PER_SECOND', 10);
const BATCH_SIZE = getConfig('BATCH_SIZE', 5);

// Default test duration in seconds (can be overridden via CLI)
const TEST_DURATION_SEC = getConfig('DURATION_SEC', 300); // 5 minutes

// Test configuration
export const options = {
  vus: 100,
  duration: '5m',
  thresholds: {
    // p99 write latency < 100ms
    write_latency: ['p(99)<100'],
    // Error rate < 1%
    write_error_rate: ['rate<0.01'],
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
        const token = getAuthToken(vuId, 'k6-writer', ['USER', 'ADMIN']);
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

    socket.on('error', () => {
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

    // Close socket slightly before sleep ends to ensure clean iteration finish
    socket.setTimeout(function () {
      socket.close();
    }, (TEST_DURATION_SEC - 2) * 1000);
  });

  // Check connection was successful
  check(res, {
    'WebSocket connected': (r) => r && r.status === 101,
  });

  // Note: ws.connect() blocks until socket closes, no sleep needed
}

/**
 * Setup function
 */
export function setup() {
  logTestHeader('Write-Heavy Test', {
    'Target': WS_URL,
    'VUs': options.vus,
    'Duration': options.duration,
    'Ops per second per VU': OPS_PER_SECOND,
    'Batch size': BATCH_SIZE,
    'Expected total ops': options.vus * OPS_PER_SECOND * 300,
  });

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
    [getResultsPath('write-heavy-summary.json')]: JSON.stringify(summary, null, 2),
  };
}

import { textSummary } from 'https://jslib.k6.io/k6-summary/0.0.1/index.js';
