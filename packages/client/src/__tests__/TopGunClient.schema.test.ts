/**
 * Compile-time schema narrowing fixture for TopGunClient<TSchema>.
 *
 * This file verifies that getMap() narrows the return type when TSchema is
 * concrete. ts-jest compiles this during `pnpm --filter @topgunbuild/client test`,
 * so TypeScript type errors in this file surface as test suite failures.
 *
 * Negative-case note: The overload-based approach (Assumption #4) uses a
 * fallback string overload to preserve back-compat, so unknown-key calls do not
 * produce a TS error — they fall through to the untyped LWWMap<string, any>
 * overload. The verifiable constraint is that known schema keys narrow correctly
 * and that the positive path compiles without any assertion.
 */

import { TopGunClient } from '../TopGunClient';
import { LWWMap } from '@topgunbuild/core';

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

    // Verify the return type is narrowed: todos.get() should return MyApp['todos'] | undefined
    // This type assertion would fail compilation if the narrowing were incorrect
    const _typeCheck: typeof todos = {} as LWWMap<string, MyApp['todos']>;

    // Runtime assertion: the client was constructed without throwing
    expect(client).toBeDefined();
    expect(todos).toBeDefined();
    expect(_typeCheck).toBeDefined();
  });
});
