import { IStorageAdapter, OpLogEntry, StorageMutation } from '../IStorageAdapter';
import { EncryptionManager } from '../crypto/EncryptionManager';
import { getWebCrypto } from '../crypto/webcrypto';

/** OWASP 2023 minimum for PBKDF2-HMAC-SHA256; also the enforced floor. */
const DEFAULT_PBKDF2_ITERATIONS = 600_000;
/**
 * Hard lower bound on PBKDF2 iterations. A caller-supplied `iterations` below
 * this is rejected rather than silently honoured, because a weak count would be
 * persisted and reused on every reopen — a permanent, invisible downgrade of the
 * only secret protecting the data.
 */
const MIN_PBKDF2_ITERATIONS = 600_000;
/** 128-bit salt — comfortably above the 16-byte NIST SP 800-132 floor. */
const SALT_BYTES = 16;

/**
 * Coerce a persisted salt back to a `Uint8Array`. Adapters that round-trip meta
 * through structured clone preserve the type, but a JSON-backed adapter turns it
 * into a plain `{ "0": .., "1": .. }` object or a number[]. Returns null when the
 * value cannot be interpreted as bytes — the caller MUST fail closed rather than
 * regenerate, or it would orphan the existing ciphertext.
 */
function coerceSalt(value: unknown): Uint8Array | null {
  if (value instanceof Uint8Array) return value.length > 0 ? value : null;
  if (value instanceof ArrayBuffer) {
    return value.byteLength > 0 ? new Uint8Array(value) : null;
  }
  // number[] or a JSON-serialised Uint8Array ({ "0": n, "1": n, ... }). Validate
  // every byte strictly — reject holes (sparse arrays) and out-of-range values
  // rather than letting Uint8Array.from coerce them to 0.
  const bytes = Array.isArray(value)
    ? value
    : value && typeof value === 'object'
      ? Object.values(value as Record<string, unknown>)
      : null;
  if (!bytes || bytes.length === 0) return null;
  for (const b of bytes) {
    if (typeof b !== 'number' || !Number.isInteger(b) || b < 0 || b > 255) return null;
  }
  return Uint8Array.from(bytes as number[]);
}
/**
 * Plaintext meta key under which the per-adapter KDF salt is persisted in the
 * wrapped adapter. Stored unencrypted by necessity: it is the input needed to
 * re-derive the key that unseals everything else (a salt is not secret).
 */
const SALT_META_KEY = '__topgun_enc_kdf__';

interface PassphraseDerivation {
  passphrase: string;
  iterations: number;
}

interface PersistedKdfParams {
  v: 1;
  salt: Uint8Array;
  iterations: number;
}

/**
 * Validate a persisted KDF record read back from the wrapped adapter. A record
 * that exists but is malformed in ANY way (unknown version, missing/short salt,
 * iteration count below the floor) is rejected by throwing — never silently
 * repaired or regenerated, because deriving from a different/weaker input would
 * orphan or downgrade the existing ciphertext. The floor is therefore enforced
 * on reopen, not just at creation.
 */
function parsePersistedKdf(value: unknown): { salt: Uint8Array; iterations: number } {
  const corrupt = (detail: string) =>
    new Error(
      `EncryptedStorageAdapter: persisted KDF params are ${detail}; refusing to regenerate (would orphan existing ciphertext)`,
    );

  if (!value || typeof value !== 'object') throw corrupt('corrupted or unreadable');
  const rec = value as Partial<PersistedKdfParams>;
  if (rec.v !== 1) throw corrupt(`an unsupported version (${String(rec.v)})`);

  const salt = coerceSalt(rec.salt);
  if (!salt || salt.length < SALT_BYTES) throw corrupt('corrupted or unreadable (salt)');

  if (
    typeof rec.iterations !== 'number' ||
    !Number.isInteger(rec.iterations) ||
    rec.iterations < MIN_PBKDF2_ITERATIONS
  ) {
    throw corrupt(`below the ${MIN_PBKDF2_ITERATIONS}-iteration floor (${String(rec.iterations)})`);
  }

  return { salt, iterations: rec.iterations };
}

