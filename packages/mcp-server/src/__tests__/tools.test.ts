/**
 * MCP Tools Unit Tests
 */

import type { ToolContext, ResolvedMCPServerConfig } from '../types';
import { QueryOnceUnsettledError } from '@topgunbuild/client';
import { handleQuery } from '../tools/query';
import { handleMutate } from '../tools/mutate';
import { handleSchema } from '../tools/schema';
import { handleStats } from '../tools/stats';
import { handleExplain } from '../tools/explain';
import { handleListMaps } from '../tools/listMaps';
import { handleSearch } from '../tools/search';

// Mock TopGunClient
class MockLWWMap {
  private data = new Map<string, unknown>();

  set(key: string, value: unknown) {
    this.data.set(key, value);
    return { value, timestamp: Date.now() };
  }

  get(key: string) {
    return this.data.get(key);
  }

  remove(key: string) {
    this.data.delete(key);
    return { value: null, timestamp: Date.now() };
  }

  entries() {
    return this.data.entries();
  }
}

interface MockSearchHit {
  key: string;
  value: null;
  score: number;
  matchedTerms: string[];
}

interface MockHybridSearchHit {
  key: string;
  score: number;
  methodScores: Partial<Record<'exact' | 'fullText' | 'semantic', number>>;
  value?: unknown;
}

class MockTopGunClient {
  private maps = new Map<string, MockLWWMap>();
  // Configurable BM25 search results (legacy — not used by handleSearch after this spec).
  searchResults: MockSearchHit[] = [];
  // Configurable hybrid search results + recorded options for assertion.
  hybridSearchResults: MockHybridSearchHit[] = [];
  lastHybridSearchOptions: unknown = null;
  // When set, hybridSearch rejects with this error to simulate server-side failures
  // (no embedding model, FTS not enabled for the map, etc.).
  hybridSearchRejection: Error | null = null;
  // When set, queryOncePaged rejects with this error to simulate offline / not-settled.
  queryOnceRejection: Error | null = null;
  // Override hasMore/cursor returned by queryOncePaged for pagination tests.
  queryOncePagedHasMore = false;
  queryOncePagedCursor: string | undefined = undefined;
  // Outcome confirmWrite resolves with — defaults to a server-confirmed write so
  // the happy path needs no setup; override to exercise offline/timeout/failed.
  confirmWriteOutcome: 'synced' | 'offline' | 'timeout' | 'failed' = 'synced';
  // Records (map, key) pairs confirmWrite was asked to confirm, for assertions.
  confirmWriteCalls: Array<{ map: string; key: string }> = [];

  getMap(name: string): MockLWWMap {
    if (!this.maps.has(name)) {
      this.maps.set(name, new MockLWWMap());
    }
    return this.maps.get(name)!;
  }

  async confirmWrite(
    map: string,
    key: string,
    _timeoutMs?: number,
  ): Promise<'synced' | 'offline' | 'timeout' | 'failed'> {
    this.confirmWriteCalls.push({ map, key });
    return this.confirmWriteOutcome;
  }

  getConnectionState() {
    return 'CONNECTED';
  }

  isCluster() {
    return false;
  }

  getPendingOpsCount() {
    return 0;
  }

  isBackpressurePaused() {
    return false;
  }

  getConnectedNodes() {
    return [];
  }

  getPartitionMapVersion() {
    return 0;
  }

  isRoutingActive() {
    return false;
  }

  async search(_map: string, _query: string, _options?: unknown) {
    return this.searchResults;
  }

  async hybridSearch(_map: string, _query: string, options?: unknown) {
    // Record the options so tests can assert which methods/k were forwarded.
    this.lastHybridSearchOptions = options;
    if (this.hybridSearchRejection) {
      throw this.hybridSearchRejection;
    }
    return this.hybridSearchResults;
  }

