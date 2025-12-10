import { EncryptedStorageAdapter } from '../adapters/EncryptedStorageAdapter';
import { IStorageAdapter, OpLogEntry } from '../IStorageAdapter';
import * as crypto from 'crypto';

// Polyfill WebCrypto for Node environment if needed
if (!globalThis.crypto) {
    // @ts-ignore
    globalThis.crypto = crypto.webcrypto;
}
if (!globalThis.window) {
    // @ts-ignore
    globalThis.window = globalThis;
}
if (!window.crypto) {
    // @ts-ignore
    window.crypto = crypto.webcrypto;
}

class MockStorageAdapter implements IStorageAdapter {
    store = new Map<string, any>();
    meta = new Map<string, any>();
    opLog: OpLogEntry[] = [];

    async initialize(dbName: string): Promise<void> { }
    async close(): Promise<void> { }

    async get<V>(key: string): Promise<any> {
        return this.store.get(key);
    }
    async put(key: string, value: any): Promise<void> {
        this.store.set(key, value);
    }
    async remove(key: string): Promise<void> {
        this.store.delete(key);
    }

    async getMeta(key: string): Promise<any> {
        return this.meta.get(key);
    }
    async setMeta(key: string, value: any): Promise<void> {
        this.meta.set(key, value);
    }

    async batchPut(entries: Map<string, any>): Promise<void> {
        for (const [k, v] of entries) {
            this.store.set(k, v);
        }
    }

    async appendOpLog(entry: Omit<OpLogEntry, 'id'>): Promise<number> {
        const id = this.opLog.length + 1;
        this.opLog.push({ ...entry, id, synced: 0 });
        return id;
    }
    async getPendingOps(): Promise<OpLogEntry[]> {
        return this.opLog.filter(o => o.synced === 0);
    }
    async markOpsSynced(lastId: number): Promise<void> {
        this.opLog.forEach(o => {
            if (o.id && o.id <= lastId) o.synced = 1;
        });
    }

    async getAllKeys(): Promise<string[]> {
        return Array.from(this.store.keys());
    }
}

describe('EncryptedStorageAdapter', () => {
    let mockAdapter: MockStorageAdapter;
    let encryptedAdapter: EncryptedStorageAdapter;
    let key: CryptoKey;

    beforeAll(async () => {
        key = await window.crypto.subtle.generateKey(
            {
                name: 'AES-GCM',
                length: 256
            },
            true,
            ['encrypt', 'decrypt']
        );
    });

    beforeEach(() => {
        mockAdapter = new MockStorageAdapter();
        encryptedAdapter = new EncryptedStorageAdapter(mockAdapter, key);
    });

    it('should encrypt data on put', async () => {
        const value = { foo: 'bar', n: 123 };
        await encryptedAdapter.put('test-key', value);

        const raw = await mockAdapter.get('test-key');
        expect(raw).toBeDefined();
        // Should NOT be the original object
        expect(raw).not.toEqual(value);
        // Should have structure { iv, data }
        expect(raw.iv).toBeInstanceOf(Uint8Array);
        expect(raw.data).toBeInstanceOf(Uint8Array);
    });

    it('should decrypt data on get', async () => {
        const value = { foo: 'bar', n: 123 };
        await encryptedAdapter.put('test-key', value);

        const retrieved = await encryptedAdapter.get('test-key');
        expect(retrieved).toEqual(value);
    });

    it('should encrypt metadata', async () => {
        const value = { cursor: 123 };
        await encryptedAdapter.setMeta('meta-key', value);

        const raw = await mockAdapter.getMeta('meta-key');
        expect(raw.iv).toBeInstanceOf(Uint8Array);

        const retrieved = await encryptedAdapter.getMeta('meta-key');
        expect(retrieved).toEqual(value);
    });

    it('should encrypt OpLog values and records', async () => {
        const entry = {
            key: 'doc1',
            op: 'PUT' as const,
            mapName: 'users',
            value: { name: 'Alice' },
            record: { value: 'Alice', clock: {} } as any, // minimal mock
            synced: 0,
            hlc: 'ts1'
        };

        await encryptedAdapter.appendOpLog(entry);

        const pending = await mockAdapter.getPendingOps();
        expect(pending.length).toBe(1);
        const rawOp = pending[0];

        // Key should be plaintext
        expect(rawOp.key).toBe('doc1');

        // Value should be encrypted
        expect(rawOp.value.name).toBeUndefined();
        expect(rawOp.value.iv).toBeDefined();

        // Record should be encrypted
        expect((rawOp.record as any).value).toBeUndefined();
        expect((rawOp.record as any).iv).toBeDefined();

        // Reading back via encrypted adapter should decrypt
        const decryptedOps = await encryptedAdapter.getPendingOps();
        expect(decryptedOps[0].value).toEqual(entry.value);
        expect(decryptedOps[0].record).toEqual(entry.record);
    });

    it('should throw error when decrypting with wrong key', async () => {
        // 1. Encrypt data with original key
        const value = { secret: 'sensitive data' };
        await encryptedAdapter.put('secret-key', value);

        // 2. Create new adapter with different key
        const wrongKey = await window.crypto.subtle.generateKey(
            {
                name: 'AES-GCM',
                length: 256
            },
            true,
            ['encrypt', 'decrypt']
        );
        const wrongAdapter = new EncryptedStorageAdapter(mockAdapter, wrongKey);

        // 3. Attempt to decrypt should throw error
        await expect(wrongAdapter.get('secret-key')).rejects.toThrow('Failed to decrypt data');
    });

    it('should encrypt all entries in batchPut', async () => {
        // 1. Call batchPut with multiple entries
        const entries = new Map<string, any>([
            ['user:1', { name: 'Alice', email: 'alice@example.com' }],
            ['user:2', { name: 'Bob', email: 'bob@example.com' }],
            ['user:3', { name: 'Charlie', email: 'charlie@example.com' }]
        ]);

        await encryptedAdapter.batchPut(entries);

        // 2. Verify each entry in underlying storage is encrypted (has iv, data structure)
        for (const [key] of entries) {
            const raw = await mockAdapter.get(key);
            expect(raw).toBeDefined();
            expect(raw.iv).toBeInstanceOf(Uint8Array);
            expect(raw.data).toBeInstanceOf(Uint8Array);
            // Should NOT contain plaintext fields
            expect(raw.name).toBeUndefined();
            expect(raw.email).toBeUndefined();
        }

        // 3. Verify decryption via encryptedAdapter returns original values
        for (const [key, originalValue] of entries) {
            const decrypted = await encryptedAdapter.get(key);
            expect(decrypted).toEqual(originalValue);
        }
    });
});
