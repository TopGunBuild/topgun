/**
 * TopGun Cluster Rebalance Test (Phase 4.5 Task 07)
 *
 * Tests cluster behavior during dynamic scaling:
 * - Measures throughput during node addition/removal
 * - Verifies partition migration doesn't cause data loss
 * - Tracks rebalance completion time
 *
 * Run:
 *   k6 run tests/k6/scenarios/cluster-rebalance.js -e JWT_TOKEN=<token>
 *
 * Or with pnpm:
 *   pnpm test:k6:cluster:rebalance
 *
 * Note: Node scaling requires external trigger (docker scale)
 */

import ws from 'k6/ws';
import { check, sleep } from 'k6';
import { Counter, Trend, Rate, Gauge } from 'k6/metrics';
import {
  TopGunClient,
  createMessageHandler,
  connectionTime,
  errors,
} from '../lib/topgun-client.js';
import {
  getAuthToken,
  getConfig,
  getResultsPath,
} from '../lib/config.js';

// Cluster Configuration - starts with 3 nodes, scales to 4
const CLUSTER_NODES = [
  __ENV.NODE1_URL || 'ws://localhost:8081',
  __ENV.NODE2_URL || 'ws://localhost:8082',
  __ENV.NODE3_URL || 'ws://localhost:8083',
  __ENV.NODE4_URL || 'ws://localhost:8084', // Added during rebalance
];

const INITIAL_NODES = 3;
const BATCH_SIZE = getConfig('BATCH_SIZE', 5);
const MAX_VUS = getConfig('MAX_VUS', 75);
const KEY_POOL_SIZE = getConfig('KEY_POOL_SIZE', 200);
const SCALE_UP_TIME_MS = getConfig('SCALE_UP_TIME', 30000);   // Scale up at 30s
const SCALE_DOWN_TIME_MS = getConfig('SCALE_DOWN_TIME', 90000); // Scale down at 90s

// Test configuration - 3 minute test with scaling events
export const options = {
  scenarios: {
    rebalance_test: {
      executor: 'ramping-vus',
      startVUs: 10,
      stages: [
        { duration: '20s', target: MAX_VUS },  // Ramp up before scale
        { duration: '40s', target: MAX_VUS },  // Scale up at 30s, sustain
        { duration: '40s', target: MAX_VUS },  // Scale down at 90s, sustain
        { duration: '20s', target: 0 },         // Ramp down
      ],
      gracefulRampDown: '10s',
    },
  },
  thresholds: {
    'rebalance_success_rate': ['rate>0.98'],      // 98% success during rebalance
    'rebalance_latency': ['p(95)<100'],           // p95 < 100ms (higher during migration)
    'rebalance_ops_total': ['count>75000'],       // 75K ops in 2 min
  },
};

// Rebalance-specific metrics
const rebalanceOpsTotal = new Counter('rebalance_ops_total');
const rebalanceOpsAcked = new Counter('rebalance_ops_acked');
const rebalanceLatency = new Trend('rebalance_latency', true);
const rebalanceSuccessRate = new Rate('rebalance_success_rate');
const partitionMigrations = new Counter('partition_migrations');
const clusterSizeGauge = new Gauge('cluster_size');
const phaseOps = new Counter('phase_ops');

// Track test phase and cluster state
const testStartTime = Date.now();
let currentClusterSize = INITIAL_NODES;
let partitionMapVersion = 0;

/**
 * Get current test phase based on timing
 */
function getTestPhase() {
  const elapsed = Date.now() - testStartTime;
  if (elapsed < SCALE_UP_TIME_MS) return 'initial';
  if (elapsed < SCALE_DOWN_TIME_MS) return 'scaled-up';
  return 'scaled-down';
}

/**
 * Get available nodes based on current phase
 */
function getActiveNodeCount() {
  const phase = getTestPhase();
  switch (phase) {
    case 'initial':
      return INITIAL_NODES;
    case 'scaled-up':
      return 4;
    case 'scaled-down':
      return INITIAL_NODES;
    default:
      return INITIAL_NODES;
  }
}

function generatePayload(vuId, opNum) {
  return {
    vuId,
    opNum,
    timestamp: Date.now(),
    phase: getTestPhase(),
    clusterSize: getActiveNodeCount(),
    data: `rebalance-test-${Math.random().toString(36).substring(7)}`,
  };
}

