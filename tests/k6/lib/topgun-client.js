/**
 * TopGun WebSocket Client for k6 Load Testing
 *
 * This library provides a WebSocket client for load testing TopGun servers.
 * Messages are sent as MessagePack binary (same format as production clients).
 * Server responses are also MessagePack encoded.
 *
 * Requires k6 built with xk6-msgpack extension:
 *   xk6 build --with github.com/tango-tango/xk6-msgpack
 *
 * Note: xk6-msgpack encodes all integers as int64, which msgpackr decodes as BigInt.
 * The server's TimestampSchema uses .transform(Number) to handle this automatically.
 *
 * @see tests/e2e/json-fallback.test.ts for protocol examples
 */

import { check } from 'k6';
import { Trend, Counter, Rate } from 'k6/metrics';
import msgpack from 'k6/x/msgpack';

// Custom metrics
export const connectionTime = new Trend('topgun_connection_time', true);
export const authTime = new Trend('topgun_auth_time', true);
export const messageLatency = new Trend('topgun_message_latency', true);
export const messagesReceived = new Counter('topgun_messages_received');
export const messagesSent = new Counter('topgun_messages_sent');
export const errors = new Counter('topgun_errors');
export const authSuccess = new Rate('topgun_auth_success');

/**
 * Encode a JavaScript value to MessagePack binary format
 * Uses native Go implementation via xk6-msgpack for performance
 * @param {any} value - Value to encode
 * @returns {ArrayBuffer} MessagePack encoded bytes
 */
export function encodeMsgPack(value) {
  return msgpack.pack(value);
}

/**
 * Decode MessagePack binary data
 * Uses native Go implementation via xk6-msgpack for performance
 * @param {ArrayBuffer|Uint8Array|string} buffer - MessagePack encoded data
 * @returns {any} Decoded value
 */
export function decodeMsgPack(buffer) {
  return msgpack.unpack(buffer);
}

/**
 * TopGun WebSocket Client
 *
 * Usage:
 * ```js
 * import { TopGunClient } from './lib/topgun-client.js';
 *
 * const client = new TopGunClient(socket, 'my-node-id');
 * client.authenticate(token);
 * client.put('users', 'user-1', { name: 'John' });
 * ```
 */
export class TopGunClient {
  /**
   * @param {WebSocket} socket - k6 WebSocket instance
   * @param {string} nodeId - Unique identifier for this client
   */
  constructor(socket, nodeId) {
    this.socket = socket;
    this.nodeId = nodeId || `k6-client-${__VU}-${Date.now()}`;
    this.isAuthenticated = false;
    this.opCounter = 0;
    this.queryCounter = 0;
    this.pendingResponses = new Map();
    this.heartbeatTimer = null;
    this.lastPingTime = null;
  }

  /**
   * Send a MessagePack-encoded binary message to the server
   * @param {Object} message - Message object to send
   */
  send(message) {
    // msgpack.pack() returns ArrayBuffer directly (native Go encoder)
    const buffer = encodeMsgPack(message);
    this.socket.sendBinary(buffer.buffer || buffer);
    messagesSent.add(1);
  }

  /**
   * Send a JSON message to the server (fallback mode)
   * @param {Object} message - Message object to send
   */
  sendJson(message) {
    const json = JSON.stringify(message);
    this.socket.send(json);
    messagesSent.add(1);
  }

  /**
   * Handle incoming message from server
   * @param {string|ArrayBuffer} data - Raw message data (MessagePack binary)
   * @returns {Object|null} Parsed message or null on error
   */
  parseMessage(data) {
    try {
      messagesReceived.add(1);

      // Decode MessagePack binary data from server
      const msg = decodeMsgPack(data);
      return msg;
    } catch (err) {
      errors.add(1);
      console.error(`Failed to parse message: ${err}`);
      return null;
    }
  }

  /**
   * Send AUTH message with JWT token
   * @param {string} token - JWT token
   */
  authenticate(token) {
    const startTime = Date.now();
    this.send({
      type: 'AUTH',
      token: token,
    });
    this._authStartTime = startTime;
  }

  /**
   * Handle AUTH_ACK response
   */
  onAuthAck() {
    this.isAuthenticated = true;
    if (this._authStartTime) {
      authTime.add(Date.now() - this._authStartTime);
      this._authStartTime = null;
    }
    authSuccess.add(1);
  }

  /**
   * Handle AUTH_ERROR response
   */
  onAuthError() {
    this.isAuthenticated = false;
    authSuccess.add(0);
    errors.add(1);
  }

  /**
   * Send a PUT operation
   * @param {string} mapName - Name of the map
   * @param {string} key - Record key
   * @param {Object} value - Value to store
   * @returns {string} Operation ID
   */
  put(mapName, key, value) {
    const opId = `${this.nodeId}-op-${++this.opCounter}`;
    this.send({
      type: 'CLIENT_OP',
      payload: {
        id: opId,
        mapName: mapName,
        opType: 'PUT',
        key: key,
        record: {
          value: value,
          timestamp: {
            millis: Date.now(),
            counter: this.opCounter,
            nodeId: this.nodeId,
          },
        },
      },
    });
    return opId;
  }

  /**
   * Send a batch of PUT operations
   * @param {Array<{mapName: string, key: string, value: Object}>} operations
   * @returns {string} Last operation ID
   */
  putBatch(operations) {
    const ops = operations.map((op, idx) => ({
      id: `${this.nodeId}-batch-${++this.opCounter}`,
      mapName: op.mapName,
      opType: 'PUT',
      key: op.key,
      record: {
        value: op.value,
        timestamp: {
          millis: Date.now(),
          counter: this.opCounter,
          nodeId: this.nodeId,
        },
      },
    }));

    this.send({
      type: 'OP_BATCH',
      payload: { ops },
    });

    return ops[ops.length - 1].id;
  }

