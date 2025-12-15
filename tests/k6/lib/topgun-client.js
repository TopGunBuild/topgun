/**
 * TopGun WebSocket Client for k6 Load Testing
 *
 * This library provides a WebSocket client for load testing TopGun servers.
 * All messages are sent as JSON (server supports JSON fallback when MessagePack fails).
 *
 * @see tests/e2e/json-fallback.test.ts for protocol examples
 */

import { check } from 'k6';
import { Trend, Counter, Rate } from 'k6/metrics';

// Custom metrics
export const connectionTime = new Trend('topgun_connection_time', true);
export const authTime = new Trend('topgun_auth_time', true);
export const messageLatency = new Trend('topgun_message_latency', true);
export const messagesReceived = new Counter('topgun_messages_received');
export const messagesSent = new Counter('topgun_messages_sent');
export const errors = new Counter('topgun_errors');
export const authSuccess = new Rate('topgun_auth_success');

/**
 * Minimal MessagePack decoder for TopGun server responses.
 * Supports the subset of MessagePack used by TopGun protocol.
 */
class MsgPackDecoder {
  constructor(buffer) {
    // Handle both ArrayBuffer and Uint8Array
    if (buffer instanceof ArrayBuffer) {
      this.data = new Uint8Array(buffer);
    } else if (buffer instanceof Uint8Array) {
      this.data = buffer;
    } else if (typeof buffer === 'string') {
      // k6 may pass binary as string - convert each char to byte
      const bytes = new Uint8Array(buffer.length);
      for (let i = 0; i < buffer.length; i++) {
        bytes[i] = buffer.charCodeAt(i) & 0xff;
      }
      this.data = bytes;
    } else {
      throw new Error('Unsupported buffer type');
    }
    this.offset = 0;
  }

  decode() {
    if (this.offset >= this.data.length) {
      throw new Error('Unexpected end of buffer');
    }

    const type = this.data[this.offset++];

    // Positive fixint (0x00 - 0x7f)
    if (type <= 0x7f) {
      return type;
    }

    // Fixmap (0x80 - 0x8f)
    if (type >= 0x80 && type <= 0x8f) {
      return this.readMap(type - 0x80);
    }

    // Fixarray (0x90 - 0x9f)
    if (type >= 0x90 && type <= 0x9f) {
      return this.readArray(type - 0x90);
    }

    // Fixstr (0xa0 - 0xbf)
    if (type >= 0xa0 && type <= 0xbf) {
      return this.readString(type - 0xa0);
    }

    // Nil
    if (type === 0xc0) {
      return null;
    }

    // False
    if (type === 0xc2) {
      return false;
    }

    // True
    if (type === 0xc3) {
      return true;
    }

    // bin8
    if (type === 0xc4) {
      const len = this.data[this.offset++];
      return this.readBytes(len);
    }

    // bin16
    if (type === 0xc5) {
      const len = this.readUint16();
      return this.readBytes(len);
    }

    // bin32
    if (type === 0xc6) {
      const len = this.readUint32();
      return this.readBytes(len);
    }

    // float32
    if (type === 0xca) {
      const view = new DataView(this.data.buffer, this.data.byteOffset + this.offset, 4);
      this.offset += 4;
      return view.getFloat32(0, false);
    }

    // float64
    if (type === 0xcb) {
      const view = new DataView(this.data.buffer, this.data.byteOffset + this.offset, 8);
      this.offset += 8;
      return view.getFloat64(0, false);
    }

    // uint8
    if (type === 0xcc) {
      return this.data[this.offset++];
    }

    // uint16
    if (type === 0xcd) {
      return this.readUint16();
    }

    // uint32
    if (type === 0xce) {
      return this.readUint32();
    }

    // uint64
    if (type === 0xcf) {
      return this.readUint64();
    }

    // int8
    if (type === 0xd0) {
      const val = this.data[this.offset++];
      return val > 127 ? val - 256 : val;
    }

    // int16
    if (type === 0xd1) {
      const view = new DataView(this.data.buffer, this.data.byteOffset + this.offset, 2);
      this.offset += 2;
      return view.getInt16(0, false);
    }

    // int32
    if (type === 0xd2) {
      const view = new DataView(this.data.buffer, this.data.byteOffset + this.offset, 4);
      this.offset += 4;
      return view.getInt32(0, false);
    }

    // int64
    if (type === 0xd3) {
      return this.readInt64();
    }

    // str8
    if (type === 0xd9) {
      const len = this.data[this.offset++];
      return this.readString(len);
    }

    // str16
    if (type === 0xda) {
      const len = this.readUint16();
      return this.readString(len);
    }

    // str32
    if (type === 0xdb) {
      const len = this.readUint32();
      return this.readString(len);
    }

    // array16
    if (type === 0xdc) {
      const len = this.readUint16();
      return this.readArray(len);
    }

    // array32
    if (type === 0xdd) {
      const len = this.readUint32();
      return this.readArray(len);
    }

    // map16
    if (type === 0xde) {
      const len = this.readUint16();
      return this.readMap(len);
    }

    // map32
    if (type === 0xdf) {
      const len = this.readUint32();
      return this.readMap(len);
    }

    // Negative fixint (0xe0 - 0xff)
    if (type >= 0xe0) {
      return type - 256;
    }

    throw new Error(`Unknown MessagePack type: 0x${type.toString(16)}`);
  }

