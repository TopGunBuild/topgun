/**
 * E2E Distributed Search Tests
 *
 * Tests real distributed search across multiple server nodes:
 * - Multi-node FTS search with RRF merge
 * - Single-node FTS fallback
 * - Search result ranking across nodes
 * - Cursor-based pagination
 *
 * Run: pnpm test:workers -- --testPathPattern=DistributedSearch --runInBand
 */

import { ServerCoordinator, ServerFactory } from '../';
import { WebSocket } from 'ws';
import jwt from 'jsonwebtoken';
import { serialize, deserialize } from '@topgunbuild/core';
import { waitForCluster, pollUntil } from './utils/test-helpers';
import { createTestHarness, ServerTestHarness } from './utils/ServerTestHarness';

const JWT_SECRET = 'test-secret-for-e2e-tests';

// Helper: Create a valid JWT token with ADMIN role for full access
function createTestToken(userId = 'test-user', roles = ['ADMIN']): string {
  return jwt.sign({ userId, roles }, JWT_SECRET, { expiresIn: '1h' });
}

// Run: npx jest --testPathPattern="DistributedSearch.e2e" --runInBand
describe('Distributed Search E2E', () => {
  let node1: ServerCoordinator;
  let node2: ServerCoordinator;

  // Helper to insert data via internal API
  async function insertData(node: ServerCoordinator, mapName: string, key: string, value: any): Promise<void> {
    const harness = createTestHarness(node);
    const map = node.getMap(mapName);
    // LWWMap.set(key, value, ttlMs?) - timestamp is generated internally by map's HLC
    (map as any).set(key, value);
    // Trigger FTS index update
    harness.searchCoordinator?.onDataChange(mapName, key, value, 'add');
  }

  // Helper to perform search via WebSocket
  async function search(
    node: ServerCoordinator,
    mapName: string,
    query: string,
    options: { limit?: number } = {}
  ): Promise<{ results: any[]; totalCount: number; nextCursor?: string; error?: string }> {
    return new Promise((resolve, reject) => {
      const client = new WebSocket(`ws://localhost:${node.port}`);
      const timeout = setTimeout(() => {
        client.close();
        reject(new Error('Search timeout'));
      }, 5000);

      client.on('open', () => {
        // First authenticate with proper JWT
        client.send(serialize({
          type: 'AUTH',
          token: createTestToken(),
        }));
      });

      client.on('message', (data: Buffer) => {
        try {
          const msg = deserialize(data) as { type: string; payload?: any; error?: string };

          if (msg.type === 'AUTH_ACK' || msg.type === 'AUTH_SUCCESS' || msg.type === 'AUTH_RESP') {
            // Now send search request
            client.send(serialize({
              type: 'SEARCH',
              payload: {
                requestId: 'search-1',
                mapName,
                query,
                options: {
                  limit: options.limit ?? 10,
                },
              },
            }));
          } else if (msg.type === 'SEARCH_RESP') {
            clearTimeout(timeout);
            client.close();
            resolve(msg.payload);
          } else if (msg.type === 'ERROR' || msg.type === 'AUTH_FAIL') {
            clearTimeout(timeout);
            client.close();
            reject(new Error(msg.error || msg.payload?.message || 'Unknown error'));
          }
        } catch (err) {
          // Skip parsing errors for non-msgpack messages
        }
      });

      client.on('error', (err) => {
        clearTimeout(timeout);
        reject(err);
      });
    });
  }

  describe('2-Node Distributed Search', () => {
    beforeAll(async () => {
      // Start first node with FTS enabled
      node1 = ServerFactory.create({
        port: 0,
        nodeId: 'search-node-1',
        host: 'localhost',
        clusterPort: 0,
        peers: [],
        jwtSecret: JWT_SECRET,
        metricsPort: 0, // Use random port to avoid conflicts
        fullTextSearch: {
          articles: {
            fields: ['title', 'content'],
          },
        },
      });
      await node1.ready();

      // Start second node with FTS enabled
      node2 = ServerFactory.create({
        port: 0,
        nodeId: 'search-node-2',
        host: 'localhost',
        clusterPort: 0,
        peers: [`localhost:${node1.clusterPort}`],
        jwtSecret: JWT_SECRET,
        metricsPort: 0, // Use random port to avoid conflicts
        fullTextSearch: {
          articles: {
            fields: ['title', 'content'],
          },
        },
      });
      await node2.ready();

      // Wait for cluster formation
      await waitForCluster([node1, node2], 2, 10000);

      // Verify cluster stabilization by checking partition service is available
      await pollUntil(
        () => {
          const h1 = createTestHarness(node1);
          const h2 = createTestHarness(node2);
          return h1.partitionService !== undefined && h2.partitionService !== undefined;
        },
        { timeoutMs: 5000, intervalMs: 100, description: 'cluster stabilization after formation' }
      );
    }, 30000);

    afterAll(async () => {
      await Promise.all([
        node1?.shutdown(),
        node2?.shutdown(),
      ]);
      // WHY: Allow pending cluster WebSocket close events to drain before Jest tears down
      await new Promise(resolve => setTimeout(resolve, 300));
    });

    test('should perform distributed search across nodes', async () => {
      // Insert documents on node1
      await insertData(node1, 'articles', 'article-1', {
        title: 'Introduction to Machine Learning',
        content: 'Machine learning is a subset of artificial intelligence.',
      });
      await insertData(node1, 'articles', 'article-2', {
        title: 'Deep Learning Fundamentals',
        content: 'Deep learning uses neural networks for learning.',
      });

      // Insert documents on node2
      await insertData(node2, 'articles', 'article-3', {
        title: 'Machine Learning Applications',
        content: 'ML is used in image recognition and NLP.',
      });
      await insertData(node2, 'articles', 'article-4', {
        title: 'Data Science Basics',
        content: 'Data science combines statistics and machine learning.',
      });

      // Wait for FTS indexes to process the inserted documents
      await pollUntil(
        () => {
          const h1 = createTestHarness(node1);
          const h2 = createTestHarness(node2);
          // Check that search coordinators have indexed the data
          const r1 = h1.searchCoordinator?.search('articles', 'machine learning', { limit: 10 });
          const r2 = h2.searchCoordinator?.search('articles', 'machine learning', { limit: 10 });
          return (r1?.results?.length ?? 0) > 0 && (r2?.results?.length ?? 0) > 0;
        },
        { timeoutMs: 5000, intervalMs: 50, description: 'FTS index updates on both nodes' }
      );

      // Search from node1 - should find results from both nodes
      const result = await search(node1, 'articles', 'machine learning');

      expect(result.error).toBeUndefined();
      expect(result.results.length).toBeGreaterThan(0);

      // Should find articles mentioning "machine learning"
      const keys = result.results.map(r => r.key);
      expect(keys).toContain('article-1');
      expect(keys).toContain('article-3');
      expect(keys).toContain('article-4');
    }, 10000);

    test('should return search results with scores', async () => {
      const result = await search(node1, 'articles', 'deep learning neural');

      expect(result.error).toBeUndefined();
      expect(result.results.length).toBeGreaterThan(0);

      // Results should have scores
      for (const r of result.results) {
        expect(r.score).toBeDefined();
        expect(typeof r.score).toBe('number');
        expect(r.score).toBeGreaterThan(0);
      }

      // Results should be sorted by score descending
      for (let i = 1; i < result.results.length; i++) {
        expect(result.results[i - 1].score).toBeGreaterThanOrEqual(result.results[i].score);
      }
    }, 10000);

    test('should respect limit option', async () => {
      const result = await search(node1, 'articles', 'learning', { limit: 2 });

      expect(result.error).toBeUndefined();
      expect(result.results.length).toBeLessThanOrEqual(2);
    }, 10000);

    test('should handle query with no matches', async () => {
      const result = await search(node1, 'articles', 'quantum physics blockchain');

      expect(result.error).toBeUndefined();
      expect(result.results.length).toBe(0);
    }, 10000);
  });

  describe('Single-Node Search', () => {
    let singleNode: ServerCoordinator;

    beforeAll(async () => {
      // Start single node with FTS enabled
      singleNode = ServerFactory.create({
        port: 0,
        nodeId: 'single-search-node',
        host: 'localhost',
        clusterPort: 0,
        peers: [],
        jwtSecret: JWT_SECRET,
        metricsPort: 0, // Use random port to avoid conflicts
        fullTextSearch: {
          docs: {
            fields: ['title', 'body'],
          },
        },
      });
      await singleNode.ready();

      // Insert test documents
      await insertData(singleNode, 'docs', 'doc-1', {
        title: 'TypeScript Guide',
        body: 'TypeScript is a typed superset of JavaScript.',
      });
      await insertData(singleNode, 'docs', 'doc-2', {
        title: 'JavaScript Basics',
        body: 'JavaScript is a dynamic programming language.',
      });

      // Wait for FTS index to process documents
      await pollUntil(
        () => {
          const h = createTestHarness(singleNode);
          const r = h.searchCoordinator?.search('docs', 'typescript', { limit: 10 });
          return (r?.results?.length ?? 0) > 0;
        },
        { timeoutMs: 5000, intervalMs: 50, description: 'single-node FTS index update' }
      );
    }, 10000);

    afterAll(async () => {
      await singleNode?.shutdown();
      // WHY: Allow pending WebSocket close events to drain before Jest tears down
      await new Promise(resolve => setTimeout(resolve, 300));
    });

    test('should perform local search on single node', async () => {
      const result = await search(singleNode, 'docs', 'typescript');

      expect(result.error).toBeUndefined();
      expect(result.results.length).toBe(1);
      expect(result.results[0].key).toBe('doc-1');
    }, 10000);

    test('should find documents with common terms', async () => {
      const result = await search(singleNode, 'docs', 'javascript');

      expect(result.error).toBeUndefined();
      expect(result.results.length).toBe(2);
    }, 10000);
  });
});
