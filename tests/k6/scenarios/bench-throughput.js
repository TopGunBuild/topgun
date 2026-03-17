/**
 * Clean throughput benchmark — measures actual server capacity.
 *
 * Unlike throughput-test.js, this test:
 * - Does NOT timeout pending ops (counts ALL acks that arrive)
 * - Tracks total acked ops regardless of latency
 * - Reports ops/sec at different VU levels
 *
 * Each VU writes for 15 seconds then closes. k6 manages VU lifecycle.
 */

import ws from 'k6/ws';
import { check } from 'k6';
import { Counter, Trend, Rate } from 'k6/metrics';
import {
  TopGunClient,
  createMessageHandler,
} from '../lib/topgun-client.js';
import {
  getWsUrl,
  getAuthToken,
} from '../lib/config.js';

const WS_URL = getWsUrl();
const BATCH_SIZE = 10;
const SEND_INTERVAL_MS = 50; // 20 batches/sec per VU
const SESSION_SECONDS = 15;  // Each VU writes for 15 seconds

export const options = {
  scenarios: {
    bench: {
      executor: 'ramping-vus',
      startVUs: 10,
      stages: [
        { duration: '5s', target: 50 },
        { duration: '5s', target: 100 },
        { duration: '5s', target: 200 },
        { duration: '10s', target: 200 },  // sustain peak
        { duration: '5s', target: 0 },
      ],
      gracefulRampDown: '0s',
      gracefulStop: '0s',
    },
  },
};

const writeLatency = new Trend('write_latency', true);
const writeOpsAcked = new Counter('write_ops_acked');
const writeOpsTotal = new Counter('write_ops_total');
const ackedBatches = new Counter('acked_batches');
const sentBatches = new Counter('sent_batches');

export default function () {
  const vuId = __VU;
  const nodeId = `bench-vu${vuId}-${__ITER}`;
  let authenticated = false;
  let client = null;
  let opCounter = 0;
  let pendingOps = new Map();
  let writesRemaining = Math.floor(SESSION_SECONDS * 1000 / SEND_INTERVAL_MS);

  const res = ws.connect(WS_URL, {}, function (socket) {
    client = new TopGunClient(socket, nodeId);

    const handleMessage = createMessageHandler(client, {
      onAuthRequired: () => {
        const token = getAuthToken(vuId, 'bench', ['USER', 'ADMIN']);
        client.authenticate(token);
      },

      onAuthAck: () => {
        authenticated = true;
        doWrite();
      },

      onOpAck: (msg) => {
        const lastId = msg.payload?.lastId;
        if (lastId && pendingOps.has(lastId)) {
          const latency = Date.now() - pendingOps.get(lastId);
          writeLatency.add(latency);
          writeOpsAcked.add(BATCH_SIZE);
          ackedBatches.add(1);
          pendingOps.delete(lastId);
        }
      },
    });

    socket.on('binaryMessage', handleMessage);

    function doWrite() {
      if (!authenticated) return;

      writesRemaining--;
      if (writesRemaining <= 0) {
        // Done writing — close immediately
        socket.close();
        return;
      }

      const ops = [];
      for (let i = 0; i < BATCH_SIZE; i++) {
        opCounter++;
        ops.push({
          mapName: `bench-${vuId % 20}`,
          key: `key-${vuId}-${opCounter % 100}`,
          value: { vuId, opCounter, ts: Date.now() },
        });
      }

      const lastOpId = client.putBatch(ops);
      pendingOps.set(lastOpId, Date.now());
      writeOpsTotal.add(BATCH_SIZE);
      sentBatches.add(1);

      socket.setTimeout(doWrite, SEND_INTERVAL_MS);
    }
  });

  check(res, { 'WebSocket connected': (r) => r && r.status === 101 });
}

export function handleSummary(data) {
  const totalSent = data.metrics.write_ops_total?.values?.count || 0;
  const totalAcked = data.metrics.write_ops_acked?.values?.count || 0;
  const batchesSent = data.metrics.sent_batches?.values?.count || 0;
  const batchesAcked = data.metrics.acked_batches?.values?.count || 0;
  const p50 = data.metrics.write_latency?.values?.med || 0;
  const p95 = data.metrics.write_latency?.values?.['p(95)'] || 0;
  const p99 = data.metrics.write_latency?.values?.['p(99)'] || 0;
  const duration = data.state?.testRunDurationMs || 30000;

  console.log('');
  console.log('═══════════════════════════════════════════════');
  console.log('  CLEAN THROUGHPUT BENCHMARK RESULTS');
  console.log('═══════════════════════════════════════════════');
  console.log(`  Batches sent:     ${batchesSent}`);
  console.log(`  Batches acked:    ${batchesAcked}`);
  console.log(`  Ops sent:         ${totalSent}`);
  console.log(`  Ops acked:        ${totalAcked}`);
  if (batchesSent > 0) {
    console.log(`  Batch ack rate:   ${(batchesAcked / batchesSent * 100).toFixed(1)}%`);
  }
  console.log(`  Acked ops/sec:    ${(totalAcked / (duration / 1000)).toFixed(0)}`);
  console.log(`  Latency p50:      ${p50.toFixed(0)}ms`);
  console.log(`  Latency p95:      ${p95.toFixed(0)}ms`);
  console.log(`  Latency p99:      ${p99.toFixed(0)}ms`);
  console.log('═══════════════════════════════════════════════');

  return {};
}
