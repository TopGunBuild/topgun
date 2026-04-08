/**
 * Client SQL Tests
 *
 * Tests for client-side sql() method.
 */

import { SyncEngine } from '../SyncEngine';
import { TopGunClient } from '../TopGunClient';
import { SyncState } from '../SyncState';
import { SingleServerProvider } from '../connection/SingleServerProvider';
import type { SqlQueryResult } from '../sync/types';

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

describe('Client SQL', () => {
  describe('SyncEngine.sql()', () => {
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
      await expect(syncEngine.sql('SELECT 1')).rejects.toThrow('Not connected to server');
    });

    it('should send SQL_QUERY message with correct payload', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      const sqlPromise = syncEngine.sql('SELECT * FROM users WHERE age > 21');

      expect(mockSendMessage).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'SQL_QUERY',
          payload: expect.objectContaining({
            sql: 'SELECT * FROM users WHERE age > 21',
            queryId: expect.any(String),
          }),
        })
      );

      // Simulate response
      const sentMessage = mockSendMessage.mock.calls[0][0];
      (syncEngine as any).handleServerMessage({
        type: 'SQL_QUERY_RESP',
        payload: {
          queryId: sentMessage.payload.queryId,
          columns: ['name', 'age'],
          rows: [['Alice', 30], ['Bob', 25]],
        },
      });

      const result = await sqlPromise;

      expect(result.columns).toEqual(['name', 'age']);
      expect(result.rows).toEqual([['Alice', 30], ['Bob', 25]]);
    });

    it('should reject on server error', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      const sqlPromise = syncEngine.sql('SELECT * FROM nonexistent');

      const sentMessage = mockSendMessage.mock.calls[0][0];
      (syncEngine as any).handleServerMessage({
        type: 'SQL_QUERY_RESP',
        payload: {
          queryId: sentMessage.payload.queryId,
          columns: [],
          rows: [],
          error: 'Table not found: nonexistent',
        },
      });

      await expect(sqlPromise).rejects.toThrow('Table not found: nonexistent');
    });

    it('should timeout if no response', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      const sqlPromise = syncEngine.sql('SELECT * FROM users');

      // Fast-forward past timeout (30 seconds)
      jest.advanceTimersByTime(35000);

      await expect(sqlPromise).rejects.toThrow('SQL query request timed out');
    });

    it('should handle empty results', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      const sqlPromise = syncEngine.sql('SELECT * FROM users WHERE 1=0');

      const sentMessage = mockSendMessage.mock.calls[0][0];
      (syncEngine as any).handleServerMessage({
        type: 'SQL_QUERY_RESP',
        payload: {
          queryId: sentMessage.payload.queryId,
          columns: ['name', 'age'],
          rows: [],
        },
      });

      const result = await sqlPromise;
      expect(result.columns).toEqual(['name', 'age']);
      expect(result.rows).toEqual([]);
    });

    it('should reject if sendMessage fails', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;
      mockSendMessage.mockReturnValue(false);

      await expect(syncEngine.sql('SELECT 1')).rejects.toThrow('Failed to send SQL query');
    });

    it('should generate unique queryIds for concurrent requests', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      const promise1 = syncEngine.sql('SELECT 1');
      const promise2 = syncEngine.sql('SELECT 2');

      const queryId1 = mockSendMessage.mock.calls[0][0].payload.queryId;
      const queryId2 = mockSendMessage.mock.calls[1][0].payload.queryId;

      expect(queryId1).not.toBe(queryId2);

      // Resolve both
      (syncEngine as any).handleServerMessage({
        type: 'SQL_QUERY_RESP',
        payload: { queryId: queryId1, columns: ['a'], rows: [[1]] },
      });
      (syncEngine as any).handleServerMessage({
        type: 'SQL_QUERY_RESP',
        payload: { queryId: queryId2, columns: ['a'], rows: [[2]] },
      });

      const result1 = await promise1;
      const result2 = await promise2;

      expect(result1.rows).toEqual([[1]]);
      expect(result2.rows).toEqual([[2]]);
    });

    it('should ignore responses for unknown queryIds', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      // Send a response with no matching pending request — should not throw
      (syncEngine as any).handleServerMessage({
        type: 'SQL_QUERY_RESP',
        payload: {
          queryId: 'unknown-query-id',
          columns: [],
          rows: [],
        },
      });
    });

    it('should clean up pending requests on close without unhandled rejections', async () => {
      (syncEngine as any).stateMachine.state = SyncState.CONNECTED;

      // Start a query but don't resolve it
      syncEngine.sql('SELECT * FROM users');

      // Close should not cause unhandled rejection
      syncEngine.close();
    });
  });

  describe('TopGunClient.sql()', () => {
    it('should delegate to SyncEngine.sql()', async () => {
      const mockStorage = createMockStorage();
      const client = new TopGunClient({
        serverUrl: 'ws://localhost:8080',
        storage: mockStorage as any,
      });

      const mockResult: SqlQueryResult = {
        columns: ['id', 'name'],
        rows: [[1, 'Alice']],
      };

      const mockSql = jest.fn().mockResolvedValue(mockResult);
      (client as any).syncEngine.sql = mockSql;

      const result = await client.sql('SELECT id, name FROM users');

      expect(mockSql).toHaveBeenCalledWith('SELECT id, name FROM users');
      expect(result).toEqual(mockResult);

      client.close();
    });
  });

  describe('SqlQueryResult type', () => {
    it('should have correct structure', () => {
      const result: SqlQueryResult = {
        columns: ['name', 'age'],
        rows: [['Alice', 30], ['Bob', 25]],
      };

      expect(result.columns).toEqual(['name', 'age']);
      expect(result.rows).toHaveLength(2);
      expect(result.rows[0]).toEqual(['Alice', 30]);
    });
  });
});
