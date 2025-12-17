/**
 * Phase 3 Benchmark Test
 *
 * Measures the impact of Phase 3 native optimizations:
 * - Native xxHash64 for Merkle tree hashing
 * - SharedArrayBuffer for worker communication
 * - Overall throughput improvement
 *
 * Run:
 *   k6 run tests/k6/scenarios/phase3-benchmark.js -e JWT_TOKEN=<token>
 *
 * Or with pnpm:
 *   pnpm test:k6:phase3
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
// Match throughput-test.js: 50ms interval = 20 batches/sec = 100 ops/sec per VU
const INTERVAL_MS = 50;

// Test stages: match throughput-test.js for fair comparison
export const options = {
  scenarios: {
    phase3_benchmark: {
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
    // Allow higher latency to find limits (same as throughput-test.js)
    write_latency: ['p(99)<500'],
    topgun_auth_success: ['rate>0.95'],
  },
};

// Custom metrics
const writeLatency = new Trend('write_latency', true);
const writeOpsTotal = new Counter('write_ops_total');
const writeOpsAcked = new Counter('write_ops_acked');
const writeErrorRate = new Rate('write_error_rate');
const batchLatency = new Trend('batch_latency', true);

function generatePayload(vuId, opNum) {
  // Same payload as throughput-test.js for fair comparison
  return {
    vuId,
    opNum,
    timestamp: Date.now(),
    data: `phase3-test-${Math.random().toString(36).substring(7)}`,
  };
}

export default function () {
  const vuId = __VU;
  const iterationId = __ITER;
  const nodeId = `k6-phase3-vu${vuId}-iter${iterationId}`;

  let authenticated = false;
  let client = null;
  let opCounter = 0;
  let pendingOps = new Map();
  const sessionStart = Date.now();

  const res = ws.connect(WS_URL, {}, function (socket) {
    client = new TopGunClient(socket, nodeId);

    const handleMessage = createMessageHandler(client, {
      onAuthRequired: () => {
        const token = getAuthToken(vuId, 'k6-phase3', ['USER', 'ADMIN']);
        client.authenticate(token);
      },

      onAuthAck: () => {
        authenticated = true;
        connectionTime.add(Date.now() - sessionStart);
        scheduleWrites();
        scheduleHeartbeat();
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
          batchLatency.add(latency);
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
      // Send as fast as possible (same as throughput-test.js)
      function doWrite() {
        if (!authenticated) return;

        const operations = [];
        for (let i = 0; i < BATCH_SIZE; i++) {
          opCounter++;
          operations.push({
            mapName: `k6-phase3-${vuId % 20}`,
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

        socket.setTimeout(doWrite, INTERVAL_MS);
      }

      doWrite();
    }

    function scheduleHeartbeat() {
      function doPing() {
        if (authenticated) {
          client.ping();
          socket.setTimeout(doPing, 10000);
        }
      }
      socket.setTimeout(doPing, 10000);
    }

    // Session timeout
    const maxSessionTime = 110 * 1000;
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
  console.log('║             TOPGUN PHASE 3 NATIVE OPTIMIZATION BENCHMARK         ║');
  console.log('╠══════════════════════════════════════════════════════════════════╣');
  console.log('║  Measuring impact of:                                            ║');
  console.log('║  • Native xxHash64 for Merkle tree hashing                       ║');
  console.log('║  • SharedArrayBuffer for worker communication                    ║');
  console.log('║  • Integrated Phase 1+2+3 optimizations                          ║');
  console.log('║                                                                  ║');
  console.log('║  Using same parameters as throughput-test.js for fair comparison ║');
  console.log('╚══════════════════════════════════════════════════════════════════╝');
  console.log('');
  console.log(`Target: ${WS_URL}`);
  console.log(`Batch size: ${BATCH_SIZE}`);
  console.log(`Interval: ${INTERVAL_MS}ms (${1000/INTERVAL_MS * BATCH_SIZE} ops/sec per VU)`);
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
  const testDuration = 120; // ~2 minutes of ramping (same as throughput-test.js)
  const avgThroughput = ackedOps / testDuration;

  const p50 = data.metrics.write_latency?.values?.med || 0;
  const p95 = data.metrics.write_latency?.values['p(95)'] || 0;
  const p99 = data.metrics.write_latency?.values['p(99)'] || 0;
  const maxLatency = data.metrics.write_latency?.values?.max || 0;
  const errorRate = data.metrics.write_error_rate?.values?.rate || 0;

  console.log('');
  console.log('╔══════════════════════════════════════════════════════════════════╗');
  console.log('║                   PHASE 3 BENCHMARK RESULTS                      ║');
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

  // Comparison with Phase 2 baseline (throughput-test.js result: ~18K ops/sec)
  console.log('');
  console.log('╔══════════════════════════════════════════════════════════════════╗');
  console.log('║                PHASE 2 → PHASE 3 COMPARISON                      ║');
  console.log('╠══════════════════════════════════════════════════════════════════╣');
  const phase2Baseline = 18000; // Phase 2 throughput-test.js result
  const improvement = ((avgThroughput - phase2Baseline) / phase2Baseline * 100);
  console.log(`║  Phase 2 Baseline:             ${phase2Baseline.toLocaleString().padStart(12)} ops/sec          ║`);
  console.log(`║  Phase 3 Result:               ${avgThroughput.toFixed(0).padStart(12)} ops/sec          ║`);
  console.log(`║  Improvement:                  ${improvement > 0 ? '+' : ''}${improvement.toFixed(1).padStart(11)}%                ║`);
  console.log('╚══════════════════════════════════════════════════════════════════╝');

  const summary = {
    timestamp: new Date().toISOString(),
    scenario: 'phase3-benchmark',
    version: 'phase3-native-optimized',
    config: {
      batchSize: BATCH_SIZE,
      intervalMs: INTERVAL_MS,
      opsPerSecPerVU: 1000 / INTERVAL_MS * BATCH_SIZE,
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
    comparison: {
      phase2Baseline: phase2Baseline,
      phase3Result: avgThroughput,
      improvementPercent: improvement,
    },
  };

  return {
    stdout: textSummary(data, { indent: ' ', enableColors: true }),
    [getResultsPath('phase3-benchmark.json')]: JSON.stringify(summary, null, 2),
  };
}

import { textSummary } from 'https://jslib.k6.io/k6-summary/0.0.1/index.js';
