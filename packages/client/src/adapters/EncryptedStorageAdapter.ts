import { IStorageAdapter, OpLogEntry } from '../IStorageAdapter';
import { EncryptionManager } from '../crypto/EncryptionManager';

/**
 * Wraps an underlying storage adapter and encrypts data at rest using AES-GCM.
 */
export class EncryptedStorageAdapter implements IStorageAdapter {
    constructor(
        private wrapped: IStorageAdapter,
        private key: CryptoKey
    ) { }

    async initialize(dbName: string): Promise<void> {
        return this.wrapped.initialize(dbName);
    }

    async close(): Promise<void> {
        return this.wrapped.close();
    }

    // --- KV Operations ---

    async get<V>(key: string): Promise<V | any | undefined> {
        const raw = await this.wrapped.get<any>(key);

        if (!raw) {
            return undefined;
        }

        // Check if it looks like an encrypted record
        // We expect { iv: Uint8Array, data: Uint8Array }
        // Note: In a real app we might want a stricter check or a version tag.
        if (this.isEncryptedRecord(raw)) {
            try {
                return await EncryptionManager.decrypt(this.key, raw);
            } catch (e) {
                // Fallback for migration or corruption?
                // For now, fail loud as per spec.
                throw e;
            }
        }

        // Return raw if not encrypted (backwards compatibility during dev, or unencrypted data)
        return raw;
    }

    async put(key: string, value: any): Promise<void> {
        const encrypted = await EncryptionManager.encrypt(this.key, value);
        // Store as plain object to be compatible with structured clone algorithm of IndexedDB
        const storedValue = {
            iv: encrypted.iv,
            data: encrypted.data
        };
        return this.wrapped.put(key, storedValue);
    }

    async remove(key: string): Promise<void> {
        return this.wrapped.remove(key);
    }

    // --- Metadata ---

    async getMeta(key: string): Promise<any> {
        const raw = await this.wrapped.getMeta(key);
        if (!raw) return undefined;

        if (this.isEncryptedRecord(raw)) {
            return EncryptionManager.decrypt(this.key, raw);
        }
        return raw;
    }

    async setMeta(key: string, value: any): Promise<void> {
        const encrypted = await EncryptionManager.encrypt(this.key, value);
        return this.wrapped.setMeta(key, {
            iv: encrypted.iv,
            data: encrypted.data
        });
    }

    // --- Batch ---

    async batchPut(entries: Map<string, any>): Promise<void> {
        const encryptedEntries = new Map<string, any>();

        for (const [key, value] of entries.entries()) {
            const encrypted = await EncryptionManager.encrypt(this.key, value);
            encryptedEntries.set(key, {
                iv: encrypted.iv,
                data: encrypted.data
            });
        }

        return this.wrapped.batchPut(encryptedEntries);
    }

    // --- OpLog ---

    async appendOpLog(entry: Omit<OpLogEntry, 'id'>): Promise<number> {
        // Encrypt sensitive fields: value, record, orRecord
        const encryptedEntry = { ...entry };

        if (entry.value !== undefined) {
            const enc = await EncryptionManager.encrypt(this.key, entry.value);
            encryptedEntry.value = { iv: enc.iv, data: enc.data };
        }

        if (entry.record !== undefined) {
            const enc = await EncryptionManager.encrypt(this.key, entry.record);
            encryptedEntry.record = { iv: enc.iv, data: enc.data } as any;
        }

        if (entry.orRecord !== undefined) {
            const enc = await EncryptionManager.encrypt(this.key, entry.orRecord);
            encryptedEntry.orRecord = { iv: enc.iv, data: enc.data } as any;
        }

        // Note: 'key', 'op', 'mapName', 'orTag', 'hlc', 'synced' remain plaintext for indexing

        return this.wrapped.appendOpLog(encryptedEntry);
    }

    async getPendingOps(): Promise<OpLogEntry[]> {
        const ops = await this.wrapped.getPendingOps();

        // Decrypt in place
        // We map concurrently for performance
        return Promise.all(ops.map(async op => {
            const decryptedOp = { ...op };

            if (this.isEncryptedRecord(op.value)) {
                decryptedOp.value = await EncryptionManager.decrypt(this.key, op.value);
            }

            if (this.isEncryptedRecord(op.record)) {
                decryptedOp.record = await EncryptionManager.decrypt(this.key, op.record as any);
            }

            if (this.isEncryptedRecord(op.orRecord)) {
                decryptedOp.orRecord = await EncryptionManager.decrypt(this.key, op.orRecord as any);
            }

            return decryptedOp;
        }));
    }

    async markOpsSynced(lastId: number): Promise<void> {
        return this.wrapped.markOpsSynced(lastId);
    }

    // --- Iteration ---

    async getAllKeys(): Promise<string[]> {
        return this.wrapped.getAllKeys();
    }

    // --- Helpers ---

    private isEncryptedRecord(data: any): data is { iv: Uint8Array, data: Uint8Array } {
        return data &&
            typeof data === 'object' &&
            data.iv instanceof Uint8Array &&
            data.data instanceof Uint8Array;
    }
}
