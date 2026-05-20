/**
 * Compile-time schema narrowing fixture for TopGunClient<TSchema>.
 *
 * This file verifies that getMap() narrows the return type when TSchema is
 * concrete, and flags unknown map names as TypeScript errors at compile time.
 * ts-jest compiles this during `pnpm --filter @topgunbuild/client test`, so
 * the @ts-expect-error directives act as the assertion mechanism — if the
 * error does NOT occur, ts-jest fails the compile step.
 */

import { TopGunClient } from '../TopGunClient';

// Minimal mock storage adapter satisfying IStorageAdapter for compile-only tests
const mockStorage = {
  initialize: async () => {},
  get: async () => undefined,
  put: async () => {},
  remove: async () => {},
  getMeta: async () => undefined,
  setMeta: async () => {},
  batchPut: async () => {},
  appendOpLog: async () => 1,
  getPendingOps: async () => [],
  markOpsSynced: async () => {},
  getAllKeys: async () => [],
  close: async () => {},
} as any;

// Schema-narrowing type fixture
type MyApp = { todos: { text: string; done: boolean } };

describe('TopGunClient schema narrowing (compile-time)', () => {
  it('compiles cleanly with the schema type argument', () => {
    const client = new TopGunClient<MyApp>({ storage: mockStorage });

    // getMap('todos') should return LWWMap<string, MyApp['todos']> — compiles fine
    const todos = client.getMap('todos');

    // @ts-expect-error — 'posts' is not a key of MyApp
    client.getMap('posts');

    // Runtime assertion: the client was constructed without throwing
    expect(client).toBeDefined();
    expect(todos).toBeDefined();
  });
});
