/**
 * TopGun HIGH-LOAD Stress Test
 *
 * Designed to find the actual limits of TopGun server.
 * Uses aggressive parameters to saturate server capacity.
 *
 * Key changes from throughput-test:
 * - More VUs (up to 500)
 * - Shorter interval (10ms instead of 50ms = 5x more ops/sec)
 * - Larger batches (10 instead of 5)
 * - Theoretical max: 500 VU × 10 batch × 100 batches/sec = 500,000 ops/sec
 *
 * Run:
 *   1. Start server: pnpm start:server
 *   2. Run test: pnpm test:k6:stress
 *
 * Or with custom parameters:
 *   MAX_VUS=1000 BATCH_SIZE=20 INTERVAL_MS=5 pnpm test:k6:stress
 */

import ws from 'k6/ws';
import { check } from 'k6';
import { Counter, Trend, Rate, Gauge } from 'k6/metrics';
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
  getResultsPath,
} from '../lib/config.js';

// Aggressive configuration for stress testing
const WS_URL = getWsUrl();
const BATCH_SIZE = getConfig('BATCH_SIZE', 10);        // 10 ops per batch (was 5)
const INTERVAL_MS = getConfig('INTERVAL_MS', 10);      // 10ms = 100 batches/sec (was 50ms)
const MAX_VUS = getConfig('MAX_VUS', 500);             // Up to 500 VUs (was 300)
const PAYLOAD_SIZE = getConfig('PAYLOAD_SIZE', 128);   // Smaller payload for speed

// Stress test - ramp up to find breaking point
export const options = {
  scenarios: {
    stress: {
      executor: 'ramping-vus',
      startVUs: 50,
      stages: [
        { duration: '10s', target: 100 },   // Warm up
        { duration: '10s', target: 200 },   // Increase
        { duration: '10s', target: 300 },   // High load
        { duration: '10s', target: 400 },   // Very high
        { duration: '20s', target: MAX_VUS }, // Maximum stress
        { duration: '10s', target: MAX_VUS }, // Sustain peak
        { duration: '10s', target: 100 },   // Cool down
      ],
      gracefulRampDown: '5s',
    },
  },
  thresholds: {
    // Relaxed thresholds for stress testing - we want to find limits
    write_latency: ['p(95)<5000'],  // Allow high latency under stress
    error_rate: ['rate<0.3'],       // Allow up to 30% errors
  },
};

// Metrics
const writeLatency = new Trend('write_latency', true);
const opsSent = new Counter('ops_sent');
const opsAcked = new Counter('ops_acked');
const errorRate = new Rate('error_rate');
const activeVUs = new Gauge('active_vus');

// Pre-generate payload template
const PAYLOAD_DATA = 'x'.repeat(PAYLOAD_SIZE);

function generatePayload(vuId, opNum) {
  return {
    vuId,
    opNum,
    ts: Date.now(),
    d: PAYLOAD_DATA,
  };
}

