import { HLC } from '../HLC';
import type { Timestamp } from '../HLC';
import { IndexedORMap } from '../IndexedORMap';
import { ORMap } from '../ORMap';
import type { ORMapRecord } from '../ORMap';
import { compareTimestamps } from '../ORMapMerkle';
import { ORMapMerkleTree } from '../ORMapMerkleTree';
import { simpleAttribute } from '../query/Attribute';

/**
 * The live-tag index (tag -> keys holding a record with it) is what lets a
 * tombstone drop its record without walking the map. Remove-wins rests on it:
 * one write path that forgets to maintain it leaves a record the index cannot
 * see, and a later tombstone for that record's tag then removes nothing.
 *
 * So the index is checked the blunt way. A seeded pseudo-random sequence of
 * every mutating operation runs against the map and, in parallel, against a
 * reference model that has no index at all and finds records by scanning. After
 * EVERY step the map must equal the model and the invariants below must hold.
 *
 * The tag space is deliberately tiny and shared between keys, so the same tag
 * does end up live under two keys at once. A real tag is unique, but nothing on
 * the way in enforces it, and the index has to stay exact for such input too.
 */

type Value = string;
type Rec = ORMapRecord<Value>;

const KEYS = ['a', 'b', 'c', 'd', 'e'];
const VALUES = ['x', 'y', 'z'];

/** Old enough that `HLC.update` never sees drift; spread so a prune can cut through the middle. */
const POOL_BASE_MILLIS = 1_700_000_000_000;
const POOL_SIZE = 14;
const poolTimestamp = (i: number): Timestamp => ({
  millis: POOL_BASE_MILLIS + i,
  counter: 0,
  nodeId: 'peer',
});
const POOL_TAGS = Array.from({ length: POOL_SIZE }, (_, i) => HLC.toString(poolTimestamp(i)));