/**
 * Wraps an underlying storage adapter and encrypts data at rest using AES-GCM.
 *
 * Construct with a pre-derived `CryptoKey`, or use the {@link fromPassphrase}
 * convenience to derive one from a user passphrase (PBKDF2) with the salt
 * managed and persisted for you.
 */
export class EncryptedStorageAdapter implements IStorageAdapter {
  private key?: CryptoKey;
  private derivation?: PassphraseDerivation;
  // Memoises the first derivation so concurrent reads/writes derive exactly once.
  private keyReady?: Promise<CryptoKey>;

  constructor(
    private wrapped: IStorageAdapter,
    key: CryptoKey,
  ) {
    this.key = key;
  }

  /**
   * Derive an at-rest encryption key from a human passphrase and wrap `adapter`.
   *
   * Handles the full Web Crypto KDF dance for you: a random per-adapter salt is
   * generated on first use and persisted (plaintext — a salt is not a secret)
   * alongside your data, then reused on every later session so the same
   * passphrase unseals the same store. The derived AES-GCM key is
   * non-extractable.
   *
   * Works in browsers and Node ≥ 20 (any runtime exposing `globalThis.crypto`).
   *
   * @param adapter    The underlying storage adapter to encrypt (e.g. `IDBAdapter`).
   * @param passphrase The user-provided secret. Stronger is better — this is the
   *                   sole input protecting the data.
   * @param options.iterations PBKDF2 iteration count for a NEW store. Defaults to
   *                   600,000 (OWASP 2023); values below 600,000 are rejected. The
   *                   count is persisted with the salt on first use and reused
   *                   verbatim on reopen, so changing it later has no effect on an
   *                   existing store (re-keying would require re-encrypting).
   *
   * @example
   * ```typescript
   * const encrypted = await EncryptedStorageAdapter.fromPassphrase(
   *   new IDBAdapter(),
   *   'correct horse battery staple',
   * );
   * const client = new TopGunClient({ storage: encrypted });
   * ```
   */
  static async fromPassphrase(
    adapter: IStorageAdapter,
    passphrase: string,
    options?: { iterations?: number },
  ): Promise<EncryptedStorageAdapter> {
    if (typeof passphrase !== 'string' || passphrase.length === 0) {
      throw new Error(
        'EncryptedStorageAdapter.fromPassphrase: passphrase must be a non-empty string',
      );
    }
    const iterations = options?.iterations ?? DEFAULT_PBKDF2_ITERATIONS;
    if (!Number.isInteger(iterations) || iterations < MIN_PBKDF2_ITERATIONS) {
      throw new Error(
        `EncryptedStorageAdapter.fromPassphrase: iterations must be an integer >= ${MIN_PBKDF2_ITERATIONS} (OWASP 2023 floor); got ${iterations}`,
      );
    }
    // Surface a missing Web Crypto implementation eagerly at call time rather
    // than on the first deferred read/write.
    getWebCrypto();

    // Construct in deferred-key mode: the real key is derived lazily on first
    // use, after the wrapped adapter has been initialised (so the salt meta is
    // reachable). `key` stays undefined until then.
    const instance = new EncryptedStorageAdapter(adapter, undefined as unknown as CryptoKey);
    instance.key = undefined;
    instance.derivation = { passphrase, iterations };
    return instance;
  }

  /**
   * Resolve the AES-GCM key, deriving it from the passphrase on first use.
   * Direct-key mode returns the constructor-supplied key immediately.
   */
  private async resolveKey(): Promise<CryptoKey> {
    if (this.key) return this.key;
    if (!this.derivation) {
      throw new Error('EncryptedStorageAdapter: no encryption key configured');
    }
    if (!this.keyReady) {
      // Clear the memo on failure so a transient cause (adapter not yet
      // initialised, a flaky IDB transaction) does not permanently brick the
      // adapter — the next call re-attempts derivation.
      this.keyReady = this.deriveKeyFromPassphrase(this.derivation).catch((err) => {
        this.keyReady = undefined;
        throw err;
      });
    }
    this.key = await this.keyReady;
    return this.key;
  }

