import { ServerCoordinator, ServerCoordinatorConfig } from '@topgunbuild/server';
import { TopGunClient } from '@topgunbuild/client';
import { serialize, deserialize } from '@topgunbuild/core';
import * as jwt from 'jsonwebtoken';
import WebSocket from 'ws';
import { MemoryStorageAdapter } from './MemoryStorageAdapter';

const JWT_SECRET = 'test-e2e-secret';

export interface TestContext {
  server: ServerCoordinator;
  clients: TestClient[];
  cleanup: () => Promise<void>;
}

export interface TestClient {
  ws: WebSocket;
  storage: MemoryStorageAdapter;
  nodeId: string;
  messages: any[];
  isAuthenticated: boolean;
  send: (message: any) => void;
  waitForMessage: (type: string, timeout?: number) => Promise<any>;
  close: () => void;
  startHeartbeat: () => void;
  stopHeartbeat: () => void;
}

let portCounter = 10000;

function getNextPort(): number {
  return portCounter++;
}

/**
 * Creates a test server with in-memory storage
 */
export async function createTestServer(
  overrides: Partial<ServerCoordinatorConfig> = {}
): Promise<ServerCoordinator> {
  const server = new ServerCoordinator({
    port: 0, // Let OS assign port
    nodeId: `test-server-${Date.now()}`,
    host: 'localhost',
    clusterPort: 0,
    metricsPort: 0, // Let OS assign port to avoid EADDRINUSE
    peers: [],
    jwtSecret: JWT_SECRET,
    ...overrides,
  });

  await server.ready();
  return server;
}

/**
 * Creates a test client connected to the server
 */
