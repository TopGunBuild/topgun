/**
 * Cluster E2E Test Helpers
 *
 * Utilities for creating, managing, and testing multi-node TopGun clusters.
 */

import { ServerCoordinator, ServerCoordinatorConfig, ServerFactory } from '@topgunbuild/server';
import { TopGunClient, TopGunClientConfig } from '@topgunbuild/client';
import { ConsistencyLevel, LWWMap, ORMap, serialize, deserialize } from '@topgunbuild/core';
import WebSocket from 'ws';
import * as jwt from 'jsonwebtoken';
import { MemoryStorageAdapter } from '../helpers/MemoryStorageAdapter';

const JWT_SECRET = 'cluster-e2e-test-secret';

// Port ranges to avoid conflicts with other tests
let clusterPortBase = 12000;
let clientPortBase = 13000;

/**
 * Get next available port pair for a cluster node
 */
export function getNextPorts(): { port: number; clusterPort: number } {
  const result = {
    port: clientPortBase++,
    clusterPort: clusterPortBase++,
  };
  return result;
}

/**
 * Reset port counters (call at start of test suite)
 */
export function resetPorts(): void {
  clusterPortBase = 12000 + Math.floor(Math.random() * 1000);
  clientPortBase = 13000 + Math.floor(Math.random() * 1000);
}

export interface ClusterNode {
  coordinator: ServerCoordinator;
  port: number;
  clusterPort: number;
  nodeId: string;
}

export interface ClusterContext {
  nodes: ClusterNode[];
  clients: Map<string, TopGunClient>;
  cleanup: () => Promise<void>;
}

/**
 * Configuration for creating a test cluster
 */
export interface ClusterConfig {
  /** Number of nodes in the cluster */
  nodeCount: number;
  /** Custom node IDs (optional) */
  nodeIds?: string[];
  /** Default consistency level */
  defaultConsistency?: ConsistencyLevel;
  /** Enable gradual rebalancing */
  gradualRebalancing?: boolean;
  /** Additional server config overrides */
  serverConfig?: Partial<ServerCoordinatorConfig>;
}

/**
 * Creates a multi-node cluster for testing
 */
export async function createCluster(config: ClusterConfig): Promise<ClusterContext> {
  const nodes: ClusterNode[] = [];
  const clients = new Map<string, TopGunClient>();

  resetPorts();

  // Pre-allocate ports for all nodes
  const portAllocations: Array<{ port: number; clusterPort: number }> = [];
  for (let i = 0; i < config.nodeCount; i++) {
    portAllocations.push(getNextPorts());
  }

  // Generate node IDs (sorted for deterministic behavior)
  const nodeIds = config.nodeIds ||
    Array.from({ length: config.nodeCount }, (_, i) => `node-${String.fromCharCode(97 + i)}`);

  // Start first node (seed node)
  const firstNodeId = nodeIds[0];
  const firstPorts = portAllocations[0];

  const firstNode = ServerFactory.create({
    port: firstPorts.port,
    nodeId: firstNodeId,
    host: 'localhost',
    clusterPort: firstPorts.clusterPort,
    metricsPort: 0,
    peers: [],
    jwtSecret: JWT_SECRET,
    replicationEnabled: true,
    defaultConsistency: config.defaultConsistency || ConsistencyLevel.EVENTUAL,
    ...config.serverConfig,
  });

  await firstNode.ready();

  nodes.push({
    coordinator: firstNode,
    port: firstNode.port,
    clusterPort: firstNode.clusterPort,
    nodeId: firstNodeId,
  });

  // Start remaining nodes, connecting to the first node
  for (let i = 1; i < config.nodeCount; i++) {
    const nodeId = nodeIds[i];
    const ports = portAllocations[i];

    // Connect to first node as seed
    const peers = [`localhost:${nodes[0].clusterPort}`];

    const node = ServerFactory.create({
      port: ports.port,
      nodeId,
      host: 'localhost',
      clusterPort: ports.clusterPort,
      metricsPort: 0,
      peers,
      jwtSecret: JWT_SECRET,
      replicationEnabled: true,
      defaultConsistency: config.defaultConsistency || ConsistencyLevel.EVENTUAL,
      ...config.serverConfig,
    });

    await node.ready();

    nodes.push({
      coordinator: node,
      port: node.port,
      clusterPort: node.clusterPort,
      nodeId,
    });
  }

  // Wait for cluster formation with extended timeout and retry
  let formed = await waitForClusterFormation(nodes, config.nodeCount);

  // If not formed, wait a bit more and retry - gossip may need time
  if (!formed) {
    await sleep(2000);
    formed = await waitForClusterFormation(nodes, config.nodeCount, 10000);
  }

  if (!formed) {
    // Cleanup on failure
    for (const node of nodes) {
      await node.coordinator.shutdown();
    }
    throw new Error(`Cluster failed to form with ${config.nodeCount} nodes`);
  }

  return {
    nodes,
    clients,
    cleanup: async () => {
      // Close all clients
      for (const client of clients.values()) {
        await client.close();
      }
      clients.clear();

      // Shutdown all nodes
      await Promise.all(nodes.map(node => node.coordinator.shutdown()));

      // Small delay for cleanup
      await sleep(200);
    },
  };
}

