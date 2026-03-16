import {
  createRustTestClient,
  spawnRustServer,
  createLWWRecord,
  waitForSync,
  waitUntil,
  TestClient,
} from './helpers';

describe('Integration: Search (Rust Server)', () => {
  let cleanup: () => Promise<void>;
  let port: number;

  beforeAll(async () => {
    const server = await spawnRustServer();
    port = server.port;
    cleanup = server.cleanup;
  });

  afterAll(async () => {
    await cleanup();
  });

  // ========================================
  // One-Shot Search (AC36)
  // ========================================
  describe('SEARCH one-shot returns BM25-ranked results', () => {
    test('basic search returns matching documents with key, score, matchedTerms', async () => {
      const mapName = `search-basic-${Date.now()}`;
      const client = await createRustTestClient(port, {
        nodeId: 'search-basic-1',
        userId: 'search-basic-user-1',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');

      // Write several articles with distinct content
      const articles = [
        {
          key: 'art-1',
          value: { title: 'Introduction to Machine Learning', body: 'Machine learning is a subset of artificial intelligence.' },
        },
        {
          key: 'art-2',
          value: { title: 'Deep Learning Fundamentals', body: 'Deep learning uses neural networks with many layers.' },
        },
        {
          key: 'art-3',
          value: { title: 'Natural Language Processing', body: 'NLP helps computers understand human language.' },
        },
        {
          key: 'art-4',
          value: { title: 'Advanced Machine Learning Techniques', body: 'Covers advanced ML algorithms and optimization.' },
        },
      ];

      for (const art of articles) {
        client.messages.length = 0;
        client.send({
          type: 'CLIENT_OP',
          payload: {
            id: `search-put-${art.key}`,
            mapName,
            opType: 'PUT',
            key: art.key,
            record: createLWWRecord(art.value),
          },
        });
        await client.waitForMessage('OP_ACK');
      }

      // Wait for Tantivy indexing (16ms batch + re-scoring)
      await waitForSync(200);

      // Search for "machine learning" -- should match art-1 and art-4
      client.messages.length = 0;
      client.send({
        type: 'SEARCH',
        payload: {
          requestId: 'search-basic-req',
          mapName,
          query: 'machine learning',
        },
      });

      const response = await client.waitForMessage('SEARCH_RESP');
      expect(response).toBeDefined();
      expect(response.payload.requestId).toBe('search-basic-req');
      expect(response.payload.results.length).toBeGreaterThanOrEqual(2);

      // Verify result shape: key, score, matchedTerms -- value is Nil/null
      for (const result of response.payload.results) {
        expect(result.key).toBeDefined();
        expect(typeof result.score).toBe('number');
        expect(result.score).toBeGreaterThan(0);
        expect(Array.isArray(result.matchedTerms)).toBe(true);
      }

      // Results should contain the machine learning articles
      const keys = response.payload.results.map((r: any) => r.key);
      expect(keys).toContain('art-1');
      expect(keys).toContain('art-4');

      // Results should be sorted by score descending
      for (let i = 1; i < response.payload.results.length; i++) {
        expect(response.payload.results[i - 1].score)
          .toBeGreaterThanOrEqual(response.payload.results[i].score);
      }

      client.close();
    });
  });

  // ========================================
  // Search with limit (AC37)
  // ========================================
  describe('SEARCH with limit returns at most N results', () => {
    test('limit constrains the number of results returned', async () => {
      const mapName = `search-limit-${Date.now()}`;
      const client = await createRustTestClient(port, {
        nodeId: 'search-limit-1',
        userId: 'search-limit-user-1',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');

      // Write 5 documents all containing the word "searchable"
      for (let i = 1; i <= 5; i++) {
        client.messages.length = 0;
        client.send({
          type: 'CLIENT_OP',
          payload: {
            id: `limit-put-${i}`,
            mapName,
            opType: 'PUT',
            key: `doc-${i}`,
            record: createLWWRecord({ content: `This is searchable document number ${i}` }),
          },
        });
        await client.waitForMessage('OP_ACK');
      }

      await waitForSync(200);

      // Search with limit 2
      client.messages.length = 0;
      client.send({
        type: 'SEARCH',
        payload: {
          requestId: 'search-limit-req',
          mapName,
          query: 'searchable',
          options: { limit: 2 },
        },
      });

      const response = await client.waitForMessage('SEARCH_RESP');
      expect(response.payload.requestId).toBe('search-limit-req');
      expect(response.payload.results.length).toBeLessThanOrEqual(2);

      client.close();
    });
  });

  // ========================================
  // Search with minScore
  // ========================================
  describe('SEARCH with minScore filters low-score results', () => {
    test('all returned results have score >= minScore', async () => {
      const mapName = `search-minscore-${Date.now()}`;
      const client = await createRustTestClient(port, {
        nodeId: 'search-minscore-1',
        userId: 'search-minscore-user-1',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');

      // Write docs with varying relevance to "quantum"
      const docs = [
        { key: 'q-1', value: { title: 'Quantum physics introduction', body: 'Quantum mechanics is fundamental.' } },
        { key: 'q-2', value: { title: 'Classical physics', body: 'Newton laws of motion and gravity.' } },
        { key: 'q-3', value: { title: 'Quantum computing', body: 'Quantum bits and quantum entanglement.' } },
      ];

      for (const doc of docs) {
        client.messages.length = 0;
        client.send({
          type: 'CLIENT_OP',
          payload: {
            id: `minscore-put-${doc.key}`,
            mapName,
            opType: 'PUT',
            key: doc.key,
            record: createLWWRecord(doc.value),
          },
        });
        await client.waitForMessage('OP_ACK');
      }

      await waitForSync(200);

      // Search with a high minScore threshold
      client.messages.length = 0;
      client.send({
        type: 'SEARCH',
        payload: {
          requestId: 'search-minscore-req',
          mapName,
          query: 'quantum',
          options: { minScore: 0.1 },
        },
      });

      const response = await client.waitForMessage('SEARCH_RESP');
      expect(response.payload.requestId).toBe('search-minscore-req');

      // All results must have score >= minScore
      for (const result of response.payload.results) {
        expect(result.score).toBeGreaterThanOrEqual(0.1);
      }

      client.close();
    });
  });

  // ========================================
  // Search with boost option (accepted without error)
  // ========================================
  describe('SEARCH with boost option is accepted without error', () => {
    test('boost option does not cause error', async () => {
      const mapName = `search-boost-${Date.now()}`;
      const client = await createRustTestClient(port, {
        nodeId: 'search-boost-1',
        userId: 'search-boost-user-1',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');

      client.messages.length = 0;
      client.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'boost-put-1',
          mapName,
          opType: 'PUT',
          key: 'boost-doc',
          record: createLWWRecord({ title: 'Boosted document', body: 'Content here' }),
        },
      });
      await client.waitForMessage('OP_ACK');
      await waitForSync(200);

      // Search with boost option -- should be accepted without error
      client.messages.length = 0;
      client.send({
        type: 'SEARCH',
        payload: {
          requestId: 'search-boost-req',
          mapName,
          query: 'document',
          options: {
            boost: { title: 5.0, body: 1.0 },
          },
        },
      });

      const response = await client.waitForMessage('SEARCH_RESP');
      expect(response.payload.requestId).toBe('search-boost-req');
      // No error should be present
      expect(response.payload.error).toBeUndefined();

      client.close();
    });
  });

  // ========================================
  // Search on unseen map returns 0 results (not error)
  // ========================================
  describe('SEARCH on previously-unseen map returns 0 results', () => {
    test('non-existent map returns empty results without error', async () => {
      const mapName = `search-nonexistent-${Date.now()}`;
      const client = await createRustTestClient(port, {
        nodeId: 'search-nonexist-1',
        userId: 'search-nonexist-user-1',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');

      // Search on a map that has never been written to
      client.messages.length = 0;
      client.send({
        type: 'SEARCH',
        payload: {
          requestId: 'search-nonexist-req',
          mapName,
          query: 'anything',
        },
      });

      const response = await client.waitForMessage('SEARCH_RESP');
      expect(response.payload.requestId).toBe('search-nonexist-req');
      expect(response.payload.results).toHaveLength(0);
      expect(response.payload.totalCount).toBe(0);
      // Lazy index creation means no error for unseen maps
      expect(response.payload.error).toBeUndefined();

      client.close();
    });
  });

  // ========================================
  // SEARCH_SUB initial results + live ENTER (AC38)
  // ========================================
  describe('SEARCH_SUB returns initial results and SEARCH_UPDATE ENTER', () => {
    test('subscription returns initial results and live ENTER on new write', async () => {
      const mapName = `search-sub-${Date.now()}`;

      const subscriber = await createRustTestClient(port, {
        nodeId: 'search-sub-sub-1',
        userId: 'search-sub-sub-user-1',
        roles: ['ADMIN'],
      });
      await subscriber.waitForMessage('AUTH_ACK');

      const writer = await createRustTestClient(port, {
        nodeId: 'search-sub-writer-1',
        userId: 'search-sub-writer-user-1',
        roles: ['ADMIN'],
      });
      await writer.waitForMessage('AUTH_ACK');

      // Pre-populate with a matching document
      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'sub-prepop-1',
          mapName,
          opType: 'PUT',
          key: 'existing-doc',
          record: createLWWRecord({ name: 'Wireless Keyboard', description: 'Bluetooth wireless keyboard' }),
        },
      });
      await writer.waitForMessage('OP_ACK');
      await waitForSync(200);

      // Subscribe to search for "wireless"
      subscriber.messages.length = 0;
      subscriber.send({
        type: 'SEARCH_SUB',
        payload: {
          subscriptionId: 'sub-wireless',
          mapName,
          query: 'wireless',
        },
      });

      // Initial response should contain the pre-populated document
      const initialResp = await subscriber.waitForMessage('SEARCH_RESP');
      expect(initialResp.payload.requestId).toBe('sub-wireless');
      expect(initialResp.payload.results.length).toBeGreaterThanOrEqual(1);
      const existingResult = initialResp.payload.results.find(
        (r: any) => r.key === 'existing-doc'
      );
      expect(existingResult).toBeDefined();
      expect(existingResult.score).toBeGreaterThan(0);

      // Clear to isolate live updates
      subscriber.messages.length = 0;

      // Writer adds a new matching document
      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'sub-new-doc',
          mapName,
          opType: 'PUT',
          key: 'new-wireless-mouse',
          record: createLWWRecord({ name: 'Wireless Mouse', description: 'Ergonomic wireless mouse' }),
        },
      });

      // Wait for SEARCH_UPDATE with ENTER
      await waitUntil(
        () =>
          subscriber.messages.some(
            (m) =>
              m.type === 'SEARCH_UPDATE' &&
              m.payload?.subscriptionId === 'sub-wireless' &&
              m.payload?.changeType === 'ENTER'
          ),
        5000
      );

      const enterUpdate = subscriber.messages.find(
        (m) =>
          m.type === 'SEARCH_UPDATE' &&
          m.payload?.subscriptionId === 'sub-wireless' &&
          m.payload?.changeType === 'ENTER'
      );
      expect(enterUpdate).toBeDefined();
      expect(enterUpdate.payload.key).toBe('new-wireless-mouse');
      expect(enterUpdate.payload.score).toBeGreaterThan(0);

      subscriber.close();
      writer.close();
    });
  });

  // ========================================
  // SEARCH_UPDATE with changeType UPDATE
  // ========================================
  describe('SEARCH_UPDATE with changeType UPDATE', () => {
    test('modified matching record triggers UPDATE notification', async () => {
      const mapName = `search-update-${Date.now()}`;

      const subscriber = await createRustTestClient(port, {
        nodeId: 'search-upd-sub-1',
        userId: 'search-upd-sub-user-1',
        roles: ['ADMIN'],
      });
      await subscriber.waitForMessage('AUTH_ACK');

      const writer = await createRustTestClient(port, {
        nodeId: 'search-upd-writer-1',
        userId: 'search-upd-writer-user-1',
        roles: ['ADMIN'],
      });
      await writer.waitForMessage('AUTH_ACK');

      // Write initial document
      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'upd-put-1',
          mapName,
          opType: 'PUT',
          key: 'updatable-doc',
          record: createLWWRecord({ title: 'Keyboard review', body: 'Great keyboard for typing' }),
        },
      });
      await writer.waitForMessage('OP_ACK');
      await waitForSync(200);

      // Subscribe to "keyboard"
      subscriber.messages.length = 0;
      subscriber.send({
        type: 'SEARCH_SUB',
        payload: {
          subscriptionId: 'sub-keyboard-upd',
          mapName,
          query: 'keyboard',
        },
      });

      const initialResp = await subscriber.waitForMessage('SEARCH_RESP');
      expect(initialResp.payload.results.length).toBeGreaterThanOrEqual(1);

      // Clear to isolate UPDATE
      subscriber.messages.length = 0;

      // Update the document -- still contains "keyboard"
      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'upd-put-2',
          mapName,
          opType: 'PUT',
          key: 'updatable-doc',
          record: {
            value: { title: 'Gaming Keyboard review', body: 'Best gaming keyboard ever' },
            timestamp: {
              millis: Date.now() + 1000,
              counter: 0,
              nodeId: 'search-upd-writer-1',
            },
          },
        },
      });

      // Wait for SEARCH_UPDATE with UPDATE changeType
      await waitUntil(
        () =>
          subscriber.messages.some(
            (m) =>
              m.type === 'SEARCH_UPDATE' &&
              m.payload?.subscriptionId === 'sub-keyboard-upd' &&
              m.payload?.changeType === 'UPDATE'
          ),
        5000
      );

      const updateMsg = subscriber.messages.find(
        (m) =>
          m.type === 'SEARCH_UPDATE' &&
          m.payload?.subscriptionId === 'sub-keyboard-upd' &&
          m.payload?.changeType === 'UPDATE'
      );
      expect(updateMsg).toBeDefined();
      expect(updateMsg.payload.key).toBe('updatable-doc');
      expect(updateMsg.payload.score).toBeGreaterThan(0);

      subscriber.close();
      writer.close();
    });
  });

  // ========================================
  // SEARCH_UPDATE with changeType LEAVE
  // ========================================
  describe('SEARCH_UPDATE with changeType LEAVE on tombstone', () => {
    test('deleted matching record triggers LEAVE notification', async () => {
      const mapName = `search-leave-${Date.now()}`;

      const subscriber = await createRustTestClient(port, {
        nodeId: 'search-leave-sub-1',
        userId: 'search-leave-sub-user-1',
        roles: ['ADMIN'],
      });
      await subscriber.waitForMessage('AUTH_ACK');

      const writer = await createRustTestClient(port, {
        nodeId: 'search-leave-writer-1',
        userId: 'search-leave-writer-user-1',
        roles: ['ADMIN'],
      });
      await writer.waitForMessage('AUTH_ACK');

      // Write a document with unique term "removable"
      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'leave-put-1',
          mapName,
          opType: 'PUT',
          key: 'removable-doc',
          record: createLWWRecord({ content: 'This is removable content for deletion test' }),
        },
      });
      await writer.waitForMessage('OP_ACK');
      await waitForSync(200);

      // Subscribe to "removable"
      subscriber.messages.length = 0;
      subscriber.send({
        type: 'SEARCH_SUB',
        payload: {
          subscriptionId: 'sub-removable',
          mapName,
          query: 'removable',
        },
      });

      const initialResp = await subscriber.waitForMessage('SEARCH_RESP');
      expect(initialResp.payload.results.length).toBeGreaterThanOrEqual(1);

      // Clear to isolate LEAVE
      subscriber.messages.length = 0;

      // Update the document so it no longer matches "removable"
      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'leave-put-2',
          mapName,
          opType: 'PUT',
          key: 'removable-doc',
          record: {
            value: { content: 'Completely different content now' },
            timestamp: {
              millis: Date.now() + 1000,
              counter: 0,
              nodeId: 'search-leave-writer-1',
            },
          },
        },
      });

      // Wait for LEAVE notification
      await waitUntil(
        () =>
          subscriber.messages.some(
            (m) =>
              m.type === 'SEARCH_UPDATE' &&
              m.payload?.subscriptionId === 'sub-removable' &&
              m.payload?.changeType === 'LEAVE'
          ),
        5000
      );

      const leaveMsg = subscriber.messages.find(
        (m) =>
          m.type === 'SEARCH_UPDATE' &&
          m.payload?.subscriptionId === 'sub-removable' &&
          m.payload?.changeType === 'LEAVE'
      );
      expect(leaveMsg).toBeDefined();
      expect(leaveMsg.payload.key).toBe('removable-doc');

      subscriber.close();
      writer.close();
    });
  });

  // ========================================
  // SEARCH_UNSUB stops delivery
  // ========================================
  describe('SEARCH_UNSUB stops SEARCH_UPDATE delivery', () => {
    test('after unsubscribe, no more SEARCH_UPDATE messages', async () => {
      const mapName = `search-unsub-${Date.now()}`;

      const subscriber = await createRustTestClient(port, {
        nodeId: 'search-unsub-sub-1',
        userId: 'search-unsub-sub-user-1',
        roles: ['ADMIN'],
      });
      await subscriber.waitForMessage('AUTH_ACK');

      const writer = await createRustTestClient(port, {
        nodeId: 'search-unsub-writer-1',
        userId: 'search-unsub-writer-user-1',
        roles: ['ADMIN'],
      });
      await writer.waitForMessage('AUTH_ACK');

      // Subscribe to "monitor"
      subscriber.messages.length = 0;
      subscriber.send({
        type: 'SEARCH_SUB',
        payload: {
          subscriptionId: 'sub-monitor',
          mapName,
          query: 'monitor',
        },
      });

      await subscriber.waitForMessage('SEARCH_RESP');

      // Write a matching doc to verify subscription works
      subscriber.messages.length = 0;
      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'unsub-verify-put',
          mapName,
          opType: 'PUT',
          key: 'monitor-1',
          record: createLWWRecord({ name: '4K Monitor', description: 'Ultra HD monitor' }),
        },
      });

      // Wait for SEARCH_UPDATE proving subscription works
      await waitUntil(
        () =>
          subscriber.messages.some(
            (m) =>
              m.type === 'SEARCH_UPDATE' &&
              m.payload?.subscriptionId === 'sub-monitor'
          ),
        5000
      );

      // Now unsubscribe
      subscriber.send({
        type: 'SEARCH_UNSUB',
        payload: {
          subscriptionId: 'sub-monitor',
        },
      });

      await waitForSync(300);

      // Clear messages
      subscriber.messages.length = 0;

      // Writer adds another matching document
      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'unsub-post-put',
          mapName,
          opType: 'PUT',
          key: 'monitor-2',
          record: createLWWRecord({ name: 'Curved Monitor', description: 'Ultrawide curved monitor' }),
        },
      });

      await writer.waitForMessage('OP_ACK');

      // Wait to ensure no SEARCH_UPDATE arrives
      await waitForSync(1000);

      const postUnsubUpdates = subscriber.messages.filter(
        (m) =>
          m.type === 'SEARCH_UPDATE' &&
          m.payload?.subscriptionId === 'sub-monitor'
      );
      expect(postUnsubUpdates.length).toBe(0);

      subscriber.close();
      writer.close();
    });
  });

  // ========================================
  // Multi-client search updates
  // ========================================
  describe('Multi-client: writer + subscriber SEARCH_SUB', () => {
    test('one client writes, another clients SEARCH_SUB receives updates', async () => {
      const mapName = `search-multi-${Date.now()}`;

      const subscriber = await createRustTestClient(port, {
        nodeId: 'search-multi-sub-1',
        userId: 'search-multi-sub-user-1',
        roles: ['ADMIN'],
      });
      await subscriber.waitForMessage('AUTH_ACK');

      const writer = await createRustTestClient(port, {
        nodeId: 'search-multi-writer-1',
        userId: 'search-multi-writer-user-1',
        roles: ['ADMIN'],
      });
      await writer.waitForMessage('AUTH_ACK');

      // Subscribe to "important"
      subscriber.messages.length = 0;
      subscriber.send({
        type: 'SEARCH_SUB',
        payload: {
          subscriptionId: 'sub-important',
          mapName,
          query: 'important',
        },
      });

      await subscriber.waitForMessage('SEARCH_RESP');

      // Clear to isolate live update
      subscriber.messages.length = 0;

      // Writer adds a matching document from a different client
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'multi-put-1',
          mapName,
          opType: 'PUT',
          key: 'important-doc',
          record: createLWWRecord({ content: 'This is an important document for the team' }),
        },
      });

      // Subscriber should receive SEARCH_UPDATE ENTER
      await waitUntil(
        () =>
          subscriber.messages.some(
            (m) =>
              m.type === 'SEARCH_UPDATE' &&
              m.payload?.subscriptionId === 'sub-important' &&
              m.payload?.key === 'important-doc'
          ),
        5000
      );

      const update = subscriber.messages.find(
        (m) =>
          m.type === 'SEARCH_UPDATE' &&
          m.payload?.subscriptionId === 'sub-important'
      );
      expect(update).toBeDefined();
      expect(update.payload.changeType).toBe('ENTER');
      expect(update.payload.key).toBe('important-doc');
      expect(update.payload.score).toBeGreaterThan(0);

      subscriber.close();
      writer.close();
    });
  });
});