/** Deterministic PRNG (mulberry32), so a failing seed replays exactly. */
function prng(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

class Random {
  private readonly next: () => number;

  constructor(seed: number) {
    this.next = prng(seed);
  }

  int(bound: number): number {
    return Math.floor(this.next() * bound);
  }

  pick<T>(from: readonly T[]): T {
    return from[this.int(from.length)];
  }

  chance(probability: number): boolean {
    return this.next() < probability;
  }

  /** Up to `max` pool tags, possibly none, possibly with repeats. */
  tags(max: number): string[] {
    return Array.from({ length: this.int(max + 1) }, () => this.pick(POOL_TAGS));
  }

  /** A record from a peer: a pool tag, with a timestamp that varies so `mergeKey` can replace it. */
  record(): Rec {
    const i = this.int(POOL_SIZE);
    return {
      value: this.pick(VALUES),
      tag: POOL_TAGS[i],
      timestamp: { ...poolTimestamp(i), counter: this.int(3) },
    };
  }
}

/**
 * The map's semantics with no index: every lookup by tag scans all keys. This
 * is the definition the indexed map is held to, operation by operation.
 */
class ReferenceModel {
  items = new Map<string, Map<string, Rec>>();
  tombstones = new Set<string>();
  keyTombstones = new Map<string, Set<string>>();

  private keyMap(key: string): Map<string, Rec> {
    let keyMap = this.items.get(key);
    if (!keyMap) {
      keyMap = new Map();
      this.items.set(key, keyMap);
    }
    return keyMap;
  }

  private attribute(key: string, tag: string): void {
    let attributed = this.keyTombstones.get(key);
    if (!attributed) {
      attributed = new Set();
      this.keyTombstones.set(key, attributed);
    }
    attributed.add(tag);
  }

  /** Removes the record with `tag` under EVERY key. Returns how many it removed. */
  purgeEverywhere(tag: string): number {
    let purged = 0;
    for (const [key, keyMap] of this.items) {
      if (keyMap.delete(tag)) purged++;
      if (keyMap.size === 0) this.items.delete(key);
    }
    return purged;
  }

  put(key: string, record: Rec): void {
    this.keyMap(key).set(record.tag, record);
  }

  remove(key: string, value: Value): string[] {
    const keyMap = this.items.get(key);
    if (!keyMap) return [];
    const removed: string[] = [];
    for (const [tag, record] of keyMap) {
      if (record.value === value) removed.push(tag);
    }
    // Only this key's records go: a remove acts on what it observed under the key.
    for (const tag of removed) {
      this.tombstones.add(tag);
      keyMap.delete(tag);
      this.attribute(key, tag);
    }
    if (keyMap.size === 0) this.items.delete(key);
    return removed;
  }

  apply(key: string, record: Rec): boolean {
    if (this.tombstones.has(record.tag)) return false;
    this.put(key, record);
    return true;
  }

  applyTombstone(tag: string, key?: string): void {
    this.tombstones.add(tag);
    this.purgeEverywhere(tag);
    if (key !== undefined) this.attribute(key, tag);
  }

  /** Returns the number of live records it removed. */
  setKeyTombstones(key: string, tags: string[]): number {
    const next = new Set(tags);
    const previous = this.keyTombstones.get(key);
    let purged = 0;
    for (const tag of next) {
      this.tombstones.add(tag);
      // Only a tag new to the key is purged; one attributed earlier is not looked at again.
      if (!previous || !previous.has(tag)) purged += this.purgeEverywhere(tag);
    }
    if (next.size === 0) this.keyTombstones.delete(key);
    else this.keyTombstones.set(key, next);
    return purged;
  }

  /** Returns the number of live records it removed. */
  addKeyTombstones(entries: Array<[string, string[]]>): number {
    let purged = 0;
    for (const [key, tags] of entries) {
      for (const tag of tags) {
        if (this.keyTombstones.get(key)?.has(tag)) continue;
        this.attribute(key, tag);
        this.tombstones.add(tag);
        purged += this.purgeEverywhere(tag);
      }
    }
    return purged;
  }

  mergeKey(key: string, records: Rec[], tombstones: string[]): void {
    for (const tag of tombstones) this.tombstones.add(tag);
    const keyMap = this.keyMap(key);
    // Only THIS key is cleaned. A record carrying one of these tags under
    // another key stays in `items` (suppressed on read) until that key is
    // merged or the tag is tombstoned through a purging path.
    for (const tag of [...keyMap.keys()]) {
      if (this.tombstones.has(tag)) keyMap.delete(tag);
    }
    for (const record of records) {
      if (this.tombstones.has(record.tag)) continue;
      const local = keyMap.get(record.tag);
      if (!local || compareTimestamps(record.timestamp, local.timestamp) > 0) {
        keyMap.set(record.tag, record);
      }
    }
    if (keyMap.size === 0) this.items.delete(key);
  }

  merge(other: ReferenceModel): void {
    for (const tag of other.tombstones) this.tombstones.add(tag);
    for (const [key, attributed] of other.keyTombstones) {
      for (const tag of attributed) this.attribute(key, tag);
    }
    for (const [key, otherKeyMap] of other.items) {
      for (const [tag, record] of otherKeyMap) {
        if (this.tombstones.has(tag)) continue;
        const keyMap = this.keyMap(key);
        if (!keyMap.has(tag)) keyMap.set(tag, record);
      }
    }
    // A merge cleans every key, not just the ones it touched.
    for (const [key, keyMap] of this.items) {
      for (const tag of [...keyMap.keys()]) {
        if (this.tombstones.has(tag)) keyMap.delete(tag);
      }
      if (keyMap.size === 0) this.items.delete(key);
    }
  }

  prune(olderThan: Timestamp): string[] {
    const removed: string[] = [];
    for (const tag of [...this.tombstones]) {
      if (HLC.compare(HLC.parse(tag), olderThan) < 0) {
        this.tombstones.delete(tag);
        removed.push(tag);
      }
    }
    for (const [key, attributed] of this.keyTombstones) {
      for (const tag of removed) attributed.delete(tag);
      if (attributed.size === 0) this.keyTombstones.delete(key);
    }
    return removed;
  }

  clear(): void {
    this.items.clear();
    this.tombstones.clear();
    this.keyTombstones.clear();
  }
}

/** One map under test with its reference model and the notifications it sent. */
class Subject {
  readonly map: ORMap<string, Value>;
  readonly model = new ReferenceModel();
  notifications = 0;

  constructor(nodeId: string) {
    this.map = new ORMap<string, Value>(new HLC(nodeId));
    this.map.subscribe(() => this.notifications++);
  }
}

const sortedTags = (tags: Iterable<string>): string[] => [...tags].sort();

/** `items` as plain data: key -> sorted [tag, record] pairs, keys sorted. */
function itemsShape(items: Map<string, Map<string, Rec>>): Array<[string, Array<[string, Rec]>]> {
  return [...items]
    .map(([key, keyMap]): [string, Array<[string, Rec]>] => [
      key,
      [...keyMap].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)),
    ])
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
}

