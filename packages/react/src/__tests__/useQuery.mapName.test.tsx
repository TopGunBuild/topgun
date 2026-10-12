/** @jest-environment-options {"customExportConditions": ["node", "node-addons"]} */
import { renderHook } from '@testing-library/react';
import { randomUUID } from 'crypto';
import React from 'react';
import { TextEncoder as NodeTextEncoder } from 'util';
import { QueryHandle, TopGunClient } from '@topgunbuild/client';
import type { IStorageAdapter, OpLogEntry } from '@topgunbuild/client';
import { useQuery } from '../hooks/useQuery';
import { TopGunProvider } from '../TopGunProvider';

/**
 * What `useQuery` returns for a map name the server will never serve.
 *
 * The hook is rendered over a real client, not a mocked `client.query`, because
 * the name rule lives in the client: a mock would answer for a rule it does not
 * have. The client is never connected — the refusal has to come from the name
 * alone, before anything is sent.
 */

// The browser-like test environment lacks two globals the client relies on.
// They are supplied here, for this file only, so a missing one shows up as the
// named probe case below instead of as an error thrown from inside the client.
if (typeof globalThis.TextEncoder === 'undefined') {
  (globalThis as any).TextEncoder = NodeTextEncoder;
}
if (typeof (globalThis as any).crypto === 'undefined') {
  (globalThis as any).crypto = {};
}
if (typeof (globalThis as any).crypto.randomUUID !== 'function') {
  Object.defineProperty((globalThis as any).crypto, 'randomUUID', {
    value: randomUUID,
    configurable: true,
  });
}

/** The least storage a client needs to be constructed and to run a local query. */
class EmptyStorage implements IStorageAdapter {
  private kv = new Map<string, unknown>();
  private meta = new Map<string, unknown>();
  private pending: OpLogEntry[] = [];

  async initialize(): Promise<void> {}
  async close(): Promise<void> {}
  async get(key: string): Promise<any> {
    return this.kv.get(key);
  }
  async put(key: string, value: unknown): Promise<void> {
    this.kv.set(key, value);
  }
  async remove(key: string): Promise<void> {
    this.kv.delete(key);
  }
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

describe('useQuery and the map-name rule', () => {
  let client: TopGunClient;
  let subscribeSpy: jest.SpyInstance;
  let unmount: (() => void) | undefined;

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <TopGunProvider client={client}>{children}</TopGunProvider>
  );

  function render(mapName: string) {
    const rendered = renderHook(() => useQuery(mapName), { wrapper });
    unmount = rendered.unmount;
    return rendered.result;
  }

  beforeEach(() => {
    client = new TopGunClient({ storage: new EmptyStorage() });
    subscribeSpy = jest.spyOn(QueryHandle.prototype, 'subscribe');
  });

  afterEach(async () => {
    unmount?.();
    unmount = undefined;
    subscribeSpy.mockRestore();
    await client.close();
  });

  it('probe: the globals the client needs are present in this environment', () => {
    expect(typeof globalThis.TextEncoder).toBe('function');
    expect(typeof globalThis.crypto.randomUUID).toBe('function');
  });

  describe.each([
    ['', /empty/i],
    ['x__backup', /__backup/],
  ])('a name the server refuses: %p', (mapName, message) => {
    it('renders without throwing', () => {
      expect(() => render(mapName)).not.toThrow();
    });

    it('returns the refusal as `error`', () => {
      const result = render(mapName);

      expect(result.current.error).toBeInstanceOf(Error);
      expect(result.current.error?.message).toMatch(message);
    });

    it('is not loading', () => {
      const result = render(mapName);

      expect(result.current.loading).toBe(false);
    });

    it('returns empty data', () => {
      const result = render(mapName);

      expect(result.current.data).toEqual([]);
    });

    it('does not subscribe', () => {
      render(mapName);

      expect(subscribeSpy).not.toHaveBeenCalled();
    });
  });

  describe('control: a name with ":" is not refused for a query', () => {
    it('returns no error', () => {
      const result = render('a:b');

      expect(result.current.error).toBeNull();
    });

    it('subscribes once', () => {
      render('a:b');

      expect(subscribeSpy).toHaveBeenCalledTimes(1);
    });
  });
});
