import {
  createRustTestClient,
  spawnRustServer,
  createLWWRecord,
  waitForSync,
  waitUntil,
} from './helpers';

/**
 * End-to-end vector + hybrid search against a real Rust server.
 *
 * This is the integration coverage that was missing while vector/hybrid were a
 * dark feature (CAPABILITY_MATRIX F2 / TODO-464): vector=0, hybrid=0 tests.
 *
 * The server is spawned with a DETERMINISTIC embedding provider
 * (TOPGUN_EMBEDDING_PROVIDER=deterministic) so the semantic path runs without
 * an external embedding service while still producing meaningful, reproducible
 * rankings — shared tokens yield higher cosine similarity. The `products` map is
 * declared in TOPGUN_VECTOR_MAPS so writes are auto-embedded on the server.
 */
const EMBED_DIM = 64;
const VECTOR_MAPS = JSON.stringify({
  products: { fields: ['title', 'description'], dimension: EMBED_DIM },
});

const PRODUCTS = [
  {
    key: 'p1',
    value: {
      title: 'Wireless Bluetooth Headphones',
      description: 'Over ear noise cancelling wireless bluetooth headphones for music',
    },
  },
  {
    key: 'p2',
    value: {
      title: 'Organic Banana Bunch',
      description: 'Fresh organic bananas tropical fruit snack',
    },
  },
  {
    key: 'p3',
    value: {
      title: 'Wireless Gaming Mouse',
      description: 'Ergonomic wireless mouse for gaming and office',
    },
  },
];

describe('Integration: Vector & Hybrid search (Rust Server)', () => {
  describe('with a configured embedding provider', () => {
    let cleanup: () => Promise<void>;
    let port: number;

    beforeAll(async () => {
      const server = await spawnRustServer({
        env: {
          TOPGUN_EMBEDDING_PROVIDER: 'deterministic',
          TOPGUN_EMBEDDING_DIMENSION: String(EMBED_DIM),
          TOPGUN_VECTOR_MAPS: VECTOR_MAPS,
        },
      });
      port = server.port;
      cleanup = server.cleanup;
    });

    afterAll(async () => {
      await cleanup();
    });

    /** Writes the product catalog and waits for server-side auto-embedding to index it. */
    async function seedProducts(client: any): Promise<void> {
      for (const p of PRODUCTS) {
        client.messages.length = 0;
        client.send({
          type: 'CLIENT_OP',
          payload: {
            id: `vec-put-${p.key}`,
            mapName: 'products',
            opType: 'PUT',
            key: p.key,
            record: createLWWRecord(p.value),
          },
        });
        await client.waitForMessage('OP_ACK');
      }
      // Auto-embed batch window (100ms) + write-back + index insert.
      await waitForSync(600);
    }

    function hybridSearch(
      client: any,
      requestId: string,
      methods: string[],
      queryText: string,
      extra: Record<string, unknown> = {},
    ): void {
      client.messages.length = 0;
      client.send({
        type: 'HYBRID_SEARCH',
        payload: { requestId, mapName: 'products', queryText, methods, k: 10, ...extra },
      });
    }

    test('semantic search returns auto-embedded results ranked by similarity', async () => {
      const client = await createRustTestClient(port, {
        nodeId: 'vec-sem-1',
        userId: 'vec-sem-user',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');
      await seedProducts(client);

      // Retry the query a few times: auto-embed is async, so the vector may not be
      // indexed on the first attempt right after seeding.
      let results: any[] = [];
      await waitUntil(async () => {
        hybridSearch(client, 'sem-req', ['semantic'], 'wireless headphones for listening to music');
        const resp = await client.waitForMessage('HYBRID_SEARCH_RESP');
        expect(resp.payload.error).toBeUndefined();
        results = resp.payload.results;
        return results.length > 0;
      }, 8000);

      const keys = results.map((r) => r.key);
      expect(keys).toContain('p1'); // headphones — shares the most query tokens

      // Every result must expose a semantic method score (proves the vector leg ran,
      // not a silent fallback to another method).
      for (const r of results) {
        expect(typeof r.score).toBe('number');
        expect(r.methodScores).toBeDefined();
        expect(typeof r.methodScores.semantic).toBe('number');
      }

      // The headphones product must outrank the unrelated banana product.
      const rank = (k: string) => keys.indexOf(k);
      if (rank('p2') !== -1) {
        expect(rank('p1')).toBeLessThan(rank('p2'));
      }

      client.close();
    });

    test('hybrid fullText + semantic fuses both method scores via RRF', async () => {
      const client = await createRustTestClient(port, {
        nodeId: 'vec-hyb-1',
        userId: 'vec-hyb-user',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');
      await seedProducts(client);

      let results: any[] = [];
      await waitUntil(async () => {
        hybridSearch(client, 'hyb-req', ['fullText', 'semantic'], 'wireless');
        const resp = await client.waitForMessage('HYBRID_SEARCH_RESP');
        expect(resp.payload.error).toBeUndefined();
        results = resp.payload.results;
        // Wait until at least one result has been scored by BOTH methods.
        return results.some(
          (r) =>
            typeof r.methodScores?.fullText === 'number' &&
            typeof r.methodScores?.semantic === 'number',
        );
      }, 8000);

      const keys = results.map((r) => r.key);
      // The two "wireless" products should surface.
      expect(keys).toContain('p1');
      expect(keys).toContain('p3');

      const fused = results.find(
        (r) =>
          typeof r.methodScores?.fullText === 'number' &&
          typeof r.methodScores?.semantic === 'number',
      );
      expect(fused).toBeDefined();
      expect(fused.score).toBeGreaterThan(0);

      client.close();
    });
  });

  describe('honesty: without a configured embedding provider', () => {
    let cleanup: () => Promise<void>;
    let port: number;

    beforeAll(async () => {
      // No TOPGUN_EMBEDDING_PROVIDER -> semantic search is disabled. It must return
      // an explicit error rather than silent empty/garbage results.
      const server = await spawnRustServer();
      port = server.port;
      cleanup = server.cleanup;
    });

    afterAll(async () => {
      await cleanup();
    });

    test('semantic search returns an explicit error, never silent fake results', async () => {
      const client = await createRustTestClient(port, {
        nodeId: 'vec-honesty-1',
        userId: 'vec-honesty-user',
        roles: ['ADMIN'],
      });
      await client.waitForMessage('AUTH_ACK');

      client.messages.length = 0;
      client.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'honesty-put',
          mapName: 'unconfigured',
          opType: 'PUT',
          key: 'x1',
          record: createLWWRecord({ title: 'Wireless Headphones' }),
        },
      });
      await client.waitForMessage('OP_ACK');
      await waitForSync(200);

      client.messages.length = 0;
      client.send({
        type: 'HYBRID_SEARCH',
        payload: {
          requestId: 'honesty-req',
          mapName: 'unconfigured',
          queryText: 'wireless headphones',
          methods: ['semantic'],
          k: 5,
        },
      });

      const resp = await client.waitForMessage('HYBRID_SEARCH_RESP');
      expect(resp.payload.requestId).toBe('honesty-req');
      // The contract: no silent fake results. An explicit error is surfaced and
      // the result set is empty (not fabricated).
      expect(resp.payload.error).toBeDefined();
      expect(resp.payload.results).toHaveLength(0);

      client.close();
    });
  });
});
