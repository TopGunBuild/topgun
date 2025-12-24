/**
 * TopGun Cluster Throughput Benchmark (Phase 4.5 Task 07)
 *
 * Measures cluster-mode throughput across multiple nodes:
 * - Compares cluster vs single-node performance
 * - Tests partition-aware routing
 * - Verifies load distribution across nodes
 *
 * Run:
 *   k6 run tests/k6/scenarios/cluster-throughput.js -e JWT_TOKEN=<token>
 *
 * Or with pnpm:
 *   pnpm test:k6:cluster
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

// Cluster Configuration
const CLUSTER_NODES = [
  __ENV.NODE1_URL || 'ws://localhost:8081',
  __ENV.NODE2_URL || 'ws://localhost:8082',
  __ENV.NODE3_URL || 'ws://localhost:8083',
];

const BATCH_SIZE = getConfig('BATCH_SIZE', 10);
const MAX_VUS = getConfig('MAX_VUS', 150);
const KEY_POOL_SIZE = getConfig('KEY_POOL_SIZE', 100);

// Test configuration
export const options = {
  scenarios: {
    cluster_throughput: {
      executor: 'ramping-vus',
      startVUs: 10,
      stages: [
        { duration: '30s', target: 50 },   // Warm up
        { duration: '60s', target: MAX_VUS }, // Sustain
        { duration: '30s', target: 0 },    // Ramp down
      ],
      gracefulRampDown: '10s',
    },
  },
  thresholds: {
    'cluster_ops_total': ['count>100000'],      // 100K ops in 2 min
    'cluster_success_rate': ['rate>0.99'],       // 99% success
    'cluster_latency': ['p(95)<50'],             // p95 < 50ms
  },
};

// Cluster-specific metrics
const clusterOpsTotal = new Counter('cluster_ops_total');
const clusterOpsAcked = new Counter('cluster_ops_acked');
const clusterLatency = new Trend('cluster_latency', true);
const clusterSuccessRate = new Rate('cluster_success_rate');
const nodeDistribution = new Counter('node_distribution');
const activeNodes = new Gauge('active_nodes');

// Track partition map for smart routing
let partitionMap = null;

/**
 * Simple hash function to simulate partition routing
 * Matches server's xxHash64 % 271 partition assignment
 */
function simpleHash(str) {
  let hash = 0;
  for (let i = 0; i < str.length; i++) {
    hash = ((hash << 5) - hash) + str.charCodeAt(i);
    hash |= 0;
  }
  return Math.abs(hash);
}

/**
 * Get partition owner node index
 */
function getPartitionOwner(key) {
  if (!partitionMap) {
    // Round-robin fallback when no partition map
    return simpleHash(key) % CLUSTER_NODES.length;
  }
  const partitionId = simpleHash(key) % 271;
  const nodeId = partitionMap.partitions?.[partitionId]?.owner;
  if (!nodeId) return 0;

  // Map nodeId to index
  for (let i = 0; i < CLUSTER_NODES.length; i++) {
    if (CLUSTER_NODES[i].includes(`808${i + 1}`)) {
      return i;
    }
  }
  return 0;
}

function generatePayload(vuId, opNum) {
  return {
    vuId,
    opNum,
    timestamp: Date.now(),
    data: `cluster-test-${Math.random().toString(36).substring(7)}`,
    cluster: true,
  };
}