export async function createTestClient(
  serverUrl: string,
  options: { nodeId?: string; autoAuth?: boolean; userId?: string; roles?: string[] } = {}
): Promise<TestClient> {
  const nodeId = options.nodeId || `test-client-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  const storage = new MemoryStorageAdapter();
  await storage.initialize('test-db');

  const messages: any[] = [];
  let isAuthenticated = false;
  let resolvers: Map<string, { resolve: (value: any) => void; timeout: NodeJS.Timeout }> = new Map();
  let heartbeatInterval: NodeJS.Timeout | null = null;

  return new Promise((resolve, reject) => {
    const ws = new WebSocket(serverUrl);
    ws.binaryType = 'arraybuffer';

    const client: TestClient = {
      ws,
      storage,
      nodeId,
      messages,
      isAuthenticated,

      send: (message: any) => {
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(serialize(message));
        }
      },

      waitForMessage: (type: string, timeout = 5000): Promise<any> => {
        // Check if message already received
        const existing = messages.find(m => m.type === type);
        if (existing) {
          return Promise.resolve(existing);
        }

        return new Promise((res, rej) => {
          const timer = setTimeout(() => {
            resolvers.delete(type);
            rej(new Error(`Timeout waiting for message type: ${type}`));
          }, timeout);

          resolvers.set(type, { resolve: res, timeout: timer });
        });
      },

      startHeartbeat: () => {
        if (heartbeatInterval) return;
        // Send PING every 10 seconds (server timeout is 20 seconds)
        heartbeatInterval = setInterval(() => {
          if (ws.readyState === WebSocket.OPEN) {
            ws.send(serialize({ type: 'PING', timestamp: Date.now() }));
          }
        }, 10000);
      },

      stopHeartbeat: () => {
        if (heartbeatInterval) {
          clearInterval(heartbeatInterval);
          heartbeatInterval = null;
        }
      },

      close: () => {
        if (heartbeatInterval) {
          clearInterval(heartbeatInterval);
          heartbeatInterval = null;
        }
        ws.close();
        for (const { timeout } of resolvers.values()) {
          clearTimeout(timeout);
        }
        resolvers.clear();
      },
    };

    ws.on('open', () => {
      // Connection opened, wait for AUTH_REQUIRED
    });

    ws.on('message', (data: ArrayBuffer | Buffer) => {
      try {
        const buf = data instanceof ArrayBuffer ? new Uint8Array(data) : data;
        const message = deserialize(buf as Uint8Array) as { type: string; [key: string]: any };

        // Process a single message (handles resolvers and special messages)
        const processMessage = (msg: { type: string; [key: string]: any }) => {
          messages.push(msg);

          // Check if someone is waiting for this message type
          const resolver = resolvers.get(msg.type);
          if (resolver) {
            clearTimeout(resolver.timeout);
            resolvers.delete(msg.type);
            resolver.resolve(msg);
          }

          // Auto-auth handling
          if (msg.type === 'AUTH_REQUIRED' && options.autoAuth !== false) {
            const token = createTestToken(
              options.userId || nodeId,
              options.roles || ['USER']
            );
            client.send({ type: 'AUTH', token });
          }

          if (msg.type === 'AUTH_ACK') {
            client.isAuthenticated = true;
            // Start heartbeat after successful authentication
            client.startHeartbeat();
          }
        };

        // Handle BATCH messages from CoalescingWriter
        if (message.type === 'BATCH') {
          const batchData = message.data as Uint8Array;
          const view = new DataView(batchData.buffer, batchData.byteOffset, batchData.byteLength);
          let offset = 0;

          const count = view.getUint32(offset, true); // little-endian
          offset += 4;

          for (let i = 0; i < count; i++) {
            const msgLen = view.getUint32(offset, true);
            offset += 4;

            const msgData = batchData.slice(offset, offset + msgLen);
            offset += msgLen;

            const innerMsg = deserialize(msgData) as { type: string; [key: string]: any };
            processMessage(innerMsg);
          }
        } else {
          processMessage(message);
        }
      } catch (err) {
        console.error('Failed to parse message:', err);
      }
    });

    ws.on('error', (err) => {
      reject(err);
    });

    ws.on('close', () => {
      // Cleanup - stop heartbeat when connection closes
      client.stopHeartbeat();
    });

    // Wait for connection to be established
    ws.on('open', () => {
      resolve(client);
    });
  });
}

/**
 * Creates a JWT token for testing
 */
export function createTestToken(userId: string, roles: string[] = ['USER']): string {
  return jwt.sign(
    { userId, roles, sub: userId },
    JWT_SECRET,
    { expiresIn: '1h' }
  );
}

/**
 * Waits for synchronization to complete
 */
export function waitForSync(ms = 100): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Waits until a condition is met
 */
export async function waitUntil(
  condition: () => boolean | Promise<boolean>,
  timeout = 5000,
  interval = 50
): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    if (await condition()) {
      return;
    }
    await waitForSync(interval);
  }
  throw new Error('waitUntil timeout');
}

/**
 * Creates a complete test context with server and clients
 */
export async function createTestContext(
  numClients = 1,
  serverConfig: Partial<ServerCoordinatorConfig> = {}
): Promise<TestContext> {
  const server = await createTestServer(serverConfig);
  const serverUrl = `ws://localhost:${server.port}`;

  const clients: TestClient[] = [];
  for (let i = 0; i < numClients; i++) {
    const client = await createTestClient(serverUrl, {
      nodeId: `client-${i}`,
      userId: `user-${i}`,
      roles: ['ADMIN'],
    });
    clients.push(client);
  }

  // Wait for all clients to authenticate
  await Promise.all(clients.map((c) => c.waitForMessage('AUTH_ACK', 5000)));

  return {
    server,
    clients,
    cleanup: async () => {
      for (const client of clients) {
        client.close();
      }
      await server.shutdown();
    },
  };
}

/**
 * Creates an LWW record for testing
 */
export function createLWWRecord<T>(value: T, nodeId = 'test-node'): any {
  return {
    value,
    timestamp: {
      millis: Date.now(),
      counter: 0,
      nodeId,
    },
  };
}

/**
 * Creates an OR record for testing
 */
export function createORRecord<T>(value: T, nodeId = 'test-node'): any {
  const ts = Date.now();
  return {
    value,
    timestamp: {
      millis: ts,
      counter: 0,
      nodeId,
    },
    tag: `${nodeId}-${ts}-0`,
  };
}

export { MemoryStorageAdapter };