export default function () {
  const vuId = __VU;
  const iterationId = __ITER;
  const nodeId = `k6-stress-vu${vuId}-iter${iterationId}`;

  let authenticated = false;
  let client = null;
  let opCounter = 0;
  let pendingOps = new Map();
  const sessionStart = Date.now();

  activeVUs.add(1);

  const res = ws.connect(WS_URL, {}, function (socket) {
    client = new TopGunClient(socket, nodeId);

    const handleMessage = createMessageHandler(client, {
      onAuthRequired: () => {
        const token = getAuthToken(vuId, 'k6-stress', ['USER', 'ADMIN']);
        client.authenticate(token);
      },

      onAuthAck: () => {
        authenticated = true;
        connectionTime.add(Date.now() - sessionStart);
        scheduleWrites();
      },

      onAuthError: () => {
        errors.add(1);
        errorRate.add(1);
        socket.close();
      },

      onOpAck: (msg) => {
        const lastId = msg.payload?.lastId;
        if (lastId && pendingOps.has(lastId)) {
          const sendTime = pendingOps.get(lastId);
          const latency = Date.now() - sendTime;
          writeLatency.add(latency);
          opsAcked.add(BATCH_SIZE);
          for (let i = 0; i < BATCH_SIZE; i++) {
            errorRate.add(0);
          }
          pendingOps.delete(lastId);
        }
      },
    });

    socket.on('binaryMessage', handleMessage);

    socket.on('error', () => {
      errors.add(1);
      errorRate.add(1);
    });

    socket.on('close', () => {
      activeVUs.add(-1);
      if (authenticated) {
        pendingOps.forEach(() => {
          for (let i = 0; i < BATCH_SIZE; i++) {
            errorRate.add(1);
          }
        });
      }
    });

    function scheduleWrites() {
      function doWrite() {
        if (!authenticated) return;

        const now = Date.now();

        // Send batch of operations
        // Reuse keys to avoid unbounded memory growth (realistic update scenario)
        const operations = [];
        for (let i = 0; i < BATCH_SIZE; i++) {
          opCounter++;
          operations.push({
            mapName: `k6-stress-${vuId % 50}`,  // Distribute across 50 maps
            key: `key-${vuId}-${opCounter % 100}`,  // Reuse 100 keys per VU
            value: generatePayload(vuId, opCounter),
          });
        }

        const sendTime = now;
        const lastOpId = client.putBatch(operations);
        pendingOps.set(lastOpId, sendTime);
        opsSent.add(BATCH_SIZE);

        // Cleanup stale pending ops (timeout 3s for stress test)
        const timeout = 3000;
        pendingOps.forEach((time, id) => {
          if (now - time > timeout) {
            for (let i = 0; i < BATCH_SIZE; i++) {
              errorRate.add(1);
            }
            pendingOps.delete(id);
          }
        });

        socket.setTimeout(doWrite, INTERVAL_MS);
      }

      doWrite();
    }

    // Session duration matches test (~80s)
    const maxSessionTime = 80 * 1000;
    socket.setTimeout(() => {
      authenticated = false;
      socket.setTimeout(() => socket.close(), 2000);
    }, maxSessionTime);
  });

  check(res, {
    'WebSocket connected': (r) => r && r.status === 101,
  });
}

export function setup() {
  const theoreticalMax = MAX_VUS * BATCH_SIZE * (1000 / INTERVAL_MS);

  console.log('');
  console.log('╔══════════════════════════════════════════════════════════════════╗');
  console.log('║              TOPGUN HIGH-LOAD STRESS TEST                        ║');
  console.log('╠══════════════════════════════════════════════════════════════════╣');
  console.log('║  Finding the limits of TopGun server                             ║');
  console.log('║  WARNING: This test will push the server to its breaking point   ║');
  console.log('╚══════════════════════════════════════════════════════════════════╝');
  console.log('');
  console.log(`  URL:            ${WS_URL}`);
  console.log(`  Max VUs:        ${MAX_VUS}`);
  console.log(`  Batch size:     ${BATCH_SIZE}`);
  console.log(`  Interval:       ${INTERVAL_MS}ms (${1000/INTERVAL_MS} batches/sec/VU)`);
  console.log(`  Payload:        ${PAYLOAD_SIZE} bytes`);
  console.log(`  Theoretical max: ${(theoreticalMax/1000).toFixed(0)}K ops/sec`);
  console.log('');
  console.log('Stages: 50 → 100 → 200 → 300 → 400 → 500 → 500 → 100 VUs');
  console.log('');

  return { startTime: Date.now(), theoreticalMax };
}

export function teardown(data) {
  const duration = (Date.now() - data.startTime) / 1000;
  console.log('');
  console.log(`Test completed in ${duration.toFixed(2)}s`);
}