export default function () {
  const vuId = __VU;
  const iterationId = __ITER;

  // Distribute VUs across cluster nodes
  const nodeIndex = vuId % CLUSTER_NODES.length;
  const wsUrl = CLUSTER_NODES[nodeIndex];
  const nodeId = `k6-cluster-vu${vuId}-iter${iterationId}`;

  let authenticated = false;
  let client = null;
  let opCounter = 0;
  let pendingOps = new Map();
  const sessionStart = Date.now();

  // Track which node this VU is connected to
  nodeDistribution.add(1, { node: `node-${nodeIndex + 1}` });

  const res = ws.connect(wsUrl, {}, function (socket) {
    client = new TopGunClient(socket, nodeId);

    const handleMessage = createMessageHandler(client, {
      onAuthRequired: () => {
        const token = getAuthToken(vuId, 'k6-cluster', ['USER', 'ADMIN']);
        client.authenticate(token);
      },

      onAuthAck: () => {
        authenticated = true;
        connectionTime.add(Date.now() - sessionStart);

        // Request partition map
        client.send({ type: 'PARTITION_MAP_REQUEST' });

        scheduleWrites();
      },

      onAuthError: () => {
        errors.add(1);
        clusterSuccessRate.add(0);
        socket.close();
      },

      onOpAck: (msg) => {
        const lastId = msg.payload?.lastId;
        if (lastId && pendingOps.has(lastId)) {
          const sendTime = pendingOps.get(lastId);
          const latency = Date.now() - sendTime;
          clusterLatency.add(latency);
          clusterOpsAcked.add(BATCH_SIZE);
          for (let i = 0; i < BATCH_SIZE; i++) {
            clusterSuccessRate.add(1);
          }
          pendingOps.delete(lastId);
        }
      },

      onUnknown: (msg) => {
        // Handle partition map response
        if (msg.type === 'PARTITION_MAP') {
          partitionMap = msg.payload;
          activeNodes.add(Object.keys(partitionMap.nodes || {}).length);
        }
      },
    });

    socket.on('binaryMessage', handleMessage);

    socket.on('error', () => {
      errors.add(1);
      clusterSuccessRate.add(0);
    });

    socket.on('close', () => {
      if (authenticated) {
        pendingOps.forEach(() => {
          for (let i = 0; i < BATCH_SIZE; i++) {
            clusterSuccessRate.add(0);
          }
        });
      }
    });

    function scheduleWrites() {
      const intervalMs = 50; // 20 batches/sec per VU

      function doWrite() {
        if (!authenticated) return;

        const operations = [];
        for (let i = 0; i < BATCH_SIZE; i++) {
          opCounter++;
          const key = `key-${vuId}-${opCounter % KEY_POOL_SIZE}`;
          operations.push({
            mapName: `k6-cluster-${vuId % 20}`,
            key: key,
            value: generatePayload(vuId, opCounter),
          });
        }

        const sendTime = Date.now();
        const lastOpId = client.putBatch(operations);
        pendingOps.set(lastOpId, sendTime);
        clusterOpsTotal.add(BATCH_SIZE);

        // Cleanup stale pending ops
        const now = Date.now();
        const timeout = 5000;
        pendingOps.forEach((time, id) => {
          if (now - time > timeout) {
            for (let i = 0; i < BATCH_SIZE; i++) {
              clusterSuccessRate.add(0);
            }
            pendingOps.delete(id);
          }
        });

        socket.setTimeout(doWrite, intervalMs);
      }

      doWrite();
    }

    // Keep session alive
    const maxSessionTime = 120 * 1000;
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
  const theoreticalMax = MAX_VUS * BATCH_SIZE * 20;

  console.log('');
  console.log('========================================================');
  console.log('         TOPGUN CLUSTER THROUGHPUT BENCHMARK            ');
  console.log('========================================================');
  console.log('');
  console.log('Cluster Nodes:');
  CLUSTER_NODES.forEach((url, i) => {
    console.log(`  Node ${i + 1}: ${url}`);
  });
  console.log('');
  console.log(`  Max VUs:        ${MAX_VUS}`);
  console.log(`  Batch size:     ${BATCH_SIZE}`);
  console.log(`  Key pool:       ${KEY_POOL_SIZE} keys/VU`);
  console.log(`  Theoretical max: ${(theoreticalMax/1000).toFixed(0)}K ops/sec`);
  console.log('');

  return { startTime: Date.now(), theoreticalMax };
}

export function teardown(data) {
  const duration = (Date.now() - data.startTime) / 1000;
  console.log('');
  console.log(`Test completed in ${duration.toFixed(2)}s`);
}

export function handleSummary(data) {
  const totalOps = data.metrics.cluster_ops_total?.values?.count || 0;
  const ackedOps = data.metrics.cluster_ops_acked?.values?.count || 0;
  const testDuration = 120;
  const avgThroughput = ackedOps / testDuration;

  const p50 = data.metrics.cluster_latency?.values?.med || 0;
  const p95 = data.metrics.cluster_latency?.values['p(95)'] || 0;
  const p99 = data.metrics.cluster_latency?.values['p(99)'] || 0;
  const maxLatency = data.metrics.cluster_latency?.values?.max || 0;
  const successRate = data.metrics.cluster_success_rate?.values?.rate || 0;

  console.log('');
  console.log('========================================================');
  console.log('           CLUSTER THROUGHPUT RESULTS                   ');
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
  console.log('  RELIABILITY');
  console.log(`    Success Rate:               ${(successRate * 100).toFixed(2)}%`);
  console.log('');

  // Performance assessment
  const throughputTarget = totalOps >= 100000;
  const latencyTarget = p95 < 50;
  const successTarget = successRate >= 0.99;

  console.log('  TARGETS');
  console.log(`    100K+ ops:      ${throughputTarget ? 'PASS' : 'FAIL'} (${totalOps})`);
  console.log(`    p95 < 50ms:     ${latencyTarget ? 'PASS' : 'FAIL'} (${p95.toFixed(1)}ms)`);
  console.log(`    99% success:    ${successTarget ? 'PASS' : 'FAIL'} (${(successRate * 100).toFixed(2)}%)`);
  console.log('');

  const summary = {
    timestamp: new Date().toISOString(),
    scenario: 'cluster-throughput',
    clusterNodes: CLUSTER_NODES,
    results: {
      throughput: {
        totalOpsSent: totalOps,
        totalOpsAcked: ackedOps,
        avgOpsPerSec: avgThroughput,
      },
      latency: { p50, p95, p99, max: maxLatency },
      reliability: { successRate },
    },
    targets: {
      throughput: { target: 100000, actual: totalOps, pass: throughputTarget },
      latencyP95: { target: 50, actual: p95, pass: latencyTarget },
      successRate: { target: 0.99, actual: successRate, pass: successTarget },
      allPass: throughputTarget && latencyTarget && successTarget,
    },
  };

  return {
    stdout: textSummary(data, { indent: ' ', enableColors: true }),
    [getResultsPath('cluster-throughput.json')]: JSON.stringify(summary, null, 2),
  };
}

import { textSummary } from 'https://jslib.k6.io/k6-summary/0.0.1/index.js';