/**
 * Wait for all nodes to see each other
 */
export async function waitForClusterFormation(
  nodes: ClusterNode[],
  expectedSize: number,
  timeoutMs = 30000
): Promise<boolean> {
  const start = Date.now();

  while (Date.now() - start < timeoutMs) {
    let allReady = true;
    const memberCounts: Record<string, number> = {};

    for (const node of nodes) {
      const members = (node.coordinator as any).cluster?.getMembers() || [];
      memberCounts[node.nodeId] = members.length;
      if (members.length < expectedSize) {
        allReady = false;
      }
    }

    if (allReady) {
      console.log('Cluster formed:', memberCounts);
      return true;
    }

    // Log progress every 5 seconds
    if ((Date.now() - start) % 5000 < 100) {
      console.log(`Cluster formation progress (${Math.floor((Date.now() - start) / 1000)}s):`, memberCounts);
    }

    await sleep(100);
  }

  // Log final state on failure
  const finalCounts: Record<string, number> = {};
  for (const node of nodes) {
    const members = (node.coordinator as any).cluster?.getMembers() || [];
    finalCounts[node.nodeId] = members.length;
  }
  console.log('Cluster formation FAILED. Final state:', finalCounts);

  return false;
}

/**
 * Wait for partition rebalancing to complete
 */
export async function waitForPartitionStability(
  nodes: ClusterNode[],
  timeoutMs = 10000
): Promise<boolean> {
  const start = Date.now();

  while (Date.now() - start < timeoutMs) {
    let allStable = true;

    for (const node of nodes) {
      const ps = (node.coordinator as any).partitionService;
      if (ps?.isRebalancing?.()) {
        allStable = false;
        break;
      }
    }

    if (allStable) {
      return true;
    }

    await sleep(100);
  }

  return false;
}

/**
 * Create a TopGunClient connected to a specific node
 */
export async function createClusterClient(
  node: ClusterNode,
  options: {
    userId?: string;
    roles?: string[];
    clientId?: string;
  } = {}
): Promise<TopGunClient> {
  const storage = new MemoryStorageAdapter();
  await storage.initialize('cluster-test');

  const clientId = options.clientId || `client-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  const token = createTestToken(options.userId || clientId, options.roles || ['ADMIN']);

  const client = new TopGunClient({
    serverUrl: `ws://localhost:${node.port}`,
    storage,
    nodeId: clientId,
  });

  // Set auth token before starting
  client.setAuthToken(token);

  // Initialize the client
  await client.start();

  return client;
}

/**
 * Create a raw WebSocket client for low-level testing
 */