  // One-shot paged read mirroring TopGunClient.queryOncePaged: resolves with
  // settled, authoritative server data including cursor metadata, or rejects
  // to simulate offline / not-settled.
  async queryOncePaged(
    mapName: string,
    filter: { where?: Record<string, unknown>; limit?: number; cursor?: string } = {},
  ): Promise<{
    items: Array<Record<string, unknown> & { _key: string }>;
    cursor?: string;
    hasMore: boolean;
  }> {
    if (this.queryOnceRejection) {
      throw this.queryOnceRejection;
    }

    const lwwMap = this.maps.get(mapName);
    let items: Array<Record<string, unknown> & { _key: string }> = [];

    if (lwwMap) {
      for (const [key, value] of lwwMap.entries()) {
        if (value !== null && value !== undefined) {
          items.push({ ...(value as Record<string, unknown>), _key: key });
        }
      }
    }

    // Apply where filter
    if (filter.where) {
      const where = filter.where as Record<string, unknown>;
      items = items.filter((item) => Object.entries(where).every(([k, v]) => item[k] === v));
    }

    // Apply limit
    if (filter.limit !== undefined && filter.limit < items.length) {
      items = items.slice(0, filter.limit);
    }

    return {
      items,
      cursor: this.queryOncePagedCursor,
      hasMore: this.queryOncePagedHasMore,
    };
  }

  // Legacy queryOnce for any remaining callers outside handleQuery
  async queryOnce(
    mapName: string,
    filter: { where?: Record<string, unknown>; limit?: number } = {},
  ): Promise<Array<Record<string, unknown> & { _key: string }>> {
    const result = await this.queryOncePaged(mapName, filter);
    return result.items;
  }
}

function createTestContext(config?: Partial<ResolvedMCPServerConfig>): ToolContext {
  return {
    client: new MockTopGunClient() as unknown as ToolContext['client'],
    config: {
      name: 'topgun-server',
      version: '1.0.0',
      topgunUrl: 'ws://localhost:8080',
      enableMutations: true,
      enableSubscriptions: true,
      defaultLimit: 10,
      maxLimit: 100,
      subscriptionTimeoutSeconds: 60,
      debug: false,
      ...config,
    },
  };
}

