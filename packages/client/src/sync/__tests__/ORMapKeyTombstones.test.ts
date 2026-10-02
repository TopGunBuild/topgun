import { HLC, ORMap, hashORMapLeaf } from '@topgunbuild/core';
import type { ORMapRecord } from '@topgunbuild/core';
import { ORMapSyncHandler } from '../ORMapSyncHandler';
import { SyncEngine } from '../../SyncEngine';
import type { OpLogEntry } from '../../SyncEngine';
import { NullConnectionProvider } from '../../connection/NullConnectionProvider';
import type { IStorageAdapter } from '../../IStorageAdapter';
import {
  orMapKeyTombstonesBucketKey,
  orMapKeyTombstonesBucketOf,
} from '../../utils/orMapKeyTombstones';

/**
 * Per-key tombstone scoping in the OR-Map sync handler.
 *
 * The server stores tombstones per key and unions whatever a push carries into
 * that key's set. A push for one key must therefore carry only the tombstones
 * that belong to it: anything else makes the server's per-key set depend on push
 * history, which no client can reproduce, so the two sides can never agree on
 * that key's leaf (TG-MRK-001).
 *
 * The same contract runs the other way: the client's attributed set for a key
 * is the server's set for that key plus the tags of this client's own removes
 * the server has not acknowledged yet, and nothing else.
 */

interface PushDiffEntry {
  key: string;
  records: unknown[];
  tombstones: string[];
}

const MAP_NAME = 'tags';

function makeHarness() {
  const map = new ORMap<string, string>(new HLC('n1'));
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- captured sent messages have no fixed shape at the test boundary
  const sent: any[] = [];
  /** Un-acknowledged local remove tags per key, as the engine would derive them. */
  const pending = new Map<string, string[]>();
  // Hands out a copy, as the engine builds a fresh grouping on every call: a
  // handler that kept the result of an earlier call would then be working from
  // a stale one, and the tests can tell.
  const getPendingRemoveTagsByKey = jest.fn<Map<string, string[]>, [string]>(
    () => new Map(pending),
  );
  const persistKeyTombstones = jest
    .fn<Promise<void>, [string, Iterable<string>]>()
    .mockResolvedValue(undefined);
  const persistTombstones = jest.fn<Promise<void>, [string]>().mockResolvedValue(undefined);
  const persistKey = jest.fn<Promise<void>, [string, string]>().mockResolvedValue(undefined);
  const onFullResync = jest.fn<Promise<void>, [string, unknown]>().mockResolvedValue(undefined);
  const confirmedEpochs: number[] = [];
  const handler = new ORMapSyncHandler({
    getMap: () => map,
    sendMessage: (msg) => {
      sent.push(msg);
      return true;
    },
    hlc: new HLC('n1'),
    onTimestampUpdate: async () => {},
    persistKey,
    persistTombstones,
    getPendingRemoveTagsByKey,
    persistKeyTombstones,
    onCoveringEpochApplied: (_mapName, epoch) => {
      confirmedEpochs.push(epoch);
    },
    onFullResync,
    getClaimedEpoch: () => 0,
  });

  /** The entries pushed for `key`, across every ORMAP_PUSH_DIFF sent so far. */
  const pushedEntriesFor = (key: string): PushDiffEntry[] =>
    sent
      .filter((msg) => msg.type === 'ORMAP_PUSH_DIFF')
      .flatMap((msg) => msg.payload.entries as PushDiffEntry[])
      .filter((entry) => entry.key === key);

  /** The paths of every ORMAP_MERKLE_REQ_BUCKET sent so far. */
  const requestedPaths = (): string[] =>
    sent.filter((msg) => msg.type === 'ORMAP_MERKLE_REQ_BUCKET').map((msg) => msg.payload.path);

  return {
    map,
    handler,
    sent,
    pending,
    getPendingRemoveTagsByKey,
    persistKeyTombstones,
    persistTombstones,
    persistKey,
    onFullResync,
    confirmedEpochs,
    pushedEntriesFor,
    requestedPaths,
  };
}

/** The trie path of the leaf bucket holding `key`. The key must be in the tree. */
function leafPathOf(map: ORMap<string, string>, key: string): string {
  const tree = map.getMerkleTree();
  const search = (path: string): string | undefined => {
    if (tree.getKeysInBucket(path).includes(key)) return path;
    for (const child of Object.keys(tree.getBuckets(path))) {
      const found = search(path + child);
      if (found !== undefined) return found;
    }
    return undefined;
  };
  const path = search('');
  if (path === undefined) throw new Error(`key ${key} is not in the tree`);
  return path;
}

/** Whether `key` currently has a leaf in the map's Merkle tree. */
function isInTree(map: ORMap<string, string>, key: string): boolean {
  const tree = map.getMerkleTree();
  const search = (path: string): boolean =>
    tree.getKeysInBucket(path).includes(key) ||
    Object.keys(tree.getBuckets(path)).some((child) => search(path + child));
  return search('');
}

/** Adds `value` under `key` and removes it again; returns the tombstoned tag. */
function addThenRemove(map: ORMap<string, string>, key: string, value: string): string {
  map.add(key, value);
  const [tag] = map.remove(key, value);
  return tag;
}

function sorted(tags: Iterable<string>): string[] {
  return Array.from(tags).sort();
}

