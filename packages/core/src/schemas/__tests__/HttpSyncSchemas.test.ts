import { HttpSyncRequestSchema, HttpSyncResponseSchema } from '../http-sync-schemas';

describe('HttpSyncSchemas', () => {
  const validTimestamp = { millis: 1700000000000, counter: 1, nodeId: 'node-1' };

  describe('HttpSyncRequestSchema', () => {
    it('validates a full request with all fields', () => {
      const input = {
        clientId: 'client-1',
        clientHlc: validTimestamp,
        operations: [
          { mapName: 'users', key: 'user-1', record: { value: { name: 'Alice' }, timestamp: validTimestamp } },
        ],
        syncMaps: [
          { mapName: 'users', lastSyncTimestamp: validTimestamp },
        ],
        queries: [
          { queryId: 'q1', mapName: 'users', filter: { name: 'Alice' }, limit: 10 },
        ],
        searches: [
          { searchId: 's1', mapName: 'users', query: 'alice', options: { limit: 5 } },
        ],
      };

      const result = HttpSyncRequestSchema.safeParse(input);
      expect(result.success).toBe(true);
    });

    it('validates a request with only operations', () => {
      const input = {
        clientId: 'client-1',
        clientHlc: validTimestamp,
        operations: [
          { mapName: 'users', key: 'user-1', record: { value: { name: 'Bob' }, timestamp: validTimestamp } },
        ],
      };

      const result = HttpSyncRequestSchema.safeParse(input);
      expect(result.success).toBe(true);
    });

    it('validates a request with only syncMaps', () => {
      const input = {
        clientId: 'client-1',
        clientHlc: validTimestamp,
        syncMaps: [
          { mapName: 'users', lastSyncTimestamp: validTimestamp },
          { mapName: 'posts', lastSyncTimestamp: { millis: 0, counter: 0, nodeId: '' } },
        ],
      };

      const result = HttpSyncRequestSchema.safeParse(input);
      expect(result.success).toBe(true);
    });

    it('validates an empty request (heartbeat/probe)', () => {
      const input = {
        clientId: 'client-1',
        clientHlc: validTimestamp,
      };

      const result = HttpSyncRequestSchema.safeParse(input);
      expect(result.success).toBe(true);
    });

    it('rejects request missing clientId', () => {
      const input = {
        clientHlc: validTimestamp,
      };

      const result = HttpSyncRequestSchema.safeParse(input);
      expect(result.success).toBe(false);
    });

    it('rejects request missing clientHlc', () => {
      const input = {
        clientId: 'client-1',
      };

      const result = HttpSyncRequestSchema.safeParse(input);
      expect(result.success).toBe(false);
    });

    it('validates lastSyncTimestamp as TimestampSchema (object with millis, counter, nodeId)', () => {
      const input = {
        clientId: 'client-1',
        clientHlc: validTimestamp,
        syncMaps: [
          { mapName: 'users', lastSyncTimestamp: { millis: 1700000000000, counter: 42, nodeId: 'node-abc' } },
        ],
      };

      const result = HttpSyncRequestSchema.safeParse(input);
      expect(result.success).toBe(true);
      if (result.success) {
        expect(result.data.syncMaps![0].lastSyncTimestamp).toEqual({
          millis: 1700000000000,
          counter: 42,
          nodeId: 'node-abc',
        });
      }
    });

    it('rejects lastSyncTimestamp as plain number', () => {
      const input = {
        clientId: 'client-1',
        clientHlc: validTimestamp,
        syncMaps: [
          { mapName: 'users', lastSyncTimestamp: 1700000000000 },
        ],
      };

      const result = HttpSyncRequestSchema.safeParse(input);
      expect(result.success).toBe(false);
    });
  });

  describe('HttpSyncResponseSchema', () => {
    it('validates a response with deltas', () => {
      const input = {
        serverHlc: validTimestamp,
        deltas: [
          {
            mapName: 'users',
            records: [
              {
                key: 'user-1',
                record: { value: { name: 'Alice' }, timestamp: validTimestamp },
                eventType: 'PUT',
              },
            ],
            serverSyncTimestamp: validTimestamp,
          },
        ],
      };

      const result = HttpSyncResponseSchema.safeParse(input);
      expect(result.success).toBe(true);
    });

    it('validates a response with errors', () => {
      const input = {
        serverHlc: validTimestamp,
        errors: [
          { code: 403, message: 'Access denied', context: 'Operation on users/key-1' },
          { code: 500, message: 'Internal error' },
        ],
      };

      const result = HttpSyncResponseSchema.safeParse(input);
      expect(result.success).toBe(true);
    });

    it('validates a response with ack and query results', () => {
      const input = {
        serverHlc: validTimestamp,
        ack: {
          lastId: 'op-5',
          results: [
            { opId: 'op-5', success: true, achievedLevel: 'MEMORY' },
          ],
        },
        queryResults: [
          {
            queryId: 'q1',
            results: [{ id: 'user-1', name: 'Alice' }],
            hasMore: false,
          },
        ],
      };

      const result = HttpSyncResponseSchema.safeParse(input);
      expect(result.success).toBe(true);
    });

    it('validates a minimal response with only serverHlc', () => {
      const input = {
        serverHlc: validTimestamp,
      };

      const result = HttpSyncResponseSchema.safeParse(input);
      expect(result.success).toBe(true);
    });
  });
});