  /**
   * Load-or-create the per-adapter salt (plaintext, via the wrapped adapter) and
   * run PBKDF2 to derive a non-extractable AES-GCM key.
   */
  private async deriveKeyFromPassphrase(derivation: PassphraseDerivation): Promise<CryptoKey> {
    const webcrypto = getWebCrypto();
    const { salt, iterations } = await this.loadOrCreateKdfParams(webcrypto, derivation.iterations);

    const baseKey = await webcrypto.subtle.importKey(
      'raw',
      new TextEncoder().encode(derivation.passphrase),
      'PBKDF2',
      false,
      ['deriveKey'],
    );

    return webcrypto.subtle.deriveKey(
      { name: 'PBKDF2', salt: salt as unknown as ArrayBuffer, iterations, hash: 'SHA-256' },
      baseKey,
      { name: 'AES-GCM', length: 256 },
      false, // non-extractable: the key never leaves the SubtleCrypto boundary
      ['encrypt', 'decrypt'],
    );
  }

  /**
   * Resolve the KDF salt + iteration count. An existing record must validate or
   * we fail closed (never regenerate over it). On genuine first use we create and
   * persist a random salt, then re-read it: if a concurrent instance won the
   * write (last-write-wins adapters), we adopt the persisted params so both
   * instances converge on the same key instead of orphaning each other's data.
   */
  private async loadOrCreateKdfParams(
    webcrypto: Crypto,
    defaultIterations: number,
  ): Promise<{ salt: Uint8Array; iterations: number }> {
    const persisted = await this.wrapped.getMeta(SALT_META_KEY);
    if (persisted != null) {
      // Reopen: reuse exactly the salt+iterations the store was sealed with.
      return parsePersistedKdf(persisted);
    }

    // Genuine first use: create, persist, then re-read to settle any race.
    const salt = webcrypto.getRandomValues(new Uint8Array(SALT_BYTES));
    const params: PersistedKdfParams = { v: 1, salt, iterations: defaultIterations };
    await this.wrapped.setMeta(SALT_META_KEY, params);
    const confirmed = await this.wrapped.getMeta(SALT_META_KEY);
    return parsePersistedKdf(confirmed ?? params);
  }

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
        return await EncryptionManager.decrypt(await this.resolveKey(), raw);
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
    const encrypted = await EncryptionManager.encrypt(await this.resolveKey(), value);
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
    this.assertNotReserved(key);
    const raw = await this.wrapped.getMeta(key);
    if (!raw) return undefined;

    if (this.isEncryptedRecord(raw)) {
      return EncryptionManager.decrypt(await this.resolveKey(), raw);
    }
    return raw;
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- meta values have no fixed schema; mirrors IStorageAdapter.setMeta contract
  async setMeta(key: string, value: any): Promise<void> {
    this.assertNotReserved(key);
    const encrypted = await EncryptionManager.encrypt(await this.resolveKey(), value);
    return this.wrapped.setMeta(key, {
      iv: encrypted.iv,
      data: encrypted.data,
    });
  }

  /**
   * Guards the reserved KDF-salt meta key from the public meta API: an external
   * `setMeta(SALT_META_KEY, …)` would encrypt over the plaintext salt and silently
   * orphan the store on reopen, so we reject access outright.
   */
  private assertNotReserved(key: string): void {
    if (key === SALT_META_KEY) {
      throw new Error(`EncryptedStorageAdapter: meta key "${SALT_META_KEY}" is reserved`);
    }
  }

  // --- Batch ---

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- batch put accepts a mixed-value map; mirrors IStorageAdapter.batchPut contract
  async batchPut(entries: Map<string, any>): Promise<void> {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- encrypted entries map holds blobs of varying shape before being passed to wrapped adapter
    const encryptedEntries = new Map<string, any>();
    const key = await this.resolveKey();

    for (const [entryKey, value] of entries.entries()) {
      const encrypted = await EncryptionManager.encrypt(key, value);
      encryptedEntries.set(entryKey, {
        iv: encrypted.iv,
        data: encrypted.data,
      });
    }

    return this.wrapped.batchPut(encryptedEntries);
  }

