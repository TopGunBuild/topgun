/**
 * VectorSearchClient Tests
 *
 * Tests for client-side vectorSearch() method.
 * Follows SqlClient.test.ts patterns.
 */

import { SyncEngine } from '../SyncEngine';
import { TopGunClient } from '../TopGunClient';
import { SyncState } from '../SyncState';
import { SingleServerProvider } from '../connection/SingleServerProvider';
import type { VectorSearchClientResult } from '../sync/types';

// Mock storage adapter
const createMockStorage = () => ({
  initialize: jest.fn().mockResolvedValue(undefined),
  get: jest.fn().mockResolvedValue(null),
  put: jest.fn().mockResolvedValue(undefined),
  remove: jest.fn().mockResolvedValue(undefined),
  getAllKeys: jest.fn().mockResolvedValue([]),
  getMeta: jest.fn().mockResolvedValue(null),
  setMeta: jest.fn().mockResolvedValue(undefined),
  clear: jest.fn().mockResolvedValue(undefined),
  getPendingOps: jest.fn().mockResolvedValue([]),
  savePendingOps: jest.fn().mockResolvedValue(undefined),
  clearPendingOps: jest.fn().mockResolvedValue(undefined),
});

// Mock WebSocket
(globalThis as any).WebSocket = class MockWebSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;
  readyState = 1;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onerror: ((error: Error) => void) | null = null;
  send = jest.fn();
  close = jest.fn();
};

