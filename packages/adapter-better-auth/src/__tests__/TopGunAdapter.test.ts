import { topGunAdapter } from '../TopGunAdapter';
import { TopGunClient } from '@topgunbuild/client';
import { IStorageAdapter, OpLogEntry } from '@topgunbuild/client';
import { LWWRecord, ORMapRecord } from '@topgunbuild/core';
import type { BetterAuthOptions } from 'better-auth';

/**
 * Mock TopGunClient type for testing.
 * Contains only the methods required by the adapter.
 */
type MockTopGunClient = Pick<TopGunClient, 'start' | 'getMap' | 'query'>;

// Mock WebSocket
class MockWebSocket {
  onopen: () => void = () => {};
  onmessage: (event: MessageEvent) => void = () => {};
  onclose: () => void = () => {};
  onerror: (error: Event) => void = () => {};
  send() {}
  close() {}
  static OPEN = 1;
  readyState = 1;
}
(globalThis as unknown as { WebSocket: typeof MockWebSocket }).WebSocket = MockWebSocket;

class MemoryStorageAdapter implements IStorageAdapter {
  private data = new Map<string, LWWRecord<unknown> | ORMapRecord<unknown>[]>();
  private meta = new Map<string, unknown>();
  private opLog: OpLogEntry[] = [];
  private opIdCounter = 1;

  async initialize(dbName: string): Promise<void> {}
  async close(): Promise<void> {}

  async get<V>(key: string): Promise<LWWRecord<V> | ORMapRecord<V>[] | undefined> {
    return this.data.get(key) as LWWRecord<V> | ORMapRecord<V>[] | undefined;
  }

  async put(key: string, value: LWWRecord<unknown> | ORMapRecord<unknown>[]): Promise<void> {
    this.data.set(key, value);
  }

  async remove(key: string): Promise<void> {
    this.data.delete(key);
  }

  async getMeta(key: string): Promise<unknown> {
    return this.meta.get(key);
  }

  async setMeta(key: string, value: unknown): Promise<void> {
    this.meta.set(key, value);
  }

  async batchPut(entries: Map<string, LWWRecord<unknown> | ORMapRecord<unknown>[]>): Promise<void> {
    for (const [k, v] of entries) {
      this.data.set(k, v);
    }
  }

  async appendOpLog(entry: Omit<OpLogEntry, 'id'>): Promise<number> {
    const id = this.opIdCounter++;
    this.opLog.push({ ...entry, id });
    return id;
  }

  async getPendingOps(): Promise<OpLogEntry[]> {
    return this.opLog.filter(op => !op.synced);
  }

  async markOpsSynced(lastId: number): Promise<void> {
    this.opLog.forEach(op => {
      if (op.id !== undefined && op.id <= lastId) {
        op.synced = 1;
      }
    });
  }

  async getAllKeys(): Promise<string[]> {
    return Array.from(this.data.keys());
  }
}

describe('TopGunAdapter', () => {
  let client: TopGunClient;
  let adapter: ReturnType<ReturnType<typeof topGunAdapter>>;
  
  beforeEach(async () => {
    const storage = new MemoryStorageAdapter();
    client = new TopGunClient({
      serverUrl: 'ws://fake-url', // Mock doesn't use net
      storage,
      nodeId: 'test-node'
    });
    // We don't start sync engine network, just storage load
    await client.start();

    const factory = topGunAdapter({ client });
    // Better Auth passes options to the factory instance
    adapter = factory({} as BetterAuthOptions);
  });

  const testUser = {
    id: 'user-1',
    email: 'test@example.com',
    name: 'Test User',
    emailVerified: true,
    createdAt: new Date(),
    updatedAt: new Date()
  };

  it('should create a user', async () => {
    const created = await adapter.create({
      model: 'user',
      data: testUser
    });

    expect(created).toEqual(testUser);

    // Verify in client map
    const map = client.getMap('auth_user');
    const fromMap = map.get('user-1');
    expect(fromMap).toMatchObject(testUser);
  });

  it('should find a user by id', async () => {
    await adapter.create({ model: 'user', data: testUser });

    const found = await adapter.findOne({
      model: 'user',
      where: [{ field: 'id', value: 'user-1' }]
    });

    expect(found).toMatchObject({
        id: testUser.id,
        email: testUser.email
    });
  });

  it('should find a user by email', async () => {
    await adapter.create({ model: 'user', data: testUser });

    const found = await adapter.findOne({
      model: 'user',
      where: [{ field: 'email', value: 'test@example.com' }]
    }) as typeof testUser | null;

    expect(found).not.toBeNull();
    expect(found?.email).toBe('test@example.com');
  });

  it('should return null if user not found', async () => {
    const found = await adapter.findOne({
      model: 'user',
      where: [{ field: 'email', value: 'missing@example.com' }]
    });

    expect(found).toBeNull();
  });

  it('should update a user', async () => {
    await adapter.create({ model: 'user', data: testUser });

    const updated = await adapter.update({
      model: 'user',
      where: [{ field: 'id', value: 'user-1' }],
      update: { name: 'Updated Name' }
    });

    expect(updated).toMatchObject({ name: 'Updated Name' });

    const inMap = client.getMap('auth_user').get('user-1');
    expect(inMap).toMatchObject({ name: 'Updated Name' });
  });

  it('should delete a user', async () => {
    await adapter.create({ model: 'user', data: testUser });

    await adapter.delete({
      model: 'user',
      where: [{ field: 'id', value: 'user-1' }]
    });

    const found = await adapter.findOne({
      model: 'user',
      where: [{ field: 'id', value: 'user-1' }]
    });

    expect(found).toBeNull();
  });

  it('should handle complex where clauses (AND)', async () => {
    const user2 = { ...testUser, id: 'user-2', email: 'other@example.com' };
    await adapter.create({ model: 'user', data: testUser });
    await adapter.create({ model: 'user', data: user2 });

    const found = await adapter.findMany({
      model: 'user',
      where: [
        { field: 'email', value: 'test@example.com' },
        { field: 'name', value: 'Test User' }
      ]
    });

    expect(found).toHaveLength(1);
    expect(found[0]).toHaveProperty('id', 'user-1');
  });
  
  it('should handle IN operator', async () => {
      const user2 = { ...testUser, id: 'user-2', email: 'other@example.com' };
      await adapter.create({ model: 'user', data: testUser });
      await adapter.create({ model: 'user', data: user2 });
      
      const found = await adapter.findMany({
          model: 'user',
          where: [
              { field: 'email', operator: 'in', value: ['test@example.com', 'other@example.com'] }
          ]
      });
      
      expect(found).toHaveLength(2);
  });

  it('should handle join (find user with accounts)', async () => {
      await adapter.create({ model: 'user', data: testUser });
      const account = {
          id: 'acc-1',
          userId: testUser.id,
          providerId: 'credential',
          accountId: testUser.id,
          password: 'hashed-password'
      };
      await adapter.create({ model: 'account', data: account });

      const found = await adapter.findOne({
          model: 'user',
          where: [{ field: 'email', value: 'test@example.com' }],
          join: { account: true }
      });

      expect(found).not.toBeNull();
      // Expect accounts array attached
      if (found && typeof found === 'object' && 'accounts' in found) {
          const accounts = found.accounts as unknown[];
          expect(accounts).toBeDefined();
          expect(accounts).toHaveLength(1);
          expect(accounts[0]).toHaveProperty('providerId', 'credential');
      } else {
          throw new Error('Expected found to have accounts property');
      }
  });
});