  // --- OpLog ---

  async appendOpLog(entry: Omit<OpLogEntry, 'id'>): Promise<number> {
    return this.wrapped.appendOpLog(await this.encryptOpEntry(entry));
  }

  /**
   * Encrypts the sensitive op-log fields (value, record, orRecord). 'key', 'op', 'mapName',
   * 'orTag', 'hlc', 'synced' remain plaintext for indexing. Shared by appendOpLog + commitWrite.
   */
  private async encryptOpEntry(entry: Omit<OpLogEntry, 'id'>): Promise<Omit<OpLogEntry, 'id'>> {
    const encryptedEntry = { ...entry };
    const key = await this.resolveKey();

    if (entry.value !== undefined) {
      const enc = await EncryptionManager.encrypt(key, entry.value);
      encryptedEntry.value = { iv: enc.iv, data: enc.data };
    }

    if (entry.record !== undefined) {
      const enc = await EncryptionManager.encrypt(key, entry.record);
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- record field is typed as LWWRecord<any> in OpLogEntry; cast to any to replace with encrypted blob shape
      encryptedEntry.record = { iv: enc.iv, data: enc.data } as any;
    }

    if (entry.orRecord !== undefined) {
      const enc = await EncryptionManager.encrypt(key, entry.orRecord);
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- orRecord field is typed as ORMapRecord<any> in OpLogEntry; cast to any to replace with encrypted blob shape
      encryptedEntry.orRecord = { iv: enc.iv, data: enc.data } as any;
    }

    return encryptedEntry;
  }

  async commitWrite(mutations: StorageMutation[], op: Omit<OpLogEntry, 'id'>): Promise<number> {
    // Encrypt each put mutation's value (removes carry no value); the wrapped adapter
    // provides the atomic single-transaction guarantee over the encrypted blobs.
    const key = await this.resolveKey();
    const encryptedMutations: StorageMutation[] = await Promise.all(
      mutations.map(async (m) => {
        if (m.type === 'put' && m.value !== undefined) {
          const enc = await EncryptionManager.encrypt(key, m.value);
          return { ...m, value: { iv: enc.iv, data: enc.data } };
        }
        return m;
      }),
    );
    return this.wrapped.commitWrite(encryptedMutations, await this.encryptOpEntry(op));
  }

  async deleteOp(id: number): Promise<void> {
    return this.wrapped.deleteOp(id);
  }

  async getPendingOps(): Promise<OpLogEntry[]> {
    const ops = await this.wrapped.getPendingOps();
    if (ops.length === 0) return ops;
    const key = await this.resolveKey();

    // Decrypt in place
    // We map concurrently for performance
    return Promise.all(
      ops.map(async (op) => {
        const decryptedOp = { ...op };

        if (this.isEncryptedRecord(op.value)) {
          decryptedOp.value = await EncryptionManager.decrypt(key, op.value);
        }

        if (this.isEncryptedRecord(op.record)) {
          // eslint-disable-next-line @typescript-eslint/no-explicit-any -- op.record is typed as LWWRecord<any> but holds an encrypted blob; cast to any to pass to decrypt
          decryptedOp.record = await EncryptionManager.decrypt(key, op.record as any);
        }

        if (this.isEncryptedRecord(op.orRecord)) {
          // eslint-disable-next-line @typescript-eslint/no-explicit-any -- op.orRecord is typed as ORMapRecord<any> but holds an encrypted blob; cast to any to pass to decrypt
          decryptedOp.orRecord = await EncryptionManager.decrypt(key, op.orRecord as any);
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

  // Key names are never encrypted (only values are) — a plain passthrough is correct.
  async getAllMetaKeys(): Promise<string[]> {
    return this.wrapped.getAllMetaKeys();
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