describe('ORMapSyncHandler per-key tombstone scoping', () => {
  describe('pushORMapDiff', () => {
    /**
     * Key A is emptied by a remove; key B keeps one live value and has one
     * removed. Only B is pushed.
     */
    async function pushKeyBAfterRemovesOnBothKeys() {
      const harness = makeHarness();
      const { map, handler } = harness;

      map.add('A', 'a1');
      const [tagRemovedFromA] = map.remove('A', 'a1');
      map.add('B', 'b1');
      map.add('B', 'b2');
      const [tagRemovedFromB] = map.remove('B', 'b1');

      await handler.pushORMapDiff(MAP_NAME, ['B'], map);

      const entries = harness.pushedEntriesFor('B');
      expect(entries).toHaveLength(1);
      return { entry: entries[0], tagRemovedFromA, tagRemovedFromB };
    }

    test('the entry for key B carries no tag that was removed from key A', async () => {
      const { entry, tagRemovedFromA } = await pushKeyBAfterRemovesOnBothKeys();

      expect(tagRemovedFromA).toEqual(expect.any(String));
      expect(entry.tombstones).not.toContain(tagRemovedFromA);
    });

    test('the entry for key B carries the tag that was removed from key B', async () => {
      const { entry, tagRemovedFromB } = await pushKeyBAfterRemovesOnBothKeys();

      expect(tagRemovedFromB).toEqual(expect.any(String));
      expect(entry.tombstones).toContain(tagRemovedFromB);
    });

    test('the entry carries exactly the tombstones attributed to the key, not the map-wide set', async () => {
      const { map, handler, pushedEntriesFor } = makeHarness();

      map.add('B', 'b1');
      map.add('B', 'b2');
      const [attributed] = map.remove('B', 'b1');
      // A tombstone the map suppresses but that belongs to no key.
      map.applyTombstone('unattributed-tag');
      expect(map.getTombstones()).toContain('unattributed-tag');

      await handler.pushORMapDiff(MAP_NAME, ['B'], map);

      const [entry] = pushedEntriesFor('B');
      expect(sorted(entry.tombstones)).toEqual([attributed]);
    });

    test('a key that holds only tombstones is still not pushed', async () => {
      const { map, handler, sent } = makeHarness();

      const tag = addThenRemove(map, 'A', 'a1');
      expect(sorted(map.getKeyTombstones('A'))).toEqual([tag]);

      await handler.pushORMapDiff(MAP_NAME, ['A'], map);

      expect(sent).toEqual([]);
    });
  });

  describe('a leaf or diff entry replaces the key attribution with the server set', () => {
    const responses = [
      [
        'ORMAP_SYNC_RESP_LEAF',
        (h: ReturnType<typeof makeHarness>, tombstones: string[]) =>
          h.handler.handleORMapSyncRespLeaf({
            mapName: MAP_NAME,
            path: leafPathOf(h.map, 'K'),
            entries: [{ key: 'K', records: [], tombstones }],
          }),
      ],
      [
        'ORMAP_DIFF_RESPONSE',
        (h: ReturnType<typeof makeHarness>, tombstones: string[]) =>
          h.handler.handleORMapDiffResponse({
            mapName: MAP_NAME,
            entries: [{ key: 'K', records: [], tombstones }],
          }),
      ],
    ] as const;

    test.each(responses)(
      '%s: a tag the server no longer attributes leaves the key, and stays suppressed map-wide',
      async (_name, respond) => {
        const harness = makeHarness();
        const { map } = harness;
        const stale = addThenRemove(map, 'K', 'v');

        await respond(harness, ['server-tag']);

        expect(sorted(map.getKeyTombstones('K'))).toEqual(['server-tag']);
        expect(map.getTombstones()).toEqual(expect.arrayContaining([stale, 'server-tag']));
      },
    );

    test.each(responses)(
      '%s: the tag of an un-acknowledged local remove is kept although the server set lacks it, and dropped once it is acknowledged',
      async (_name, respond) => {
        const harness = makeHarness();
        const { map, pending } = harness;
        const removed = addThenRemove(map, 'K', 'v');
        pending.set('K', [removed]);

        await respond(harness, ['server-tag']);
        expect(sorted(map.getKeyTombstones('K'))).toEqual(sorted(['server-tag', removed]));

        // The remove is acknowledged: it no longer counts as pending, and the
        // server set (which still lacks it in this response) is authoritative.
        pending.delete('K');
        await respond(harness, ['server-tag']);
        expect(sorted(map.getKeyTombstones('K'))).toEqual(['server-tag']);
        expect(map.getTombstones()).toContain(removed);
      },
    );

    test('attribution is applied before the merge, so a record whose tag is being attributed is never rendered', async () => {
      const harness = makeHarness();
      const { map, handler, pending } = harness;
      // The map-wide set does not hold the tag: only the pending remove names it.
      pending.set('K', ['removed-tag']);
      expect(map.getTombstones()).not.toContain('removed-tag');

      const rendered: Array<Array<[string, string[]]>> = [];
      map.subscribe((entries) => rendered.push(entries));

      const record: ORMapRecord<string> = {
        value: 'removed-value',
        tag: 'removed-tag',
        timestamp: new HLC('server').now(),
      };
      await handler.handleORMapDiffResponse({
        mapName: MAP_NAME,
        entries: [{ key: 'K', records: [record], tombstones: [] }],
      });

      const everRendered = rendered.flat().some(([, values]) => values.includes('removed-value'));
      expect(everRendered).toBe(false);
      expect(map.get('K')).not.toContain('removed-value');
      expect(map.getTombstones()).toContain('removed-tag');
    });
  });

  describe('server absence clears the attribution down to the pending removes', () => {
    /**
     * Each trigger is a response that shows the server holds no leaf for key K.
     * K holds no records, so it is in the tree only through its attribution.
     */
    const triggers = [
      [
        'a leaf response that omits the key',
        (h: ReturnType<typeof makeHarness>) =>
          h.handler.handleORMapSyncRespLeaf({
            mapName: MAP_NAME,
            path: leafPathOf(h.map, 'K'),
            entries: [],
          }),
      ],
      [
        'a bucket response that does not list the key child',
        (h: ReturnType<typeof makeHarness>) =>
          h.handler.handleORMapSyncRespBuckets({ mapName: MAP_NAME, path: '', buckets: {} }),
      ],
      [
        'a zero root that is not a full resync',
        (h: ReturnType<typeof makeHarness>) =>
          h.handler.handleORMapSyncRespRoot({ mapName: MAP_NAME, rootHash: 0 }),
      ],
    ] as const;

    test.each(triggers)(
      '%s: with no pending remove the key loses its attribution and leaves the tree',
      async (_name, trigger) => {
        const harness = makeHarness();
        const { map } = harness;
        const tag = addThenRemove(map, 'K', 'v');
        expect(map.allKeys()).not.toContain('K');
        expect(sorted(map.getKeyTombstones('K'))).toEqual([tag]);
        expect(isInTree(map, 'K')).toBe(true);

        await trigger(harness);

        expect(map.getKeyTombstones('K').size).toBe(0);
        expect(isInTree(map, 'K')).toBe(false);
        expect(map.getMerkleTree().getRootHash()).toBe(0);
        // Absence clears attribution only: the tag stays suppressed.
        expect(map.getTombstones()).toContain(tag);
      },
    );

    test.each(triggers)(
      '%s: with a pending remove the key keeps exactly the pending tags',
      async (_name, trigger) => {
        const harness = makeHarness();
        const { map, pending } = harness;
        const acked = addThenRemove(map, 'K', 'v1');
        const unacked = addThenRemove(map, 'K', 'v2');
        pending.set('K', [unacked]);
        expect(sorted(map.getKeyTombstones('K'))).toEqual(sorted([acked, unacked]));

        await trigger(harness);

        expect(sorted(map.getKeyTombstones('K'))).toEqual([unacked]);
        expect(isInTree(map, 'K')).toBe(true);
        expect(map.getMerkleTree().getEntryHashes(leafPathOf(map, 'K')).get('K')).toBe(
          hashORMapLeaf('K', [], [unacked]),
        );
        expect(map.getTombstones()).toEqual(expect.arrayContaining([acked, unacked]));
      },
    );

    test('a leaf response for another path leaves the key attribution alone', async () => {
      const harness = makeHarness();
      const { map, handler, persistKeyTombstones } = harness;
      const tag = addThenRemove(map, 'K', 'v');
      const ownPath = leafPathOf(map, 'K');
      const otherPath = ownPath.slice(0, -1) + (ownPath.endsWith('0') ? '1' : '0');

      await handler.handleORMapSyncRespLeaf({ mapName: MAP_NAME, path: otherPath, entries: [] });

      expect(sorted(map.getKeyTombstones('K'))).toEqual([tag]);
      expect(persistKeyTombstones).not.toHaveBeenCalled();
    });

    test('a leaf response that lists the key does not reset it', async () => {
      const harness = makeHarness();
      const { map, handler } = harness;
      const tag = addThenRemove(map, 'K', 'v');

      await handler.handleORMapSyncRespLeaf({
        mapName: MAP_NAME,
        path: leafPathOf(map, 'K'),
        entries: [{ key: 'K', records: [], tombstones: [tag] }],
      });

      expect(sorted(map.getKeyTombstones('K'))).toEqual([tag]);
    });

    test('a zero root that IS a full resync is left to the replace path', async () => {
      const harness = makeHarness();
      const { map, handler, onFullResync, persistKeyTombstones } = harness;
      const tag = addThenRemove(map, 'K', 'v');

      await handler.handleORMapSyncRespRoot({ mapName: MAP_NAME, rootHash: 0, fullResync: true });

      expect(onFullResync).toHaveBeenCalledTimes(1);
      // The replace callback is a stub here, so nothing else may have touched
      // the attribution.
      expect(sorted(map.getKeyTombstones('K'))).toEqual([tag]);
      expect(persistKeyTombstones).not.toHaveBeenCalled();
    });
  });

  describe('bucket responses treat a child listed with hash 0 as absent', () => {
    /**
     * Key K holds one live value and one attributed tombstone. The response is
     * for the node directly above K's leaf bucket, so the child is that bucket.
     */
    function keyWithLiveValueAndTombstone() {
      const harness = makeHarness();
      const { map } = harness;
      map.add('K', 'live');
      const tag = addThenRemove(map, 'K', 'gone');
      const leafPath = leafPathOf(map, 'K');
      return {
        harness,
        tag,
        parentPath: leafPath.slice(0, -1),
        child: leafPath.slice(-1),
      };
    }

    test('a zero-hash child with no local bucket is not requested', async () => {
      const { handler, requestedPaths, sent } = makeHarness();

      await handler.handleORMapSyncRespBuckets({
        mapName: MAP_NAME,
        path: '',
        buckets: { a: 0, b: 0 },
      });

      expect(requestedPaths()).toEqual([]);
      expect(sent).toEqual([]);
    });

    test('a zero-hash child with a non-zero local bucket is not requested; its keys are reset and then pushed', async () => {
      const { harness, tag, parentPath, child } = keyWithLiveValueAndTombstone();
      const { map, handler, requestedPaths, pushedEntriesFor } = harness;
      expect(map.getMerkleTree().getBuckets(parentPath)[child]).not.toBe(0);

      await handler.handleORMapSyncRespBuckets({
        mapName: MAP_NAME,
        path: parentPath,
        buckets: { [child]: 0 },
      });

      expect(requestedPaths()).toEqual([]);
      expect(map.getKeyTombstones('K').size).toBe(0);
      expect(map.getTombstones()).toContain(tag);
      // The push follows the reset, so it carries the cleared attribution.
      const entries = pushedEntriesFor('K');
      expect(entries).toHaveLength(1);
      expect(entries[0].records).toHaveLength(1);
      expect(entries[0].tombstones).toEqual([]);
    });

    test('an unlisted child with a non-zero local bucket takes the same reset and push', async () => {
      const { harness, parentPath } = keyWithLiveValueAndTombstone();
      const { map, handler, requestedPaths, pushedEntriesFor } = harness;

      await handler.handleORMapSyncRespBuckets({
        mapName: MAP_NAME,
        path: parentPath,
        buckets: {},
      });

      expect(requestedPaths()).toEqual([]);
      expect(map.getKeyTombstones('K').size).toBe(0);
      expect(pushedEntriesFor('K')).toHaveLength(1);
    });

    test('a child listed with a different non-zero hash is requested and not reset', async () => {
      const { harness, tag, parentPath, child } = keyWithLiveValueAndTombstone();
      const { map, handler, requestedPaths, pushedEntriesFor, persistKeyTombstones } = harness;
      const localHash = map.getMerkleTree().getBuckets(parentPath)[child];

      await handler.handleORMapSyncRespBuckets({
        mapName: MAP_NAME,
        path: parentPath,
        buckets: { [child]: localHash + 1 },
      });

      expect(requestedPaths()).toEqual([parentPath + child]);
      expect(sorted(map.getKeyTombstones('K'))).toEqual([tag]);
      expect(pushedEntriesFor('K')).toEqual([]);
      expect(persistKeyTombstones).not.toHaveBeenCalled();
    });
  });

  describe('attribution is persisted whenever it changed, and only then', () => {
    test('a replace that adds and updates no record still persists the attribution', async () => {
      const { map, handler, persistKeyTombstones, persistTombstones } = makeHarness();
      const record = map.add('K', 'live');

      await handler.handleORMapSyncRespLeaf({
        mapName: MAP_NAME,
        path: leafPathOf(map, 'K'),
        entries: [{ key: 'K', records: [record], tombstones: ['server-tag'] }],
      });

      expect(persistKeyTombstones).toHaveBeenCalledTimes(1);
      expect(persistKeyTombstones).toHaveBeenCalledWith(MAP_NAME, ['K']);
      // Nothing was added or updated, so the map-wide tombstone persist, which
      // is tied to record changes, did not run.
      expect(persistTombstones).not.toHaveBeenCalled();
    });

    test('a diff replace that adds and updates no record still persists the attribution', async () => {
      const { map, handler, persistKeyTombstones } = makeHarness();
      const record = map.add('K', 'live');

      await handler.handleORMapDiffResponse({
        mapName: MAP_NAME,
        entries: [{ key: 'K', records: [record], tombstones: ['server-tag'] }],
      });

      expect(persistKeyTombstones).toHaveBeenCalledTimes(1);
    });

    test.each([
      [
        'leaf',
        (h: ReturnType<typeof makeHarness>) =>
          h.handler.handleORMapSyncRespLeaf({
            mapName: MAP_NAME,
            path: leafPathOf(h.map, 'K'),
            entries: [],
          }),
      ],
      [
        'bucket',
        (h: ReturnType<typeof makeHarness>) =>
          h.handler.handleORMapSyncRespBuckets({ mapName: MAP_NAME, path: '', buckets: {} }),
      ],
      [
        'root',
        (h: ReturnType<typeof makeHarness>) =>
          h.handler.handleORMapSyncRespRoot({ mapName: MAP_NAME, rootHash: 0 }),
      ],
    ] as const)(
      'an absence reset on a %s response persists the attribution although no record changed',
      async (_name, trigger) => {
        const harness = makeHarness();
        addThenRemove(harness.map, 'K', 'v');

        await trigger(harness);

        expect(harness.persistKeyTombstones).toHaveBeenCalledTimes(1);
        expect(harness.persistTombstones).not.toHaveBeenCalled();
      },
    );

    test('only the keys whose attribution changed are handed to the persist', async () => {
      const { map, handler, persistKeyTombstones } = makeHarness();
      map.add('changed', 'live');
      map.add('same', 'live');
      const kept = addThenRemove(map, 'same', 'gone');
      const dropped = addThenRemove(map, 'omitted', 'gone');
      expect(dropped).toBeDefined();

      // One entry changes its key's set, one repeats it, and a third local key
      // is left out of the response, which resets it.
      await handler.handleORMapSyncRespLeaf({
        mapName: MAP_NAME,
        path: '',
        entries: [
          { key: 'changed', records: map.getRecords('changed'), tombstones: ['server-tag'] },
          { key: 'same', records: map.getRecords('same'), tombstones: [kept] },
        ],
      });

      expect(persistKeyTombstones).toHaveBeenCalledTimes(1);
      const [mapName, keys] = persistKeyTombstones.mock.calls[0];
      expect(mapName).toBe(MAP_NAME);
      expect(sorted(keys)).toEqual(['changed', 'omitted']);
    });

    test.each([
      [
        'bucket',
        (h: ReturnType<typeof makeHarness>) =>
          h.handler.handleORMapSyncRespBuckets({ mapName: MAP_NAME, path: '', buckets: {} }),
      ],
      [
        'root',
        (h: ReturnType<typeof makeHarness>) =>
          h.handler.handleORMapSyncRespRoot({ mapName: MAP_NAME, rootHash: 0 }),
      ],
    ] as const)(
      'an absence reset on a %s response hands over every key it reset, in one call',
      async (_name, trigger) => {
        const harness = makeHarness();
        const reset: string[] = [];
        for (let i = 0; i < 30; i++) {
          addThenRemove(harness.map, `key-${i}`, 'v');
          reset.push(`key-${i}`);
        }
        // A key with records only has no attribution to reset.
        harness.map.add('live-only', 'v');

        await trigger(harness);

        expect(harness.persistKeyTombstones).toHaveBeenCalledTimes(1);
        expect(sorted(harness.persistKeyTombstones.mock.calls[0][1])).toEqual(sorted(reset));
      },
    );

    test('a walk that leaves every attribution as it was writes nothing', async () => {
      const harness = makeHarness();
      const { map, handler, persistKeyTombstones } = harness;
      map.add('K', 'live');
      const tag = addThenRemove(map, 'K', 'gone');
      map.add('other', 'value');
      const leafPath = leafPathOf(map, 'K');
      const root = map.getMerkleTree().getRootHash();

      // Root differs, buckets list every local child, the leaf repeats the
      // attribution the client already holds, and so does the diff.
      await handler.handleORMapSyncRespRoot({ mapName: MAP_NAME, rootHash: root + 1 });
      const localBuckets = map.getMerkleTree().getBuckets('');
      await handler.handleORMapSyncRespBuckets({
        mapName: MAP_NAME,
        path: '',
        buckets: Object.fromEntries(Object.keys(localBuckets).map((c) => [c, 1])),
      });
      await handler.handleORMapSyncRespLeaf({
        mapName: MAP_NAME,
        path: leafPath,
        entries: [{ key: 'K', records: map.getRecords('K'), tombstones: [tag] }],
      });
      await handler.handleORMapDiffResponse({
        mapName: MAP_NAME,
        entries: [{ key: 'K', records: [], tombstones: [tag] }],
      });

      expect(persistKeyTombstones).not.toHaveBeenCalled();
    });

    test('a failed attribution persist rejects the response before the covering epoch is confirmed', async () => {
      const { map, handler, persistKeyTombstones, confirmedEpochs } = makeHarness();
      map.add('K', 'live');
      persistKeyTombstones.mockRejectedValueOnce(new Error('simulated persist failure'));

      await expect(
        handler.handleORMapSyncRespLeaf({
          mapName: MAP_NAME,
          path: leafPathOf(map, 'K'),
          coveringEpoch: 4,
          entries: [{ key: 'K', records: [], tombstones: ['server-tag'] }],
        }),
      ).rejects.toThrow('simulated persist failure');

      expect(confirmedEpochs).toEqual([]);
    });
  });

  describe('equal roots require the same per-key tombstones', () => {
    /**
     * The client and the server hold the same record under K; the server also
     * attributes a tombstone to K. The server root is computed by the same tree
     * the client uses, over the server's tag and tombstone sets.
     */
    function clientMissingAServerTombstone() {
      const harness = makeHarness();
      const { map } = harness;
      const record = map.add('K', 'live');

      const server = new ORMap<string, string>(new HLC('server'));
      server.apply('K', record);
      server.setKeyTombstones('K', ['server-tag']);

      return { harness, serverRoot: server.getMerkleTree().getRootHash() };
    }

    test('a client whose key lacks a tag the server leaf includes does not take the equal-roots confirm path', async () => {
      const { harness, serverRoot } = clientMissingAServerTombstone();
      const { map, handler, confirmedEpochs, requestedPaths } = harness;
      expect(map.getMerkleTree().getRootHash()).not.toBe(serverRoot);

      await handler.handleORMapSyncRespRoot({
        mapName: MAP_NAME,
        rootHash: serverRoot,
        coveringEpoch: 7,
      });

      expect(confirmedEpochs).toEqual([]);
      expect(requestedPaths()).toEqual(['']);
    });

    test('once the client attributes that tag the roots match and the epoch is confirmed', async () => {
      const { harness, serverRoot } = clientMissingAServerTombstone();
      const { map, handler, confirmedEpochs, requestedPaths } = harness;
      map.setKeyTombstones('K', ['server-tag']);
      expect(map.getMerkleTree().getRootHash()).toBe(serverRoot);

      await handler.handleORMapSyncRespRoot({
        mapName: MAP_NAME,
        rootHash: serverRoot,
        coveringEpoch: 7,
      });

      expect(confirmedEpochs).toEqual([7]);
      expect(requestedPaths()).toEqual([]);
    });
  });

  /**
   * A tag is created once, so a record the server still serves as live under a
   * tag this client has tombstoned can mean only one thing: the server does not
   * hold that tombstone. It acknowledged the remove and then lost it (an unclean
   * stop before the write reached disk), or it never applied it.
   *
   * The key's attribution is otherwise a mirror of the server's set, and an
   * acknowledged remove is no longer pending, so without a rule of its own this
   * tag would leave the key's attributed set, the push for the key would not
   * carry it, and the server would keep the value forever while this client
   * hides it. The tag has to stay attributed to the key so the push hands the
   * tombstone back, and the key has to be pushed even when the removed value was
   * the only one it held.
   */
  describe('a remove the server lost is handed back to it for the key it belongs to', () => {
    type Harness = ReturnType<typeof makeHarness>;
    type Entry = { key: string; records: ORMapRecord<string>[]; tombstones: string[] };

    const responses = [
      [
        'ORMAP_SYNC_RESP_LEAF',
        (h: Harness, entry: Entry) =>
          h.handler.handleORMapSyncRespLeaf({
            mapName: MAP_NAME,
            path: leafPathOf(h.map, entry.key),
            entries: [entry],
          }),
      ],
      [
        'ORMAP_DIFF_RESPONSE',
        (h: Harness, entry: Entry) =>
          h.handler.handleORMapDiffResponse({ mapName: MAP_NAME, entries: [entry] }),
      ],
    ] as const;

    /** Every value any subscriber notification showed under `key`. */
    function recordRendered(map: ORMap<string, string>, key: string): () => string[] {
      const rendered: string[] = [];
      map.subscribe((entries) => {
        for (const [entryKey, values] of entries) {
          if (entryKey === key) rendered.push(...values);
        }
      });
      return () => rendered;
    }

    /**
     * Adds `value` under `key` and removes it, the remove being acknowledged
     * (nothing is pending). Returns the record as the server would still serve
     * it if it had lost the remove.
     */
    function addThenRemoveAcked(h: Harness, key: string, value: string): ORMapRecord<string> {
      const record = h.map.add(key, value);
      expect(h.map.remove(key, value)).toEqual([record.tag]);
      expect(h.pending.get(key)).toBeUndefined();
      return record;
    }

    test.each(responses)(
      '%s: a live record whose tag this client removed keeps the tag attributed, and the push for the key carries it',
      async (_name, respond) => {
        const harness = makeHarness();
        const { map, pushedEntriesFor } = harness;
        const kept = map.add('K', 'kept');
        const lost = addThenRemoveAcked(harness, 'K', 'removed-value');
        const rendered = recordRendered(map, 'K');

        // The server serves both records as live and reports no tombstone for K.
        await respond(harness, { key: 'K', records: [kept, lost], tombstones: [] });

        const entries = pushedEntriesFor('K');
        expect(entries).toHaveLength(1);
        expect(entries[0].tombstones).toEqual([lost.tag]);
        expect(entries[0].records).toEqual([kept]);
        expect(sorted(map.getKeyTombstones('K'))).toEqual([lost.tag]);

        expect(map.get('K')).toEqual(['kept']);
        expect(rendered()).not.toContain('removed-value');
      },
    );

    test.each(responses)(
      '%s: a tag suppressed map-wide but attributed to no key becomes attributed to the key that serves it live',
      async (_name, respond) => {
        const harness = makeHarness();
        const { map, pushedEntriesFor } = harness;
        const kept = map.add('K', 'kept');
        // A tombstone whose key was never recorded (a store written before
        // attribution existed).
        const lost: ORMapRecord<string> = {
          value: 'removed-value',
          tag: 'legacy-removed-tag',
          timestamp: new HLC('server').now(),
        };
        map.applyTombstone(lost.tag);
        expect(map.getKeyTombstones('K').size).toBe(0);
        const rendered = recordRendered(map, 'K');

        await respond(harness, { key: 'K', records: [kept, lost], tombstones: [] });

        const entries = pushedEntriesFor('K');
        expect(entries).toHaveLength(1);
        expect(entries[0].tombstones).toEqual([lost.tag]);
        expect(sorted(map.getKeyTombstones('K'))).toEqual([lost.tag]);

        expect(map.get('K')).toEqual(['kept']);
        expect(rendered()).not.toContain('removed-value');
      },
    );

    test.each(responses)(
      '%s: the key is pushed with the tag even when this client holds no live record for it',
      async (_name, respond) => {
        const harness = makeHarness();
        const { map, pushedEntriesFor } = harness;
        const lost = addThenRemoveAcked(harness, 'K', 'removed-value');
        expect(map.allKeys()).not.toContain('K');
        const rendered = recordRendered(map, 'K');

        await respond(harness, { key: 'K', records: [lost], tombstones: [] });

        const entries = pushedEntriesFor('K');
        expect(entries).toHaveLength(1);
        expect(entries[0].records).toEqual([]);
        expect(entries[0].tombstones).toEqual([lost.tag]);
        expect(sorted(map.getKeyTombstones('K'))).toEqual([lost.tag]);

        expect(map.get('K')).toEqual([]);
        expect(rendered()).not.toContain('removed-value');
      },
    );

    test.each(responses)(
      '%s: a live record whose tag this client never removed is admitted and attributes nothing',
      async (_name, respond) => {
        const harness = makeHarness();
        const { map, pushedEntriesFor } = harness;
        map.add('K', 'kept');
        // An unrelated remove elsewhere, so the map-wide set is not empty.
        const elsewhere = addThenRemove(map, 'other', 'x');
        const incoming: ORMapRecord<string> = {
          value: 'from-server',
          tag: 'server-live-tag',
          timestamp: new HLC('server').now(),
        };

        await respond(harness, { key: 'K', records: [incoming], tombstones: [] });

        expect(sorted(map.get('K'))).toEqual(['from-server', 'kept']);
        expect(map.getKeyTombstones('K').size).toBe(0);
        expect(map.getTombstones()).toEqual([elsewhere]);
        for (const entry of pushedEntriesFor('K')) {
          expect(entry.tombstones).toEqual([]);
        }
      },
    );

    test.each(responses)(
      '%s: a removed tag the response does not serve live is still dropped from the key',
      async (_name, respond) => {
        const harness = makeHarness();
        const { map, pushedEntriesFor } = harness;
        const kept = map.add('K', 'kept');
        const removed = addThenRemoveAcked(harness, 'K', 'removed-value');
        expect(sorted(map.getKeyTombstones('K'))).toEqual([removed.tag]);

        // The server neither serves the record nor attributes its tag: it has
        // already forgotten the tombstone for good (pruned it), which is not a
        // loss to repair.
        await respond(harness, { key: 'K', records: [kept], tombstones: [] });

        expect(map.getKeyTombstones('K').size).toBe(0);
        expect(map.getTombstones()).toContain(removed.tag);
        for (const entry of pushedEntriesFor('K')) {
          expect(entry.tombstones).toEqual([]);
        }
      },
    );

    test.each(responses)(
      '%s: a tag that becomes tombstoned only through this entry is not a lost remove, so an emptied key is not pushed',
      async (name, respond) => {
        const harness = makeHarness();
        const { map, pending, sent } = harness;
        // The state after a full resync wiped the map: the remove survives only
        // as a pending operation, the map-wide set no longer holds its tag, and
        // the server, which has not received the remove yet, serves the record.
        const record = map.add('K', 'removed-value');
        pending.set('K', [record.tag]);
        const leafPath = leafPathOf(map, 'K');
        map.clear();
        expect(map.isTombstoned(record.tag)).toBe(false);
        const rendered = recordRendered(map, 'K');

        // The key is no longer in the local tree, so the leaf's path is the one
        // it had before the wipe rather than one looked up now.
        await (name === 'ORMAP_SYNC_RESP_LEAF'
          ? harness.handler.handleORMapSyncRespLeaf({
              mapName: MAP_NAME,
              path: leafPath,
              entries: [{ key: 'K', records: [record], tombstones: [] }],
            })
          : respond(harness, { key: 'K', records: [record], tombstones: [] }));

        // The pending operation itself delivers the remove; nothing is pushed.
        expect(sent.filter((msg) => msg.type === 'ORMAP_PUSH_DIFF')).toEqual([]);
        expect(sorted(map.getKeyTombstones('K'))).toEqual([record.tag]);
        expect(map.get('K')).toEqual([]);
        expect(rendered()).not.toContain('removed-value');
      },
    );

    test.each(responses)(
      '%s: only the key that is served a removed tag is pushed for it; a tombstone-only key beside it is not',
      async (_name, respond) => {
        const harness = makeHarness();
        const { map, sent } = harness;
        const lost = addThenRemoveAcked(harness, 'K', 'removed-value');
        const bystander = addThenRemove(map, 'bystander', 'x');

        await respond(harness, { key: 'K', records: [lost], tombstones: [] });

        const pushed = sent
          .filter((msg) => msg.type === 'ORMAP_PUSH_DIFF')
          .flatMap((msg) => msg.payload.entries as PushDiffEntry[]);
        expect(pushed).toEqual([{ key: 'K', records: [], tombstones: [lost.tag] }]);
        // The other key's own tombstone is not handed out under K.
        expect(pushed[0].tombstones).not.toContain(bystander);
      },
    );

    test.each(responses)(
      '%s: the kept attribution is persisted before the covering epoch is confirmed',
      async (name) => {
        const harness = makeHarness();
        const { map, persistKeyTombstones, confirmedEpochs } = harness;
        const lost: ORMapRecord<string> = {
          value: 'removed-value',
          tag: 'legacy-removed-tag',
          timestamp: new HLC('server').now(),
        };
        map.applyTombstone(lost.tag);
        map.add('K', 'kept');
        const epochsAtPersist: number[][] = [];
        persistKeyTombstones.mockImplementation(async () => {
          epochsAtPersist.push([...confirmedEpochs]);
        });

        const payload = {
          mapName: MAP_NAME,
          coveringEpoch: 9,
          entries: [{ key: 'K', records: [lost], tombstones: [] }],
        };
        await (name === 'ORMAP_SYNC_RESP_LEAF'
          ? harness.handler.handleORMapSyncRespLeaf({ ...payload, path: leafPathOf(map, 'K') })
          : harness.handler.handleORMapDiffResponse(payload));

        expect(epochsAtPersist).toEqual([[]]);
        expect(confirmedEpochs).toEqual([9]);
      },
    );
  });

  describe('pending removes are read from the op log once per response', () => {
    type Harness = ReturnType<typeof makeHarness>;

    /** N keys, each emptied by a remove, so each is in the tree by attribution only. */
    function manyTombstoneOnlyKeys(h: Harness, count: number): string[] {
      const keys: string[] = [];
      for (let i = 0; i < count; i++) {
        const key = `key-${i}`;
        addThenRemove(h.map, key, 'v');
        keys.push(key);
      }
      return keys;
    }

    test('a zero-root reset over many keys asks for the pending removes once', async () => {
      const harness = makeHarness();
      const { map, handler, pending, getPendingRemoveTagsByKey } = harness;
      const keys = manyTombstoneOnlyKeys(harness, 50);
      const stillPending = [...map.getKeyTombstones(keys[7])];
      pending.set(keys[7], stillPending);

      await handler.handleORMapSyncRespRoot({ mapName: MAP_NAME, rootHash: 0 });

      expect(getPendingRemoveTagsByKey).toHaveBeenCalledTimes(1);
      expect(getPendingRemoveTagsByKey).toHaveBeenCalledWith(MAP_NAME);
      // The single read still served every key.
      for (const key of keys) {
        expect(sorted(map.getKeyTombstones(key))).toEqual(key === keys[7] ? stillPending : []);
      }
    });

    test('a bucket response that resets several local-only children asks once', async () => {
      const harness = makeHarness();
      const { map, handler, getPendingRemoveTagsByKey } = harness;
      manyTombstoneOnlyKeys(harness, 50);
      expect(Object.keys(map.getMerkleTree().getBuckets('')).length).toBeGreaterThan(1);

      await handler.handleORMapSyncRespBuckets({ mapName: MAP_NAME, path: '', buckets: {} });

      expect(getPendingRemoveTagsByKey).toHaveBeenCalledTimes(1);
      expect(map.getMerkleTree().getRootHash()).toBe(0);
    });

    test.each([
      [
        'leaf',
        (h: Harness, entries: Array<{ key: string; records: never[]; tombstones: string[] }>) =>
          h.handler.handleORMapSyncRespLeaf({ mapName: MAP_NAME, path: '', entries }),
      ],
      [
        'diff',
        (h: Harness, entries: Array<{ key: string; records: never[]; tombstones: string[] }>) =>
          h.handler.handleORMapDiffResponse({ mapName: MAP_NAME, entries }),
      ],
    ] as const)('a %s response over many keys asks once', async (_name, respond) => {
      const harness = makeHarness();
      const keys = manyTombstoneOnlyKeys(harness, 50);

      await respond(
        harness,
        keys.map((key) => ({ key, records: [], tombstones: ['server-tag'] })),
      );

      expect(harness.getPendingRemoveTagsByKey).toHaveBeenCalledTimes(1);
    });

    test('a response that touches no key does not ask at all', async () => {
      const harness = makeHarness();
      const { map, handler, getPendingRemoveTagsByKey } = harness;
      map.add('K', 'live');
      const root = map.getMerkleTree().getRootHash();

      await handler.handleORMapSyncRespRoot({ mapName: MAP_NAME, rootHash: root });
      await handler.handleORMapSyncRespRoot({ mapName: MAP_NAME, rootHash: root + 1 });
      await handler.handleORMapDiffResponse({ mapName: MAP_NAME, entries: [] });

      expect(getPendingRemoveTagsByKey).not.toHaveBeenCalled();
    });

    test('a remove acknowledged between two responses is no longer pending in the second', async () => {
      const harness = makeHarness();
      const { map, handler, pending, getPendingRemoveTagsByKey } = harness;
      const keys = manyTombstoneOnlyKeys(harness, 5);
      const tagOf = (key: string) => sorted(map.getKeyTombstones(key));
      const firstTags = tagOf(keys[0]);
      for (const key of keys) pending.set(key, tagOf(key));

      await handler.handleORMapSyncRespRoot({ mapName: MAP_NAME, rootHash: 0 });
      for (const key of keys) {
        expect(map.getKeyTombstones(key).size).toBe(1);
      }

      // Every remove but the first key's is acknowledged before the next response.
      for (const key of keys.slice(1)) pending.delete(key);
      await handler.handleORMapSyncRespRoot({ mapName: MAP_NAME, rootHash: 0 });

      expect(getPendingRemoveTagsByKey).toHaveBeenCalledTimes(2);
      expect(sorted(map.getKeyTombstones(keys[0]))).toEqual(firstTags);
      for (const key of keys.slice(1)) {
        expect(map.getKeyTombstones(key).size).toBe(0);
      }
    });
  });
});

