import { EncryptedStorageAdapter } from '../adapters/EncryptedStorageAdapter';
import { IStorageAdapter, OpLogEntry } from '../IStorageAdapter';
import './test-polyfills';

class MockStorageAdapter implements IStorageAdapter {
  store = new Map<string, any>();
  meta = new Map<string, any>();
  opLog: OpLogEntry[] = [];

  async initialize(_dbName: string): Promise<void> {}
  async close(): Promise<void> {}

  // Generic type param V required by IStorageAdapter interface contract but not used in mock return
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
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
    return this.opLog.filter((o) => o.synced === 0);
  }
  async markOpsSynced(lastId: number): Promise<void> {
    this.opLog = this.opLog.filter((o) => !o.id || o.id > lastId);
  }

  async deleteOp(id: number): Promise<void> {
    this.opLog = this.opLog.filter((o) => o.id !== id);
  }

  async commitWrite(
    mutations: Array<{ store: 'kv' | 'meta'; type: 'put' | 'remove'; key: string; value?: any }>,
    op: Omit<OpLogEntry, 'id'>,
  ): Promise<number> {
    for (const m of mutations) {
      const target = m.store === 'meta' ? this.meta : this.store;
      if (m.type === 'remove') target.delete(m.key);
      else target.set(m.key, m.value);
    }
    return this.appendOpLog(op);
  }

  async getAllKeys(): Promise<string[]> {
    return Array.from(this.store.keys());
  }

  async getAllMetaKeys(): Promise<string[]> {
    return Array.from(this.meta.keys());
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
        length: 256,
      },
      true,
      ['encrypt', 'decrypt'],
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
      hlc: 'ts1',
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
        length: 256,
      },
      true,
      ['encrypt', 'decrypt'],
    );
    const wrongAdapter = new EncryptedStorageAdapter(mockAdapter, wrongKey);

    // 3. Attempt to decrypt should throw error
    await expect(wrongAdapter.get('secret-key')).rejects.toThrow('Failed to decrypt data');
  });

  it('should derive a working key from a passphrase (round-trip)', async () => {
    const adapter = await EncryptedStorageAdapter.fromPassphrase(
      mockAdapter,
      'correct horse battery staple',
    );
    await adapter.initialize('test-db');

    const value = { foo: 'bar', n: 123 };
    await adapter.put('k1', value);

    // Stored blob is ciphertext, not the original.
    const raw = await mockAdapter.get('k1');
    expect(raw.iv).toBeInstanceOf(Uint8Array);
    expect(raw.data).toBeInstanceOf(Uint8Array);
    expect(raw.foo).toBeUndefined();

    expect(await adapter.get('k1')).toEqual(value);
  });

  it('should persist the salt so the same passphrase reopens the store', async () => {
    const passphrase = 'reopen-me';
    const writer = await EncryptedStorageAdapter.fromPassphrase(mockAdapter, passphrase);
    await writer.initialize('test-db');
    await writer.put('k1', { secret: 42 });

    // A fresh adapter over the same underlying store + same passphrase must
    // re-derive the identical key from the persisted salt and decrypt.
    const reopened = await EncryptedStorageAdapter.fromPassphrase(mockAdapter, passphrase);
    await reopened.initialize('test-db');
    expect(await reopened.get('k1')).toEqual({ secret: 42 });
  });

  it('should reject the wrong passphrase on reopen', async () => {
    const writer = await EncryptedStorageAdapter.fromPassphrase(mockAdapter, 'right');
    await writer.initialize('test-db');
    await writer.put('k1', { secret: 'sensitive' });

    const wrong = await EncryptedStorageAdapter.fromPassphrase(mockAdapter, 'wrong');
    await wrong.initialize('test-db');
    await expect(wrong.get('k1')).rejects.toThrow('Failed to decrypt data');
  });

  it('should generate a unique salt per store', async () => {
    const a1 = await EncryptedStorageAdapter.fromPassphrase(new MockStorageAdapter(), 'pw');
    const a2 = await EncryptedStorageAdapter.fromPassphrase(new MockStorageAdapter(), 'pw');
    // Force derivation (and thus salt persistence) on each.
    await a1.put('k', 1);
    await a2.put('k', 1);

    const meta1 = (a1 as any).wrapped.meta.get('__topgun_enc_kdf__');
    const meta2 = (a2 as any).wrapped.meta.get('__topgun_enc_kdf__');
    expect(meta1.salt).toBeInstanceOf(Uint8Array);
    expect(meta2.salt).toBeInstanceOf(Uint8Array);
    expect(Array.from(meta1.salt)).not.toEqual(Array.from(meta2.salt));
  });

  it('should use a fresh IV per write', async () => {
    const adapter = await EncryptedStorageAdapter.fromPassphrase(mockAdapter, 'pw');
    await adapter.put('k1', { v: 1 });
    await adapter.put('k2', { v: 1 });

    const iv1 = (await mockAdapter.get('k1')).iv as Uint8Array;
    const iv2 = (await mockAdapter.get('k2')).iv as Uint8Array;
    expect(Array.from(iv1)).not.toEqual(Array.from(iv2));
  });

  it('should detect tampering (AES-GCM auth tag)', async () => {
    const adapter = await EncryptedStorageAdapter.fromPassphrase(mockAdapter, 'pw');
    await adapter.put('k1', { secret: 'do not flip' });

    // Flip a byte in the ciphertext; GCM verification must fail on read.
    const raw = await mockAdapter.get('k1');
    raw.data[0] ^= 0xff;
    await mockAdapter.put('k1', raw);

    await expect(adapter.get('k1')).rejects.toThrow('Failed to decrypt data');
  });

  it('should reject iterations below the OWASP floor', async () => {
    await expect(
      EncryptedStorageAdapter.fromPassphrase(mockAdapter, 'pw', { iterations: 1000 }),
    ).rejects.toThrow('iterations must be an integer >= 600000');
  });

  it('should reopen when the persisted salt was JSON-serialised (not a Uint8Array)', async () => {
    const writer = await EncryptedStorageAdapter.fromPassphrase(mockAdapter, 'pw');
    await writer.put('k1', { secret: 1 });

    // Simulate a JSON-backed adapter that lost the Uint8Array type on the salt.
    const params = mockAdapter.meta.get('__topgun_enc_kdf__');
    const jsonRoundTripped = JSON.parse(JSON.stringify(params));
    expect(jsonRoundTripped.salt).not.toBeInstanceOf(Uint8Array);
    mockAdapter.meta.set('__topgun_enc_kdf__', jsonRoundTripped);

    const reopened = await EncryptedStorageAdapter.fromPassphrase(mockAdapter, 'pw');
    expect(await reopened.get('k1')).toEqual({ secret: 1 });
  });

  it('should fail closed (not regenerate) when the persisted salt is unreadable', async () => {
    const writer = await EncryptedStorageAdapter.fromPassphrase(mockAdapter, 'pw');
    await writer.put('k1', { secret: 1 });

    // Corrupt the salt into something coerceSalt cannot interpret.
    mockAdapter.meta.set('__topgun_enc_kdf__', { v: 1, salt: 'not-bytes', iterations: 600000 });

    const reopened = await EncryptedStorageAdapter.fromPassphrase(mockAdapter, 'pw');
    await expect(reopened.get('k1')).rejects.toThrow('corrupted or unreadable');
  });

  it('should reject a persisted iteration count below the floor on reopen', async () => {
    // A tampered/corrupted record must not downgrade the KDF on reopen.
    mockAdapter.meta.set('__topgun_enc_kdf__', {
      v: 1,
      salt: new Uint8Array(16).fill(7),
      iterations: 1,
    });
    const reopened = await EncryptedStorageAdapter.fromPassphrase(mockAdapter, 'pw');
    await expect(reopened.put('k1', { v: 1 })).rejects.toThrow('floor');
  });

  it('should reject an unsupported persisted KDF version on reopen', async () => {
    mockAdapter.meta.set('__topgun_enc_kdf__', {
      v: 2,
      salt: new Uint8Array(16).fill(7),
      iterations: 600000,
    });
    const reopened = await EncryptedStorageAdapter.fromPassphrase(mockAdapter, 'pw');
    await expect(reopened.put('k1', { v: 1 })).rejects.toThrow('unsupported version');
  });

  it('should reject a persisted salt shorter than 16 bytes on reopen', async () => {
    mockAdapter.meta.set('__topgun_enc_kdf__', {
      v: 1,
      salt: new Uint8Array(8).fill(7),
      iterations: 600000,
    });
    const reopened = await EncryptedStorageAdapter.fromPassphrase(mockAdapter, 'pw');
    await expect(reopened.put('k1', { v: 1 })).rejects.toThrow('corrupted or unreadable');
  });

  it('should reserve the salt meta key from the public meta API', async () => {
    const adapter = await EncryptedStorageAdapter.fromPassphrase(mockAdapter, 'pw');
    await expect(adapter.setMeta('__topgun_enc_kdf__', { evil: true })).rejects.toThrow(
      'is reserved',
    );
    await expect(adapter.getMeta('__topgun_enc_kdf__')).rejects.toThrow('is reserved');
  });

  it('should recover after a transient derivation failure (no permanent brick)', async () => {
    const flaky = new MockStorageAdapter();
    const original = flaky.getMeta.bind(flaky);
    let calls = 0;
    flaky.getMeta = async (key: string) => {
      // Fail only the first salt read, then behave normally.
      if (key === '__topgun_enc_kdf__' && calls++ === 0) {
        throw new Error('transient IDB failure');
      }
      return original(key);
    };

    const adapter = await EncryptedStorageAdapter.fromPassphrase(flaky, 'pw');
    await expect(adapter.put('k1', { v: 1 })).rejects.toThrow('transient IDB failure');
    // The cached rejection must not brick the adapter — a retry succeeds.
    await adapter.put('k1', { v: 1 });
    expect(await adapter.get('k1')).toEqual({ v: 1 });
  });

  it('should reject an empty passphrase', async () => {
    await expect(EncryptedStorageAdapter.fromPassphrase(mockAdapter, '')).rejects.toThrow(
      'non-empty string',
    );
  });

  it('should encrypt all entries in batchPut', async () => {
    // 1. Call batchPut with multiple entries
    const entries = new Map<string, any>([
      ['user:1', { name: 'Alice', email: 'alice@example.com' }],
      ['user:2', { name: 'Bob', email: 'bob@example.com' }],
      ['user:3', { name: 'Charlie', email: 'charlie@example.com' }],
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