function attributionShape(keyTombstones: Map<string, Set<string>>): Array<[string, string[]]> {
  return [...keyTombstones]
    .map(([key, tags]): [string, string[]] => [key, sortedTags(tags)])
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
}

/** The index as it has to be, derived from `items` alone: tag -> sorted holder keys. */
function indexFromItems(items: Map<string, Map<string, Rec>>): Array<[string, string[]]> {
  const index = new Map<string, string[]>();
  for (const [key, keyMap] of items) {
    for (const tag of keyMap.keys()) {
      const holders = index.get(tag) ?? [];
      holders.push(key);
      index.set(tag, holders);
    }
  }
  return [...index]
    .map(([tag, holders]): [string, string[]] => [tag, holders.sort()])
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
}

function indexShape(index: Map<string, string[]>): Array<[string, string[]]> {
  return [...index]
    .map(([tag, holders]): [string, string[]] => [tag, [...holders].sort()])
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
}

function holdersOf(map: ORMap<string, Value>, tag: string): string[] {
  const holders: string[] = [];
  for (const [key, keyMap] of map.getSnapshot().items) {
    if (keyMap.has(tag)) holders.push(key);
  }
  return holders;
}

/** Everything that must hold after any step, whatever the step was. */
function checkInvariants(subject: Subject): void {
  const { map, model } = subject;
  const snapshot = map.getSnapshot();

  // The map equals the index-free reference: same records under the same keys,
  // same map-wide tombstones, same attribution.
  expect(itemsShape(snapshot.items)).toEqual(itemsShape(model.items));
  expect(sortedTags(snapshot.tombstones)).toEqual(sortedTags(model.tombstones));
  expect(attributionShape(snapshot.keyTombstones)).toEqual(attributionShape(model.keyTombstones));

  // No empty container is left behind: an empty record map or an empty
  // attributed set would make a key look present while it has no leaf.
  for (const keyMap of snapshot.items.values()) expect(keyMap.size).toBeGreaterThan(0);
  for (const attributed of snapshot.keyTombstones.values()) {
    expect(attributed.size).toBeGreaterThan(0);
  }

  // (a) The index is exactly the reverse of `items`: no missing holder (a
  // record a tombstone could not find) and no stale one.
  expect(indexShape(map.getLiveTagIndex())).toEqual(indexFromItems(snapshot.items));

  // (c) Attributed implies tombstoned map-wide.
  for (const [key, attributed] of snapshot.keyTombstones) {
    for (const tag of attributed) {
      expect([key, tag, snapshot.tombstones.has(tag)]).toEqual([key, tag, true]);
    }
  }

  // (d) The incrementally maintained tree equals one built from scratch.
  const rebuilt = new ORMapMerkleTree();
  rebuilt.updateFromORMap(map);
  expect(map.getMerkleTree().getRootHash()).toBe(rebuilt.getRootHash());
}