describe('cold start handling', () => {
  it('waits for storage ready before first operation', async () => {
    // Create a mock client with delayed start
    let startResolved = false;
    const mockClient: MockTopGunClient = {
      start: jest.fn().mockImplementation(() => {
        return new Promise<void>(resolve => {
          setTimeout(() => {
            startResolved = true;
            resolve();
          }, 50);
        });
      }),
      getMap: jest.fn().mockReturnValue({
        set: jest.fn(),
        get: jest.fn(),
        remove: jest.fn(),
      }),
      query: jest.fn().mockReturnValue({
        subscribe: jest.fn((cb) => {
          // Delay callback to allow unsubscribe to be assigned first
          setTimeout(() => cb([]), 0);
          return () => {};
        }),
      }),
    };

    const adapter = topGunAdapter({ client: mockClient as TopGunClient })({} as BetterAuthOptions);

    // Start operation before storage is ready
    const createPromise = adapter.create({ model: 'user', data: { name: 'test' } });

    // Verify start was called
    expect(mockClient.start).toHaveBeenCalled();

    // Wait for create to complete
    await createPromise;

    // Verify storage was ready before create proceeded
    expect(startResolved).toBe(true);
  });

  it('concurrent requests share same ready promise', async () => {
    let startCallCount = 0;
    const mockClient: MockTopGunClient = {
      start: jest.fn().mockImplementation(() => {
        startCallCount++;
        return new Promise<void>(resolve => setTimeout(resolve, 50));
      }),
      getMap: jest.fn().mockReturnValue({
        set: jest.fn(),
        get: jest.fn(),
      }),
      query: jest.fn().mockReturnValue({
        subscribe: jest.fn((cb) => {
          // Delay callback to allow unsubscribe to be assigned first
          setTimeout(() => cb([]), 0);
          return () => {};
        }),
      }),
    };

    const adapter = topGunAdapter({ client: mockClient as TopGunClient })({} as BetterAuthOptions);

    // Fire multiple concurrent requests
    await Promise.all([
      adapter.create({ model: 'user', data: { name: 'test1' } }),
      adapter.create({ model: 'user', data: { name: 'test2' } }),
      adapter.findMany({ model: 'user' }),
    ]);

    // start() should only be called once
    expect(startCallCount).toBe(1);
  });

  it('subsequent requests do not wait if already ready', async () => {
    let startCallCount = 0;
    const mockClient: MockTopGunClient = {
      start: jest.fn().mockImplementation(() => {
        startCallCount++;
        return Promise.resolve();
      }),
      getMap: jest.fn().mockReturnValue({
        set: jest.fn(),
      }),
      query: jest.fn().mockReturnValue({
        subscribe: jest.fn((cb) => {
          // Delay callback to allow unsubscribe to be assigned first
          setTimeout(() => cb([]), 0);
          return () => {};
        }),
      }),
    };

    const adapter = topGunAdapter({ client: mockClient as TopGunClient })({} as BetterAuthOptions);

    // First request triggers ready
    await adapter.create({ model: 'user', data: { name: 'test1' } });

    // Second request should not call start again
    await adapter.create({ model: 'user', data: { name: 'test2' } });

    expect(startCallCount).toBe(1);
  });

  it('can disable waitForReady via option', async () => {
    const mockClient: Partial<MockTopGunClient> = {
      start: jest.fn(),
      getMap: jest.fn().mockReturnValue({
        set: jest.fn(),
      }),
    };

    const adapter = topGunAdapter({
      client: mockClient as TopGunClient,
      waitForReady: false
    })({} as BetterAuthOptions);

    await adapter.create({ model: 'user', data: { name: 'test' } });

    // start() should not be called when waitForReady is false
    expect(mockClient.start).not.toHaveBeenCalled();
  });
});

