# @topgunbuild/core

[![npm](https://img.shields.io/npm/v/@topgunbuild/core)](https://www.npmjs.com/package/@topgunbuild/core) [![License](https://img.shields.io/npm/l/@topgunbuild/core)](https://github.com/TopGunBuild/topgun/blob/main/LICENSE)

Low-level CRDT primitives, Hybrid Logical Clocks, Merkle trees, and query predicates that power [TopGun](https://topgun.build).

> **Most apps should depend on [`@topgunbuild/client`](https://www.npmjs.com/package/@topgunbuild/client) (or [`@topgunbuild/react`](https://www.npmjs.com/package/@topgunbuild/react)) instead.** This package is exported for advanced users who want to embed CRDTs directly, write custom transports, or build server-side tooling.

## Install

```bash
npm install @topgunbuild/core
```

## What's in here

- **`LWWMap<K, V>`** — Last-Write-Wins Map. Single value per key, conflict resolution by HLC timestamp.
- **`ORMap<K, V>`** — Observed-Remove Map (multimap). Concurrent additions resolve without data loss.
- **`HLC`** — Hybrid Logical Clock. Globally orderable timestamps under clock drift.
- **`MerkleTree` / `ORMapMerkleTree`** — Efficient delta-sync hashing.
- **`Predicates`** — Composable query filters used by `client.query(...)`.
- **`PNCounterImpl`** — Distributed counter (positive/negative increment).
- **Search internals** — `FullTextIndex`, `BM25Scorer`, `Tokenizer`, etc.
- **Deterministic testing** — `VirtualClock`, `ScenarioRunner`, `InvariantChecker`.

## Quickstart

### Standalone LWWMap

```typescript
import { HLC, LWWMap } from '@topgunbuild/core';

const hlc = new HLC({ nodeId: 'node-1' });
const map = new LWWMap<string, { text: string }>(hlc);

map.subscribe((entries) => {
  console.log('Map changed:', entries);  // entries: Array<[string, { text: string }]>
});

map.set('todo-1', { text: 'Buy milk' });
map.remove('todo-1');
```

### Predicates

```typescript
import { Predicates } from '@topgunbuild/core';

const filter = Predicates.and(
  Predicates.equal('status', 'open'),
  Predicates.greaterThan('priority', 5),
  Predicates.or(
    Predicates.contains('tags', 'urgent'),
    Predicates.match('title', 'critical'),
  ),
);
```

Available operators (verbose names only):

- Comparison: `equal`, `notEqual`, `greaterThan`, `greaterThanOrEqual`, `lessThan`, `lessThanOrEqual`, `between`
- Membership: `isIn`, `isNull`, `isNotNull`
- Text: `like`, `regex`, `contains`, `containsAll`, `containsAny`
- Full-text search: `match`, `matchPhrase`, `matchPrefix`
- Logical: `and`, `or`, `not`

### Hybrid Logical Clock

```typescript
import { HLC } from '@topgunbuild/core';

const hlc = new HLC({ nodeId: 'node-1' });
const ts1 = hlc.now();
const ts2 = hlc.now();
// ts2 is guaranteed to sort after ts1, even under wall-clock skew.
```

## Documentation

- Full docs: [topgun.build/docs](https://topgun.build/docs)
- Architecture: [specifications/02_DATA_STRUCTURES_CRDT.md](https://github.com/TopGunBuild/topgun/blob/main/specifications/02_DATA_STRUCTURES_CRDT.md)
- GitHub: [TopGunBuild/topgun](https://github.com/TopGunBuild/topgun)

## License

Apache-2.0
