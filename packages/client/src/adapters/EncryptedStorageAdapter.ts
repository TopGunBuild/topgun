import { IStorageAdapter, OpLogEntry } from '../IStorageAdapter';
import { EncryptionManager } from '../crypto/EncryptionManager';

/**
 * Wraps an underlying storage adapter and encrypts data at rest using AES-GCM.
 */
export class EncryptedStorageAdapter implements IStorageAdapter {
  constructor(
    private wrapped: IStorageAdapter,
    private key: CryptoKey,
  ) {}

  async initialize(dbName: string): Promise<void> {
    return this.wrapped.initialize(dbName);
  }

  async close(): Promise<void> {
    return this.wrapped.close();
  }

  // --- KV Operations ---

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- return type includes any to cover non-V metadata keys; mirrors IStorageAdapter.get contract
  async get<V>(key: string): Promise<V | any | undefined> {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- fetching with any because the raw value may be an encrypted blob or a typed record; decryption follows
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

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- IStorageAdapter.put contract accepts any serialisable value; encrypted wrapper does not know the original value type
  async put(key: string, value: any): Promise<void> {
    const encrypted = await EncryptionManager.encrypt(this.key, value);
    // Store as plain object to be compatible with structured clone algorithm of IndexedDB
    const storedValue = {
      iv: encrypted.iv,
      data: encrypted.data,
    };
    return this.wrapped.put(key, storedValue);
  }

  async remove(key: string): Promise<void> {
    return this.wrapped.remove(key);
  }

  // --- Metadata ---

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- meta values have no fixed schema; mirrors IStorageAdapter.getMeta contract
  async getMeta(key: string): Promise<any> {
    const raw = await this.wrapped.getMeta(key);
    if (!raw) return undefined;

    if (this.isEncryptedRecord(raw)) {
      return EncryptionManager.decrypt(this.key, raw);
    }
    return raw;
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- meta values have no fixed schema; mirrors IStorageAdapter.setMeta contract
  async setMeta(key: string, value: any): Promise<void> {
    const encrypted = await EncryptionManager.encrypt(this.key, value);
    return this.wrapped.setMeta(key, {
      iv: encrypted.iv,
      data: encrypted.data,
    });
  }

  // --- Batch ---

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- batch put accepts a mixed-value map; mirrors IStorageAdapter.batchPut contract
  async batchPut(entries: Map<string, any>): Promise<void> {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- encrypted entries map holds blobs of varying shape before being passed to wrapped adapter
    const encryptedEntries = new Map<string, any>();

    for (const [key, value] of entries.entries()) {
      const encrypted = await EncryptionManager.encrypt(this.key, value);
      encryptedEntries.set(key, {
        iv: encrypted.iv,
        data: encrypted.data,
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
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- record field is typed as LWWRecord<any> in OpLogEntry; cast to any to replace with encrypted blob shape
      encryptedEntry.record = { iv: enc.iv, data: enc.data } as any;
    }

    if (entry.orRecord !== undefined) {
      const enc = await EncryptionManager.encrypt(this.key, entry.orRecord);
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- orRecord field is typed as ORMapRecord<any> in OpLogEntry; cast to any to replace with encrypted blob shape
      encryptedEntry.orRecord = { iv: enc.iv, data: enc.data } as any;
    }

    // Note: 'key', 'op', 'mapName', 'orTag', 'hlc', 'synced' remain plaintext for indexing

    return this.wrapped.appendOpLog(encryptedEntry);
  }

  async getPendingOps(): Promise<OpLogEntry[]> {
    const ops = await this.wrapped.getPendingOps();

    // Decrypt in place
    // We map concurrently for performance
    return Promise.all(
      ops.map(async (op) => {
        const decryptedOp = { ...op };

        if (this.isEncryptedRecord(op.value)) {
          decryptedOp.value = await EncryptionManager.decrypt(this.key, op.value);
        }

        if (this.isEncryptedRecord(op.record)) {
          // eslint-disable-next-line @typescript-eslint/no-explicit-any -- op.record is typed as LWWRecord<any> but holds an encrypted blob; cast to any to pass to decrypt
          decryptedOp.record = await EncryptionManager.decrypt(this.key, op.record as any);
        }

        if (this.isEncryptedRecord(op.orRecord)) {
          // eslint-disable-next-line @typescript-eslint/no-explicit-any -- op.orRecord is typed as ORMapRecord<any> but holds an encrypted blob; cast to any to pass to decrypt
          decryptedOp.orRecord = await EncryptionManager.decrypt(this.key, op.orRecord as any);
        }

        return decryptedOp;
      }),
    );
  }

  async markOpsSynced(lastId: number): Promise<void> {
    return this.wrapped.markOpsSynced(lastId);
  }

  // --- Iteration ---

  async getAllKeys(): Promise<string[]> {
    return this.wrapped.getAllKeys();
  }

  // --- Helpers ---

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- type guard accepts unknown/any to check structural shape at runtime; narrowing produces the typed result
  private isEncryptedRecord(data: any): data is { iv: Uint8Array; data: Uint8Array } {
    return (
      data &&
      typeof data === 'object' &&
      data.iv instanceof Uint8Array &&
      data.data instanceof Uint8Array
    );
  }
}
