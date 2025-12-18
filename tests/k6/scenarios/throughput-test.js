/**
 * TopGun Throughput Benchmark
 *
 * Primary benchmark for measuring TopGun server performance.
 * Determines maximum sustainable throughput by ramping up load
 * until latency degrades or errors appear.
 *
 * Measures:
 * - Maximum throughput (ops/sec)
 * - Latency distribution (p50, p95, p99)
 * - Error rate under load
 *
 * Run:
 *   k6 run tests/k6/scenarios/throughput-test.js -e JWT_TOKEN=<token>
 *
 * Or with pnpm:
 *   pnpm test:k6:throughput
 */

import ws from 'k6/ws';
import { check } from 'k6';
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
  getResultsPath,
} from '../lib/config.js';

// Configuration
const WS_URL = getWsUrl();
const BATCH_SIZE = getConfig('BATCH_SIZE', 5);

// Ramping stages to find saturation point
export const options = {
  scenarios: {
    throughput_ramp: {
      executor: 'ramping-vus',
      startVUs: 10,
      stages: [
        { duration: '20s', target: 50 },   // Warm up
        { duration: '20s', target: 100 },  // Increase
        { duration: '20s', target: 150 },  // More load
        { duration: '20s', target: 200 },  // High load
        { duration: '20s', target: 250 },  // Very high
        { duration: '20s', target: 300 },  // Peak
      ],
      gracefulRampDown: '10s',
    },
  },
  thresholds: {
    write_latency: ['p(99)<500'],  // Allow higher latency to find limits
    topgun_auth_success: ['rate>0.95'],
  },
};

// Metrics
const writeLatency = new Trend('write_latency', true);
const writeOpsTotal = new Counter('write_ops_total');
const writeOpsAcked = new Counter('write_ops_acked');
const writeErrorRate = new Rate('write_error_rate');

function generatePayload(vuId, opNum) {
  return {
    vuId,
    opNum,
    timestamp: Date.now(),
    data: `throughput-test-${Math.random().toString(36).substring(7)}`,
  };
}