describe('MCP Tools', () => {
  describe('handleListMaps', () => {
    it('should list allowed maps when configured', async () => {
      const ctx = createTestContext({
        allowedMaps: ['tasks', 'users', 'products'],
      });

      const result = await handleListMaps({}, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('tasks');
      expect(result.content[0].text).toContain('users');
      expect(result.content[0].text).toContain('products');
    });

    it('should indicate no restrictions when allowedMaps not set', async () => {
      const ctx = createTestContext();

      const result = await handleListMaps({}, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('all maps');
    });
  });

  describe('handleQuery', () => {
    it('should query an empty map', async () => {
      const ctx = createTestContext();

      const result = await handleQuery({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('No results found');
    });

    it('should return results from populated map', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { title: 'Test Task', status: 'todo' });
      map.set('task2', { title: 'Another Task', status: 'done' });

      const result = await handleQuery({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('2 result');
      expect(result.content[0].text).toContain('Test Task');
      expect(result.content[0].text).toContain('Another Task');
    });

    it('should filter results', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { title: 'Test Task', status: 'todo' });
      map.set('task2', { title: 'Done Task', status: 'done' });

      const result = await handleQuery({ map: 'tasks', filter: { status: 'done' } }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('1 result');
      expect(result.content[0].text).toContain('Done Task');
      expect(result.content[0].text).not.toContain('Test Task');
    });

    it('should respect limit', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      for (let i = 0; i < 20; i++) {
        map.set(`task${i}`, { title: `Task ${i}`, index: i });
      }

      const result = await handleQuery({ map: 'tasks', limit: 5 }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('5 result');
    });

    it('should signal truncation and provide continuation cursor when hasMore is true', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      const map = mockClient.getMap('tasks');
      for (let i = 0; i < 5; i++) {
        map.set(`task${i}`, { title: `Task ${i}`, index: i });
      }
      // Server-authoritative pagination signal: more results exist
      mockClient.queryOncePagedHasMore = true;
      mockClient.queryOncePagedCursor = 'next-page-cursor-abc';

      const result = await handleQuery({ map: 'tasks', limit: 5 }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('5 result');
      // The agent must be told the view was capped — never a silent truncation.
      expect(result.content[0].text).toContain('More rows match than were returned');
      // The real continuation cursor must be surfaced so the agent can page forward.
      expect(result.content[0].text).toContain('cursor: "next-page-cursor-abc"');
    });

    it('should NOT signal truncation when results fit within the limit', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { title: 'Only Task', status: 'todo' });

      const result = await handleQuery({ map: 'tasks', limit: 10 }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('1 result');
      expect(result.content[0].text).not.toContain('More rows match');
    });

    it('should thread cursor arg through to queryOncePaged when provided', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      const map = mockClient.getMap('tasks');
      map.set('task1', { title: 'Only Task', status: 'todo' });

      // Cursor is a valid schema field — the call must succeed and return normally.
      const result = await handleQuery({ map: 'tasks', cursor: 'page-2-cursor' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('1 result');
    });

    it('should return continuation cursor in result text when hasMore is true', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      mockClient.getMap('tasks').set('task1', { title: 'Task 1' });
      mockClient.queryOncePagedHasMore = true;
      mockClient.queryOncePagedCursor = 'cursor-token-xyz';

      const result = await handleQuery({ map: 'tasks', limit: 1 }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('cursor: "cursor-token-xyz"');
      // Should NOT say there is no cursor to page through
      expect(result.content[0].text).not.toContain('no cursor to page through');
    });

    it('should NOT include continuation note when hasMore is false', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      mockClient.getMap('tasks').set('task1', { title: 'Only Task', status: 'todo' });
      // Default: queryOncePagedHasMore = false

      const result = await handleQuery({ map: 'tasks', limit: 10 }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('1 result');
      expect(result.content[0].text).not.toContain('More rows match');
      expect(result.content[0].text).not.toContain('cursor:');
    });

    it('should deny access to restricted maps', async () => {
      const ctx = createTestContext({
        allowedMaps: ['users'],
      });

      const result = await handleQuery({ map: 'tasks' }, ctx);

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('not allowed');
    });

    it('should return settled server data (not a silent empty)', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      // A record that exists authoritatively on the server.
      map.set('task1', { title: 'Server Record', status: 'todo' });

      const result = await handleQuery({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('1 result');
      expect(result.content[0].text).toContain('Server Record');
      // Settled server data must never read as the offline/not-settled message.
      expect(result.content[0].text).not.toContain('not settled');
    });

    it('should surface an explicit not-settled message when offline', async () => {
      const ctx = createTestContext();
      (ctx.client as unknown as MockTopGunClient).queryOnceRejection = new QueryOnceUnsettledError(
        'offline',
        'tasks',
      );

      const result = await handleQuery({ map: 'tasks' }, ctx);

      expect(result.isError).toBe(true);
      // Must explicitly signal offline / not-settled, never the silent empty text.
      expect(result.content[0].text).toContain('not settled');
      expect(result.content[0].text).toContain('offline');
      expect(result.content[0].text).not.toContain('No results found');
    });

    it('should surface an explicit not-settled message on timeout', async () => {
      const ctx = createTestContext();
      (ctx.client as unknown as MockTopGunClient).queryOnceRejection = new QueryOnceUnsettledError(
        'timeout',
        'tasks',
      );

      const result = await handleQuery({ map: 'tasks' }, ctx);

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('not settled');
      expect(result.content[0].text).toContain('timed out');
      expect(result.content[0].text).not.toContain('No results found');
    });

    it('should still report a settled-but-empty server result as no results', async () => {
      const ctx = createTestContext();

      const result = await handleQuery({ map: 'tasks' }, ctx);

      // Genuinely empty (but settled) server answer — distinct from offline.
      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('No results found');
    });
  });

  describe('handleMutate', () => {
    it('should save a record only after the server confirms it', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;

      const result = await handleMutate(
        {
          map: 'tasks',
          operation: 'set',
          key: 'task1',
          data: { title: 'New Task', status: 'todo' },
        },
        ctx,
      );

      expect(result.isError).toBeUndefined();
      // Upsert wording — never claims create-vs-update (cold cache can't tell).
      expect(result.content[0].text).toContain('Successfully saved');
      expect(result.content[0].text).toContain('confirmed on server');
      expect(result.content[0].text).toContain('task1');
      // The success was gated on a server confirmation for this exact (map, key).
      expect(mockClient.confirmWriteCalls).toEqual([{ map: 'tasks', key: 'task1' }]);
    });

    it('should use upsert wording for an existing record too', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { title: 'Old Title', status: 'todo' });

      const result = await handleMutate(
        {
          map: 'tasks',
          operation: 'set',
          key: 'task1',
          data: { title: 'Updated Title', status: 'done' },
        },
        ctx,
      );

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('Successfully saved');
    });

    it('should NOT report success when a set is not confirmed (offline)', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      mockClient.confirmWriteOutcome = 'offline';

      const result = await handleMutate(
        {
          map: 'tasks',
          operation: 'set',
          key: 'task1',
          data: { title: 'Unsynced', status: 'todo' },
        },
        ctx,
      );

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('NOT yet durable');
      expect(result.content[0].text).not.toContain('Successfully');
    });

    it('should report an error when a set times out without server confirmation', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      mockClient.confirmWriteOutcome = 'timeout';

      const result = await handleMutate(
        { map: 'tasks', operation: 'set', key: 'task1', data: { title: 'Slow' } },
        ctx,
      );

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('NOT yet confirmed durable');
    });

    it('should remove a record once the server confirms it', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { title: 'To Delete', status: 'todo' });

      const result = await handleMutate(
        {
          map: 'tasks',
          operation: 'remove',
          key: 'task1',
        },
        ctx,
      );

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('Successfully removed');
      expect(result.content[0].text).toContain('confirmed on server');
    });

    it('should issue a server-authoritative remove for a key absent from the local cache', async () => {
      // F5: the record exists on the server but is NOT in the cold MCP cache. The
      // remove must still be issued (server-authoritative) and reported as done —
      // never silently no-op'd with a false "does not exist".
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      const removeSpy = jest.spyOn(mockClient.getMap('tasks'), 'remove');

      const result = await handleMutate(
        {
          map: 'tasks',
          operation: 'remove',
          key: 'server-only-key',
        },
        ctx,
      );

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('Successfully removed');
      expect(result.content[0].text).not.toContain('does not exist');
      // The tombstone was actually written (not short-circuited on cold cache)...
      expect(removeSpy).toHaveBeenCalledWith('server-only-key');
      // ...and success was gated on a server confirmation.
      expect(mockClient.confirmWriteCalls).toEqual([{ map: 'tasks', key: 'server-only-key' }]);
    });

    it('should NOT report a remove as done when it is not confirmed (offline)', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      mockClient.confirmWriteOutcome = 'offline';

      const result = await handleMutate({ map: 'tasks', operation: 'remove', key: 'task1' }, ctx);

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('NOT yet durable');
      expect(result.content[0].text).not.toContain('Successfully');
    });

    it('should error when mutations are disabled', async () => {
      const ctx = createTestContext({ enableMutations: false });

      const result = await handleMutate(
        {
          map: 'tasks',
          operation: 'set',
          key: 'task1',
          data: { title: 'Test' },
        },
        ctx,
      );

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('disabled');
    });

    it('should require data for set operation', async () => {
      const ctx = createTestContext();

      const result = await handleMutate(
        {
          map: 'tasks',
          operation: 'set',
          key: 'task1',
        },
        ctx,
      );

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('"data" is required');
    });
  });

  describe('handleSchema', () => {
    it('should report empty map', async () => {
      const ctx = createTestContext();

      const result = await handleSchema({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('empty');
    });

    it('should infer schema from data', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', {
        title: 'Test',
        count: 42,
        active: true,
        createdAt: '2025-01-01T00:00:00Z',
      });

      const result = await handleSchema({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('title: string');
      expect(result.content[0].text).toContain('count: number');
      expect(result.content[0].text).toContain('active: boolean');
      expect(result.content[0].text).toContain('timestamp');
    });

    it('should detect enum fields', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { status: 'todo' });
      map.set('task2', { status: 'in-progress' });
      map.set('task3', { status: 'done' });

      const result = await handleSchema({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('enum');
      expect(result.content[0].text).toContain('todo');
      expect(result.content[0].text).toContain('done');
    });
  });

  describe('handleStats', () => {
    it('should return connection stats', async () => {
      const ctx = createTestContext();

      const result = await handleStats({}, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('CONNECTED');
      expect(result.content[0].text).toContain('Pending Operations: 0');
    });

    it('should return map stats when specified', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { title: 'Task 1' });
      map.set('task2', { title: 'Task 2' });

      const result = await handleStats({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('tasks');
      expect(result.content[0].text).toContain('Records: 2');
    });
  });

  describe('handleExplain', () => {
    it('should explain full scan without filter', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { title: 'Task 1' });
      map.set('task2', { title: 'Task 2' });

      const result = await handleExplain({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('FULL_SCAN');
      expect(result.content[0].text).toContain('Total Records: 2');
    });

    it('should explain filter scan', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      for (let i = 0; i < 10; i++) {
        map.set(`task${i}`, { title: `Task ${i}`, status: i % 2 === 0 ? 'done' : 'todo' });
      }

      const result = await handleExplain({ map: 'tasks', filter: { status: 'done' } }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('FILTER_SCAN');
      expect(result.content[0].text).toContain('Apply filter');
      expect(result.content[0].text).toContain('Estimated Results: 5');
      expect(result.content[0].text).toContain('Selectivity: 50');
    });

    it('should provide recommendations for large datasets', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      for (let i = 0; i < 1000; i++) {
        map.set(`task${i}`, { title: `Task ${i}`, status: 'todo' });
      }

      const result = await handleExplain({ map: 'tasks', filter: { status: 'todo' } }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('Recommendations');
    });
  });

  describe('handleSearch', () => {
    it('should return hydrated record body for a matched hit', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      // Pre-populate the map so the local read-by-key finds the body.
      mockClient.getMap('tasks').set('task1', { title: 'Test Task', status: 'todo' });
      // Wire hybridSearch() to return a hit with the hybrid result shape (methodScores, no matchedTerms).
      mockClient.hybridSearchResults = [
        { key: 'task1', score: 0.42, methodScores: { fullText: 0.42 } },
      ];

      const result = await handleSearch({ map: 'tasks', query: 'test' }, ctx);

      expect(result.isError).toBeUndefined();
      // Body fields must be present in the output.
      expect(result.content[0].text).toContain('Test Task');
      expect(result.content[0].text).toContain('todo');
      // The old null placeholder must not appear.
      expect(result.content[0].text).not.toContain('Data: null');
    });

    it('should preserve score with fixed-precision rendering and per-method score breakdown', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      mockClient.getMap('tasks').set('task1', { title: 'Test Task', status: 'todo' });
      mockClient.getMap('tasks').set('task2', { title: 'Smoke Task', status: 'done' });
      // Two hits: one plain three-decimal score, one rounding-sensitive score matching
      // the real smoke output (0.2876 → "0.288") to guard toFixed(3) against digit-dropping.
      mockClient.hybridSearchResults = [
        { key: 'task1', score: 0.42, methodScores: { fullText: 0.42 } },
        { key: 'task2', score: 0.2876, methodScores: { fullText: 0.2876 } },
      ];

      const result = await handleSearch({ map: 'tasks', query: 'test smoke' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('[Score: 0.420]');
      // Rounding-sensitive assertion: 0.2876 must render as 0.288, not 0.287.
      expect(result.content[0].text).toContain('[Score: 0.288]');
      // Per-method scores must appear in the output — the old matchedTerms line is gone.
      expect(result.content[0].text).toContain('Method scores:');
      expect(result.content[0].text).not.toContain('Matched:');
    });

    it('should emit the not-available-locally marker when the key is absent from the local replica', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      // Intentionally do NOT pre-populate the map — simulates an evicted or
      // partially-replicated record where lwwMap.get() returns undefined.
      mockClient.hybridSearchResults = [
        { key: 'missing-key', score: 0.9, methodScores: { fullText: 0.9 } },
      ];

      // Handler must not throw; the hit must still appear with the pinned fallback marker.
      const result = await handleSearch({ map: 'tasks', query: 'term' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('Data: (record body not available locally)');
    });

    it('should return the empty-results message when search returns no hits', async () => {
      const ctx = createTestContext();
      // hybridSearchResults defaults to [] — no configuration needed.

      const result = await handleSearch({ map: 'tasks', query: 'nothing' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('No results found');
    });

    it('should call hybridSearch (not the BM25 search path) with the default fullText method', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      mockClient.hybridSearchResults = [
        { key: 'doc1', score: 0.75, methodScores: { fullText: 0.75 } },
      ];

      // Invoke with no methods arg — the default ['fullText'] must be forwarded.
      await handleSearch({ map: 'docs', query: 'auth' }, ctx);

      // The tool must have gone through hybridSearch, not the old BM25 search path.
      // If hybridSearch was called, lastHybridSearchOptions is populated; if search()
      // was called instead, it would remain null.
      const opts = mockClient.lastHybridSearchOptions as {
        methods: string[];
        k: number;
        minScore: number;
      };
      expect(opts).not.toBeNull();
      expect(opts.methods).toEqual(['fullText']);
    });

    it('should forward custom methods to hybridSearch and render multi-method score breakdown', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      mockClient.hybridSearchResults = [
        { key: 'doc1', score: 0.82, methodScores: { exact: 0.6, fullText: 0.5 } },
      ];

      const result = await handleSearch(
        { map: 'docs', query: 'login', methods: ['exact', 'fullText'] },
        ctx,
      );

      const opts = mockClient.lastHybridSearchOptions as {
        methods: string[];
        k: number;
        minScore: number;
      };
      // Both requested methods must be forwarded verbatim.
      expect(opts.methods).toEqual(['exact', 'fullText']);
      // Output must render per-method scores for both legs so the agent can see the breakdown.
      expect(result.content[0].text).toContain('exact:');
      expect(result.content[0].text).toContain('fullText:');
    });

    it('should surface an actionable retry message when the server cannot embed for the semantic leg', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      mockClient.hybridSearchRejection = new Error(
        'failed to embed query: no embedding model configured',
      );

      const result = await handleSearch(
        { map: 'docs', query: 'login', methods: ['fullText', 'semantic'] },
        ctx,
      );

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('Semantic search requires server-side embedding');
      // The message must tell the agent how to retry without the semantic leg.
      expect(result.content[0].text).toContain('methods: ["fullText"]');
    });

    it('should point the agent at topgun_query when full-text search is not enabled for the map', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      mockClient.hybridSearchRejection = new Error("FTS is not enabled for map 'docs'");

      const result = await handleSearch({ map: 'docs', query: 'login' }, ctx);

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain("Full-text search is not enabled for map 'docs'");
      expect(result.content[0].text).toContain('topgun_query');
    });
  });
});
