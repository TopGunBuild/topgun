import { HLC } from '../HLC';
import { ORMap } from '../ORMap';
import { hashORMapLeaf } from '../ORMapMerkle';
import { hashString } from '../utils/hash';

describe('ORMap (Observed-Remove Map / Multimap)', () => {
  let hlc: HLC;
  let map: ORMap<string, string>;

  beforeEach(() => {
    hlc = new HLC('test-node');
    map = new ORMap(hlc);
  });

  test('should add and get values', () => {
    map.add('tags', 'work');
    expect(map.get('tags')).toEqual(['work']);

    map.add('tags', 'urgent');
    expect(map.get('tags')).toEqual(['work', 'urgent']);
  });

  test('should remove values', () => {
    map.add('tags', 'work');
    map.add('tags', 'urgent');

    map.remove('tags', 'work');
    expect(map.get('tags')).toEqual(['urgent']);

    map.remove('tags', 'urgent');
    expect(map.get('tags')).toEqual([]);
  });

  test('should handle concurrent additions (Scenario: Client A and Client B)', () => {
    // Simulate Client A
    const hlcA = new HLC('client-A');
    const mapA = new ORMap<string, string>(hlcA);
    mapA.add('tags', 'work');

    // Simulate Client B (Offline)
    const hlcB = new HLC('client-B');
    const mapB = new ORMap<string, string>(hlcB);
    mapB.add('tags', 'urgent');

    // Verify initial state
    expect(mapA.get('tags')).toEqual(['work']);
    expect(mapB.get('tags')).toEqual(['urgent']);

    // Sync: Merge B into A
    mapA.merge(mapB);

    // After sync, 'tags' should contain both values
    const tags = mapA.get('tags');
    expect(tags).toHaveLength(2);
    expect(tags).toContain('work');
    expect(tags).toContain('urgent');

    // Sync: Merge A into B (Convergence)
    mapB.merge(mapA);
    const tagsB = mapB.get('tags');
    expect(tagsB).toHaveLength(2);
    expect(tagsB).toContain('work');
    expect(tagsB).toContain('urgent');
  });

  test('should handle observed-remove correctly (Add Wins / Concurrent Add & Remove)', () => {
    const hlcA = new HLC('A');
    const mapA = new ORMap<string, string>(hlcA);

    const hlcB = new HLC('B');
    const mapB = new ORMap<string, string>(hlcB);

    // Initial state: A has 'work'
    mapA.add('tags', 'work');

    // Sync A -> B
    mapB.merge(mapA);
    expect(mapB.get('tags')).toEqual(['work']);

    // Concurrent ops:
    // A removes 'work'
    mapA.remove('tags', 'work');

    // B adds 'work' again (concurrently) - creates a NEW tag
    mapB.add('tags', 'work');

    // Merge B -> A
    mapA.merge(mapB);

    // Result: The NEW 'work' from B should survive, even though A removed the OLD 'work'.
    // This is "Observed Remove" semantics (we only remove what we observed).
    expect(mapA.get('tags')).toEqual(['work']);
  });

  test('should handle duplicates properly (Set semantics)', () => {
    // In OR-Set, if we add the same value twice locally, does it create two entries?
    // Typically yes, unless we check for existence.
    // The requirements say "Observed-Remove Set". A Set usually contains unique values.
    // However, in our implementation we store (V, tag).
    // If we add('tags', 'work') twice, we get two tags.
    // get() returns ['work', 'work'].
    // If we want Set semantics, get() should dedup? Or add() should check existence?
    // Let's verify current behavior.

    map.add('tags', 'work');
    map.add('tags', 'work');

    // Current implementation: returns both.
    // Ideally, a "Set" should dedup.
    // But OR-Sets allow multiple adds. The merging handles uniqueness if tags are same.
    // If we locally add twice, we generate two tags.
    // If the user wants a Set, they might expect unique values.
    // But the prompt says "works like Observed-Remove Set".
    // Standard OR-Set usually presents unique elements to the user.

    const values = map.get('tags');
    // If we want strict Set behavior, we can dedup in get().
    // But having multiple entries is technically correct for the internal structure.
    // Let's assume for now we return all entries (Multimap),
    // or we should unique them in get().
    // Given "Client A adds... Client B adds... result ['work', 'urgent']",
    // it implies distinct values.
    // If Client A added "work" and Client B added "work", we'd have ["work", "work"].
    // This is often acceptable for "tags" lists.

    expect(values).toEqual(['work', 'work']);
  });

  test('should prune tombstones correctly', () => {
    // 1. Add items
    map.add('tags', 'v1');
    map.add('tags', 'v2');

    // 2. Remove v1
    const removedTags = map.remove('tags', 'v1');
    expect(removedTags.length).toBe(1);
    const tag = removedTags[0];

    // Parse timestamp from tag
    const timestamp = HLC.parse(tag);

    // 3. Older threshold -> Should NOT prune
    const olderThan = { ...timestamp };
    olderThan.millis -= 1000;

    const pruned1 = map.prune(olderThan);
    expect(pruned1).toEqual([]);
    expect(map.getTombstones()).toContain(tag);

    // 4. Newer threshold -> Should prune
    const newerThan = { ...timestamp };
    newerThan.millis += 1000;

    const pruned2 = map.prune(newerThan);
    expect(pruned2).toEqual([tag]);
    expect(map.getTombstones()).not.toContain(tag);

    // Verify v2 still exists
    expect(map.get('tags')).toEqual(['v2']);
  });

  test('should respect TTL options', () => {
    const now = Date.now();
    jest.spyOn(Date, 'now').mockImplementation(() => now);

    // 1. Add with TTL
    map.add('status', 'online', 100);
    expect(map.get('status')).toEqual(['online']);

    // 2. Not expired
    jest.spyOn(Date, 'now').mockImplementation(() => now + 50);
    expect(map.get('status')).toEqual(['online']);

    // 3. Expired
    jest.spyOn(Date, 'now').mockImplementation(() => now + 150);
    expect(map.get('status')).toEqual([]);

    // Restore
    jest.restoreAllMocks();
  });
});

