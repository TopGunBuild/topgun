import {
  createRustTestContext,
  createRustTestClient,
  createLWWRecord,
  waitForSync,
  completeMerkleSync,
  RustTestContext,
  TestClient,
} from './helpers';

describe('Integration: Merkle Sync (Rust Server)', () => {
  let ctx: RustTestContext;

  beforeAll(async () => {
    ctx = await createRustTestContext(1);
  });

  afterAll(async () => {
    await ctx.cleanup();
  });

  describe('Late-joiner receives non-zero root hash', () => {
    test('Device B gets non-zero root_hash after Device A writes data', async () => {
      const [deviceA] = ctx.clients;

      // Device A writes data to the "users" map with an arbitrary key.
      // Any key should work because the Merkle dual-write ensures partition 0
      // (client-sync) always has the complete tree, and record lookups use
      // per-key hash_to_partition to find the actual storage partition.
      const record = createLWWRecord({ name: 'Alice', age: 30 });
      deviceA.messages.length = 0;

      deviceA.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'op-merkle-1',
          mapName: 'users',
          opType: 'PUT',
          key: 'alice',
          record,
        },
      });

      // Wait for the write to be processed
      await deviceA.waitForMessage('OP_ACK');
      await waitForSync(200);

      // Device B connects as a late joiner
      const deviceB: TestClient = await createRustTestClient(ctx.port, {
        nodeId: 'device-b',
        userId: 'user-b',
        roles: ['ADMIN'],
      });

      try {
        // Wait for auth handshake to complete
        await deviceB.waitForMessage('AUTH_ACK', 10_000);
        deviceB.messages.length = 0;

        // Device B sends SYNC_INIT for the "users" map
        deviceB.send({
          type: 'SYNC_INIT',
          mapName: 'users',
        });

        // Device B should receive SYNC_RESP_ROOT with a non-zero rootHash
        const syncResp = await deviceB.waitForMessage('SYNC_RESP_ROOT', 5000);
        expect(syncResp).toBeDefined();
        expect(syncResp.type).toBe('SYNC_RESP_ROOT');
        expect(syncResp.payload).toBeDefined();
        expect(syncResp.payload.mapName).toBe('users');
        expect(syncResp.payload.rootHash).not.toBe(0);
      } finally {
        deviceB.close();
      }
    });
  });

  describe('Full Merkle sync protocol', () => {
    test('delivers single record to late-joiner', async () => {
      const [deviceA] = ctx.clients;

      const record = createLWWRecord({ name: 'Alice', age: 30 }, deviceA.nodeId);
      deviceA.messages.length = 0;

      deviceA.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'op-single-1',
          mapName: 'sync-single',
          opType: 'PUT',
          key: 'user:alice',
          record,
        },
      });

      await deviceA.waitForMessage('OP_ACK');
      await waitForSync(200);

      const deviceB = await createRustTestClient(ctx.port, {
        nodeId: 'device-b-single',
        userId: 'user-b-single',
        roles: ['ADMIN'],
      });

      try {
        await deviceB.waitForMessage('AUTH_ACK', 10_000);
        deviceB.messages.length = 0;

        const records = await completeMerkleSync(deviceB, 'sync-single');

        expect(records.size).toBe(1);
        expect(records.has('user:alice')).toBe(true);
        const entry = records.get('user:alice')!;
        expect(entry.value).toEqual({ name: 'Alice', age: 30 });
      } finally {
        deviceB.close();
      }
    });

    test('multi-key sync convergence (12 keys)', async () => {
      const [deviceA] = ctx.clients;

      const keys: { key: string; value: any }[] = [];
      for (let i = 1; i <= 4; i++) {
        const num = String(i).padStart(3, '0');
        keys.push({ key: `todo:${num}:title`, value: `Task ${i}` });
        keys.push({ key: `todo:${num}:done`, value: false });
        keys.push({ key: `todo:${num}:priority`, value: i });
      }

      // Write each key and wait for ACK before proceeding
      for (const { key, value } of keys) {
        deviceA.messages.length = 0;
        const record = createLWWRecord(value, deviceA.nodeId);
        deviceA.send({
          type: 'CLIENT_OP',
          payload: {
            id: `op-todo-${key}`,
            mapName: 'todos',
            opType: 'PUT',
            key,
            record,
          },
        });
        await deviceA.waitForMessage('OP_ACK');
      }

      // Wait for Merkle tree to settle
      await waitForSync(500);

      const deviceB = await createRustTestClient(ctx.port, {
        nodeId: 'device-b-multi',
        userId: 'user-b-multi',
        roles: ['ADMIN'],
      });

      try {
        await deviceB.waitForMessage('AUTH_ACK', 10_000);
        deviceB.messages.length = 0;

        const records = await completeMerkleSync(deviceB, 'todos');

        expect(records.size).toBe(12);
        for (const { key, value } of keys) {
          expect(records.has(key)).toBe(true);
          expect(records.get(key)!.value).toEqual(value);
        }
      } finally {
        deviceB.close();
      }
    });

    test('empty map sync returns zero root hash and no records', async () => {
      const deviceB = await createRustTestClient(ctx.port, {
        nodeId: 'device-b-empty',
        userId: 'user-b-empty',
        roles: ['ADMIN'],
      });

      try {
        await deviceB.waitForMessage('AUTH_ACK', 10_000);
        deviceB.messages.length = 0;

        const records = await completeMerkleSync(deviceB, 'empty-map');

        expect(records.size).toBe(0);
      } finally {
        deviceB.close();
      }
    });

    test('sync with diverse key patterns', async () => {
      const [deviceA] = ctx.clients;

      const diverseKeys = [
        { key: 'a', value: 'short' },
        { key: 'zzz', value: 'triple-z' },
        { key: 'key-with-dashes', value: 'dashed' },
        { key: 'nested:path:deep', value: 'nested' },
        { key: '12345', value: 'numeric' },
      ];

      for (const { key, value } of diverseKeys) {
        deviceA.messages.length = 0;
        const record = createLWWRecord(value, deviceA.nodeId);
        deviceA.send({
          type: 'CLIENT_OP',
          payload: {
            id: `op-diverse-${key}`,
            mapName: 'diverse',
            opType: 'PUT',
            key,
            record,
          },
        });
        await deviceA.waitForMessage('OP_ACK');
      }

      await waitForSync(200);

      const deviceB = await createRustTestClient(ctx.port, {
        nodeId: 'device-b-diverse',
        userId: 'user-b-diverse',
        roles: ['ADMIN'],
      });

      try {
        await deviceB.waitForMessage('AUTH_ACK', 10_000);
        deviceB.messages.length = 0;

        const records = await completeMerkleSync(deviceB, 'diverse');

        expect(records.size).toBe(5);
        for (const { key, value } of diverseKeys) {
          expect(records.has(key)).toBe(true);
          expect(records.get(key)!.value).toEqual(value);
        }
      } finally {
        deviceB.close();
      }
    });
  });
});