export default function () {
  const vuId = __VU;
  const iterationId = __ITER;
  const nodeId = `k6-throughput-vu${vuId}-iter${iterationId}`;

  let authenticated = false;
  let client = null;
  let opCounter = 0;
  let pendingOps = new Map();
  const sessionStart = Date.now();

  const res = ws.connect(WS_URL, {}, function (socket) {
    client = new TopGunClient(socket, nodeId);

    const handleMessage = createMessageHandler(client, {
      onAuthRequired: () => {
        const token = getAuthToken(vuId, 'k6-throughput', ['USER', 'ADMIN']);
        client.authenticate(token);
      },

      onAuthAck: () => {
        authenticated = true;
        connectionTime.add(Date.now() - sessionStart);
        scheduleWrites();
      },

      onAuthError: () => {
        errors.add(1);
        socket.close();
      },

      onOpAck: (msg) => {
        const lastId = msg.payload?.lastId;
        if (lastId && pendingOps.has(lastId)) {
          const sendTime = pendingOps.get(lastId);
          const latency = Date.now() - sendTime;
          writeLatency.add(latency);
          writeOpsAcked.add(BATCH_SIZE);
          for (let i = 0; i < BATCH_SIZE; i++) {
            writeErrorRate.add(0);
          }
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
      if (authenticated) {
        pendingOps.forEach(() => {
          for (let i = 0; i < BATCH_SIZE; i++) {
            writeErrorRate.add(1);
          }
        });
      }
    });

    function scheduleWrites() {
      // Send as fast as possible to find limits
      const intervalMs = 50; // 20 batches/sec per VU = 100 ops/sec per VU

      function doWrite() {
        if (!authenticated) return;

        const operations = [];
        for (let i = 0; i < BATCH_SIZE; i++) {
          opCounter++;
          operations.push({
            mapName: `k6-throughput-${vuId % 20}`,
            key: `key-${vuId}-${opCounter}`,
            value: generatePayload(vuId, opCounter),
          });
        }

        const sendTime = Date.now();
        const lastOpId = client.putBatch(operations);
        pendingOps.set(lastOpId, sendTime);
        writeOpsTotal.add(BATCH_SIZE);

        // Cleanup stale pending ops
        const now = Date.now();
        const timeout = 5000;
        pendingOps.forEach((time, id) => {
          if (now - time > timeout) {
            for (let i = 0; i < BATCH_SIZE; i++) {
              writeErrorRate.add(1);
            }
            pendingOps.delete(id);
          }
        });

        socket.setTimeout(doWrite, intervalMs);
      }

      doWrite();
    }

    // Keep session alive for test duration
    const maxSessionTime = 120 * 1000; // 2 minutes max
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
  console.log('');
  console.log('╔══════════════════════════════════════════════════════════════════╗');
  console.log('║                 TOPGUN THROUGHPUT BENCHMARK                      ║');
  console.log('╠══════════════════════════════════════════════════════════════════╣');
  console.log('║  Measuring maximum sustainable throughput under increasing load  ║');
  console.log('╚══════════════════════════════════════════════════════════════════╝');
  console.log('');
  console.log(`Target: ${WS_URL}`);
  console.log(`Batch size: ${BATCH_SIZE}`);
  console.log('Stages: 10 → 50 → 100 → 150 → 200 → 250 → 300 VUs');
  console.log('');

  return { startTime: Date.now() };
}

export function teardown(data) {
  const duration = (Date.now() - data.startTime) / 1000;
  console.log('');
  console.log(`Test completed in ${duration.toFixed(2)}s`);
}

export function handleSummary(data) {
  const totalOps = data.metrics.write_ops_total?.values?.count || 0;
  const ackedOps = data.metrics.write_ops_acked?.values?.count || 0;
  const testDuration = 120; // ~2 minutes of ramping
  const avgThroughput = ackedOps / testDuration;

  const p50 = data.metrics.write_latency?.values?.med || 0;
  const p95 = data.metrics.write_latency?.values['p(95)'] || 0;
  const p99 = data.metrics.write_latency?.values['p(99)'] || 0;
  const maxLatency = data.metrics.write_latency?.values?.max || 0;
  const errorRate = data.metrics.write_error_rate?.values?.rate || 0;

  console.log('');
  console.log('╔══════════════════════════════════════════════════════════════════╗');
  console.log('║                   THROUGHPUT BENCHMARK RESULTS                   ║');
  console.log('╠══════════════════════════════════════════════════════════════════╣');
  console.log('║  THROUGHPUT                                                      ║');
  console.log(`║    Total Operations Sent:      ${totalOps.toLocaleString().padStart(12)}                  ║`);
  console.log(`║    Total Operations Acked:     ${ackedOps.toLocaleString().padStart(12)}                  ║`);
  console.log(`║    Average Throughput:         ${avgThroughput.toFixed(0).padStart(12)} ops/sec          ║`);
  console.log('╠══════════════════════════════════════════════════════════════════╣');
  console.log('║  LATENCY (time to Early ACK, before in-memory write)             ║');
  console.log(`║    p50:                        ${p50.toFixed(1).padStart(12)} ms                ║`);
  console.log(`║    p95:                        ${p95.toFixed(1).padStart(12)} ms                ║`);
  console.log(`║    p99:                        ${p99.toFixed(1).padStart(12)} ms                ║`);
  console.log(`║    max:                        ${maxLatency.toFixed(1).padStart(12)} ms                ║`);
  console.log('╠══════════════════════════════════════════════════════════════════╣');
  console.log('║  RELIABILITY                                                     ║');
  console.log(`║    Error Rate:                 ${(errorRate * 100).toFixed(3).padStart(12)}%               ║`);
  console.log(`║    Success Rate:               ${((1 - errorRate) * 100).toFixed(3).padStart(12)}%               ║`);
  console.log('╚══════════════════════════════════════════════════════════════════╝');

  // Performance assessment
  console.log('');
  console.log('╔══════════════════════════════════════════════════════════════════╗');
  console.log('║                    PERFORMANCE ASSESSMENT                        ║');
  console.log('╠══════════════════════════════════════════════════════════════════╣');

  const throughputTarget = avgThroughput >= 15000;
  const latencyTarget = p99 < 100;
  const errorTarget = errorRate < 0.01;

  console.log(`║  Throughput ≥15K ops/sec:      ${throughputTarget ? '✅ PASS' : '❌ FAIL'}  (${avgThroughput.toFixed(0)} ops/sec)`.padEnd(69) + '║');
  console.log(`║  p99 Latency <100ms:           ${latencyTarget ? '✅ PASS' : '❌ FAIL'}  (${p99.toFixed(1)}ms)`.padEnd(69) + '║');
  console.log(`║  Error Rate <1%:               ${errorTarget ? '✅ PASS' : '❌ FAIL'}  (${(errorRate * 100).toFixed(3)}%)`.padEnd(69) + '║');

  const allPass = throughputTarget && latencyTarget && errorTarget;
  console.log('╠══════════════════════════════════════════════════════════════════╣');
  console.log(`║  Overall:                      ${allPass ? '✅ ALL TARGETS MET' : '⚠️  TARGETS NOT MET'}`.padEnd(69) + '║');
  console.log('╚══════════════════════════════════════════════════════════════════╝');

  const summary = {
    timestamp: new Date().toISOString(),
    scenario: 'throughput-benchmark',
    version: __ENV.VERSION || 'current',
    config: {
      batchSize: BATCH_SIZE,
      wsUrl: WS_URL,
    },
    results: {
      throughput: {
        totalOpsSent: totalOps,
        totalOpsAcked: ackedOps,
        avgOpsPerSec: avgThroughput,
      },
      latency: {
        p50: p50,
        p95: p95,
        p99: p99,
        max: maxLatency,
      },
      reliability: {
        errorRate: errorRate,
        successRate: 1 - errorRate,
      },
    },
    targets: {
      throughput: { target: 15000, actual: avgThroughput, pass: throughputTarget },
      latencyP99: { target: 100, actual: p99, pass: latencyTarget },
      errorRate: { target: 0.01, actual: errorRate, pass: errorTarget },
      allPass: allPass,
    },
  };

  return {
    stdout: textSummary(data, { indent: ' ', enableColors: true }),
    [getResultsPath('throughput-benchmark.json')]: JSON.stringify(summary, null, 2),
  };
}

import { textSummary } from 'https://jslib.k6.io/k6-summary/0.0.1/index.js';
