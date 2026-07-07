import { HLC, ORMap } from '@topgunbuild/core';
import { ORMapSyncHandler } from '../ORMapSyncHandler';

/**
 * Covering-epoch ACK (SPEC-342b, AC3b client half): the client learns the
 * covering epoch from OR-Map sync responses and confirms it AFTER durable apply,
 * INCLUDING on an empty diff (root already matches) so an up-to-date client still
 * advances its confirmed-apply cursor instead of pinning the server low-water-mark.
 */
describe('ORMapSyncHandler covering-epoch ACK', () => {
  function makeHandler(map: ORMap<string, string>) {
    const acked: number[] = [];
    const handler = new ORMapSyncHandler({
      getMap: () => map,
      sendMessage: () => true,
      hlc: new HLC('n1'),
      onTimestampUpdate: async () => {},
      persistKey: async () => {},
      persistTombstones: async () => {},
      onCoveringEpochApplied: (epoch) => acked.push(epoch),
    });
    return { handler, acked };
  }

  test('empty diff (root matches) confirms the conveyed covering epoch', async () => {
    const map = new ORMap<string, string>(new HLC('n1'));
    const { handler, acked } = makeHandler(map);
    const rootHash = map.getMerkleTree().getRootHash();

    await handler.handleORMapSyncRespRoot({ mapName: 'tags', rootHash, coveringEpoch: 5 });

    expect(acked).toEqual([5]);
  });

  test('root MISMATCH does NOT ACK at root time (client lacks the set until leaves)', async () => {
    const map = new ORMap<string, string>(new HLC('n1'));
    const { handler, acked } = makeHandler(map);
    const rootHash = map.getMerkleTree().getRootHash();

    await handler.handleORMapSyncRespRoot({
      mapName: 'tags',
      rootHash: rootHash + 12345, // force a mismatch
      coveringEpoch: 9,
    });

    expect(acked).toEqual([]);
  });

  test('leaf apply confirms the covering epoch', async () => {
    const map = new ORMap<string, string>(new HLC('n1'));
    const { handler, acked } = makeHandler(map);

    await handler.handleORMapSyncRespLeaf({
      mapName: 'tags',
      coveringEpoch: 7,
      entries: [
        {
          key: 'list1',
          records: [{ value: 'work', tag: 't1', timestamp: new HLC('n1').now() }],
          tombstones: [],
        },
      ],
    });

    expect(acked).toEqual([7]);
  });

  test('diff apply confirms the covering epoch', async () => {
    const map = new ORMap<string, string>(new HLC('n1'));
    const { handler, acked } = makeHandler(map);

    await handler.handleORMapDiffResponse({
      mapName: 'tags',
      coveringEpoch: 3,
      entries: [
        {
          key: 'list1',
          records: [{ value: 'x', tag: 't2', timestamp: new HLC('n1').now() }],
          tombstones: [],
        },
      ],
    });

    expect(acked).toEqual([3]);
  });

  test('a missing / zero covering epoch is never ACKed (nothing stamped)', async () => {
    const map = new ORMap<string, string>(new HLC('n1'));
    const { handler, acked } = makeHandler(map);
    const rootHash = map.getMerkleTree().getRootHash();

    // In-sync root with no covering epoch conveyed.
    await handler.handleORMapSyncRespRoot({ mapName: 'tags', rootHash });
    // In-sync root with an explicit 0 (server has stamped nothing).
    await handler.handleORMapSyncRespRoot({ mapName: 'tags', rootHash, coveringEpoch: 0 });

    expect(acked).toEqual([]);
  });
});