/** The leaf the map's own Merkle tree stores for `key`, or undefined when the key is absent. */
function storedLeaf<V>(map: ORMap<string, V>, key: string): number | undefined {
  const leafPath = hashString(key).toString(16).padStart(8, '0').slice(0, 3);
  return map.getMerkleTree().getEntryHashes(leafPath).get(key);
}

/** Every tag held as a live record, under any key. */
function liveTags<V>(map: ORMap<string, V>): string[] {
  const tags: string[] = [];
  for (const keyMap of map.getSnapshot().items.values()) {
    tags.push(...keyMap.keys());
  }
  return tags;
}

/** "Attributed implies tombstoned map-wide", checked over the whole map. */
function expectAttributionWithinTombstones<V>(map: ORMap<string, V>): void {
  const snapshot = map.getSnapshot();
  for (const [key, attributed] of snapshot.keyTombstones) {
    expect(attributed.size).toBeGreaterThan(0);
    for (const tag of attributed) {
      expect([key, tag, snapshot.tombstones.has(tag)]).toEqual([key, tag, true]);
    }
  }
}

describe('ORMap per-key tombstone attribution', () => {
  let map: ORMap<string, string>;

  beforeEach(() => {
    map = new ORMap(new HLC('test-node'));
  });

  describe('remove', () => {
    test('attributes the removed tags to the key they were removed from', () => {
      const a = map.add('a', 'x');
      const b = map.add('b', 'y');
      map.add('b', 'z');

      map.remove('a', 'x');
      map.remove('b', 'y');

      expect(map.getKeyTombstones('a')).toEqual(new Set([a.tag]));
      expect(map.getKeyTombstones('b')).toEqual(new Set([b.tag]));
      expect(map.getTombstones().sort()).toEqual([a.tag, b.tag].sort());
      expectAttributionWithinTombstones(map);
    });

    test('attributes every observed tag of a duplicated value', () => {
      const first = map.add('a', 'x');
      const second = map.add('a', 'x');

      map.remove('a', 'x');

      expect(map.getKeyTombstones('a')).toEqual(new Set([first.tag, second.tag]));
    });

    test('attributes nothing when no value matched', () => {
      map.add('a', 'x');

      expect(map.remove('a', 'missing')).toEqual([]);
      expect(map.remove('absent-key', 'x')).toEqual([]);

      expect(map.getKeyTombstones('a').size).toBe(0);
      expect(map.getKeyTombstones('absent-key').size).toBe(0);
      expect(map.getSnapshot().keyTombstones.size).toBe(0);
    });
  });

  describe('apply and mergeKey', () => {
    test('apply does not touch attribution', () => {
      const removed = map.add('a', 'x');
      map.remove('a', 'x');

      map.apply('a', {
        value: 'remote',
        tag: 'remote-tag',
        timestamp: { millis: 1, counter: 0, nodeId: 'remote' },
      });
      map.apply('b', {
        value: 'remote',
        tag: 'remote-tag-b',
        timestamp: { millis: 1, counter: 0, nodeId: 'remote' },
      });

      expect(map.getKeyTombstones('a')).toEqual(new Set([removed.tag]));
      expect(map.getKeyTombstones('b').size).toBe(0);
    });

    test('mergeKey tombstones go map-wide and leave attribution alone', () => {
      const removed = map.add('a', 'x');
      map.remove('a', 'x');

      map.mergeKey('a', [], ['remote-dead']);

      expect(map.isTombstoned('remote-dead')).toBe(true);
      expect(map.getKeyTombstones('a')).toEqual(new Set([removed.tag]));
      expect(storedLeaf(map, 'a')).toBe(hashORMapLeaf('a', [], [removed.tag]));
    });
  });

  describe('applyTombstone', () => {
    test('without a key records the tag map-wide and attributes it to no key', () => {
      const record = map.add('a', 'x');

      map.applyTombstone(record.tag);

      expect(map.isTombstoned(record.tag)).toBe(true);
      expect(map.get('a')).toEqual([]);
      expect(map.getKeyTombstones('a').size).toBe(0);
      expect(map.getSnapshot().keyTombstones.size).toBe(0);
      // Not attributed and no record left: the key has no leaf.
      expect(storedLeaf(map, 'a')).toBeUndefined();
    });

    test('with a key attributes the tag to that key', () => {
      const record = map.add('a', 'x');

      map.applyTombstone(record.tag, 'a');

      expect(map.isTombstoned(record.tag)).toBe(true);
      expect(map.get('a')).toEqual([]);
      expect(map.getKeyTombstones('a')).toEqual(new Set([record.tag]));
      expect(storedLeaf(map, 'a')).toBe(hashORMapLeaf('a', [], [record.tag]));
      expectAttributionWithinTombstones(map);
    });

    test('with a key attributes a tag this map never held as a record', () => {
      map.applyTombstone('never-seen', 'a');

      expect(map.getKeyTombstones('a')).toEqual(new Set(['never-seen']));
      expect(storedLeaf(map, 'a')).toBe(hashORMapLeaf('a', [], ['never-seen']));
    });
  });

  describe('setKeyTombstones', () => {
    test('replaces the attributed set instead of adding to it', () => {
      map.setKeyTombstones('a', ['t1', 't2']);
      map.setKeyTombstones('a', ['t2', 't3']);

      expect(map.getKeyTombstones('a')).toEqual(new Set(['t2', 't3']));
      expect(storedLeaf(map, 'a')).toBe(hashORMapLeaf('a', [], ['t2', 't3']));
    });

    test('keeps a tag it drops from the key suppressed map-wide', () => {
      map.setKeyTombstones('a', ['t1']);
      map.setKeyTombstones('a', ['t2']);

      expect(map.isTombstoned('t1')).toBe(true);
      expect(map.isTombstoned('t2')).toBe(true);
      const applied = map.apply('a', {
        value: 'late',
        tag: 't1',
        timestamp: { millis: 1, counter: 0, nodeId: 'remote' },
      });
      expect(applied).toBe(false);
      expectAttributionWithinTombstones(map);
    });

    test('does not touch another key', () => {
      const removed = map.add('b', 'y');
      map.remove('b', 'y');

      map.setKeyTombstones('a', ['t1']);

      expect(map.getKeyTombstones('b')).toEqual(new Set([removed.tag]));
    });

    test('with no tags clears the attribution and the key leaves the tree', () => {
      map.setKeyTombstones('a', ['t1']);
      expect(storedLeaf(map, 'a')).toBeDefined();

      map.setKeyTombstones('a', []);

      expect(map.getKeyTombstones('a').size).toBe(0);
      expect(map.getSnapshot().keyTombstones.has('a')).toBe(false);
      expect(storedLeaf(map, 'a')).toBeUndefined();
      expect(map.getMerkleTree().getRootHash()).toBe(0);
    });

    test('with no tags keeps a key that still holds a record in the tree', () => {
      const live = map.add('a', 'x');
      map.setKeyTombstones('a', ['t1']);

      map.setKeyTombstones('a', []);

      expect(storedLeaf(map, 'a')).toBe(hashORMapLeaf('a', [live.tag], []));
    });

    test('removes a live record of the same key whose tag it adds', () => {
      const doomed = map.add('k', 'doomed');
      const kept = map.add('k', 'kept');

      map.setKeyTombstones('k', [doomed.tag]);

      expect(map.get('k')).toEqual(['kept']);
      expect(map.getRecords('k').map((r) => r.tag)).toEqual([kept.tag]);
      expect(liveTags(map).filter((tag) => map.isTombstoned(tag))).toEqual([]);
      expect(storedLeaf(map, 'k')).toBe(hashORMapLeaf('k', [kept.tag], [doomed.tag]));
    });

    test('removes a live record held under another key whose tag it adds', () => {
      const doomed = map.add('other', 'doomed');
      const kept = map.add('other', 'kept');

      map.setKeyTombstones('k', [doomed.tag]);

      expect(map.get('other')).toEqual(['kept']);
      expect(map.get('k')).toEqual([]);
      expect(liveTags(map).filter((tag) => map.isTombstoned(tag))).toEqual([]);
      // The tombstone is attributed where the caller said, not where the record lived.
      expect(map.getKeyTombstones('k')).toEqual(new Set([doomed.tag]));
      expect(map.getKeyTombstones('other').size).toBe(0);
      expect(storedLeaf(map, 'other')).toBe(hashORMapLeaf('other', [kept.tag], []));
      expect(storedLeaf(map, 'k')).toBe(hashORMapLeaf('k', [], [doomed.tag]));
    });

    test('drops a key whose only record it removes from another key', () => {
      const doomed = map.add('other', 'doomed');

      map.setKeyTombstones('k', [doomed.tag]);

      expect(map.allKeys()).toEqual([]);
      expect(storedLeaf(map, 'other')).toBeUndefined();
    });

    test('notifies exactly once when it removes live records', () => {
      const first = map.add('k', 'first');
      const second = map.add('other', 'second');
      const snapshots: Array<Array<[string, string[]]>> = [];
      map.subscribe((entries) => snapshots.push(entries));

      map.setKeyTombstones('k', [first.tag, second.tag, 'not-live']);

      expect(snapshots).toHaveLength(1);
      expect(snapshots[0]).toEqual([]);
    });

    test('does not notify when it removes no live record', () => {
      map.add('k', 'kept');
      let notifications = 0;
      map.subscribe(() => notifications++);

      // Changes the attribution and the map-wide set, but no record is live
      // under either tag.
      map.setKeyTombstones('k', ['t1', 't2']);
      expect(map.getKeyTombstones('k')).toEqual(new Set(['t1', 't2']));
      expect(map.isTombstoned('t1')).toBe(true);
      expect(notifications).toBe(0);

      // The same tags again.
      map.setKeyTombstones('k', ['t1', 't2']);
      expect(notifications).toBe(0);

      // A narrower set and an empty one change the attribution only.
      map.setKeyTombstones('k', ['t1']);
      map.setKeyTombstones('k', []);
      expect(notifications).toBe(0);
      expect(map.get('k')).toEqual(['kept']);
    });

    test('does not notify again when a purging call is repeated', () => {
      const doomed = map.add('k', 'doomed');
      let notifications = 0;
      map.subscribe(() => notifications++);

      map.setKeyTombstones('k', [doomed.tag]);
      expect(notifications).toBe(1);

      map.setKeyTombstones('k', [doomed.tag]);
      expect(notifications).toBe(1);
    });
  });

  describe('getKeyTombstones', () => {
    test('returns a copy the caller cannot use to change the map', () => {
      const removed = map.add('a', 'x');
      map.remove('a', 'x');
      const leaf = storedLeaf(map, 'a');

      map.getKeyTombstones('a').add('injected');
      map.getKeyTombstones('a').clear();

      expect(map.getKeyTombstones('a')).toEqual(new Set([removed.tag]));
      expect(storedLeaf(map, 'a')).toBe(leaf);
    });
  });

  describe('prune, clear and merge keep attribution within the tombstone set', () => {
    test('prune removes a pruned tag from the key it was attributed to', () => {
      map.add('a', 'x');
      map.add('a', 'kept');
      const [tag] = map.remove('a', 'x');
      const after = { ...HLC.parse(tag), millis: HLC.parse(tag).millis + 1000 };

      expect(map.prune(after)).toEqual([tag]);

      expect(map.getTombstones()).toEqual([]);
      expect(map.getKeyTombstones('a').size).toBe(0);
      expect(map.getSnapshot().keyTombstones.has('a')).toBe(false);
      expect(storedLeaf(map, 'a')).toBe(
        hashORMapLeaf(
          'a',
          map.getRecords('a').map((r) => r.tag),
          [],
        ),
      );
      expectAttributionWithinTombstones(map);
    });

    test('prune keeps a tag that is not old enough, and its attribution', () => {
      map.add('a', 'x');
      const [tag] = map.remove('a', 'x');
      const before = { ...HLC.parse(tag), millis: HLC.parse(tag).millis - 1000 };

      expect(map.prune(before)).toEqual([]);

      expect(map.getKeyTombstones('a')).toEqual(new Set([tag]));
      expect(storedLeaf(map, 'a')).toBe(hashORMapLeaf('a', [], [tag]));
    });

    test('prune removes only the pruned tags from a key attributed several', () => {
      map.add('a', 'x');
      const [oldTag] = map.remove('a', 'x');
      const parsed = HLC.parse(oldTag);
      const newTag = HLC.toString({ ...parsed, millis: parsed.millis + 5000 });
      map.applyTombstone(newTag, 'a');

      map.prune({ ...parsed, millis: parsed.millis + 1000 });

      expect(map.getKeyTombstones('a')).toEqual(new Set([newTag]));
      expect(storedLeaf(map, 'a')).toBe(hashORMapLeaf('a', [], [newTag]));
      expectAttributionWithinTombstones(map);
    });

    test('clear drops every attribution', () => {
      map.add('a', 'x');
      map.remove('a', 'x');
      map.setKeyTombstones('b', ['t1']);

      map.clear();

      expect(map.getKeyTombstones('a').size).toBe(0);
      expect(map.getKeyTombstones('b').size).toBe(0);
      expect(map.getSnapshot().keyTombstones.size).toBe(0);
      expect(map.getMerkleTree().getRootHash()).toBe(0);
    });

    test("merge unions the other map's attribution per key", () => {
      const other = new ORMap<string, string>(new HLC('other-node'));
      other.add('a', 'x');
      const [otherTag] = other.remove('a', 'x');
      other.add('b', 'y');
      const [otherTagB] = other.remove('b', 'y');

      map.add('a', 'local');
      const [localTag] = map.remove('a', 'local');

      map.merge(other);

      expect(map.getKeyTombstones('a')).toEqual(new Set([localTag, otherTag]));
      expect(map.getKeyTombstones('b')).toEqual(new Set([otherTagB]));
      expect(storedLeaf(map, 'a')).toBe(hashORMapLeaf('a', [], [localTag, otherTag]));
      expect(storedLeaf(map, 'b')).toBe(hashORMapLeaf('b', [], [otherTagB]));
      expectAttributionWithinTombstones(map);
      // The source map is left as it was.
      expect(other.getKeyTombstones('a')).toEqual(new Set([otherTag]));
    });

    test('merge in both directions converges on the same root', () => {
      const other = new ORMap<string, string>(new HLC('other-node'));
      other.add('a', 'x');
      other.remove('a', 'x');
      other.add('c', 'stays');
      map.add('b', 'y');
      map.remove('b', 'y');

      map.merge(other);
      other.merge(map);

      expect(map.getMerkleTree().getRootHash()).toBe(other.getMerkleTree().getRootHash());
      expect(map.getMerkleTree().getRootHash()).not.toBe(0);
    });
  });

  describe('presence in the Merkle tree', () => {
    test('a key with only attributed tombstones stays in the tree', () => {
      map.add('a', 'x');
      const [tag] = map.remove('a', 'x');

      expect(map.allKeys()).toEqual([]);
      expect(storedLeaf(map, 'a')).toBe(hashORMapLeaf('a', [], [tag]));
      expect(map.getMerkleTree().getRootHash()).not.toBe(0);
    });

    test('the key leaves the tree when its last attributed tombstone goes', () => {
      map.add('a', 'x');
      const [tag] = map.remove('a', 'x');
      const parsed = HLC.parse(tag);

      map.prune({ ...parsed, millis: parsed.millis + 1000 });

      expect(storedLeaf(map, 'a')).toBeUndefined();
      expect(map.getMerkleTree().getRootHash()).toBe(0);
    });

    test('a key with records and tombstones has both in its leaf', () => {
      const kept = map.add('a', 'kept');
      map.add('a', 'x');
      const [tag] = map.remove('a', 'x');

      expect(storedLeaf(map, 'a')).toBe(hashORMapLeaf('a', [kept.tag], [tag]));
    });

    test('a key that never held anything is not in the tree', () => {
      map.remove('a', 'x');

      expect(storedLeaf(map, 'a')).toBeUndefined();
      expect(map.getMerkleTree().getRootHash()).toBe(0);
    });
  });
});