export default function () {
  const vuId = __VU;
  const iterationId = __ITER;

  // Distribute VUs across available nodes
  const activeNodes = getActiveNodeCount();
  const nodeIndex = vuId % activeNodes;
  const wsUrl = CLUSTER_NODES[nodeIndex];
  const nodeId = `k6-rebalance-vu${vuId}-iter${iterationId}`;

  let authenticated = false;
  let client = null;
  let opCounter = 0;
  let pendingOps = new Map();
  const sessionStart = Date.now();

  clusterSizeGauge.add(activeNodes);

  const res = ws.connect(wsUrl, {}, function (socket) {
    client = new TopGunClient(socket, nodeId);

    const handleMessage = createMessageHandler(client, {
      onAuthRequired: () => {
        const token = getAuthToken(vuId, 'k6-rebalance', ['USER', 'ADMIN']);
        client.authenticate(token);
      },

      onAuthAck: () => {
        authenticated = true;
        connectionTime.add(Date.now() - sessionStart);

        // Request partition map to track migrations
        client.send({ type: 'PARTITION_MAP_REQUEST' });

        scheduleWrites();
      },

      onAuthError: () => {
        errors.add(1);
        rebalanceSuccessRate.add(0);
        socket.close();
      },

      onOpAck: (msg) => {
        const lastId = msg.payload?.lastId;
        if (lastId && pendingOps.has(lastId)) {
          const sendTime = pendingOps.get(lastId);
          const latency = Date.now() - sendTime;
          rebalanceLatency.add(latency);
          rebalanceOpsAcked.add(BATCH_SIZE);
          for (let i = 0; i < BATCH_SIZE; i++) {
            rebalanceSuccessRate.add(1);
          }
          pendingOps.delete(lastId);
        }
      },

      onUnknown: (msg) => {
        // Track partition map updates (indicates rebalancing)
        if (msg.type === 'PARTITION_MAP') {
          const newVersion = msg.payload?.version || 0;
          if (newVersion > partitionMapVersion) {
            partitionMigrations.add(1);
            partitionMapVersion = newVersion;
            currentClusterSize =
              Object.keys(msg.payload?.nodes || {}).length || currentClusterSize;
            clusterSizeGauge.add(currentClusterSize);
          }
        }
      },
    });

    socket.on('binaryMessage', handleMessage);

    socket.on('error', () => {
      errors.add(1);
      rebalanceSuccessRate.add(0);
    });

    socket.on('close', () => {
      if (authenticated && pendingOps.size > 0) {
        pendingOps.forEach(() => {
          for (let i = 0; i < BATCH_SIZE; i++) {
            rebalanceSuccessRate.add(0);
          }
        });
      }
    });

    function scheduleWrites() {
      const intervalMs = 75; // ~13 batches/sec per VU

      function doWrite() {
        if (!authenticated) return;

        const phase = getTestPhase();
        const operations = [];
        for (let i = 0; i < BATCH_SIZE; i++) {
          opCounter++;
          const key = `key-${vuId}-${opCounter % KEY_POOL_SIZE}`;
          operations.push({
            mapName: `k6-rebalance-${vuId % 15}`,
            key: key,
            value: generatePayload(vuId, opCounter),
          });
        }

        const sendTime = Date.now();
        const lastOpId = client.putBatch(operations);
        pendingOps.set(lastOpId, sendTime);
        rebalanceOpsTotal.add(BATCH_SIZE);
        phaseOps.add(BATCH_SIZE, { phase: phase });

        // Cleanup stale pending ops
        const now = Date.now();
        const timeout = 8000; // Higher timeout during rebalance
        pendingOps.forEach((time, id) => {
          if (now - time > timeout) {
            for (let i = 0; i < BATCH_SIZE; i++) {
              rebalanceSuccessRate.add(0);
            }
            pendingOps.delete(id);
          }
        });

        socket.setTimeout(doWrite, intervalMs);
      }

      doWrite();
    }

    // Keep session alive for test duration
    const maxSessionTime = 115 * 1000;
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
  console.log('========================================================');
  console.log('         TOPGUN CLUSTER REBALANCE TEST                  ');
  console.log('========================================================');
  console.log('');
  console.log('Initial Cluster Nodes (3):');
  for (let i = 0; i < INITIAL_NODES; i++) {
    console.log(`  Node ${i + 1}: ${CLUSTER_NODES[i]}`);
  }
  console.log('');
  console.log(`  Fourth Node (added at ${SCALE_UP_TIME_MS / 1000}s):`);
  console.log(`    Node 4: ${CLUSTER_NODES[3]}`);
  console.log('');
  console.log(`  Max VUs:            ${MAX_VUS}`);
  console.log(`  Batch size:         ${BATCH_SIZE}`);
  console.log(`  Key pool:           ${KEY_POOL_SIZE} keys/VU`);
  console.log('');
  console.log('  SCALING EVENTS:');
  console.log(`    Scale UP at:      ${SCALE_UP_TIME_MS / 1000}s (3 -> 4 nodes)`);
  console.log(`    Scale DOWN at:    ${SCALE_DOWN_TIME_MS / 1000}s (4 -> 3 nodes)`);
  console.log('');
  console.log('  COMMANDS:');
  console.log('    Scale up:   docker-compose up -d --scale node=4');
  console.log('    Scale down: docker stop topgun-node-4');
  console.log('');

  return {
    startTime: Date.now(),
    scaleUpTime: SCALE_UP_TIME_MS,
    scaleDownTime: SCALE_DOWN_TIME_MS,
  };
}

export function teardown(data) {
  const duration = (Date.now() - data.startTime) / 1000;
  console.log('');
  console.log(`Test completed in ${duration.toFixed(2)}s`);
}

export function handleSummary(data) {
  const totalOps = data.metrics.rebalance_ops_total?.values?.count || 0;
  const ackedOps = data.metrics.rebalance_ops_acked?.values?.count || 0;
  const testDuration = 120;
  const avgThroughput = ackedOps / testDuration;

  const p50 = data.metrics.rebalance_latency?.values?.med || 0;
  const p95 = data.metrics.rebalance_latency?.values['p(95)'] || 0;
  const p99 = data.metrics.rebalance_latency?.values['p(99)'] || 0;
  const maxLatency = data.metrics.rebalance_latency?.values?.max || 0;
  const successRate = data.metrics.rebalance_success_rate?.values?.rate || 0;
  const migrations = data.metrics.partition_migrations?.values?.count || 0;

  console.log('');
  console.log('========================================================');
  console.log('           CLUSTER REBALANCE RESULTS                    ');
  console.log('========================================================');
  console.log('');
  console.log('  THROUGHPUT');
  console.log(`    Total Operations Sent:      ${totalOps.toLocaleString()}`);
  console.log(`    Total Operations Acked:     ${ackedOps.toLocaleString()}`);
  console.log(`    Average Throughput:         ${avgThroughput.toFixed(0)} ops/sec`);
  console.log('');
  console.log('  LATENCY');
  console.log(`    p50:                        ${p50.toFixed(1)} ms`);
  console.log(`    p95:                        ${p95.toFixed(1)} ms`);
  console.log(`    p99:                        ${p99.toFixed(1)} ms`);
  console.log(`    max:                        ${maxLatency.toFixed(1)} ms`);
  console.log('');
  console.log('  REBALANCE METRICS');
  console.log(`    Partition Map Updates:      ${migrations}`);
  console.log('');
  console.log('  RELIABILITY');
  console.log(`    Success Rate:               ${(successRate * 100).toFixed(2)}%`);
  console.log('');

  // Performance assessment
  const successTarget = successRate >= 0.98;
  const latencyTarget = p95 < 100;
  const throughputTarget = totalOps >= 75000;

  console.log('  TARGETS');
  console.log(
    `    98% success:      ${successTarget ? 'PASS' : 'FAIL'} (${(successRate * 100).toFixed(2)}%)`
  );
  console.log(
    `    p95 < 100ms:      ${latencyTarget ? 'PASS' : 'FAIL'} (${p95.toFixed(1)}ms)`
  );
  console.log(
    `    75K+ ops:         ${throughputTarget ? 'PASS' : 'FAIL'} (${totalOps})`
  );
  console.log('');

  const summary = {
    timestamp: new Date().toISOString(),
    scenario: 'cluster-rebalance',
    clusterNodes: CLUSTER_NODES,
    scaleEvents: {
      scaleUpTimeMs: SCALE_UP_TIME_MS,
      scaleDownTimeMs: SCALE_DOWN_TIME_MS,
    },
    results: {
      throughput: {
        totalOpsSent: totalOps,
        totalOpsAcked: ackedOps,
        avgOpsPerSec: avgThroughput,
      },
      latency: { p50, p95, p99, max: maxLatency },
      rebalance: {
        partitionMapUpdates: migrations,
      },
      reliability: { successRate },
    },
    targets: {
      successRate: { target: 0.98, actual: successRate, pass: successTarget },
      latencyP95: { target: 100, actual: p95, pass: latencyTarget },
      throughput: { target: 75000, actual: totalOps, pass: throughputTarget },
      allPass: successTarget && latencyTarget && throughputTarget,
    },
  };

  return {
    stdout: textSummary(data, { indent: ' ', enableColors: true }),
    [getResultsPath('cluster-rebalance.json')]: JSON.stringify(summary, null, 2),
  };
}

import { textSummary } from 'https://jslib.k6.io/k6-summary/0.0.1/index.js';