export function handleSummary(data) {
  const sent = data.metrics.ops_sent?.values?.count || 0;
  const acked = data.metrics.ops_acked?.values?.count || 0;
  const testDuration = 80; // ~80 seconds
  const throughput = acked / testDuration;

  const p50 = data.metrics.write_latency?.values?.med || 0;
  const p95 = data.metrics.write_latency?.values['p(95)'] || 0;
  const p99 = data.metrics.write_latency?.values['p(99)'] || 0;
  const maxLatency = data.metrics.write_latency?.values?.max || 0;
  const errRate = data.metrics.error_rate?.values?.rate || 0;

  // Calculate efficiency vs theoretical max
  const theoreticalMax = MAX_VUS * BATCH_SIZE * (1000 / INTERVAL_MS) * testDuration;
  const efficiency = (acked / theoreticalMax * 100).toFixed(1);

  console.log('');
  console.log('╔══════════════════════════════════════════════════════════════════╗');
  console.log('║                TOPGUN STRESS TEST RESULTS                        ║');
  console.log('╠══════════════════════════════════════════════════════════════════╣');
  console.log('║  THROUGHPUT                                                      ║');
  console.log(`║    Operations Sent:            ${sent.toLocaleString().padStart(12)}                  ║`);
  console.log(`║    Operations Acked:           ${acked.toLocaleString().padStart(12)}                  ║`);
  console.log(`║    THROUGHPUT:                 ${throughput.toFixed(0).padStart(12)} ops/sec          ║`);
  console.log(`║    Efficiency:                 ${efficiency.padStart(12)}%                  ║`);
  console.log('╠══════════════════════════════════════════════════════════════════╣');
  console.log('║  LATENCY                                                         ║');
  console.log(`║    p50:                        ${p50.toFixed(1).padStart(12)} ms                ║`);
  console.log(`║    p95:                        ${p95.toFixed(1).padStart(12)} ms                ║`);
  console.log(`║    p99:                        ${p99.toFixed(1).padStart(12)} ms                ║`);
  console.log(`║    max:                        ${maxLatency.toFixed(1).padStart(12)} ms                ║`);
  console.log('╠══════════════════════════════════════════════════════════════════╣');
  console.log('║  RELIABILITY                                                     ║');
  console.log(`║    Error Rate:                 ${(errRate * 100).toFixed(2).padStart(12)}%               ║`);
  console.log(`║    Loss Rate:                  ${((1 - acked/sent) * 100).toFixed(2).padStart(12)}%               ║`);
  console.log('╚══════════════════════════════════════════════════════════════════╝');

  // Comparison with Pure WS
  console.log('');
  console.log('╔══════════════════════════════════════════════════════════════════╗');
  console.log('║                    COMPARISON GUIDE                              ║');
  console.log('╠══════════════════════════════════════════════════════════════════╣');
  console.log('║  Compare with pure WebSocket baseline:                           ║');
  console.log('║    pnpm test:k6:pure-ws:stress      (ws library limit)           ║');
  console.log('║    pnpm test:k6:pure-uws:stress     (uWebSockets.js limit)       ║');
  console.log('║                                                                  ║');
  console.log('║  TopGun overhead = (Pure WS - TopGun) / Pure WS × 100%           ║');
  console.log('╚══════════════════════════════════════════════════════════════════╝');

  const summary = {
    timestamp: new Date().toISOString(),
    scenario: 'topgun-stress-test',
    version: __ENV.VERSION || 'current',
    config: {
      maxVUs: MAX_VUS,
      batchSize: BATCH_SIZE,
      intervalMs: INTERVAL_MS,
      payloadSize: PAYLOAD_SIZE,
      wsUrl: WS_URL,
    },
    results: {
      throughput: {
        opsSent: sent,
        opsAcked: acked,
        avgOpsPerSec: throughput,
        efficiency: parseFloat(efficiency),
      },
      latency: {
        p50: p50,
        p95: p95,
        p99: p99,
        max: maxLatency,
      },
      reliability: {
        errorRate: errRate,
        lossRate: 1 - acked/sent,
      },
    },
  };

  return {
    stdout: textSummary(data, { indent: ' ', enableColors: true }),
    [getResultsPath('topgun-stress-test.json')]: JSON.stringify(summary, null, 2),
  };
}

import { textSummary } from 'https://jslib.k6.io/k6-summary/0.0.1/index.js';