export async function createRawClient(
  node: ClusterNode,
  options: {
    autoAuth?: boolean;
    userId?: string;
    roles?: string[];
  } = {}
): Promise<RawTestClient> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(`ws://localhost:${node.port}`);
    ws.binaryType = 'arraybuffer';

    const messages: any[] = [];
    const messageWaiters = new Map<string, {
      resolve: (msg: any) => void;
      reject: (err: Error) => void;
      timeout: NodeJS.Timeout;
    }>();

    const client: RawTestClient = {
      ws,
      nodeId: node.nodeId,
      messages,

      send(message: any): void {
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(serialize(message));
        }
      },

      waitForMessage(type: string, timeoutMs = 5000): Promise<any> {
        // Check if already received
        const existing = messages.find(m => m.type === type);
        if (existing) {
          return Promise.resolve(existing);
        }

        return new Promise((res, rej) => {
          const timeout = setTimeout(() => {
            messageWaiters.delete(type);
            rej(new Error(`Timeout waiting for message: ${type}`));
          }, timeoutMs);

          messageWaiters.set(type, { resolve: res, reject: rej, timeout });
        });
      },

      close(): void {
        for (const { timeout } of messageWaiters.values()) {
          clearTimeout(timeout);
        }
        messageWaiters.clear();
        ws.close();
      },
    };

    ws.on('open', () => {
      resolve(client);
    });

    ws.on('message', (data: ArrayBuffer | Buffer) => {
      try {
        const buf = data instanceof ArrayBuffer ? new Uint8Array(data) : data;
        const message = deserialize(buf as Uint8Array) as any;
        messages.push(message);

        // Check if someone is waiting
        const waiter = messageWaiters.get(message.type);
        if (waiter) {
          clearTimeout(waiter.timeout);
          messageWaiters.delete(message.type);
          waiter.resolve(message);
        }

        // Auto-auth handling
        if (message.type === 'AUTH_REQUIRED' && options.autoAuth !== false) {
          const token = createTestToken(
            options.userId || 'test-user',
            options.roles || ['ADMIN']
          );
          client.send({ type: 'AUTH', token });
        }
      } catch (err) {
        console.error('Failed to parse message:', err);
      }
    });

    ws.on('error', reject);
  });
}

export interface RawTestClient {
  ws: WebSocket;
  nodeId: string;
  messages: any[];
  send(message: any): void;
  waitForMessage(type: string, timeoutMs?: number): Promise<any>;
  close(): void;
}

/**
 * Create a JWT token for testing
 */
export function createTestToken(userId: string, roles: string[] = ['USER']): string {
  return jwt.sign(
    { userId, roles, sub: userId },
    JWT_SECRET,
    { expiresIn: '1h' }
  );
}

/**
 * Get partition distribution across nodes
 */
export function getPartitionDistribution(node: ClusterNode): Map<string, number[]> {
  const ps = (node.coordinator as any).partitionService;
  const partitionMap = ps.getPartitionMap();
  const distribution = new Map<string, number[]>();

  for (const partition of partitionMap.partitions) {
    const existing = distribution.get(partition.ownerNodeId) || [];
    existing.push(partition.partitionId);
    distribution.set(partition.ownerNodeId, existing);
  }

  return distribution;
}

/**
 * Check if partitions are evenly distributed
 */
export function isPartitionBalanced(
  distribution: Map<string, number[]>,
  tolerancePercent = 20
): boolean {
  const counts = Array.from(distribution.values()).map(p => p.length);
  if (counts.length === 0) return true;

  const total = counts.reduce((a, b) => a + b, 0);
  const expected = total / counts.length;
  const tolerance = expected * (tolerancePercent / 100);

  return counts.every(count =>
    count >= expected - tolerance && count <= expected + tolerance
  );
}

/**
 * Get keys owned by a specific node
 */
export async function getKeysOnNode(
  node: ClusterNode,
  mapName: string
): Promise<string[]> {
  const map = node.coordinator.getMap(mapName) as LWWMap<string, any> | undefined;
  if (!map) return [];

  const keys: string[] = [];
  const ps = (node.coordinator as any).partitionService;

  for (const [key] of map.entries()) {
    if (ps.isLocalOwner(key)) {
      keys.push(key);
    }
  }

  return keys;
}

/**
 * Write data through a client and wait for replication
 */
export async function writeAndWaitForReplication(
  client: TopGunClient,
  mapName: string,
  key: string,
  value: any,
  replicationDelayMs = 500
): Promise<void> {
  const map = client.getMap<string, any>(mapName);
  map.set(key, value);
  await sleep(replicationDelayMs);
}

/**
 * Verify data exists on all nodes
 */