/** Runs one random operation on a subject and on its model, with the per-operation checks. */
function step(subject: Subject, peer: Subject, random: Random, log: string[]): void {
  const { map, model } = subject;
  const before = subject.notifications;
  const roll = random.int(100);

  if (roll < 14) {
    const key = random.pick(KEYS);
    const value = random.pick(VALUES);
    log.push(`add(${key}, ${value})`);
    model.put(key, map.add(key, value));
  } else if (roll < 26) {
    const key = random.pick(KEYS);
    const value = random.pick(VALUES);
    log.push(`remove(${key}, ${value})`);
    expect(sortedTags(map.remove(key, value))).toEqual(sortedTags(model.remove(key, value)));
  } else if (roll < 44) {
    const key = random.pick(KEYS);
    const record = random.record();
    log.push(`apply(${key}, ${record.tag}@${record.timestamp.counter})`);
    expect(map.apply(key, record)).toBe(model.apply(key, record));
  } else if (roll < 54) {
    const tag = random.chance(0.3)
      ? (random.pick([...map.getLiveTagIndex().keys()]) ?? random.pick(POOL_TAGS))
      : random.pick(POOL_TAGS);
    const key = random.chance(0.5) ? random.pick(KEYS) : undefined;
    log.push(`applyTombstone(${tag}, ${key})`);
    map.applyTombstone(tag, key);
    model.applyTombstone(tag, key);
    // (b) A tombstone drops its record under every key that held one.
    expect(holdersOf(map, tag)).toEqual([]);
  } else if (roll < 68) {
    const key = random.pick(KEYS);
    const tags = random.tags(4);
    log.push(`setKeyTombstones(${key}, [${tags.join(', ')}])`);
    const previous = map.getKeyTombstones(key);
    map.setKeyTombstones(key, tags);
    const purged = model.setKeyTombstones(key, tags);
    // (b) Every tag newly attributed to the key is live nowhere afterwards.
    for (const tag of tags) {
      if (!previous.has(tag)) expect(holdersOf(map, tag)).toEqual([]);
    }
    // At most one notification, and only when a live record was removed.
    expect(subject.notifications - before).toBe(purged > 0 ? 1 : 0);
  } else if (roll < 78) {
    const entries: Array<[string, string[]]> = Array.from({ length: random.int(4) }, () => [
      random.pick(KEYS),
      random.tags(3),
    ]);
    log.push(`addKeyTombstones(${JSON.stringify(entries)})`);
    const previous = new Map(KEYS.map((key) => [key, map.getKeyTombstones(key)]));
    map.addKeyTombstones(entries);
    const purged = model.addKeyTombstones(entries);
    // (b) As for setKeyTombstones, over the whole batch.
    for (const [key, tags] of entries) {
      for (const tag of tags) {
        if (!previous.get(key)?.has(tag)) expect(holdersOf(map, tag)).toEqual([]);
      }
    }
    expect(subject.notifications - before).toBe(purged > 0 ? 1 : 0);
  } else if (roll < 90) {
    const key = random.pick(KEYS);
    const records = Array.from({ length: random.int(4) }, () => random.record());
    const tombstones = random.tags(3);
    log.push(
      `mergeKey(${key}, [${records.map((r) => `${r.tag}@${r.timestamp.counter}`).join(', ')}], [${tombstones.join(', ')}])`,
    );
    map.mergeKey(key, records, tombstones);
    model.mergeKey(key, records, tombstones);
    // (b) mergeKey guarantees a clean key for the key it merged, and only for
    // that one: no tombstoned record is left under it.
    const merged = map.getSnapshot().items.get(key);
    for (const tag of merged?.keys() ?? []) expect(map.isTombstoned(tag)).toBe(false);
  } else if (roll < 95) {
    log.push('merge(peer)');
    map.merge(peer.map);
    model.merge(peer.model);
    // (b) A merge leaves no tombstoned record under any key.
    for (const keyMap of map.getSnapshot().items.values()) {
      for (const tag of keyMap.keys()) expect(map.isTombstoned(tag)).toBe(false);
    }
  } else if (roll < 99) {
    // A cut through the pool prunes some of the peer tags; a cut in the far
    // future prunes every tombstone, this map's own included.
    const olderThan = random.chance(0.2)
      ? { millis: Number.MAX_SAFE_INTEGER, counter: 0, nodeId: '' }
      : poolTimestamp(random.int(POOL_SIZE + 1));
    log.push(`prune(${HLC.toString(olderThan)})`);
    expect(sortedTags(map.prune(olderThan))).toEqual(sortedTags(model.prune(olderThan)));
  } else {
    log.push('clear()');
    map.clear();
    model.clear();
  }
}