  readUint16() {
    const val = (this.data[this.offset] << 8) | this.data[this.offset + 1];
    this.offset += 2;
    return val;
  }

  readUint32() {
    const val =
      (this.data[this.offset] << 24) |
      (this.data[this.offset + 1] << 16) |
      (this.data[this.offset + 2] << 8) |
      this.data[this.offset + 3];
    this.offset += 4;
    return val >>> 0; // Convert to unsigned
  }

  readUint64() {
    // JavaScript can't handle full 64-bit integers, but we can handle timestamps
    const high = this.readUint32();
    const low = this.readUint32();
    // For timestamps that fit in 53 bits (safe integer range)
    return high * 0x100000000 + low;
  }

  readInt64() {
    const high = this.readUint32();
    const low = this.readUint32();
    // Check sign bit
    if (high & 0x80000000) {
      // Negative number - this is a simplification
      return -(~high * 0x100000000 + ~low + 1);
    }
    return high * 0x100000000 + low;
  }

  readBytes(len) {
    const bytes = this.data.slice(this.offset, this.offset + len);
    this.offset += len;
    return bytes;
  }

  readString(len) {
    const bytes = this.data.slice(this.offset, this.offset + len);
    this.offset += len;

    // UTF-8 decode
    let str = '';
    for (let i = 0; i < bytes.length; ) {
      const byte = bytes[i];
      if (byte < 0x80) {
        str += String.fromCharCode(byte);
        i++;
      } else if (byte < 0xe0) {
        str += String.fromCharCode(((byte & 0x1f) << 6) | (bytes[i + 1] & 0x3f));
        i += 2;
      } else if (byte < 0xf0) {
        str += String.fromCharCode(
          ((byte & 0x0f) << 12) | ((bytes[i + 1] & 0x3f) << 6) | (bytes[i + 2] & 0x3f),
        );
        i += 3;
      } else {
        // 4-byte UTF-8 (surrogate pair)
        const codepoint =
          ((byte & 0x07) << 18) |
          ((bytes[i + 1] & 0x3f) << 12) |
          ((bytes[i + 2] & 0x3f) << 6) |
          (bytes[i + 3] & 0x3f);
        const adjusted = codepoint - 0x10000;
        str += String.fromCharCode(0xd800 + (adjusted >> 10), 0xdc00 + (adjusted & 0x3ff));
        i += 4;
      }
    }
    return str;
  }

  readArray(len) {
    const arr = [];
    for (let i = 0; i < len; i++) {
      arr.push(this.decode());
    }
    return arr;
  }

  readMap(len) {
    const obj = {};
    for (let i = 0; i < len; i++) {
      const key = this.decode();
      const value = this.decode();
      obj[key] = value;
    }
    return obj;
  }
}

/**
 * Decode MessagePack binary data
 * @param {ArrayBuffer|Uint8Array|string} buffer - MessagePack encoded data
 * @returns {any} Decoded value
 */
export function decodeMsgPack(buffer) {
  const decoder = new MsgPackDecoder(buffer);
  return decoder.decode();
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
   * Send a JSON message to the server
   * @param {Object} message - Message object to send
   */
  send(message) {
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
