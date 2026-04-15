/**
 * HybridSearchClient Tests
 *
 * Unit tests for HybridSearchClient covering request/response matching,
 * vector conversion, timeout, error handling, and close behaviour.
 * Mirrors VectorSearchClient.test.ts structure, adapted for HYBRID_SEARCH wire protocol.
 */

import { HybridSearchClient } from '../HybridSearchClient';
import { HybridSearchPayloadSchema } from '@topgunbuild/core';
import type { HybridSearchClientResult } from '../types';

// Helper to create a configured client with mock sendMessage
function createClient(options?: {
  sendMessage?: jest.Mock;
  isAuthenticated?: () => boolean;
  timeoutMs?: number;
}) {
  const sendMessage = options?.sendMessage ?? jest.fn().mockReturnValue(true);
  const isAuthenticated = options?.isAuthenticated ?? (() => true);
  const client = new HybridSearchClient({
    sendMessage,
    isAuthenticated,
    timeoutMs: options?.timeoutMs,
  });
  return { client, sendMessage };
}

describe('HybridSearchClient', () => {
  beforeEach(() => {
    jest.useFakeTimers();
  });

  afterEach(() => {
    jest.useRealTimers();
  });

  // ============================================
  // AC #1: Correct payload shape + schema parse
  // ============================================

  describe('hybridSearch() payload', () => {
    it('should send HYBRID_SEARCH with correct payload and pass schema parse (AC #1)', async () => {
      const { client, sendMessage } = createClient();

      const searchPromise = client.hybridSearch('docs', 'machine learning', {
        methods: ['fullText', 'semantic'],
        k: 5,
      });

      expect(sendMessage).toHaveBeenCalledTimes(1);
      const sentMsg = sendMessage.mock.calls[0][0];
      expect(sentMsg.type).toBe('HYBRID_SEARCH');

      const payload = sentMsg.payload;
      expect(payload.requestId).toMatch(
        /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
      );
      expect(payload.mapName).toBe('docs');
      expect(payload.queryText).toBe('machine learning');
      expect(payload.methods).toEqual(['fullText', 'semantic']);
      expect(payload.k).toBe(5);

      // Cheapest defense against wire-drift between this spec and SPEC-207 schema
      expect(() => HybridSearchPayloadSchema.parse(payload)).not.toThrow();

      // Clean up
      client.handleResponse({
        requestId: payload.requestId,
        results: [],
        searchTimeMs: 1,
      });
      await searchPromise;
    });

    it('should default methods to ["fullText"] and k to 10 when omitted', async () => {
      const { client, sendMessage } = createClient();

      const searchPromise = client.hybridSearch('notes', 'hello');

      const payload = sendMessage.mock.calls[0][0].payload;
      expect(payload.methods).toEqual(['fullText']);
      expect(payload.k).toBe(10);

      // Schema parse passes with defaults
      expect(() => HybridSearchPayloadSchema.parse(payload)).not.toThrow();

      client.handleResponse({ requestId: payload.requestId, results: [], searchTimeMs: 0 });
      await searchPromise;
    });

    it('should omit optional fields entirely when not provided', async () => {
      const { client, sendMessage } = createClient();

      const searchPromise = client.hybridSearch('notes', 'test');
      const payload = sendMessage.mock.calls[0][0].payload;

      expect(payload.queryVector).toBeUndefined();
      expect(payload.predicate).toBeUndefined();
      expect(payload.includeValue).toBeUndefined();
      expect(payload.minScore).toBeUndefined();

      client.handleResponse({ requestId: payload.requestId, results: [], searchTimeMs: 0 });
      await searchPromise;
    });

    it('should include optional fields when provided', async () => {
      const { client, sendMessage } = createClient();

      const searchPromise = client.hybridSearch('notes', 'test', {
        predicate: { field: 'status', value: 'active' },
        includeValue: true,
        minScore: 0.5,
      });
      const payload = sendMessage.mock.calls[0][0].payload;

      expect(payload.predicate).toEqual({ field: 'status', value: 'active' });
      expect(payload.includeValue).toBe(true);
      expect(payload.minScore).toBe(0.5);

      client.handleResponse({ requestId: payload.requestId, results: [], searchTimeMs: 0 });
      await searchPromise;
    });
  });

  // ============================================
  // AC #2: queryVector conversion
  // ============================================

  describe('queryVector conversion (AC #2)', () => {
    it('should convert Float32Array to little-endian Uint8Array via vectorToBytes', async () => {
      const { client, sendMessage } = createClient();

      const queryVector = new Float32Array([0.1, 0.2]);
      const searchPromise = client.hybridSearch('docs', 'query', {
        methods: ['semantic'],
        queryVector,
      });

      const payload = sendMessage.mock.calls[0][0].payload;

      // 2 floats * 4 bytes = 8 bytes little-endian Uint8Array
      expect(payload.queryVector).toBeInstanceOf(Uint8Array);
      expect(payload.queryVector.byteLength).toBe(8);

      // Schema parse passes with queryVector as Uint8Array
      expect(() => HybridSearchPayloadSchema.parse(payload)).not.toThrow();

      client.handleResponse({ requestId: payload.requestId, results: [], searchTimeMs: 0 });
      await searchPromise;
    });

    it('should convert number[] queryVector to Uint8Array', async () => {
      const { client, sendMessage } = createClient();

      const searchPromise = client.hybridSearch('docs', 'query', {
        queryVector: [1.0, 2.0, 3.0],
      });

      const payload = sendMessage.mock.calls[0][0].payload;

      // 3 floats * 4 bytes = 12 bytes
      expect(payload.queryVector).toBeInstanceOf(Uint8Array);
      expect(payload.queryVector.byteLength).toBe(12);

      client.handleResponse({ requestId: payload.requestId, results: [], searchTimeMs: 0 });
      await searchPromise;
    });
  });

  // ============================================
  // AC #3: handleResponse resolves with correct results
  // ============================================

  describe('handleResponse() resolve (AC #3)', () => {
    it('should resolve with HybridSearchClientResult[] preserving all fields', async () => {
      const { client, sendMessage } = createClient();

      const searchPromise = client.hybridSearch('docs', 'test');
      const { requestId } = sendMessage.mock.calls[0][0].payload;

      const serverResults: HybridSearchClientResult[] = [
        {
          key: 'doc-1',
          score: 0.95,
          methodScores: { fullText: 0.9, semantic: 0.85 },
          value: { title: 'Machine Learning Basics' },
        },
        {
          key: 'doc-2',
          score: 0.87,
          methodScores: { fullText: 0.8 },
        },
      ];

      client.handleResponse({
        requestId,
        results: serverResults,
        searchTimeMs: 5,
      });

      const results = await searchPromise;

      expect(results).toHaveLength(2);
      expect(results[0]).toEqual({
        key: 'doc-1',
        score: 0.95,
        methodScores: { fullText: 0.9, semantic: 0.85 },
        value: { title: 'Machine Learning Basics' },
      });
      expect(results[1]).toEqual({
        key: 'doc-2',
        score: 0.87,
        methodScores: { fullText: 0.8 },
      });
      // value omitted when not present
      expect(results[1].value).toBeUndefined();
    });

    it('should handle empty results array', async () => {
      const { client, sendMessage } = createClient();

      const searchPromise = client.hybridSearch('notes', 'no match');
      const { requestId } = sendMessage.mock.calls[0][0].payload;

      client.handleResponse({ requestId, results: [], searchTimeMs: 2 });

      const results = await searchPromise;
      expect(results).toEqual([]);
    });
  });

  // ============================================
  // AC #4: handleResponse rejects on error
  // ============================================

  describe('handleResponse() error (AC #4)', () => {
    it('should reject with Error when payload.error is set', async () => {
      const { client, sendMessage } = createClient();

      const searchPromise = client.hybridSearch('docs', 'test');
      const { requestId } = sendMessage.mock.calls[0][0].payload;

      client.handleResponse({
        requestId,
        results: [],
        searchTimeMs: 1,
        error: 'Index not found: docs',
      });

      await expect(searchPromise).rejects.toThrow('Index not found: docs');
    });
  });

  // ============================================
  // AC #5: Timeout behaviour
  // ============================================

  describe('timeout (AC #5)', () => {
    it('should reject after 30s default timeout', async () => {
      const { client, sendMessage } = createClient();

      const searchPromise = client.hybridSearch('notes', 'slow');

      jest.advanceTimersByTime(35000);

      await expect(searchPromise).rejects.toThrow('Hybrid search request timed out');

      // Pending entry removed from map — late response is a no-op
      const { requestId } = sendMessage.mock.calls[0][0].payload;
      expect(() =>
        client.handleResponse({ requestId, results: [], searchTimeMs: 0 })
      ).not.toThrow();
    });

    it('should respect custom timeoutMs', async () => {
      const { client } = createClient({ timeoutMs: 5000 });

      const searchPromise = client.hybridSearch('notes', 'fast timeout');

      jest.advanceTimersByTime(6000);

      await expect(searchPromise).rejects.toThrow('Hybrid search request timed out');
    });
  });

  // ============================================
  // AC #6: close() behaviour
  // ============================================

  describe('close() (AC #6)', () => {
    it('should clear timeouts without rejecting when called without error', async () => {
      const { client, sendMessage } = createClient();

      const p1 = client.hybridSearch('notes', 'query 1');
      const p2 = client.hybridSearch('notes', 'query 2');

      // Close without error — pending promises stay unresolved (not rejected)
      client.close();

      // Advance timers: no timeout rejections should fire
      jest.advanceTimersByTime(60000);

      // The promises should be settled only if we resolve them, but after close
      // the map is cleared so handleResponse is a no-op
      const r1 = sendMessage.mock.calls[0][0].payload.requestId;
      const r2 = sendMessage.mock.calls[1][0].payload.requestId;

      client.handleResponse({ requestId: r1, results: [], searchTimeMs: 0 });
      client.handleResponse({ requestId: r2, results: [], searchTimeMs: 0 });

      // Promises neither resolve nor reject — they hang, matching VectorSearchClient.close() contract
      // We verify no unhandled rejections by ensuring the settled state is pending via a race
      const settled = await Promise.race([
        p1.then(() => 'resolved').catch(() => 'rejected'),
        p2.then(() => 'resolved').catch(() => 'rejected'),
        new Promise<string>((r) => setTimeout(() => r('pending'), 0)),
      ]);
      expect(settled).toBe('pending');
    });

    it('should reject all pending promises when called with an error', async () => {
      const { client } = createClient();

      const p1 = client.hybridSearch('notes', 'query 1');
      const p2 = client.hybridSearch('notes', 'query 2');

      const closeError = new Error('SyncEngine closed');
      client.close(closeError);

      await expect(p1).rejects.toThrow('SyncEngine closed');
      await expect(p2).rejects.toThrow('SyncEngine closed');
    });
  });

  // ============================================
  // sendMessage failure
  // ============================================

  describe('sendMessage failure', () => {
    it('should reject immediately when sendMessage returns false', async () => {
      const { client } = createClient({ sendMessage: jest.fn().mockReturnValue(false) });

      await expect(client.hybridSearch('notes', 'query')).rejects.toThrow(
        'Failed to send hybrid search request'
      );
    });
  });

  // ============================================
  // Authentication guard
  // ============================================

  describe('authentication guard', () => {
    it('should throw when not authenticated', async () => {
      const { client } = createClient({ isAuthenticated: () => false });

      await expect(client.hybridSearch('notes', 'query')).rejects.toThrow(
        'Not connected to server'
      );
    });
  });

  // ============================================
  // Concurrent requests / request id isolation
  // ============================================

  describe('concurrent requests', () => {
    it('should generate unique request ids for concurrent requests', async () => {
      const { client, sendMessage } = createClient();

      const p1 = client.hybridSearch('notes', 'query 1');
      const p2 = client.hybridSearch('notes', 'query 2');

      const id1 = sendMessage.mock.calls[0][0].payload.requestId;
      const id2 = sendMessage.mock.calls[1][0].payload.requestId;

      expect(id1).not.toBe(id2);

      const results1: HybridSearchClientResult[] = [{ key: 'a', score: 0.9, methodScores: {} }];
      const results2: HybridSearchClientResult[] = [{ key: 'b', score: 0.8, methodScores: {} }];

      client.handleResponse({ requestId: id1, results: results1, searchTimeMs: 1 });
      client.handleResponse({ requestId: id2, results: results2, searchTimeMs: 1 });

      const [r1, r2] = await Promise.all([p1, p2]);

      expect(r1[0].key).toBe('a');
      expect(r2[0].key).toBe('b');
    });

    it('should ignore responses for unknown request ids (no throw, logs warn)', () => {
      const { client } = createClient();

      // Should not throw
      expect(() =>
        client.handleResponse({
          requestId: 'unknown-id-xyz',
          results: [],
          searchTimeMs: 0,
        })
      ).not.toThrow();
    });
  });
});