describe('VectorSearchClient', () => {
  describe('SyncEngine.vectorSearch()', () => {
    let syncEngine: SyncEngine;
    let mockSendMessage: jest.SpyInstance;

    beforeEach(() => {
      jest.useFakeTimers();

      syncEngine = new SyncEngine({
        nodeId: 'test-node',
        connectionProvider: new SingleServerProvider({ url: 'ws://localhost:8080' }),
        storageAdapter: createMockStorage() as any,
      });

      mockSendMessage = jest.spyOn(syncEngine as any, 'sendMessage').mockReturnValue(true);
    });

    afterEach(() => {
      jest.useRealTimers();
      syncEngine.close();
    });

    it('should throw error when not authenticated', async () => {
      await expect(
        syncEngine.vectorSearch('notes', new Float32Array([0.1, 0.2, 0.3]))
      ).rejects.toThrow('Not connected to server');
    });

    it('should send VECTOR_SEARCH message with correct payload', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      const queryVector = new Float32Array([0.1, 0.2, 0.3]);
      const searchPromise = syncEngine.vectorSearch('notes', queryVector, { k: 5 });

      expect(mockSendMessage).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'VECTOR_SEARCH',
          payload: expect.objectContaining({
            id: expect.any(String),
            mapName: 'notes',
            queryVector: expect.any(Uint8Array),
            k: 5,
          }),
        })
      );

      // Verify queryVector is wire-format Uint8Array (3 floats * 4 bytes = 12 bytes)
      const sentMessage = mockSendMessage.mock.calls[0][0];
      expect(sentMessage.payload.queryVector).toBeInstanceOf(Uint8Array);
      expect(sentMessage.payload.queryVector.byteLength).toBe(12);

      // Simulate response
      (syncEngine as any).handleServerMessage({
        type: 'VECTOR_SEARCH_RESP',
        payload: {
          id: sentMessage.payload.id,
          results: [
            { key: 'doc-1', score: 0.95 },
            { key: 'doc-2', score: 0.87 },
          ],
          totalCandidates: 100,
          searchTimeMs: 5,
        },
      });

      const results = await searchPromise;

      expect(results).toHaveLength(2);
      expect(results[0]).toEqual({ key: 'doc-1', score: 0.95 });
      expect(results[1]).toEqual({ key: 'doc-2', score: 0.87 });
    });

    it('should accept number[] as query vector', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      const searchPromise = syncEngine.vectorSearch('notes', [1.0, 2.0], { k: 3 });

      const sentMessage = mockSendMessage.mock.calls[0][0];
      // number[] should be converted to Uint8Array (2 floats * 4 bytes = 8 bytes)
      expect(sentMessage.payload.queryVector).toBeInstanceOf(Uint8Array);
      expect(sentMessage.payload.queryVector.byteLength).toBe(8);

      // Simulate response
      (syncEngine as any).handleServerMessage({
        type: 'VECTOR_SEARCH_RESP',
        payload: {
          id: sentMessage.payload.id,
          results: [{ key: 'doc-1', score: 0.99 }],
          totalCandidates: 50,
          searchTimeMs: 2,
        },
      });

      const results = await searchPromise;
      expect(results).toHaveLength(1);
      expect(results[0].key).toBe('doc-1');
    });

    it('should default k to 10 when not specified', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      syncEngine.vectorSearch('notes', new Float32Array([0.1]));

      const sentMessage = mockSendMessage.mock.calls[0][0];
      expect(sentMessage.payload.k).toBe(10);

      // Clean up
      (syncEngine as any).handleServerMessage({
        type: 'VECTOR_SEARCH_RESP',
        payload: {
          id: sentMessage.payload.id,
          results: [],
          totalCandidates: 0,
          searchTimeMs: 0,
        },
      });
    });

    it('should convert response vector from Uint8Array to Float32Array', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      const searchPromise = syncEngine.vectorSearch('notes', [1.0, 2.0], {
        k: 1,
        includeVectors: true,
      });

      const sentMessage = mockSendMessage.mock.calls[0][0];

      // Create a wire-format Uint8Array for vector [3.0, 4.0]
      const wireVector = new Uint8Array(new Float32Array([3.0, 4.0]).buffer);

      (syncEngine as any).handleServerMessage({
        type: 'VECTOR_SEARCH_RESP',
        payload: {
          id: sentMessage.payload.id,
          results: [{ key: 'doc-1', score: 0.9, vector: wireVector }],
          totalCandidates: 10,
          searchTimeMs: 1,
        },
      });

      const results = await searchPromise;
      expect(results[0].vector).toBeInstanceOf(Float32Array);
      expect(Array.from(results[0].vector!)).toEqual([3.0, 4.0]);
    });

    it('should reject on server error', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      const searchPromise = syncEngine.vectorSearch('notes', [0.1]);

      const sentMessage = mockSendMessage.mock.calls[0][0];
      (syncEngine as any).handleServerMessage({
        type: 'VECTOR_SEARCH_RESP',
        payload: {
          id: sentMessage.payload.id,
          results: [],
          totalCandidates: 0,
          searchTimeMs: 0,
          error: 'Index not found: notes',
        },
      });

      await expect(searchPromise).rejects.toThrow('Index not found: notes');
    });

    it('should timeout if no response', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      const searchPromise = syncEngine.vectorSearch('notes', [0.1]);

      // Fast-forward past timeout (30 seconds)
      jest.advanceTimersByTime(35000);

      await expect(searchPromise).rejects.toThrow('Vector search request timed out');
    });

    it('should reject if sendMessage fails', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;
      mockSendMessage.mockReturnValue(false);

      await expect(
        syncEngine.vectorSearch('notes', [0.1])
      ).rejects.toThrow('Failed to send vector search request');
    });

    it('should generate unique request ids for concurrent requests', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      const promise1 = syncEngine.vectorSearch('notes', [0.1]);
      const promise2 = syncEngine.vectorSearch('notes', [0.2]);

      const id1 = mockSendMessage.mock.calls[0][0].payload.id;
      const id2 = mockSendMessage.mock.calls[1][0].payload.id;

      expect(id1).not.toBe(id2);

      // Resolve both
      (syncEngine as any).handleServerMessage({
        type: 'VECTOR_SEARCH_RESP',
        payload: { id: id1, results: [{ key: 'a', score: 0.9 }], totalCandidates: 1, searchTimeMs: 1 },
      });
      (syncEngine as any).handleServerMessage({
        type: 'VECTOR_SEARCH_RESP',
        payload: { id: id2, results: [{ key: 'b', score: 0.8 }], totalCandidates: 1, searchTimeMs: 1 },
      });

      const result1 = await promise1;
      const result2 = await promise2;

      expect(result1[0].key).toBe('a');
      expect(result2[0].key).toBe('b');
    });

    it('should ignore responses for unknown request ids', () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      // Should not throw
      (syncEngine as any).handleServerMessage({
        type: 'VECTOR_SEARCH_RESP',
        payload: {
          id: 'unknown-id',
          results: [],
          totalCandidates: 0,
          searchTimeMs: 0,
        },
      });
    });

    it('should handle empty results', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      const searchPromise = syncEngine.vectorSearch('notes', [0.1], { k: 5 });

      const sentMessage = mockSendMessage.mock.calls[0][0];
      (syncEngine as any).handleServerMessage({
        type: 'VECTOR_SEARCH_RESP',
        payload: {
          id: sentMessage.payload.id,
          results: [],
          totalCandidates: 0,
          searchTimeMs: 1,
        },
      });

      const results = await searchPromise;
      expect(results).toEqual([]);
    });

    it('should pass optional search options in payload', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      syncEngine.vectorSearch('notes', [0.1], {
        k: 5,
        indexName: 'embedding_idx',
        efSearch: 200,
        includeValue: true,
        includeVectors: true,
        minScore: 0.5,
      });

      const sentMessage = mockSendMessage.mock.calls[0][0];
      expect(sentMessage.payload.indexName).toBe('embedding_idx');
      expect(sentMessage.payload.efSearch).toBe(200);
      expect(sentMessage.payload.options).toEqual({
        includeValue: true,
        includeVectors: true,
        minScore: 0.5,
      });

      // Clean up
      (syncEngine as any).handleServerMessage({
        type: 'VECTOR_SEARCH_RESP',
        payload: {
          id: sentMessage.payload.id,
          results: [],
          totalCandidates: 0,
          searchTimeMs: 0,
        },
      });
    });

    it('should clean up pending requests on close', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      // Start a search but don't resolve it
      const searchPromise = syncEngine.vectorSearch('notes', [0.1]);

      // Close rejects pending promises with a close error
      syncEngine.close();

      await expect(searchPromise).rejects.toThrow('SyncEngine closed');
    });
  });

  describe('TopGunClient.vectorSearch()', () => {
    it('should delegate to SyncEngine.vectorSearch()', async () => {
      const mockStorage = createMockStorage();
      const client = new TopGunClient({
        serverUrl: 'ws://localhost:8080',
        storage: mockStorage as any,
      });

      const mockResults: VectorSearchClientResult[] = [
        { key: 'doc-1', score: 0.95 },
        { key: 'doc-2', score: 0.87, value: { title: 'Test' } },
      ];

      const mockVectorSearch = jest.fn().mockResolvedValue(mockResults);
      (client as any).syncEngine.vectorSearch = mockVectorSearch;

      const queryVector = new Float32Array([0.1, 0.2, 0.3]);
      const result = await client.vectorSearch('notes', queryVector, { k: 5 });

      expect(mockVectorSearch).toHaveBeenCalledWith('notes', queryVector, { k: 5 });
      expect(result).toEqual(mockResults);

      client.close();
    });
  });
});
