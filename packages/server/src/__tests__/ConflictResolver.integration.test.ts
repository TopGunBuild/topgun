/**
 * Conflict Resolver Integration Tests
 *
 * Tests for Phase 5.05: Custom Conflict Resolvers
 * - Registering resolvers from client
 * - Merge rejection flow (client -> server -> client notification)
 * - Built-in resolvers
 * - Key pattern matching
 */

import { ServerCoordinator, ServerCoordinatorConfig } from '../ServerCoordinator';
import { TopGunClient } from '@topgunbuild/client';
import { MemoryStorageAdapter } from './utils/MemoryStorageAdapter';
import { MergeRejection } from '@topgunbuild/core';

const TEST_PORT_BASE = 12200;

describe('Conflict Resolver Integration', () => {
  let server: ServerCoordinator;
  let client1: TopGunClient;
  let client2: TopGunClient;
  let port: number;

  beforeEach(async () => {
    port = TEST_PORT_BASE + Math.floor(Math.random() * 1000);

    const serverConfig: ServerCoordinatorConfig = {
      port,
      nodeId: 'test-server',
      jwtSecret: 'test-secret',
    };

    server = new ServerCoordinator(serverConfig);
    await server.ready();

    // Create two clients to simulate concurrent updates
    client1 = new TopGunClient({
      serverUrl: `ws://localhost:${port}`,
      storage: new MemoryStorageAdapter(),
      nodeId: 'client-1',
    });

    client2 = new TopGunClient({
      serverUrl: `ws://localhost:${port}`,
      storage: new MemoryStorageAdapter(),
      nodeId: 'client-2',
    });

    await client1.start();
    await client2.start();

    const jwt = require('jsonwebtoken');
    client1.setAuthToken(jwt.sign({ sub: 'user-1', roles: ['ADMIN'] }, 'test-secret'));
    client2.setAuthToken(jwt.sign({ sub: 'user-2', roles: ['ADMIN'] }, 'test-secret'));

    // Wait for both connections
    await Promise.all([
      waitForConnection(client1),
      waitForConnection(client2),
    ]);
  }, 15000);

  afterEach(async () => {
    client1?.close();
    client2?.close();
    await server?.shutdown();
  });

  describe('resolver registration', () => {
    it('should register a resolver from client', async () => {
      const resolvers = client1.getConflictResolvers();
      const result = await resolvers.register('bookings', {
        name: 'first-write-wins',
        code: `
          if (context.localValue !== undefined) {
            return { action: 'reject', reason: 'Slot already booked' };
          }
          return { action: 'accept', value: context.remoteValue };
        `,
        priority: 100,
      });

      expect(result.success).toBe(true);
    });

    it('should list registered resolvers', async () => {
      const resolvers = client1.getConflictResolvers();

      await resolvers.register('test-map', {
        name: 'resolver-1',
        code: `return { action: 'accept', value: context.remoteValue };`,
        priority: 50,
      });

      await resolvers.register('test-map', {
        name: 'resolver-2',
        code: `return { action: 'accept', value: context.remoteValue };`,
        priority: 100,
      });

      const list = await resolvers.list('test-map');

      expect(list.length).toBe(2);
      expect(list.map((r: { name: string }) => r.name).sort()).toEqual(['resolver-1', 'resolver-2']);
    });

    it('should unregister a resolver', async () => {
      const resolvers = client1.getConflictResolvers();

      await resolvers.register('test-map', {
        name: 'temp-resolver',
        code: `return { action: 'accept', value: context.remoteValue };`,
      });

      let list = await resolvers.list('test-map');
      expect(list.length).toBe(1);

      const result = await resolvers.unregister('test-map', 'temp-resolver');
      expect(result.success).toBe(true);

      list = await resolvers.list('test-map');
      expect(list.length).toBe(0);
    });

    it('should reject invalid resolver code', async () => {
      const resolvers = client1.getConflictResolvers();
      const result = await resolvers.register('test-map', {
        name: 'evil-resolver',
        code: `eval('alert(1)'); return { action: 'accept', value: context.remoteValue };`,
      });

      expect(result.success).toBe(false);
      expect(result.error).toBeDefined();
    });
  });

  describe('merge rejection flow', () => {
    it('should notify client of merge rejection with first-write-wins resolver', async () => {
      // Register first-write-wins resolver via client API
      const resolvers1 = client1.getConflictResolvers();
      const result = await resolvers1.register('bookings', {
        name: 'first-write-wins',
        code: `
          if (context.localValue !== undefined) {
            return { action: 'reject', reason: 'Slot already booked' };
          }
          return { action: 'accept', value: context.remoteValue };
        `,
        priority: 100,
      });
      expect(result.success).toBe(true);

      const rejections: MergeRejection[] = [];
      const resolvers2 = client2.getConflictResolvers();
      const unsubscribe = resolvers2.onRejection((rejection: MergeRejection) => {
        rejections.push(rejection);
      });

      // Client1 books slot first
      const map1 = client1.getMap<string, { user: string; time: string }>('bookings');
      map1.set('slot-1', { user: 'user-1', time: '10:00' });

      // Wait for sync
      await new Promise(r => setTimeout(r, 300));

      // Client2 tries to book the same slot
      const map2 = client2.getMap<string, { user: string; time: string }>('bookings');
      map2.set('slot-1', { user: 'user-2', time: '10:00' });

      // Wait for rejection notification
      await new Promise(r => setTimeout(r, 500));

      unsubscribe();

      expect(rejections.length).toBe(1);
      expect(rejections[0].mapName).toBe('bookings');
      expect(rejections[0].key).toBe('slot-1');
      expect(rejections[0].reason).toContain('already booked');
    });

    it('should allow first write but reject second with immutable resolver', async () => {
      // Register immutable resolver via client API
      const resolvers1 = client1.getConflictResolvers();
      const result = await resolvers1.register('configs', {
        name: 'immutable',
        code: `
          if (context.localValue !== undefined) {
            return { action: 'reject', reason: 'Value is immutable' };
          }
          return { action: 'accept', value: context.remoteValue };
        `,
        priority: 100,
      });
      expect(result.success).toBe(true);

      const rejections: MergeRejection[] = [];
      resolvers1.onRejection((r: MergeRejection) => rejections.push(r));

      const map = client1.getMap<string, { value: number }>('configs');

      // First write should succeed
      map.set('app-config', { value: 1 });
      await new Promise(r => setTimeout(r, 300));

      // Second write should be rejected
      map.set('app-config', { value: 2 });
      await new Promise(r => setTimeout(r, 500));

      expect(rejections.length).toBe(1);
      expect(rejections[0].reason).toContain('immutable');
    });

    it('should reject negative values with non-negative resolver', async () => {
      // Register non-negative resolver via client API
      const resolvers1 = client1.getConflictResolvers();
      const result = await resolvers1.register('balances', {
        name: 'non-negative',
        code: `
          if (typeof context.remoteValue === 'number' && context.remoteValue < 0) {
            return { action: 'reject', reason: 'Value cannot be negative' };
          }
          return { action: 'accept', value: context.remoteValue };
        `,
        priority: 100,
      });
      expect(result.success).toBe(true);

      const rejections: MergeRejection[] = [];
      resolvers1.onRejection((r: MergeRejection) => rejections.push(r));

      const map = client1.getMap<string, number>('balances');

      // Valid positive value
      map.set('account-1', 100);
      await new Promise(r => setTimeout(r, 300));

      // Invalid negative value
      map.set('account-2', -50);
      await new Promise(r => setTimeout(r, 500));

      expect(rejections.length).toBe(1);
      expect(rejections[0].key).toBe('account-2');
      expect(rejections[0].reason).toContain('negative');
    });
  });

  describe('key pattern matching', () => {
    it('should apply resolver only to matching keys', async () => {
      // Register resolver with key pattern
      const resolvers1 = client1.getConflictResolvers();
      await resolvers1.register('data', {
        name: 'user-fww',
        code: `
          if (context.localValue !== undefined) {
            return { action: 'reject', reason: 'User already exists' };
          }
          return { action: 'accept', value: context.remoteValue };
        `,
        keyPattern: 'user:*',
        priority: 100,
      });

      const rejections: MergeRejection[] = [];
      const resolvers2 = client2.getConflictResolvers();
      resolvers2.onRejection((r: MergeRejection) => rejections.push(r));

      const map1 = client1.getMap<string, any>('data');
      const map2 = client2.getMap<string, any>('data');

      // Create user:123 from client1
      map1.set('user:123', { name: 'Alice' });
      await new Promise(r => setTimeout(r, 300));

      // Client2 tries to overwrite user:123 - should be rejected
      map2.set('user:123', { name: 'Bob' });
      await new Promise(r => setTimeout(r, 500));

      // Client2 writes to post:123 - should succeed (no pattern match)
      map2.set('post:123', { title: 'Hello' });
      await new Promise(r => setTimeout(r, 300));

      // Only user:123 should be rejected
      expect(rejections.length).toBe(1);
      expect(rejections[0].key).toBe('user:123');
    });
  });

  describe('resolver priority', () => {
    it('should execute resolvers in priority order', async () => {
      const resolvers1 = client1.getConflictResolvers();

      // Low priority resolver that accepts everything
      await resolvers1.register('priority-test', {
        name: 'low-accept',
        code: `return { action: 'local' };`,
        priority: 10,
      });

      // High priority resolver that rejects
      await resolvers1.register('priority-test', {
        name: 'high-reject',
        code: `
          if (context.remoteValue === 'blocked') {
            return { action: 'reject', reason: 'Value is blocked' };
          }
          return { action: 'local' };
        `,
        priority: 100,
      });

      const rejections: MergeRejection[] = [];
      resolvers1.onRejection((r: MergeRejection) => rejections.push(r));

      const map = client1.getMap<string, string>('priority-test');

      // Should be rejected by high priority resolver
      map.set('key1', 'blocked');
      await new Promise(r => setTimeout(r, 500));

      expect(rejections.length).toBe(1);
      expect(rejections[0].reason).toBe('Value is blocked');
    });
  });

  describe('deletion protection', () => {
    it('should reject deletion with immutable resolver', async () => {
      // Register immutable resolver that blocks both updates and deletions
      const resolvers1 = client1.getConflictResolvers();
      const result = await resolvers1.register('immutable-data', {
        name: 'immutable',
        code: `
          // Reject any modification (update or delete) if value exists
          if (context.localValue !== undefined) {
            return { action: 'reject', reason: 'Entry is immutable' };
          }
          return { action: 'accept', value: context.remoteValue };
        `,
        priority: 100,
      });
      expect(result.success).toBe(true);

      const rejections: MergeRejection[] = [];
      resolvers1.onRejection((r: MergeRejection) => rejections.push(r));

      const map = client1.getMap<string, { data: string }>('immutable-data');

      // First write should succeed
      map.set('config', { data: 'initial' });
      await new Promise(r => setTimeout(r, 300));

      expect(rejections.length).toBe(0);

      // Deletion should be rejected
      map.delete('config');
      await new Promise(r => setTimeout(r, 500));

      expect(rejections.length).toBe(1);
      expect(rejections[0].key).toBe('config');
      expect(rejections[0].reason).toContain('immutable');
      // attemptedValue should be null for deletions
      expect(rejections[0].attemptedValue).toBeNull();
    });

    it('should allow deletion when no protective resolver exists', async () => {
      // No resolver registered for this map
      const map1 = client1.getMap<string, { value: number }>('deletable-data');

      // Create entry
      map1.set('item1', { value: 100 });
      await new Promise(r => setTimeout(r, 300));

      // Delete should succeed (no resolver to block it)
      map1.delete('item1');
      await new Promise(r => setTimeout(r, 300));

      // Verify deletion propagated
      const value = map1.get('item1');
      expect(value).toBeUndefined();
    });

    it('should reject deletion from other client with owner-only pattern', async () => {
      // Register owner-only resolver that checks auth
      const resolvers1 = client1.getConflictResolvers();
      await resolvers1.register('owned-data', {
        name: 'owner-only',
        code: `
          // First write sets owner
          if (context.localValue === undefined) {
            return { action: 'accept', value: context.remoteValue };
          }
          // Check if current user is owner
          const ownerId = context.localValue.ownerId;
          if (ownerId && context.auth?.userId !== ownerId) {
            return { action: 'reject', reason: 'Only owner can modify or delete' };
          }
          return { action: 'accept', value: context.remoteValue };
        `,
        priority: 100,
      });

      const rejections: MergeRejection[] = [];
      const resolvers2 = client2.getConflictResolvers();
      resolvers2.onRejection((r: MergeRejection) => rejections.push(r));

      // Client1 creates owned entry
      const map1 = client1.getMap<string, { ownerId: string; data: string }>('owned-data');
      map1.set('doc1', { ownerId: 'user-1', data: 'secret' });
      await new Promise(r => setTimeout(r, 300));

      // Client2 tries to delete - should be rejected
      const map2 = client2.getMap<string, { ownerId: string; data: string }>('owned-data');
      map2.delete('doc1');
      await new Promise(r => setTimeout(r, 500));

      expect(rejections.length).toBe(1);
      expect(rejections[0].reason).toContain('owner');
    });
  });
});

// Helper functions

async function waitForConnection(client: TopGunClient): Promise<void> {
  return new Promise<void>((resolve) => {
    const checkConnection = () => {
      if (client.getConnectionState() === 'CONNECTED') {
        resolve();
      } else {
        setTimeout(checkConnection, 50);
      }
    };
    checkConnection();
  });
}

