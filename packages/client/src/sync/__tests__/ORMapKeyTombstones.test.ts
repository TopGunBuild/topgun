import { HLC, ORMap } from '@topgunbuild/core';
import { ORMapSyncHandler } from '../ORMapSyncHandler';

/**
 * Per-key tombstone scoping in the OR-Map sync handler.
 *
 * The server stores tombstones per key and unions whatever a push carries into
 * that key's set. A push for one key must therefore carry only the tombstones
 * that belong to it: anything else makes the server's per-key set depend on push
 * history, which no client can reproduce, so the two sides can never agree on
 * that key's leaf (TG-MRK-001).
 */

interface PushDiffEntry {
  key: string;
  records: unknown[];
  tombstones: string[];
}

function makeHarness() {
  const map = new ORMap<string, string>(new HLC('n1'));
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- captured sent messages have no fixed shape at the test boundary
  const sent: any[] = [];
  const handler = new ORMapSyncHandler({
    getMap: () => map,
    sendMessage: (msg) => {
      sent.push(msg);
      return true;
    },
    hlc: new HLC('n1'),
    onTimestampUpdate: async () => {},
    persistKey: async () => {},
    persistTombstones: async () => {},
    onCoveringEpochApplied: () => {},
    onFullResync: async () => {},
    getClaimedEpoch: () => 0,
  });

  /** The entries pushed for `key`, across every ORMAP_PUSH_DIFF sent so far. */
  const pushedEntriesFor = (key: string): PushDiffEntry[] =>
    sent
      .filter((msg) => msg.type === 'ORMAP_PUSH_DIFF')
      .flatMap((msg) => msg.payload.entries as PushDiffEntry[])
      .filter((entry) => entry.key === key);

  return { map, handler, sent, pushedEntriesFor };
}

const MAP_NAME = 'tags';

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
  });
});
