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
    const ackedForMap: Array<{ mapName: string; epoch: number }> = [];
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- captured sent messages have no fixed shape at the test boundary
    const sent: any[] = [];
    const fullResyncs: Array<{ mapName: string; boundary: unknown }> = [];
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
      onCoveringEpochApplied: (mapName, epoch) => {
        acked.push(epoch);
        ackedForMap.push({ mapName, epoch });
      },
      onFullResync: async (mapName, boundary) => {
        fullResyncs.push({ mapName, boundary });
      },
      getClaimedEpoch: () => 42,
    });
    return { handler, acked, ackedForMap, sent, fullResyncs };
  }

  test('empty diff (root matches) confirms the conveyed covering epoch', async () => {
    const map = new ORMap<string, string>(new HLC('n1'));
    const { handler, acked, ackedForMap } = makeHandler(map);
    const rootHash = map.getMerkleTree().getRootHash();

    await handler.handleORMapSyncRespRoot({ mapName: 'tags', rootHash, coveringEpoch: 5 });

    expect(acked).toEqual([5]);
    // The map name is reported alongside the epoch — SyncEngine's cross-map
    // min-barrier (applyMapCoverage) folds coverage PER map, not globally.
    expect(ackedForMap).toEqual([{ mapName: 'tags', epoch: 5 }]);
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

  test('fullResync routes to REPLACE: discards local, pulls snapshot, does NOT ACK at root', async () => {
    const map = new ORMap<string, string>(new HLC('n1'));
    map.add('list1', 'stale'); // some local state to discard
    const { handler, acked, sent, fullResyncs } = makeHandler(map);
    const boundary = new HLC('n1').now();

    // Even though the covering epoch is conveyed, a full_resync must NOT confirm it
    // at root time — the ACK is deferred to post-snapshot apply (delivered_conn on
    // completion). It must trigger the REPLACE discard and pull the snapshot.
    await handler.handleORMapSyncRespRoot({
      mapName: 'tags',
      rootHash: 999,
      coveringEpoch: 5,
      fullResync: true,
      timestamp: boundary,
    });

    expect(acked).toEqual([]); // no covering-epoch ACK on a REPLACE root
    expect(fullResyncs).toEqual([{ mapName: 'tags', boundary }]);
    // Pulls the full snapshot via a merkle bucket request at the root path.
    expect(sent).toContainEqual({
      type: 'ORMAP_MERKLE_REQ_BUCKET',
      payload: { mapName: 'tags', path: '' },
    });
  });

  test('sendSyncInit reports the confirmed-apply cursor as claimedEpoch', () => {
    const map = new ORMap<string, string>(new HLC('n1'));
    const { handler, sent } = makeHandler(map);
    handler.sendSyncInit('tags', 0);
    const init = sent.find((m) => m.type === 'ORMAP_SYNC_INIT');
    expect(init).toBeDefined();
    expect(init.claimedEpoch).toBe(42);
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
