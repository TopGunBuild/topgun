import {
  MessageSchema,
  AuthMessageSchema,
  QuerySubMessageSchema,
  ClientOpMessageSchema,
  OpRejectedMessageSchema,
} from '../schemas';

describe('Message Schemas', () => {
  test('validates AUTH message', () => {
    const validAuth = {
      type: 'AUTH',
      token: 'some-jwt-token',
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
          limit: 10,
        },
      },
    };
    expect(QuerySubMessageSchema.safeParse(validQuery).success).toBe(true);

    const invalidQuery = {
      type: 'QUERY_SUB',
      payload: {
        // missing queryId
        mapName: 'users',
        query: {},
      },
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
          timestamp: { millis: 100, counter: 0, nodeId: 'client1' },
        },
      },
    };
    expect(ClientOpMessageSchema.safeParse(validOp).success).toBe(true);
  });

  test('validates full MessageSchema union', () => {
    const msg = {
      type: 'SYNC_INIT',
      mapName: 'todos',
      lastSyncTimestamp: 123456,
    };
    const result = MessageSchema.safeParse(msg);
    expect(result.success).toBe(true);
    if (result.success) {
      expect(result.data.type).toBe('SYNC_INIT');
    }
  });

  test('OP_REJECTED requires permanent', () => {
    const validRejection = {
      type: 'OP_REJECTED',
      payload: {
        opId: 'op1',
        reason: 'write access denied for map: users',
        code: 403,
        permanent: true,
      },
    };
    expect(OpRejectedMessageSchema.safeParse(validRejection).success).toBe(true);

    // Omitting `permanent` must throw rather than default: a client that cannot
    // tell a terminal refusal from a transient one retries a doomed write forever.
    const payloadWithoutPermanent: Record<string, unknown> = { ...validRejection.payload };
    delete payloadWithoutPermanent.permanent;
    expect(() =>
      OpRejectedMessageSchema.parse({
        type: 'OP_REJECTED',
        payload: payloadWithoutPermanent,
      }),
    ).toThrow();

    // `code` stays optional -- only `permanent` is required on both sides.
    expect(
      OpRejectedMessageSchema.safeParse({
        type: 'OP_REJECTED',
        payload: { opId: 'op2', reason: 'server overloaded', permanent: false },
      }).success,
    ).toBe(true);
  });

  test('rejects unknown message types', () => {
    const unknownMsg = {
      type: 'UNKNOWN_TYPE',
      payload: {},
    };
    const result = MessageSchema.safeParse(unknownMsg);
    expect(result.success).toBe(false);
  });
});
