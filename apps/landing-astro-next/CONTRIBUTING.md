# Contributing to landing-astro-next

This site replaces `apps/docs-astro` and will be the public face of TopGun for
the v2.0 launch. Two ground rules keep the code snippets honest:

## 1. SDK source is the only authoritative API reference

When you write or copy a code example, the truth lives in:

- `packages/client/src/`
- `packages/core/src/`
- `packages/react/src/`
- `packages/adapters/src/`

Before pasting a snippet, grep the SDK for every method name it uses:

```bash
grep -rn "subscribe\|getMap\|onDelta" packages/{client,core,react}/src
```

If a method doesn't appear in source, it doesn't exist — rewrite using
primitives that do.

## 2. Do NOT copy from `apps/docs-astro`

`apps/docs-astro` is the legacy site. It has drifted from the SDK and contains
known-stale patterns:

- `.onChange(...)` on maps — now `.subscribe(...)`
- `.onChanges(...)` on queries — now `.onDelta(...)`
- `Predicates.gt() / .lt() / .eq()` — short aliases do not exist in SDK
- `.put(...)` on maps — never existed, use `.set(...)`
- `client.connect() / client.disconnect()` — methods are `start()` and `close()`
- `new MemoryAdapter()` — not exported, only `IDBAdapter` is public

If you find yourself reaching for `docs-astro` for inspiration, stop. Read
the SDK source instead, then write the snippet fresh.

## Canonical API snapshot (2026-05-21)

```typescript
// Construction
const client = new TopGunClient({
  storage: new IDBAdapter('my-app'),
  // serverUrl is optional — omit for local-only
});
client.start();              // wire up IndexedDB persistence

// Maps
const m = client.getMap<K, V>(name);
m.set(key, value); m.get(key); m.remove(key);
m.subscribe((entries) => render(entries));   // entries: Array<[K, V]>

// ORMap (multimap with concurrent-add semantics)
const o = client.getORMap<K, V>(name);
o.add(key, value); o.remove(key, value); o.get(key); // returns V[]
o.subscribe((entries) => ...);   // entries: Array<[K, V[]]> — V is array per key

// Queries
const q = client.query<T>(mapName, { where: { done: false } });
q.subscribe((results) => ...);   // full result set
q.onDelta((changes) => ...);     // add / update / remove deltas

// Topics (pub/sub, no persistence)
const t = client.topic(name);
t.publish(data);
t.subscribe((data, ctx) => ...);

// Counters & locks
client.getPNCounter(name).subscribe((value) => ...);
await client.getLock(name).lock(ttlMs);

// Predicates (verbose names only)
Predicates.equal('done', false)
Predicates.greaterThan('priority', 5)
Predicates.and(...) / .or(...) / .not(...)
```

If you add new examples that touch any other surface (search, sql,
distributed locks), check the SDK first.