describe('ORMap live-tag index', () => {
  const STEPS = 600;
  const SEEDS = [1, 2, 3, 7, 42, 1337, 20261001, 0xdeadbeef];

  test.each(SEEDS)(
    `stays exact through ${STEPS} random operations (seed %i)`,
    (seed) => {
      const random = new Random(seed);
      const subject = new Subject('n1');
      // A second map, mutated by the same kind of steps, serves as the merge
      // source and is held to the same checks.
      const peer = new Subject('n2');
      const log: string[] = [];
      let duplicatedTagSeen = false;

      for (let i = 0; i < STEPS; i++) {
        const peerTurn = random.chance(0.25);
        const [actor, other] = peerTurn ? [peer, subject] : [subject, peer];
        try {
          step(actor, other, random, log);
          checkInvariants(subject);
          checkInvariants(peer);
        } catch (error) {
          const recent = log
            .slice(-12)
            .map((entry) => `  ${entry}`)
            .join('\n');
          throw new Error(
            `seed ${seed}, step ${i} (${peerTurn ? 'peer' : 'subject'}) failed.\n` +
              `Last operations:\n${recent}\n\n${(error as Error).message}`,
          );
        }
        for (const holders of subject.map.getLiveTagIndex().values()) {
          if (holders.length > 1) duplicatedTagSeen = true;
        }
      }

      // The run is only worth its name if it reached the hard case: a tag live
      // under two keys at once.
      expect(duplicatedTagSeen).toBe(true);
    },
    60_000,
  );

  test('a tombstone removes a tag that is live under two keys from both', () => {
    const map = new ORMap<string, Value>(new HLC('n1'));
    const record: Rec = { value: 'x', tag: POOL_TAGS[0], timestamp: poolTimestamp(0) };
    map.apply('a', record);
    map.apply('b', record);
    map.add('b', 'kept');

    map.applyTombstone(record.tag);

    expect(holdersOf(map, record.tag)).toEqual([]);
    expect(map.allKeys()).toEqual(['b']);
    expect(map.get('b')).toEqual(['kept']);
  });

  test('removing one holder of a duplicated tag keeps the other reachable by a later tombstone', () => {
    const map = new ORMap<string, Value>(new HLC('n1'));
    const record: Rec = { value: 'x', tag: POOL_TAGS[0], timestamp: poolTimestamp(0) };
    map.apply('a', record);
    map.apply('b', record);
    map.apply('c', record);

    // Drops the record under 'a' only, through a path that is not a purge.
    map.mergeKey('a', [], [record.tag]);
    expect(holdersOf(map, record.tag)).toEqual(['b', 'c']);

    map.setKeyTombstones('k', [record.tag]);

    expect(holdersOf(map, record.tag)).toEqual([]);
    expect(map.size).toBe(0);
  });
});

