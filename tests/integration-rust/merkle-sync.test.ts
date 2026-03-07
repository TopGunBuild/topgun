import {
  createRustTestContext,
  createRustTestClient,
  createLWWRecord,
  waitForSync,
  RustTestContext,
  TestClient,
} from './helpers';

describe('Integration: Merkle Sync (Rust Server)', () => {
  describe('Late-joiner receives non-zero root hash', () => {
    let ctx: RustTestContext;

    beforeAll(async () => {
      // Spawn server with one client (Device A)
      ctx = await createRustTestContext(1);
    });

    afterAll(async () => {
      await ctx.cleanup();
    });

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
});
