/**
 * E2E Tests: Full-Text Search (Phase 11)
 *
 * Integration tests for server-side BM25 search and live search subscriptions.
 */
import { ServerCoordinator } from '@topgunbuild/server';
import { LWWMap } from '@topgunbuild/core';
import {
  createTestServer,
  createTestClient,
  createTestContext,
  createLWWRecord,
  waitForSync,
  waitUntil,
  TestClient,
} from './helpers';

describe('E2E: Full-Text Search', () => {
  // ========================================
  // One-Shot Search Tests
  // ========================================
  describe('One-Shot Search (SEARCH)', () => {
    let server: ServerCoordinator;
    let client: TestClient;

    beforeEach(async () => {
      server = await createTestServer();

      // Pre-populate with test articles BEFORE enabling FTS
      const map = server.getMap('articles') as LWWMap<string, any>;
      map.merge('art-1', createLWWRecord({
        title: 'Introduction to Machine Learning',
        body: 'Machine learning is a subset of artificial intelligence.',
      }));
      map.merge('art-2', createLWWRecord({
        title: 'Deep Learning Fundamentals',
        body: 'Deep learning uses neural networks with many layers.',
      }));
      map.merge('art-3', createLWWRecord({
        title: 'Natural Language Processing',
        body: 'NLP helps computers understand human language.',
      }));
      map.merge('art-4', createLWWRecord({
        title: 'Computer Vision Basics',
        body: 'Computer vision enables machines to interpret images.',
      }));
      map.merge('art-5', createLWWRecord({
        title: 'Advanced Machine Learning Techniques',
        body: 'This article covers advanced ML algorithms and optimization.',
      }));

      // Enable FTS for 'articles' map (will index existing data)
      server.enableFullTextSearch('articles', {
        fields: ['title', 'body'],
      });

      client = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');
    });

    afterEach(async () => {
      client.close();
      await server.shutdown();
    });

    test('basic search returns matching documents', async () => {
      client.send({
        type: 'SEARCH',
        payload: {
          requestId: 'search-1',
          mapName: 'articles',
          query: 'machine learning',
        },
      });

      const response = await client.waitForMessage('SEARCH_RESP');

      expect(response).toBeDefined();
      expect(response.payload.requestId).toBe('search-1');
      expect(response.payload.results.length).toBeGreaterThanOrEqual(2);

      // Results should contain machine learning articles
      const titles = response.payload.results.map((r: any) => r.value.title);
      expect(titles).toContain('Introduction to Machine Learning');
      expect(titles).toContain('Advanced Machine Learning Techniques');
    });

    test('search results are sorted by relevance score', async () => {
      client.send({
        type: 'SEARCH',
        payload: {
          requestId: 'search-sorted',
          mapName: 'articles',
          query: 'machine learning',
        },
      });

      const response = await client.waitForMessage('SEARCH_RESP');
      const results = response.payload.results;

      // Scores should be in descending order
      for (let i = 1; i < results.length; i++) {
        expect(results[i - 1].score).toBeGreaterThanOrEqual(results[i].score);
      }
    });

    test('search with limit returns max N results', async () => {
      client.send({
        type: 'SEARCH',
        payload: {
          requestId: 'search-limit',
          mapName: 'articles',
          query: 'learning',
          options: { limit: 2 },
        },
      });

      const response = await client.waitForMessage('SEARCH_RESP');

      expect(response.payload.results.length).toBeLessThanOrEqual(2);
    });

    test('search with minScore filters low-score results', async () => {
      client.send({
        type: 'SEARCH',
        payload: {
          requestId: 'search-minscore',
          mapName: 'articles',
          query: 'machine',
          options: { minScore: 0.1 },
        },
      });

      const response = await client.waitForMessage('SEARCH_RESP');

      // All results should have score >= minScore
      for (const result of response.payload.results) {
        expect(result.score).toBeGreaterThanOrEqual(0.1);
      }
    });

    test('search with boost affects ranking', async () => {
      client.send({
        type: 'SEARCH',
        payload: {
          requestId: 'search-boost',
          mapName: 'articles',
          query: 'machine',
          options: {
            boost: { title: 5.0, body: 1.0 },
          },
        },
      });

      const response = await client.waitForMessage('SEARCH_RESP');

      // Articles with 'machine' in title should rank higher
      const results = response.payload.results;
      if (results.length >= 2) {
        const topResult = results[0];
        expect(topResult.value.title.toLowerCase()).toContain('machine');
      }
    });

    test('search with no matches returns empty results', async () => {
      client.send({
        type: 'SEARCH',
        payload: {
          requestId: 'search-empty',
          mapName: 'articles',
          query: 'xyznonexistent123',
        },
      });

      const response = await client.waitForMessage('SEARCH_RESP');

      expect(response.payload.results).toHaveLength(0);
      expect(response.payload.totalCount).toBe(0);
    });

    test('search on non-indexed map returns error', async () => {
      client.send({
        type: 'SEARCH',
        payload: {
          requestId: 'search-noindex',
          mapName: 'unindexed-map',
          query: 'test',
        },
      });

      const response = await client.waitForMessage('SEARCH_RESP');

      expect(response.payload.error).toBeDefined();
      expect(response.payload.error).toContain('not enabled');
    });

    test('search results include matchedTerms', async () => {
      client.send({
        type: 'SEARCH',
        payload: {
          requestId: 'search-terms',
          mapName: 'articles',
          query: 'deep learning neural',
        },
      });

      const response = await client.waitForMessage('SEARCH_RESP');

      // At least one result should have matchedTerms
      const resultWithTerms = response.payload.results.find(
        (r: any) => r.matchedTerms && r.matchedTerms.length > 0
      );
      expect(resultWithTerms).toBeDefined();
    });
  });

  // ========================================
  // Live Search Subscription Tests
  // ========================================
  describe('Live Search (SEARCH_SUB)', () => {
    let server: ServerCoordinator;
    let client: TestClient;

    beforeEach(async () => {
      server = await createTestServer();

      server.enableFullTextSearch('products', {
        fields: ['name', 'description'],
      });

      client = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');
    });

    afterEach(async () => {
      client.close();
      await server.shutdown();
    });

    test('subscribe returns initial results', async () => {
      // Pre-populate via CLIENT_OP (triggers FTS indexing)
      client.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'products',
          opType: 'PUT',
          key: 'prod-1',
          record: createLWWRecord({ name: 'Laptop Pro', description: 'Powerful laptop for professionals' }),
        },
      });
      client.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'products',
          opType: 'PUT',
          key: 'prod-2',
          record: createLWWRecord({ name: 'Desktop PC', description: 'High performance desktop computer' }),
        },
      });

      await waitForSync(100);

      client.send({
        type: 'SEARCH_SUB',
        payload: {
          subscriptionId: 'sub-products',
          mapName: 'products',
          query: 'laptop',
        },
      });

      const response = await client.waitForMessage('SEARCH_RESP');

      expect(response.payload.requestId).toBe('sub-products');
      expect(response.payload.results.length).toBeGreaterThanOrEqual(1);

      const laptopResult = response.payload.results.find(
        (r: any) => r.value.name === 'Laptop Pro'
      );
      expect(laptopResult).toBeDefined();
    });

    test('new matching document triggers SEARCH_UPDATE ENTER', async () => {
      // Subscribe first
      client.send({
        type: 'SEARCH_SUB',
        payload: {
          subscriptionId: 'sub-enter',
          mapName: 'products',
          query: 'wireless',
        },
      });

      await client.waitForMessage('SEARCH_RESP');
      client.messages.length = 0;

      // Add matching document
      const writer = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
        nodeId: 'writer',
      });
      await writer.waitForMessage('AUTH_ACK');

      writer.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'products',
          opType: 'PUT',
          key: 'prod-wireless',
          record: createLWWRecord({
            name: 'Wireless Mouse',
            description: 'Ergonomic wireless mouse',
          }),
        },
      });

      // Wait for SEARCH_UPDATE
      await waitUntil(
        () => client.messages.some((m) => m.type === 'SEARCH_UPDATE'),
        5000
      );

      const update = client.messages.find((m) => m.type === 'SEARCH_UPDATE');
      expect(update).toBeDefined();
      expect(update.payload.subscriptionId).toBe('sub-enter');
      expect(update.payload.type).toBe('ENTER');
      expect(update.payload.key).toBe('prod-wireless');
      expect(update.payload.value.name).toBe('Wireless Mouse');
      expect(update.payload.score).toBeGreaterThan(0);

      writer.close();
    });

    test('updated document triggers SEARCH_UPDATE UPDATE', async () => {
      // First, add the initial document via CLIENT_OP
      client.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'products',
          opType: 'PUT',
          key: 'prod-update',
          record: createLWWRecord({
            name: 'Keyboard',
            description: 'Mechanical keyboard',
          }),
        },
      });

      await waitForSync(100);

      // Subscribe
      client.send({
        type: 'SEARCH_SUB',
        payload: {
          subscriptionId: 'sub-update',
          mapName: 'products',
          query: 'keyboard',
        },
      });

      const initialResp = await client.waitForMessage('SEARCH_RESP');
      expect(initialResp.payload.results.length).toBeGreaterThanOrEqual(1);
      client.messages.length = 0;

      // Update the document (still matches but changes content)
      client.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'products',
          opType: 'PUT',
          key: 'prod-update',
          record: {
            value: {
              name: 'Gaming Keyboard',
              description: 'RGB mechanical keyboard for gaming',
            },
            timestamp: {
              millis: Date.now() + 1000,
              counter: 0,
              nodeId: 'client',
            },
          },
        },
      });

      // Wait for SEARCH_UPDATE
      await waitUntil(
        () => client.messages.some((m) => m.type === 'SEARCH_UPDATE'),
        5000
      );

      const update = client.messages.find((m) => m.type === 'SEARCH_UPDATE');
      expect(update).toBeDefined();
      expect(update.payload.type).toBe('UPDATE');
      expect(update.payload.value.name).toBe('Gaming Keyboard');
    });

    test('document no longer matching triggers SEARCH_UPDATE LEAVE', async () => {
      // First, add the document via CLIENT_OP
      client.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'products',
          opType: 'PUT',
          key: 'prod-leave',
          record: createLWWRecord({
            name: 'USB Cable',
            description: 'High-speed USB cable',
          }),
        },
      });

      await waitForSync(100);

      // Subscribe
      client.send({
        type: 'SEARCH_SUB',
        payload: {
          subscriptionId: 'sub-leave',
          mapName: 'products',
          query: 'USB',
        },
      });

      const resp = await client.waitForMessage('SEARCH_RESP');
      expect(resp.payload.results.length).toBeGreaterThanOrEqual(1);
      client.messages.length = 0;

      // Update document to no longer match
      client.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'products',
          opType: 'PUT',
          key: 'prod-leave',
          record: {
            value: {
              name: 'HDMI Cable',
              description: 'High-speed HDMI cable',
            },
            timestamp: {
              millis: Date.now() + 1000,
              counter: 0,
              nodeId: 'client',
            },
          },
        },
      });

      // Wait for LEAVE update
      await waitUntil(
        () => client.messages.some(
          (m) => m.type === 'SEARCH_UPDATE' && m.payload.type === 'LEAVE'
        ),
        5000
      );

      const update = client.messages.find(
        (m) => m.type === 'SEARCH_UPDATE' && m.payload.type === 'LEAVE'
      );
      expect(update).toBeDefined();
      expect(update.payload.key).toBe('prod-leave');
    });

    test('unsubscribe stops updates', async () => {
      // Subscribe
      client.send({
        type: 'SEARCH_SUB',
        payload: {
          subscriptionId: 'sub-unsub',
          mapName: 'products',
          query: 'monitor',
        },
      });

      await client.waitForMessage('SEARCH_RESP');

      // Unsubscribe
      client.send({
        type: 'SEARCH_UNSUB',
        payload: { subscriptionId: 'sub-unsub' },
      });

      await waitForSync(100);
      client.messages.length = 0;

      // Add matching document
      const writer = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
        nodeId: 'writer',
      });
      await writer.waitForMessage('AUTH_ACK');

      writer.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'products',
          opType: 'PUT',
          key: 'prod-monitor',
          record: createLWWRecord({
            name: '4K Monitor',
            description: 'Ultra HD monitor',
          }),
        },
      });

      await waitForSync(300);

      // Should NOT receive SEARCH_UPDATE
      const update = client.messages.find((m) => m.type === 'SEARCH_UPDATE');
      expect(update).toBeUndefined();

      writer.close();
    });

    test('multiple subscriptions with different queries', async () => {
      // Clear any previous messages
      client.messages.length = 0;

      // Subscribe to two different queries
      client.send({
        type: 'SEARCH_SUB',
        payload: {
          subscriptionId: 'sub-mouse',
          mapName: 'products',
          query: 'mouse',
        },
      });

      // Wait for first SEARCH_RESP before sending second
      await client.waitForMessage('SEARCH_RESP');

      client.send({
        type: 'SEARCH_SUB',
        payload: {
          subscriptionId: 'sub-keyboard',
          mapName: 'products',
          query: 'keyboard',
        },
      });

      // Wait for second SEARCH_RESP
      await waitUntil(
        () => client.messages.filter((m) => m.type === 'SEARCH_RESP').length >= 2,
        5000
      );
      client.messages.length = 0;

      // Add mouse product
      const writer = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
        nodeId: 'writer',
      });
      await writer.waitForMessage('AUTH_ACK');

      writer.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'products',
          opType: 'PUT',
          key: 'prod-mouse',
          record: createLWWRecord({ name: 'Gaming Mouse', description: 'RGB mouse' }),
        },
      });

      // Give time for the update to propagate
      await waitForSync(100);

      await waitUntil(
        () => client.messages.some((m) => m.type === 'SEARCH_UPDATE'),
        5000
      );

      // Only 'sub-mouse' subscription should receive update
      const updates = client.messages.filter((m) => m.type === 'SEARCH_UPDATE');
      expect(updates.some((u) => u.payload.subscriptionId === 'sub-mouse')).toBe(true);
      expect(updates.some((u) => u.payload.subscriptionId === 'sub-keyboard')).toBe(false);

      writer.close();
    });
  });

  // ========================================
  // Multi-Client Search Tests
  // ========================================
  describe('Multi-Client Search', () => {
    test('multiple clients receive search updates', async () => {
      const ctx = await createTestContext(3);

      try {
        const [subscriber1, subscriber2, writer] = ctx.clients;

        // Enable FTS
        ctx.server.enableFullTextSearch('shared-docs', { fields: ['content'] });

        // Both subscribers subscribe to same query
        subscriber1.send({
          type: 'SEARCH_SUB',
          payload: {
            subscriptionId: 'sub-shared-1',
            mapName: 'shared-docs',
            query: 'important',
          },
        });

        subscriber2.send({
          type: 'SEARCH_SUB',
          payload: {
            subscriptionId: 'sub-shared-2',
            mapName: 'shared-docs',
            query: 'important',
          },
        });

        await subscriber1.waitForMessage('SEARCH_RESP');
        await subscriber2.waitForMessage('SEARCH_RESP');
        subscriber1.messages.length = 0;
        subscriber2.messages.length = 0;

        // Writer adds matching document
        writer.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'shared-docs',
            opType: 'PUT',
            key: 'doc-important',
            record: createLWWRecord({ content: 'This is an important document' }),
          },
        });

        // Both subscribers should receive update
        await waitUntil(
          () => subscriber1.messages.some((m) => m.type === 'SEARCH_UPDATE'),
          5000
        );
        await waitUntil(
          () => subscriber2.messages.some((m) => m.type === 'SEARCH_UPDATE'),
          5000
        );

        const update1 = subscriber1.messages.find((m) => m.type === 'SEARCH_UPDATE');
        const update2 = subscriber2.messages.find((m) => m.type === 'SEARCH_UPDATE');

        expect(update1).toBeDefined();
        expect(update2).toBeDefined();
        expect(update1.payload.key).toBe('doc-important');
        expect(update2.payload.key).toBe('doc-important');
      } finally {
        await ctx.cleanup();
      }
    });
  });

  // ========================================
  // Edge Cases
  // ========================================
  describe('Edge Cases', () => {
    let server: ServerCoordinator;
    let client: TestClient;

    beforeEach(async () => {
      server = await createTestServer();
      server.enableFullTextSearch('edge-cases', { fields: ['text'] });

      client = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');
    });

    afterEach(async () => {
      client.close();
      await server.shutdown();
    });

    test('empty query returns empty results', async () => {
      client.send({
        type: 'SEARCH',
        payload: {
          requestId: 'empty-query',
          mapName: 'edge-cases',
          query: '',
        },
      });

      const response = await client.waitForMessage('SEARCH_RESP');
      expect(response.payload.results).toHaveLength(0);
    });

    test('whitespace-only query returns empty results', async () => {
      client.send({
        type: 'SEARCH',
        payload: {
          requestId: 'whitespace-query',
          mapName: 'edge-cases',
          query: '   ',
        },
      });

      const response = await client.waitForMessage('SEARCH_RESP');
      expect(response.payload.results).toHaveLength(0);
    });

    test('special characters in query are handled', async () => {
      const map = server.getMap('edge-cases') as LWWMap<string, any>;
      map.merge('special', createLWWRecord({ text: 'C++ programming' }));

      client.send({
        type: 'SEARCH',
        payload: {
          requestId: 'special-chars',
          mapName: 'edge-cases',
          query: 'C++',
        },
      });

      const response = await client.waitForMessage('SEARCH_RESP');
      // Should not throw error
      expect(response.payload.error).toBeUndefined();
    });

    test('very long query is handled', async () => {
      const longQuery = 'word '.repeat(100).trim();

      client.send({
        type: 'SEARCH',
        payload: {
          requestId: 'long-query',
          mapName: 'edge-cases',
          query: longQuery,
        },
      });

      const response = await client.waitForMessage('SEARCH_RESP');
      // Should not throw error
      expect(response.payload.error).toBeUndefined();
    });

    test('document deletion triggers LEAVE', async () => {
      // First add the document via CLIENT_OP
      client.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'edge-cases',
          opType: 'PUT',
          key: 'to-delete',
          record: createLWWRecord({ text: 'deletable content' }),
        },
      });

      await waitForSync(100);

      // Subscribe
      client.send({
        type: 'SEARCH_SUB',
        payload: {
          subscriptionId: 'sub-delete',
          mapName: 'edge-cases',
          query: 'deletable',
        },
      });

      const resp = await client.waitForMessage('SEARCH_RESP');
      expect(resp.payload.results.length).toBeGreaterThanOrEqual(1);
      client.messages.length = 0;

      // Delete the document
      client.send({
        type: 'CLIENT_OP',
        payload: {
          mapName: 'edge-cases',
          opType: 'REMOVE',
          key: 'to-delete',
          record: {
            value: null,
            timestamp: {
              millis: Date.now() + 1000,
              counter: 0,
              nodeId: 'client',
            },
          },
        },
      });

      // Should receive LEAVE
      await waitUntil(
        () => client.messages.some(
          (m) => m.type === 'SEARCH_UPDATE' && m.payload.type === 'LEAVE'
        ),
        5000
      );

      const leave = client.messages.find(
        (m) => m.type === 'SEARCH_UPDATE' && m.payload.type === 'LEAVE'
      );
      expect(leave).toBeDefined();
      expect(leave.payload.key).toBe('to-delete');
    });

    test('rapid updates are handled correctly', async () => {
      // Clear previous messages
      client.messages.length = 0;

      // Subscribe
      client.send({
        type: 'SEARCH_SUB',
        payload: {
          subscriptionId: 'sub-rapid',
          mapName: 'edge-cases',
          query: 'rapid',
        },
      });

      await client.waitForMessage('SEARCH_RESP');
      client.messages.length = 0;

      // Rapidly add multiple matching documents with small delays
      for (let i = 0; i < 5; i++) {
        client.send({
          type: 'CLIENT_OP',
          payload: {
            mapName: 'edge-cases',
            opType: 'PUT',
            key: `rapid-${i}`,
            record: createLWWRecord({ text: `rapid update ${i}` }),
          },
        });
        // Small delay between each to allow processing
        await waitForSync(50);
      }

      // Wait for at least some updates (may not get all due to batching/timing)
      await waitUntil(
        () => client.messages.filter((m) => m.type === 'SEARCH_UPDATE').length >= 1,
        5000
      );

      const updates = client.messages.filter((m) => m.type === 'SEARCH_UPDATE');
      // Expect at least 1 update (batching may reduce the number of individual messages)
      expect(updates.length).toBeGreaterThanOrEqual(1);
    });
  });

  // ========================================
  // Performance Tests
  // ========================================
  describe('Performance', () => {
    test('search latency is reasonable', async () => {
      const server = await createTestServer();
      server.enableFullTextSearch('perf-test', { fields: ['content'] });

      // Add 100 documents
      const map = server.getMap('perf-test') as LWWMap<string, any>;
      for (let i = 0; i < 100; i++) {
        map.merge(`doc-${i}`, createLWWRecord({
          content: `Document ${i} with some searchable content about topic ${i % 10}`,
        }));
      }

      const client = await createTestClient(`ws://localhost:${server.port}`, {
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');

      const startTime = Date.now();

      client.send({
        type: 'SEARCH',
        payload: {
          requestId: 'perf-search',
          mapName: 'perf-test',
          query: 'searchable content',
          options: { limit: 20 },
        },
      });

      await client.waitForMessage('SEARCH_RESP');
      const endTime = Date.now();
      const latency = endTime - startTime;

      // Search should complete in reasonable time (< 500ms for 100 docs)
      expect(latency).toBeLessThan(500);

      client.close();
      await server.shutdown();
    });
  });
});
