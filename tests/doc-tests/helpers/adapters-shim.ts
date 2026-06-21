/**
 * Execution-time stand-in for `@topgunbuild/adapters`.
 *
 * The run tier executes documentation snippets in Node, where IndexedDB does
 * not exist. The doc-tests jest config maps `@topgunbuild/adapters` to this
 * module so a snippet's verbatim `import { IDBAdapter } from
 * '@topgunbuild/adapters'` resolves to an in-memory adapter at runtime. The
 * TYPECHECK tier is unaffected — it resolves the real package types (so e.g.
 * `new IDBAdapter('name')` is still flagged against the real 0-arg constructor).
 */
export { MemoryStorageAdapter as IDBAdapter } from './memory-adapter';
