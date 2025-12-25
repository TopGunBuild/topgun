/**
 * Smoke Test Scenario for TopGun Server
 *
 * Basic load test to verify TopGun server can handle concurrent connections.
 * - 10 VUs (Virtual Users)
 * - 30 seconds duration
 * - Each VU: connects, authenticates, performs one PUT, disconnects
 *
 * Run:
 *   k6 run tests/k6/scenarios/smoke.js
 *
 * With custom server:
 *   k6 run tests/k6/scenarios/smoke.js -e WS_URL=ws://localhost:3000
 *   k6 run tests/k6/scenarios/smoke.js -e JWT_TOKEN=<your-token>
 */

import ws from 'k6/ws';
import { check, sleep } from 'k6';
import { Counter, Trend, Rate } from 'k6/metrics';
import {
  TopGunClient,
  createMessageHandler,
  connectionTime,
  authTime,
  messageLatency,
  authSuccess,
  errors,
} from '../lib/topgun-client.js';

// Test configuration
export const options = {
  vus: 10,
  duration: '30s',
  thresholds: {
    // 95% of connections should complete within 1s
    topgun_connection_time: ['p(95)<1000'],
    // 95% of auth should complete within 500ms
    topgun_auth_time: ['p(95)<500'],
    // 99% auth success rate
    topgun_auth_success: ['rate>0.99'],
    // Less than 1% errors
    topgun_errors: ['count<10'],
  },
};

// Custom metrics for this scenario
const putOperations = new Counter('smoke_put_operations');
const successfulSessions = new Counter('smoke_successful_sessions');
const sessionDuration = new Trend('smoke_session_duration', true);

// Configuration from environment
const WS_URL = __ENV.WS_URL || 'ws://localhost:8080';
const JWT_TOKEN = __ENV.JWT_TOKEN || null;

/**
 * Generate a test JWT token
 * For real load testing, provide pre-generated tokens via JWT_TOKEN env var
 */
function getAuthToken(vuId) {
  if (JWT_TOKEN) {
    return JWT_TOKEN;
  }

  // Fallback: This creates a mock token that won't work with real JWT verification
  // For testing, configure server with matching secret or use pre-generated tokens
  console.warn(`VU ${vuId}: Using mock token. Set JWT_TOKEN for production testing.`);

  // Create a basic JWT structure (header.payload.signature)
  // Server must be configured with 'test-k6-secret' or verification disabled
  const header = JSON.stringify({ alg: 'HS256', typ: 'JWT' });
  const payload = JSON.stringify({
    userId: `k6-user-${vuId}`,
    roles: ['USER', 'ADMIN'],
    sub: `k6-user-${vuId}`,
    iat: Math.floor(Date.now() / 1000),
    exp: Math.floor(Date.now() / 1000) + 3600,
  });

  // Base64URL encode (simplified)
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

  return `${b64(header)}.${b64(payload)}.mock-signature-for-testing`;
}

/**
 * Main test function - runs for each VU
 */
export default function () {
  const sessionStart = Date.now();
  const vuId = __VU;
  const iterationId = __ITER;
  const nodeId = `k6-vu${vuId}-iter${iterationId}`;

  let authenticated = false;
  let putCompleted = false;
  let client = null;

  const connectStart = Date.now();

  const res = ws.connect(WS_URL, {}, function (socket) {
    client = new TopGunClient(socket, nodeId);

    // Set up message handler
    const handleMessage = createMessageHandler(client, {
      onAuthRequired: () => {
        // Send authentication
        const token = getAuthToken(vuId);
        client.authenticate(token);
      },

      onAuthAck: () => {
        authenticated = true;
        connectionTime.add(Date.now() - connectStart);

        // Perform PUT operation using batch (which gets OP_ACK)
        client.putBatch([
          {
            mapName: 'k6-smoke-test',
            key: `key-${nodeId}`,
            value: {
              vuId: vuId,
              iteration: iterationId,
              timestamp: Date.now(),
              data: 'smoke test data',
            },
          },
        ]);
        putOperations.add(1);

        // Send a ping to measure latency
        client.ping();
      },

      onAuthError: (msg) => {
        console.error(`VU ${vuId}: Auth failed`);
        errors.add(1);
        socket.close();
      },

      onOpAck: () => {
        putCompleted = true;
        // Wait a bit then close
        socket.close();
      },

      onPong: () => {
        // Latency recorded in client.onPong
      },
    });

    // k6 uses 'binaryMessage' for binary data (MessagePack responses)
    socket.on('binaryMessage', handleMessage);

    socket.on('error', (e) => {
      console.error(`VU ${vuId}: WebSocket error: ${e}`);
      errors.add(1);
    });

    socket.on('close', () => {
      // Session complete
    });

    // Timeout - close if not completed in 10 seconds
    socket.setTimeout(function () {
      if (!putCompleted) {
        console.warn(`VU ${vuId}: Session timeout`);
        socket.close();
      }
    }, 10000);
  });

  // Check connection was successful
  check(res, {
    'WebSocket connected': (r) => r && r.status === 101,
  });

  // Record session metrics
  const sessionEnd = Date.now();
  sessionDuration.add(sessionEnd - sessionStart);

  if (authenticated && putCompleted) {
    successfulSessions.add(1);
  }

  // Small delay between iterations
  sleep(0.5);
}

/**
 * Setup function - runs once before test starts
 */
export function setup() {
  console.log(`Starting smoke test against ${WS_URL}`);
  console.log(`VUs: ${options.vus}, Duration: ${options.duration}`);

  if (!JWT_TOKEN) {
    console.warn('No JWT_TOKEN provided. Using mock tokens.');
    console.warn('For production testing, set: -e JWT_TOKEN=<your-jwt-token>');
  }

  return {
    startTime: Date.now(),
    wsUrl: WS_URL,
  };
}

/**
 * Teardown function - runs once after test completes
 */
export function teardown(data) {
  const duration = (Date.now() - data.startTime) / 1000;
  console.log(`Test completed in ${duration.toFixed(2)}s`);
}

/**
 * Handle summary - customize output
 */
export function handleSummary(data) {
  const summary = {
    timestamp: new Date().toISOString(),
    wsUrl: WS_URL,
    metrics: {
      // Connection metrics
      connectionTime: {
        avg: data.metrics.topgun_connection_time?.values?.avg || 0,
        p95: data.metrics.topgun_connection_time?.values['p(95)'] || 0,
      },
      authTime: {
        avg: data.metrics.topgun_auth_time?.values?.avg || 0,
        p95: data.metrics.topgun_auth_time?.values['p(95)'] || 0,
      },
      // Success rates
      authSuccessRate: data.metrics.topgun_auth_success?.values?.rate || 0,
      // Counts
      messagesSent: data.metrics.topgun_messages_sent?.values?.count || 0,
      messagesReceived: data.metrics.topgun_messages_received?.values?.count || 0,
      putOperations: data.metrics.smoke_put_operations?.values?.count || 0,
      successfulSessions: data.metrics.smoke_successful_sessions?.values?.count || 0,
      errors: data.metrics.topgun_errors?.values?.count || 0,
    },
    thresholds: data.thresholds || {},
  };

  return {
    stdout: textSummary(data, { indent: ' ', enableColors: true }),
    'tests/k6/results/smoke-summary.json': JSON.stringify(summary, null, 2),
  };
}

// Import text summary helper
import { textSummary } from 'https://jslib.k6.io/k6-summary/0.0.1/index.js';