/**
 * The same rules through the real engine, where the pending remove tags come
 * from the op log and the persist callbacks write to storage.
 */
describe('SyncEngine per-key tombstone attribution', () => {
  const bucketKeyOf = (key: string) =>
    orMapKeyTombstonesBucketKey(MAP_NAME, orMapKeyTombstonesBucketOf(key));
  const KEY_TOMBSTONES_META = bucketKeyOf('K');

  function memoryAdapter() {
    const kv = new Map<string, unknown>();
    const meta = new Map<string, unknown>();
    /** The key of every `setMeta` call, in order. */
    const metaWrites: string[] = [];
    let nextOpId = 0;
    const adapter = {
      initialize: async () => {},
      close: async () => {},
      get: async (key: string) => kv.get(key),
      put: async (key: string, value: unknown) => void kv.set(key, value),
      remove: async (key: string) => void kv.delete(key),
      getMeta: async (key: string) => meta.get(key),
      setMeta: async (key: string, value: unknown) => {
        metaWrites.push(key);
        meta.set(key, value);
      },
      batchPut: async (entries: Map<string, unknown>) =>
        entries.forEach((value, key) => kv.set(key, value)),
      appendOpLog: async () => ++nextOpId,
      commitWrite: async () => ++nextOpId,
      getPendingOps: async () => [],
      markOpsSynced: async () => {},
      deleteOp: async () => {},
      getAllKeys: async () => Array.from(kv.keys()),
      getAllMetaKeys: async () => Array.from(meta.keys()),
      clear: async () => {
        kv.clear();
        meta.clear();
      },
    };
    return { adapter: adapter as unknown as IStorageAdapter, meta, metaWrites };
  }

  async function makeEngine() {
    const { adapter, meta, metaWrites } = memoryAdapter();
    const engine = new SyncEngine({
      nodeId: 'n1',
      connectionProvider: new NullConnectionProvider(),
      storageAdapter: adapter,
    });
    // The constructor starts an async op-log load that resets the op log; let it
    // settle so the ops recorded below are not cleared.
    await new Promise((resolve) => setTimeout(resolve, 25));

    const map = new ORMap<string, string>(engine.getHLC());
    engine.registerMap(MAP_NAME, map);
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the test drives the engine's private message handlers directly
    const internals = engine as any;
    const handler = internals.orMapSyncHandler as ORMapSyncHandler;
    /** The attribution buckets written since the last call, in order. */
    const takeBucketWrites = (): string[] =>
      metaWrites.splice(0).filter((key) => key.startsWith(`__sys__:${MAP_NAME}:keyTombstones`));
    return { engine, internals, handler, map, meta, takeBucketWrites };
  }

  /** Two keys that share a leaf of the Merkle tree, and so a storage bucket. */
  function twoKeysOfOneLeaf(map: ORMap<string, string>): { keys: [string, string]; path: string } {
    const byPath = new Map<string, string>();
    for (let i = 0; ; i++) {
      const key = `key-${i}`;
      map.add(key, 'live');
      const path = leafPathOf(map, key);
      const earlier = byPath.get(path);
      if (earlier !== undefined) return { keys: [earlier, key], path };
      byPath.set(path, key);
    }
  }

  /**
   * A local remove of `value` under `key`, recorded as the client does it. The
   * returned promise resolves once the op is committed and in the op log, which
   * is what a response handler reads the pending tags from.
   */
  async function localRemove(
    engine: SyncEngine,
    map: ORMap<string, string>,
    key: string,
    value: string,
  ): Promise<{ tag: string; opId: string }> {
    map.add(key, value);
    const [tag] = map.remove(key, value);
    const opId = await engine.recordOperation(MAP_NAME, 'OR_REMOVE', key, {
      orTag: tag,
      timestamp: engine.getHLC().now(),
    });
    return { tag, opId };
  }

  const leafFor = (key: string, tombstones: string[]) => ({
    mapName: MAP_NAME,
    entries: [{ key, records: [], tombstones }],
  });

  test('an un-acknowledged local remove keeps its tag through a server replace; the acknowledged one drops it', async () => {
    const { engine, internals, handler, map } = await makeEngine();
    const { tag, opId } = await localRemove(engine, map, 'K', 'v');

    await handler.handleORMapSyncRespLeaf(leafFor('K', ['server-tag']));
    expect(sorted(map.getKeyTombstones('K'))).toEqual(sorted(['server-tag', tag]));

    internals.handleOpAck({ type: 'OP_ACK', payload: { lastId: opId } });
    expect(internals.opLog.some((op: OpLogEntry) => op.id === opId)).toBe(false);

    await handler.handleORMapSyncRespLeaf(leafFor('K', ['server-tag']));
    expect(sorted(map.getKeyTombstones('K'))).toEqual(['server-tag']);
    expect(map.getTombstones()).toContain(tag);
    engine.close();
  });

  test('the tag of a remove the server refused is not kept', async () => {
    const { engine, internals, handler, map } = await makeEngine();
    const { tag, opId } = await localRemove(engine, map, 'K', 'v');

    // Flag the op the way a permanent refusal does, while it is still in the op
    // log, so the exclusion is the flag's doing rather than the op's removal.
    const op = internals.opLog.find((entry: OpLogEntry) => entry.id === opId) as OpLogEntry;
    op.rejected = true;
    expect(op.synced).toBe(false);

    await handler.handleORMapSyncRespLeaf(leafFor('K', ['server-tag']));

    expect(sorted(map.getKeyTombstones('K'))).toEqual(['server-tag']);
    expect(map.getTombstones()).toContain(tag);
    engine.close();
  });

  test('a pending remove of another key or another map does not leak into this key', async () => {
    const { engine, handler, map } = await makeEngine();
    await localRemove(engine, map, 'other', 'v');
    await engine.recordOperation('another-map', 'OR_REMOVE', 'K', {
      orTag: 'foreign-tag',
      timestamp: engine.getHLC().now(),
    });

    await handler.handleORMapSyncRespLeaf(leafFor('K', ['server-tag']));

    expect(sorted(map.getKeyTombstones('K'))).toEqual(['server-tag']);
    engine.close();
  });

  test('a zero-root reset over many keys reads the op log once, and an acknowledged remove is gone from the next response', async () => {
    const { engine, internals, handler, map } = await makeEngine();
    const removes: Array<{ key: string; tag: string; opId: string }> = [];
    for (let i = 0; i < 20; i++) {
      const key = `key-${i}`;
      removes.push({ key, ...(await localRemove(engine, map, key, 'v')) });
    }

    // Count full traversals of the op log: every way of walking an array goes
    // through its iterator or its length, and the handler is the only reader
    // while a root response is handled.
    const opLog = internals.opLog as OpLogEntry[];
    let traversals = 0;
    const counted = new Proxy(opLog, {
      get(target, property, receiver) {
        if (property === Symbol.iterator) traversals++;
        return Reflect.get(target, property, receiver);
      },
    });
    internals.opLog = counted;
    try {
      await handler.handleORMapSyncRespRoot({ mapName: MAP_NAME, rootHash: 0 });
    } finally {
      internals.opLog = opLog;
    }
    expect(traversals).toBe(1);
    for (const { key, tag } of removes) {
      expect(sorted(map.getKeyTombstones(key))).toEqual([tag]);
    }

    // The first remove is acknowledged; the next response must not see it.
    const [acked, ...rest] = removes;
    internals.handleOpAck({ type: 'OP_ACK', payload: { lastId: acked.opId } });
    await handler.handleORMapSyncRespRoot({ mapName: MAP_NAME, rootHash: 0 });

    expect(map.getKeyTombstones(acked.key).size).toBe(0);
    for (const { key, tag } of rest) {
      expect(sorted(map.getKeyTombstones(key))).toEqual([tag]);
    }
    engine.close();
  });

  test('a server remove event attributes the tag to its key and persists the attribution', async () => {
    const { engine, internals, map, meta } = await makeEngine();
    const record = map.add('K', 'v');

    await internals.applyServerEvent(MAP_NAME, 'OR_REMOVE', 'K', undefined, undefined, record.tag);

    expect(sorted(map.getKeyTombstones('K'))).toEqual([record.tag]);
    expect(map.get('K')).toEqual([]);
    expect(meta.get(KEY_TOMBSTONES_META)).toEqual([['K', [record.tag]]]);
    engine.close();
  });

  test('a response that changes the attribution writes it to storage as key/tags pairs', async () => {
    const { engine, handler, map, meta } = await makeEngine();
    map.add('K', 'live');
    expect(meta.has(KEY_TOMBSTONES_META)).toBe(false);

    await handler.handleORMapSyncRespLeaf(leafFor('K', ['server-tag']));
    expect(meta.get(KEY_TOMBSTONES_META)).toEqual([['K', ['server-tag']]]);

    // The server stops attributing the tag: the persisted entry follows.
    await handler.handleORMapSyncRespLeaf(leafFor('K', []));
    expect(meta.get(KEY_TOMBSTONES_META)).toEqual([]);
    engine.close();
  });

  test('a leaf response changing the attribution of keys of one Merkle path writes exactly one bucket; repeating it writes none', async () => {
    const { engine, handler, map, meta, takeBucketWrites } = await makeEngine();
    const { keys, path } = twoKeysOfOneLeaf(map);
    // Every other key the search added sits in the tree as well; keep the ones
    // under this leaf's path in the response so that none of them is reset.
    const underPath = map.getMerkleTree().getKeysInBucket(path);
    expect(underPath.sort()).toEqual([...keys].sort());
    const response = {
      mapName: MAP_NAME,
      path,
      entries: keys.map((key, i) => ({
        key,
        records: map.getRecords(key),
        tombstones: [`server-tag-${i}`],
      })),
    };
    takeBucketWrites();

    await handler.handleORMapSyncRespLeaf(response);

    expect(bucketKeyOf(keys[0])).toBe(bucketKeyOf(keys[1]));
    expect(takeBucketWrites()).toEqual([bucketKeyOf(keys[0])]);
    expect(meta.get(bucketKeyOf(keys[0]))).toEqual([
      [keys[0], ['server-tag-0']],
      [keys[1], ['server-tag-1']],
    ]);

    // The same response again changes nothing, and so does the rest of a walk.
    await handler.handleORMapSyncRespLeaf(response);
    await handler.handleORMapSyncRespRoot({
      mapName: MAP_NAME,
      rootHash: map.getMerkleTree().getRootHash() + 1,
    });
    await handler.handleORMapSyncRespBuckets({
      mapName: MAP_NAME,
      path: '',
      buckets: Object.fromEntries(
        Object.keys(map.getMerkleTree().getBuckets('')).map((child) => [child, 1]),
      ),
    });
    await handler.handleORMapDiffResponse({
      mapName: MAP_NAME,
      entries: [{ key: keys[0], records: [], tombstones: ['server-tag-0'] }],
    });

    expect(takeBucketWrites()).toEqual([]);
    engine.close();
  });

  test('a response over keys of several buckets writes each of those buckets once, and no other', async () => {
    const { engine, handler, map, meta, takeBucketWrites } = await makeEngine();
    const keys = Array.from({ length: 40 }, (_, i) => `key-${i}`);
    for (const key of keys) addThenRemove(map, key, 'v');
    await engine.persistORMapKeyTombstones(MAP_NAME, keys);
    const buckets = new Set(keys.map(bucketKeyOf));
    expect(buckets.size).toBeGreaterThanOrEqual(3);
    expect(new Set(takeBucketWrites())).toEqual(buckets);

    // The server stops attributing anything to half of the keys.
    const changed = keys.filter((_, i) => i % 2 === 0);
    await handler.handleORMapDiffResponse({
      mapName: MAP_NAME,
      entries: changed.map((key) => ({ key, records: [], tombstones: [] })),
    });

    const written = takeBucketWrites();
    const expected = new Set(changed.map(bucketKeyOf));
    expect(new Set(written)).toEqual(expected);
    expect(written).toHaveLength(expected.size);
    // What is left on disk is exactly the attribution of the untouched keys.
    const persisted = [...buckets].flatMap(
      (bucket) => meta.get(bucket) as Array<[string, string[]]>,
    );
    expect(sorted(persisted.map(([key]) => key))).toEqual(
      sorted(keys.filter((_, i) => i % 2 === 1)),
    );
    engine.close();
  });
});
