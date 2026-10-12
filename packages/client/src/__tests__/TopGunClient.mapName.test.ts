import { TopGunClient } from '../TopGunClient';
import { QueryHandle } from '../QueryHandle';
import { QueryOnceUnsettledError } from '../errors/QueryOnceError';
import type { IStorageAdapter, OpLogEntry } from '../IStorageAdapter';

/**
 * The map-name rule at the client's entry points.
 *
 * `getMap` / `getORMap` take the full client rule (the server's four clauses
 * plus no ":"), and refuse before anything reaches storage. `query`,
 * `queryOnce` and `queryOncePaged` take the server's clauses only: the server
 * answers a query for a name it cannot hold with silence, so the client has to
 * be the one to say no — and a name with ":" stays readable.
 */

class MemoryStorageAdapter implements IStorageAdapter {
  private kv = new Map<string, unknown>();
  private meta = new Map<string, unknown>();
  private pending: OpLogEntry[] = [];

  async initialize(): Promise<void> {}
  async close(): Promise<void> {}
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the adapter interface is untyped at this boundary
  async get(key: string): Promise<any> {
    return this.kv.get(key);
  }
  async put(key: string, value: unknown): Promise<void> {
    this.kv.set(key, value);
  }
  async remove(key: string): Promise<void> {
    this.kv.delete(key);
  }
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the adapter interface is untyped at this boundary
  async getMeta(key: string): Promise<any> {
    return this.meta.get(key);
  }
  async setMeta(key: string, value: unknown): Promise<void> {
    this.meta.set(key, value);
  }
  async batchPut(entries: Map<string, unknown>): Promise<void> {
    for (const [k, v] of entries) this.kv.set(k, v);
  }
  async appendOpLog(entry: Omit<OpLogEntry, 'id'>): Promise<number> {
    const id = this.pending.length + 1;
    this.pending.push({ ...entry, id } as OpLogEntry);
    return id;
  }
  async getPendingOps(): Promise<OpLogEntry[]> {
    return this.pending;
  }
  async markOpsSynced(lastId: number): Promise<void> {
    this.pending = this.pending.filter((op) => Number(op.id) > lastId);
  }
  async deleteOp(id: number): Promise<void> {
    this.pending = this.pending.filter((op) => Number(op.id) !== id);
  }
  async commitWrite(): Promise<number> {
    return 1;
  }
  async getAllKeys(): Promise<string[]> {
    return Array.from(this.kv.keys());
  }
  async getAllMetaKeys(): Promise<string[]> {
    return Array.from(this.meta.keys());
  }
}

const ASCII_513 = 'a'.repeat(513);
const BOUND_MESSAGE = /512 bytes/;

describe('TopGunClient and the map-name rule', () => {
  let storage: MemoryStorageAdapter;
  let client: TopGunClient;

  beforeEach(() => {
    storage = new MemoryStorageAdapter();
    // No server URL: the client never has a connection.
    client = new TopGunClient({ storage });
  });

  afterEach(async () => {
    jest.restoreAllMocks();
    await client.close();
  });

  describe('getMap and getORMap refuse a name the server would refuse, before storage is touched', () => {
    const refused: Array<[string, string, RegExp]> = [
      ['the reserved suffix', 'x__backup', /__backup/],
      ['513 bytes', ASCII_513, BOUND_MESSAGE],
      ['a NUL character', 'a\0b', /NUL/],
    ];

    it.each(refused)('getMap throws for %s', async (_label, name, message) => {
      expect(() => client.getMap(name)).toThrow(message);

      expect(await storage.getPendingOps()).toEqual([]);
      expect(await storage.getAllKeys()).toEqual([]);
    });

    it.each(refused)('getORMap throws for %s', async (_label, name, message) => {
      expect(() => client.getORMap(name)).toThrow(message);

      expect(await storage.getPendingOps()).toEqual([]);
      expect(await storage.getAllKeys()).toEqual([]);
    });
  });

  describe('query', () => {
    it('throws for the empty name', () => {
      expect(() => client.query('', {})).toThrow(/must not be empty/);
    });

    it('throws for a name ending in the reserved suffix', () => {
      expect(() => client.query('x__backup', {})).toThrow(/__backup/);
    });

    it('throws for a 513-byte name, stating the bound', () => {
      expect(() => client.query(ASCII_513, {})).toThrow(BOUND_MESSAGE);
    });

    it('does not throw for a name with ":"', () => {
      expect(() => client.query('a:b', {})).not.toThrow();
    });
  });

  describe.each([
    ['queryOnce', (name: string, timeoutMs?: number) => client.queryOnce(name, {}, { timeoutMs })],
    [
      'queryOncePaged',
      (name: string, timeoutMs?: number) => client.queryOncePaged(name, {}, { timeoutMs }),
    ],
  ])('%s', (_method, call) => {
    let subscribeSpy: jest.SpyInstance;

    beforeEach(() => {
      subscribeSpy = jest.spyOn(QueryHandle.prototype, 'subscribe');
    });

    afterEach(() => {
      subscribeSpy.mockRestore();
    });

    it('rejects for the empty name and subscribes nothing', async () => {
      await expect(call('')).rejects.toThrow(/must not be empty/);
      expect(subscribeSpy).not.toHaveBeenCalled();
    });

    it('rejects for a name ending in the reserved suffix and subscribes nothing', async () => {
      await expect(call('x__backup')).rejects.toThrow(/__backup/);
      expect(subscribeSpy).not.toHaveBeenCalled();
    });

    it('rejects for a 513-byte name, stating the bound, and subscribes nothing', async () => {
      await expect(call(ASCII_513)).rejects.toThrow(BOUND_MESSAGE);
      expect(subscribeSpy).not.toHaveBeenCalled();
    });

    it('is not rejected for the name when it holds ":" — only for having no server', async () => {
      await expect(call('a:b', 50)).rejects.toBeInstanceOf(QueryOnceUnsettledError);
    });
  });
});