export async function verifyDataOnAllNodes(
  nodes: ClusterNode[],
  mapName: string,
  key: string,
  expectedValue: any
): Promise<boolean> {
  for (const node of nodes) {
    const map = node.coordinator.getMap(mapName) as LWWMap<string, any>;
    const value = map?.get(key);

    if (JSON.stringify(value) !== JSON.stringify(expectedValue)) {
      return false;
    }
  }

  return true;
}

/**
 * Wait for data to appear on a specific node
 */
export async function waitForDataOnNode(
  node: ClusterNode,
  mapName: string,
  key: string,
  expectedValue: any,
  timeoutMs = 5000
): Promise<boolean> {
  const start = Date.now();

  while (Date.now() - start < timeoutMs) {
    const map = node.coordinator.getMap(mapName) as LWWMap<string, any>;
    const value = map?.get(key);

    if (JSON.stringify(value) !== JSON.stringify(expectedValue)) {
      await sleep(50);
      continue;
    }

    return true;
  }

  return false;
}

/**
 * Simulate network partition by disconnecting cluster connections
 */
export async function simulateNetworkPartition(
  isolatedNode: ClusterNode,
  otherNodes: ClusterNode[]
): Promise<() => Promise<void>> {
  const cluster = (isolatedNode.coordinator as any).cluster;

  // Store original connections for restoration
  const originalConnections = new Map(cluster.peers);

  // Close connections to isolated node
  for (const [nodeId, peer] of cluster.peers) {
    if (peer.socket) {
      peer.socket.close();
    }
  }

  // Return function to heal the partition
  return async () => {
    // Reconnect to peers
    const peers = otherNodes.map(n => `localhost:${n.clusterPort}`);
    for (const peer of peers) {
      try {
        await cluster.connectToPeer(peer);
      } catch (e) {
        // Ignore connection errors during healing
      }
    }
    await sleep(500); // Wait for connections to stabilize
  };
}

/**
 * Sleep utility
 */
export function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}

/**
 * Generate test data with unique keys
 */
export function generateTestData(
  prefix: string,
  count: number
): Array<{ key: string; value: any }> {
  const data: Array<{ key: string; value: any }> = [];

  for (let i = 0; i < count; i++) {
    data.push({
      key: `${prefix}-${i}`,
      value: {
        index: i,
        data: `test-data-${i}`,
        timestamp: Date.now(),
      },
    });
  }

  return data;
}

/**
 * Mock client connection for direct server testing
 */
export function createMockClient(id: string): any {
  const mockWrite = jest.fn();
  return {
    id,
    socket: {
      send: jest.fn(),
      readyState: WebSocket.OPEN,
      close: jest.fn()
    },
    writer: {
      write: mockWrite,
      close: jest.fn(),
    },
    isAuthenticated: true,
    subscriptions: new Set(),
    principal: { userId: id, roles: ['ADMIN'] },
    lastPingReceived: Date.now(),
  };
}

/**
 * Assert related nodes (owner + backups) have consistent data
 * In a partitioned data grid, only owner and backup nodes store the data.
 * This function checks that nodes which HAVE the data are consistent.
 */
export async function assertClusterConsistency(
  nodes: ClusterNode[],
  mapName: string,
  expectedKeys: string[]
): Promise<{ consistent: boolean; details: string[] }> {
  const details: string[] = [];
  let consistent = true;

  for (const key of expectedKeys) {
    const values = new Map<string, any>();
    const relatedNodes: string[] = [];

    for (const node of nodes) {
      // Only check nodes that are owner or backup for this key
      const ps = (node.coordinator as any).partitionService;
      if (ps.isRelated(key)) {
        relatedNodes.push(node.nodeId);
        const map = node.coordinator.getMap(mapName) as LWWMap<string, any>;
        const value = map?.get(key);
        values.set(node.nodeId, value);
      }
    }

    // Check all related nodes have equal values
    const valuesArray = Array.from(values.values());
    if (valuesArray.length === 0) {
      // No related nodes found - this shouldn't happen
      details.push(`Key ${key}: no related nodes found`);
      consistent = false;
      continue;
    }

    const firstValue = JSON.stringify(valuesArray[0]);

    for (const [nodeId, value] of values) {
      if (JSON.stringify(value) !== firstValue) {
        consistent = false;
        details.push(`Key ${key}: inconsistent value on ${nodeId}`);
      }
    }
  }

  return { consistent, details };
}
