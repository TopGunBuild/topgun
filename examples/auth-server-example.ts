import { ServerCoordinator, IServerStorage, StorageValue } from '@topgunbuild/server';
import { TopGunClient, IStorageAdapter, OpLogEntry } from '@topgunbuild/client';
import { topGunAdapter } from '@topgunbuild/adapter-better-auth';
import { betterAuth } from 'better-auth';
import { LWWRecord, ORMapRecord } from '@topgunbuild/core';

// --- 1. In-Memory Server Storage ---
class MemoryServerStorage implements IServerStorage {
    private data = new Map<string, Map<string, any>>();

    async initialize() {}
    async close() {}

    private getMap(name: string) {
        if (!this.data.has(name)) this.data.set(name, new Map());
        return this.data.get(name)!;
    }

    async load(mapName: string, key: string): Promise<StorageValue<any> | undefined> {
        return this.getMap(mapName).get(key);
    }

    async loadAll(mapName: string, keys: string[]): Promise<Map<string, StorageValue<any>>> {
        const result = new Map();
        const map = this.getMap(mapName);
        for (const key of keys) {
            if (map.has(key)) result.set(key, map.get(key));
        }
        return result;
    }

    async loadAllKeys(mapName: string): Promise<string[]> {
        return Array.from(this.getMap(mapName).keys());
    }

    async store(mapName: string, key: string, record: StorageValue<any>): Promise<void> {
        this.getMap(mapName).set(key, record);
    }

    async storeAll(mapName: string, records: Map<string, StorageValue<any>>): Promise<void> {
        const map = this.getMap(mapName);
        for (const [key, val] of records) {
            map.set(key, val);
        }
    }

    async delete(mapName: string, key: string): Promise<void> {
        this.getMap(mapName).delete(key);
    }

    async deleteAll(mapName: string, keys: string[]): Promise<void> {
        const map = this.getMap(mapName);
        for (const key of keys) map.delete(key);
    }
}

// --- 2. In-Memory Client Storage ---
class MemoryClientStorage implements IStorageAdapter {
  private data = new Map<string, any>();
  private meta = new Map<string, any>();
  private opLog: OpLogEntry[] = [];
  private opIdCounter = 1;

  async initialize(dbName: string): Promise<void> {}
  async close(): Promise<void> {}

  async get<V>(key: string): Promise<LWWRecord<V> | ORMapRecord<V>[] | any | undefined> {
    return this.data.get(key);
  }

  async put(key: string, value: any): Promise<void> {
    this.data.set(key, value);
  }

  async remove(key: string): Promise<void> {
    this.data.delete(key);
  }

  async getMeta(key: string): Promise<any> {
    return this.meta.get(key);
  }

  async setMeta(key: string, value: any): Promise<void> {
    this.meta.set(key, value);
  }

  async batchPut(entries: Map<string, any>): Promise<void> {
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

async function main() {
    // 1. Start Server
    const serverPort = 4000;
    const server = new ServerCoordinator({
        port: serverPort,
        clusterPort: 4001,
        nodeId: 'server-1',
        storage: new MemoryServerStorage(),
        securityPolicies: [
            // Allow everything for this demo
            { role: 'USER', mapNamePattern: '*', actions: ['ALL'] },
            { role: 'ADMIN', mapNamePattern: '*', actions: ['ALL'] }
        ]
    });
    console.log(`Server started on port ${serverPort}`);

    // 2. Start Client
    const client = new TopGunClient({
        serverUrl: `ws://localhost:${serverPort}`,
        storage: new MemoryClientStorage(),
        nodeId: 'client-adapter'
    });
    
    // Admin access for the adapter
    client.setAuthToken('ADMIN_SECRET'); // In real app, this would be a system token
    
    await client.start();
    console.log('Client started and connected');

    // 3. Initialize Better Auth
    const auth = betterAuth({
        database: topGunAdapter({
            client,
            modelMap: {
                user: 'users',
                session: 'sessions',
                account: 'accounts',
                verification: 'verifications'
            }
        }),
        emailAndPassword: {
            enabled: true
        },
        advanced: {
            // Generate unique IDs for better auth models if needed, adapter handles UUIDs usually
        }
    });

    console.log('Better Auth Initialized');

    // 4. Simulate Auth Flow
    try {
        console.log('\n--- Creating User ---');
        const user = await auth.api.signUpEmail({
            body: {
                email: "test@example.com",
                password: "password123",
                name: "Maverick"
            }
        });
        console.log('User Created:', user);

        // Debug: Verify Adapter findOne with Join
        const debugAdapter = (auth.options.database as any)(auth.options);
        const debugUser = await debugAdapter.findOne({
            model: 'user',
            where: [{ field: 'email', value: 'test@example.com' }],
            join: { account: true }
        });
        console.log('Debug Fetch User with Accounts:', JSON.stringify(debugUser, null, 2));
        
        if (debugUser && (debugUser as any).accounts) {
            const found = (debugUser as any).accounts.find((a: any) => a.providerId === 'credential');
            console.log('Debug Manual Find Account:', found);
        } else {
            console.log('Debug: No accounts array on user');
        }

        console.log('\n--- Logging In ---');
        const session = await auth.api.signInEmail({
            body: {
                email: "test@example.com",
                password: "password123"
            }
        });
        console.log('Logged In:', session);

        console.log('\n--- Verify Data in TopGun ---');
        // Wait a moment for async ops (although better-auth awaits adapter calls, TopGun sync might be async, but adapter writes locally instantly)
        const userMap = client.getMap('users');
        const users = await client.query('users', {}).subscribe((res) => {
            // just to peek
        }); 
        // Accessing internal map directly
        // We can use the adapter to find it back to verify
        const adapter = (auth.options.database as any)(auth.options);
        const foundUser = await adapter.findOne({
            model: 'user',
            where: [{ field: 'email', value: 'test@example.com' }]
        });
        console.log('Found User via Adapter:', foundUser);

    } catch (e) {
        console.error('Error:', e);
    } finally {
        console.log('\n--- Inspecting DB State ---');
        
        // Check Users
        const userMap = client.getMap('users');
        const users = await client.query('users', {}).subscribe((res) => { /*... */ });
        // Since subscribe is async/reactive and we need to wait for results in this script context:
        // We can just peek into the map since we are using MemoryAdapter which writes synchronously to memory map in client
        // But `client.query` might be empty if sync hasn't happened? 
        // No, `runLocalQuery` checks storage.
        
        // Let's dump all keys from client storage
        const keys = await (client as any).storageAdapter.getAllKeys();
        console.log('All Keys in Storage:', keys);

        const accounts = keys.filter((k: string) => k.startsWith('accounts:'));
        console.log('Accounts found:', accounts.length);
        for(const k of accounts) {
             console.log(k, await (client as any).storageAdapter.get(k));
        }

        process.exit(0);
    }
}

main();