describe('ORMap.addKeyTombstones', () => {
  let map: ORMap<string, Value>;

  beforeEach(() => {
    map = new ORMap(new HLC('n1'));
  });

  test('adds to what a key already has attributed instead of replacing it', () => {
    map.setKeyTombstones('a', ['t1']);

    map.addKeyTombstones([
      ['a', ['t2']],
      ['b', ['t3', 't3']],
    ]);

    expect(map.getKeyTombstones('a')).toEqual(new Set(['t1', 't2']));
    expect(map.getKeyTombstones('b')).toEqual(new Set(['t3']));
    expect(sortedTags(map.getTombstones())).toEqual(['t1', 't2', 't3']);
  });

  test('an entry with no tags leaves no trace of its key', () => {
    map.addKeyTombstones([['a', []]]);

    expect(map.getSnapshot().keyTombstones.size).toBe(0);
    expect(map.getMerkleTree().getRootHash()).toBe(0);
  });

  test('equals one setKeyTombstones call per entry on the merged set', () => {
    const build = (): ORMap<string, Value> => {
      const built = new ORMap<string, Value>(new HLC('n1'));
      built.apply('a', { value: 'x', tag: POOL_TAGS[0], timestamp: poolTimestamp(0) });
      built.apply('b', { value: 'y', tag: POOL_TAGS[1], timestamp: poolTimestamp(1) });
      built.apply('b', { value: 'z', tag: POOL_TAGS[2], timestamp: poolTimestamp(2) });
      built.setKeyTombstones('a', ['already']);
      return built;
    };
    const entries: Array<[string, string[]]> = [
      ['a', [POOL_TAGS[1], 'dead-1']],
      ['c', [POOL_TAGS[0], 'dead-2']],
      ['a', ['dead-3']],
    ];

    const bulk = build();
    bulk.addKeyTombstones(entries);

    const oneByOne = build();
    for (const [key, tags] of entries) {
      oneByOne.setKeyTombstones(key, [...oneByOne.getKeyTombstones(key), ...tags]);
    }

    expect(itemsShape(bulk.getSnapshot().items)).toEqual(itemsShape(oneByOne.getSnapshot().items));
    expect(sortedTags(bulk.getTombstones())).toEqual(sortedTags(oneByOne.getTombstones()));
    expect(attributionShape(bulk.getSnapshot().keyTombstones)).toEqual(
      attributionShape(oneByOne.getSnapshot().keyTombstones),
    );
    expect(bulk.getMerkleTree().getRootHash()).toBe(oneByOne.getMerkleTree().getRootHash());
    expect(bulk.get('b')).toEqual(['z']);
    expect(bulk.allKeys()).toEqual(['b']);
  });

  test('notifies once for a batch that removes live records under several keys', () => {
    const first = map.add('a', 'x');
    const second = map.add('b', 'y');
    let notifications = 0;
    map.subscribe(() => notifications++);

    map.addKeyTombstones([
      ['k1', [first.tag]],
      ['k2', [second.tag, 'not-live']],
    ]);

    expect(notifications).toBe(1);
    expect(map.size).toBe(0);
  });

  test('does not notify when no live record is removed', () => {
    map.add('a', 'kept');
    let notifications = 0;
    map.subscribe(() => notifications++);

    map.addKeyTombstones([['a', ['t1', 't2']]]);
    map.addKeyTombstones([['a', ['t1', 't2']]]);

    expect(notifications).toBe(0);
    expect(map.get('a')).toEqual(['kept']);
  });

  test('keeps the secondary indexes of an indexed map in step with the records it removes', () => {
    interface Product {
      category: string;
    }
    const indexed = new IndexedORMap<string, Product>(new HLC('n1'));
    indexed.addHashIndex(simpleAttribute<Product, string>('category', (p) => p.category));
    const doomed = indexed.add('p1', { category: 'tools' });
    indexed.add('p2', { category: 'tools' });

    indexed.addKeyTombstones([['elsewhere', [doomed.tag]]]);

    const found = indexed.query({ type: 'eq', attribute: 'category', value: 'tools' });
    expect(found.map((result) => result.key)).toEqual(['p2']);
    expect(indexed.count({ type: 'eq', attribute: 'category', value: 'tools' })).toBe(1);
  });
});
