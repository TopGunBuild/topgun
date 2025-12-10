import { 
  MessageSchema, 
  AuthMessageSchema, 
  QuerySubMessageSchema,
  ClientOpMessageSchema 
} from '../schemas';

describe('Message Schemas', () => {
  
  test('validates AUTH message', () => {
    const validAuth = {
      type: 'AUTH',
      token: 'some-jwt-token'
    };
    expect(AuthMessageSchema.safeParse(validAuth).success).toBe(true);

    const invalidAuth = {
      type: 'AUTH',
      // missing token
    };
    expect(AuthMessageSchema.safeParse(invalidAuth).success).toBe(false);
  });

  test('validates QUERY_SUB message', () => {
    const validQuery = {
      type: 'QUERY_SUB',
      payload: {
        queryId: 'q1',
        mapName: 'users',
        query: {
          where: { age: { $gt: 18 } },
          limit: 10
        }
      }
    };
    expect(QuerySubMessageSchema.safeParse(validQuery).success).toBe(true);

    const invalidQuery = {
      type: 'QUERY_SUB',
      payload: {
        // missing queryId
        mapName: 'users',
        query: {}
      }
    };
    expect(QuerySubMessageSchema.safeParse(invalidQuery).success).toBe(false);
  });

  test('validates CLIENT_OP message (LWW)', () => {
    const validOp = {
      type: 'CLIENT_OP',
      payload: {
        id: 'op1',
        mapName: 'users',
        key: 'user1',
        opType: 'PUT',
        record: {
          value: { name: 'Alice' },
          timestamp: { millis: 100, counter: 0, nodeId: 'client1' }
        }
      }
    };
    expect(ClientOpMessageSchema.safeParse(validOp).success).toBe(true);
  });

  test('validates full MessageSchema union', () => {
    const msg = {
      type: 'SYNC_INIT',
      mapName: 'todos',
      lastSyncTimestamp: 123456
    };
    const result = MessageSchema.safeParse(msg);
    expect(result.success).toBe(true);
    if (result.success) {
        expect(result.data.type).toBe('SYNC_INIT');
    }
  });

  test('rejects unknown message types', () => {
    const unknownMsg = {
      type: 'UNKNOWN_TYPE',
      payload: {}
    };
    const result = MessageSchema.safeParse(unknownMsg);
    expect(result.success).toBe(false);
  });
});