  /**
   * Subscribe to a query
   * @param {string} mapName - Name of the map
   * @param {Object} query - Query filter (empty {} for all)
   * @returns {string} Query ID
   */
  subscribe(mapName, query = {}) {
    const queryId = `${this.nodeId}-query-${++this.queryCounter}`;
    this.send({
      type: 'QUERY_SUB',
      payload: {
        queryId: queryId,
        mapName: mapName,
        query: query,
      },
    });
    return queryId;
  }

  /**
   * Unsubscribe from a query
   * @param {string} queryId - Query ID to unsubscribe
   */
  unsubscribe(queryId) {
    this.send({
      type: 'QUERY_UNSUB',
      payload: { queryId },
    });
  }

  /**
   * Send a PING message
   * @returns {number} Ping timestamp
   */
  ping() {
    const timestamp = Date.now();
    this.lastPingTime = timestamp;
    this.send({
      type: 'PING',
      timestamp: timestamp,
    });
    return timestamp;
  }

  /**
   * Handle PONG response
   * @param {Object} msg - PONG message with timestamp
   */
  onPong(msg) {
    if (this.lastPingTime && msg.timestamp === this.lastPingTime) {
      messageLatency.add(Date.now() - this.lastPingTime);
    }
  }

  /**
   * Start heartbeat interval
   * @param {number} intervalMs - Interval in milliseconds (default: 10000)
   */
  startHeartbeat(intervalMs = 10000) {
    // Note: k6 doesn't support setInterval in the same way
    // Heartbeat should be managed in the test scenario
    this._heartbeatInterval = intervalMs;
  }

  /**
   * Stop heartbeat
   */
  stopHeartbeat() {
    this._heartbeatInterval = null;
  }
}

/**
 * Create a message handler function for k6 WebSocket scenarios
 *
 * @param {TopGunClient} client - TopGunClient instance
 * @param {Object} callbacks - Callback functions for different message types
 * @returns {Function} Message handler function
 */
export function createMessageHandler(client, callbacks = {}) {
  return function (data) {
    const msg = client.parseMessage(data);
    if (!msg) return;

    // Handle specific message types
    switch (msg.type) {
      case 'AUTH_REQUIRED':
        if (callbacks.onAuthRequired) {
          callbacks.onAuthRequired(msg);
        }
        break;

      case 'AUTH_ACK':
        client.onAuthAck();
        if (callbacks.onAuthAck) {
          callbacks.onAuthAck(msg);
        }
        break;

      case 'AUTH_ERROR':
        client.onAuthError();
        if (callbacks.onAuthError) {
          callbacks.onAuthError(msg);
        }
        break;

      case 'PONG':
        client.onPong(msg);
        if (callbacks.onPong) {
          callbacks.onPong(msg);
        }
        break;

      case 'QUERY_RESP':
        if (callbacks.onQueryResponse) {
          callbacks.onQueryResponse(msg);
        }
        break;

      case 'OP_ACK':
        if (callbacks.onOpAck) {
          callbacks.onOpAck(msg);
        }
        break;

      case 'SERVER_STATE':
        if (callbacks.onServerState) {
          callbacks.onServerState(msg);
        }
        break;

      default:
        if (callbacks.onUnknown) {
          callbacks.onUnknown(msg);
        }
    }
  };
}

/**
 * Generate a simple JWT-like token for testing
 * Note: In production, use proper JWT generation or pre-generated tokens
 *
 * @param {string} userId - User ID
 * @param {string[]} roles - User roles
 * @param {string} secret - JWT secret (must match server config)
 * @returns {string} Base64-encoded token (NOT a real JWT - for testing only)
 */
export function generateTestToken(userId, roles = ['USER'], secret = 'test-k6-secret') {
  // k6 doesn't have native JWT support
  // For load testing, use pre-generated tokens or a token generation service
  // This is a placeholder that won't work with real JWT verification

  console.warn(
    'generateTestToken creates mock tokens. Use real JWTs for production testing.',
  );

  const header = { alg: 'HS256', typ: 'JWT' };
  const payload = {
    userId: userId,
    roles: roles,
    sub: userId,
    iat: Math.floor(Date.now() / 1000),
    exp: Math.floor(Date.now() / 1000) + 3600,
  };

  // This won't produce valid signatures - use external token generation
  const base64Header = __base64Encode(JSON.stringify(header));
  const base64Payload = __base64Encode(JSON.stringify(payload));

  return `${base64Header}.${base64Payload}.mock-signature`;
}

/**
 * Base64 encode helper
 * @private
 */
function __base64Encode(str) {
  const chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  let result = '';
  const bytes = new TextEncoder().encode(str);

  for (let i = 0; i < bytes.length; i += 3) {
    const b1 = bytes[i];
    const b2 = bytes[i + 1] || 0;
    const b3 = bytes[i + 2] || 0;

    result += chars[b1 >> 2];
    result += chars[((b1 & 3) << 4) | (b2 >> 4)];
    result += i + 1 < bytes.length ? chars[((b2 & 15) << 2) | (b3 >> 6)] : '=';
    result += i + 2 < bytes.length ? chars[b3 & 63] : '=';
  }

  return result;
}

/**
 * Default export with all utilities
 */
export default {
  TopGunClient,
  createMessageHandler,
  generateTestToken,
  encodeMsgPack,
  decodeMsgPack,
  // Metrics
  connectionTime,
  authTime,
  messageLatency,
  messagesReceived,
  messagesSent,
  errors,
  authSuccess,
};
