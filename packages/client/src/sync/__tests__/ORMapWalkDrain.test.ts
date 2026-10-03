import { HLC, ORMap } from '@topgunbuild/core';
import { ORMapSyncHandler } from '../ORMapSyncHandler';

/**
 * When the OR-Map sync handler may confirm a covering epoch during a Merkle
 * walk.
 *
 * Confirming an epoch tells the server that this client holds every tombstone
 * stamped at or below it; the server then moves the client's cursor and may
 * prune those tombstones. A walk visits several subtrees through separate
 * requests, so an epoch confirmed while one of them is still outstanding lets
 * the server prune a tombstone this client has not applied yet, and the value
 * it removed comes back on the client's next push (TG-MRK-002).
 *
 * The rule these tests pin: a walk confirms once, when its last response has
 * been handled, with the lowest epoch any of its responses conveyed (the root
 * included), and never when a response failed, conveyed no epoch, went
 * unanswered, or belonged to an earlier walk.
 */

const MAP_NAME = 'tags';

/** The first trie level of the key's path: the child of the root that holds it. */
function rootBucketOf(key: string): string {
  const probe = new ORMap<string, string>(new HLC('probe'));
  probe.add(key, 'probe');
  return Object.keys(probe.getMerkleTree().getBuckets(''))[0];
}

/** `count` keys that each sit under a different child of the root. */
function keysInDistinctRootBuckets(count: number): string[] {
  const keys: string[] = [];
  const taken = new Set<string>();
  for (let i = 0; keys.length < count; i++) {
    const key = `key-${i}`;
    const bucket = rootBucketOf(key);
    if (!taken.has(bucket)) {
      taken.add(bucket);
      keys.push(key);
    }
  }
  return keys;
}

// K is the key whose removal the client missed, J a key the server gained, and
// L a key only the client holds. Distinct root children make each of them a
// separate request of the walk.
const [K, J, L] = keysInDistinctRootBuckets(3);

function deferred(): { promise: Promise<void>; resolve: () => void } {
  let resolve!: () => void;
  const promise = new Promise<void>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

function sorted(values: Iterable<string>): string[] {
  return Array.from(values).sort();
}

function makeHarness() {
  const map = new ORMap<string, string>(new HLC('client'));
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- captured sent messages have no fixed shape at the test boundary
  const sent: any[] = [];
  const persistKey = jest.fn<Promise<void>, [string, string]>().mockResolvedValue(undefined);
  const persistTombstones = jest.fn<Promise<void>, [string]>().mockResolvedValue(undefined);
  const persistKeyTombstones = jest
    .fn<Promise<void>, [string, Iterable<string>]>()
    .mockResolvedValue(undefined);
  const onFullResync = jest.fn<Promise<void>, [string, unknown]>().mockResolvedValue(undefined);
  const onCoveringEpochApplied = jest.fn<void, [string, number]>();
  const handler = new ORMapSyncHandler({
    getMap: () => map,
    sendMessage: (msg) => {
      sent.push(msg);
      return true;
    },
    hlc: new HLC('client'),
    onTimestampUpdate: async () => {},
    persistKey,
    persistTombstones,
    getPendingRemoveTagsByKey: () => new Map(),
    persistKeyTombstones,
    onCoveringEpochApplied,
    onFullResync,
    getClaimedEpoch: () => 0,
  });

  /** The paths of every ORMAP_MERKLE_REQ_BUCKET sent so far. */
  const requestedPaths = (): string[] =>
    sent.filter((msg) => msg.type === 'ORMAP_MERKLE_REQ_BUCKET').map((msg) => msg.payload.path);

  return {
    map,
    handler,
    sent,
    persistKey,
    persistTombstones,
    persistKeyTombstones,
    onFullResync,
    onCoveringEpochApplied,
    requestedPaths,
  };
}

/**
 * A client that was away while the server removed K's value and gained J.
 *
 * The client still holds K's record live under tag `tK`; the server holds that
 * tag as a tombstone of K. J exists only on the server, with one live record
 * and one tombstone, so applying J's leaf changes the client's attribution and
 * goes through the attribution persist.
 *
 * With `localOnlyKey` the client also holds L, kept in its tree only by a
 * tombstone the server does not attribute: a bucket response then has
 * attribution to clear and to persist.
 *
 * The returned frame builders read the server state at call time, so a change
 * made to `server` between two walks shows in the second walk's frames. A leaf
 * answers the request for the key's root child directly: the handler does not
 * care how deep a leaf is, and one request per key keeps the count readable.
 */
function stageClientBehindServer(options: { localOnlyKey?: boolean } = {}) {
  const harness = makeHarness();
  const { map } = harness;
  const server = new ORMap<string, string>(new HLC('server'));

  const kRecord = map.add(K, 'k-value');
  server.apply(K, kRecord);
  const [tK] = server.remove(K, 'k-value');

  server.add(J, 'j-live');
  server.add(J, 'j-removed');
  server.remove(J, 'j-removed');

  if (options.localOnlyKey) {
    map.add(L, 'l-value');
    map.remove(L, 'l-value');
  }

  const epochField = (coveringEpoch?: number) =>
    coveringEpoch === undefined ? {} : { coveringEpoch };

  /** The root frame as the server sends it when no full resync is needed: no `fullResync` key. */
  const rootFrame = (coveringEpoch?: number) => ({
    mapName: MAP_NAME,
    rootHash: server.getMerkleTree().getRootHash(),
    ...epochField(coveringEpoch),
  });

  const bucketsFrame = () => ({
    mapName: MAP_NAME,
    path: '',
    buckets: server.getMerkleTree().getBuckets(''),
  });

  const leafFrameAt = (path: string, keys: string[], coveringEpoch?: number) => ({
    mapName: MAP_NAME,
    path,
    ...epochField(coveringEpoch),
    entries: keys.map((key) => ({
      key,
      records: server.getRecords(key),
      tombstones: Array.from(server.getKeyTombstones(key)),
    })),
  });

  const leafFrame = (key: string, coveringEpoch?: number) =>
    leafFrameAt(rootBucketOf(key), [key], coveringEpoch);

  return { ...harness, server, tK, rootFrame, bucketsFrame, leafFrameAt, leafFrame };
}

type Staged = ReturnType<typeof stageClientBehindServer>;

/**
 * Handles the differing root and the root-level bucket response, and checks
 * that the walk now has one request out for K's subtree and one for J's.
 */
async function openTwoRequestWalk(staged: Staged, rootEpoch?: number): Promise<void> {
  const before = staged.requestedPaths().length;
  await staged.handler.handleORMapSyncRespRoot(staged.rootFrame(rootEpoch));
  expect(staged.requestedPaths().slice(before)).toEqual(['']);
  await staged.handler.handleORMapSyncRespBuckets(staged.bucketsFrame());
  expect(sorted(staged.requestedPaths().slice(before + 1))).toEqual(
    sorted([rootBucketOf(K), rootBucketOf(J)]),
  );
}

describe('ORMapSyncHandler confirms a covering epoch only for a drained walk', () => {
  it('does not confirm after the first of two leaves, and confirms once after the leaf that carries the missing tombstone', async () => {
    const staged = stageClientBehindServer();
    const { map, handler, onCoveringEpochApplied, tK } = staged;
    const e2 = 2;

    await openTwoRequestWalk(staged, e2);

    await handler.handleORMapSyncRespLeaf(staged.leafFrame(J, e2));
    // K's request is still out: the client still shows the removed value.
    expect(map.get(K)).toEqual(['k-value']);
    expect(onCoveringEpochApplied).not.toHaveBeenCalled();

    const kLeaf = staged.leafFrame(K, e2);
    expect(kLeaf.entries).toEqual([{ key: K, records: [], tombstones: [tK] }]);
    await handler.handleORMapSyncRespLeaf(kLeaf);

    expect(onCoveringEpochApplied.mock.calls).toEqual([[MAP_NAME, e2]]);
    expect(map.get(K)).toEqual([]);
    expect(map.isTombstoned(tK)).toBe(true);
  });

  it('confirms the lowest epoch its leaves conveyed, once', async () => {
    const staged = stageClientBehindServer();
    const { handler, onCoveringEpochApplied } = staged;

    await openTwoRequestWalk(staged, 5);
    await handler.handleORMapSyncRespLeaf(staged.leafFrame(J, 5));
    await handler.handleORMapSyncRespLeaf(staged.leafFrame(K, 3));

    expect(onCoveringEpochApplied.mock.calls).toEqual([[MAP_NAME, 3]]);
  });

  it('confirms nothing when one of its leaves conveyed no epoch', async () => {
    const staged = stageClientBehindServer();
    const { handler, onCoveringEpochApplied } = staged;

    await openTwoRequestWalk(staged, 2);
    await handler.handleORMapSyncRespLeaf(staged.leafFrame(J));
    await handler.handleORMapSyncRespLeaf(staged.leafFrame(K, 2));

    expect(onCoveringEpochApplied.mock.calls).toEqual([]);
  });

  it('confirms nothing for a walk cut short by a new sync init, and a late leaf of it is applied without confirming', async () => {
    const staged = stageClientBehindServer();
    const { map, handler, sent, onCoveringEpochApplied, tK } = staged;

    await openTwoRequestWalk(staged, 2);
    await handler.handleORMapSyncRespLeaf(staged.leafFrame(J, 2));

    // The connection is replaced before K's leaf arrives.
    handler.sendSyncInit(MAP_NAME, 0);
    expect(sent.filter((msg) => msg.type === 'ORMAP_SYNC_INIT')).toHaveLength(1);
    const callsForTheOldWalk = [...onCoveringEpochApplied.mock.calls];

    // K's leaf of the old walk arrives with nothing requested on the new one.
    await handler.handleORMapSyncRespLeaf(staged.leafFrame(K, 2));

    // Its data is still applied.
    expect(map.get(K)).toEqual([]);
    expect(map.isTombstoned(tK)).toBe(true);
    expect({
      afterTheNewSyncInit: callsForTheOldWalk,
      afterTheLateLeaf: onCoveringEpochApplied.mock.calls,
    }).toEqual({ afterTheNewSyncInit: [], afterTheLateLeaf: [] });
  });

  it('confirms nothing when the handling of one of its leaves failed', async () => {
    const staged = stageClientBehindServer();
    const { handler, persistKeyTombstones, onCoveringEpochApplied } = staged;

    await openTwoRequestWalk(staged, 2);
    expect(persistKeyTombstones).not.toHaveBeenCalled();

    persistKeyTombstones.mockRejectedValueOnce(new Error('simulated persist failure'));
    await expect(handler.handleORMapSyncRespLeaf(staged.leafFrame(J, 2))).rejects.toThrow(
      'simulated persist failure',
    );
    await handler.handleORMapSyncRespLeaf(staged.leafFrame(K, 2));

    expect(onCoveringEpochApplied.mock.calls).toEqual([]);
  });

  it('confirms nothing while one of its requests is never answered', async () => {
    const staged = stageClientBehindServer();
    const { handler, onCoveringEpochApplied } = staged;

    await openTwoRequestWalk(staged, 2);
    await handler.handleORMapSyncRespLeaf(staged.leafFrame(J, 2));
    // K's request gets no reply. Let anything still queued run before looking.
    await new Promise<void>((resolve) => setImmediate(resolve));

    expect(onCoveringEpochApplied.mock.calls).toEqual([]);
  });

  it('still confirms at once on equal roots, once on a one-leaf walk, and once when a full-resync walk drains', async () => {
    // Equal roots: no walk is opened, the epoch is confirmed on the root.
    const inSync = makeHarness();
    inSync.map.add(K, 'k-value');
    await inSync.handler.handleORMapSyncRespRoot({
      mapName: MAP_NAME,
      rootHash: inSync.map.getMerkleTree().getRootHash(),
      coveringEpoch: 4,
    });
    expect(inSync.requestedPaths()).toEqual([]);
    expect(inSync.onCoveringEpochApplied.mock.calls).toEqual([[MAP_NAME, 4]]);

    // A walk of one request, answered by one leaf.
    const oneLeaf = stageClientBehindServer();
    await oneLeaf.handler.handleORMapSyncRespRoot(oneLeaf.rootFrame(4));
    expect(oneLeaf.requestedPaths()).toEqual(['']);
    expect(oneLeaf.onCoveringEpochApplied).not.toHaveBeenCalled();
    await oneLeaf.handler.handleORMapSyncRespLeaf(oneLeaf.leafFrameAt('', [K, J], 4));
    expect(oneLeaf.onCoveringEpochApplied.mock.calls).toEqual([[MAP_NAME, 4]]);
    expect(oneLeaf.map.get(K)).toEqual([]);

    // A full resync: the local map is discarded, the snapshot is pulled by one
    // request and confirmed when its leaf has been applied.
    const resync = stageClientBehindServer();
    resync.onFullResync.mockImplementation(async () => {
      resync.map.clear();
    });
    await resync.handler.handleORMapSyncRespRoot({ ...resync.rootFrame(4), fullResync: true });
    expect(resync.onFullResync).toHaveBeenCalledTimes(1);
    expect(resync.requestedPaths()).toEqual(['']);
    expect(resync.onCoveringEpochApplied).not.toHaveBeenCalled();
    await resync.handler.handleORMapSyncRespLeaf(resync.leafFrameAt('', [K, J], 4));
    expect(resync.onCoveringEpochApplied.mock.calls).toEqual([[MAP_NAME, 4]]);
    expect(resync.map.get(J)).toEqual(['j-live']);
  });

  it('a leaf of an earlier walk that finishes during the next walk neither confirms nor shortens that walk', async () => {
    const staged = stageClientBehindServer();
    const { map, handler, server, persistKey, onCoveringEpochApplied, requestedPaths, tK } = staged;

    // Walk 1: J's leaf is handed over and parks on storage before K's arrives.
    await openTwoRequestWalk(staged, 2);
    const parked = deferred();
    persistKey.mockImplementationOnce(() => parked.promise);
    const leafOfWalkOne = handler.handleORMapSyncRespLeaf(staged.leafFrame(J, 2));
    expect(persistKey).toHaveBeenCalledTimes(1);

    // The connection is replaced. The server gained a value for J meanwhile, so
    // J's subtree differs again on the new walk, and K's still does.
    server.add(J, 'j-added-while-away');
    handler.sendSyncInit(MAP_NAME, 0);
    const requestsOfWalkOne = requestedPaths().length;
    await handler.handleORMapSyncRespRoot(staged.rootFrame(3));
    expect(requestedPaths().slice(requestsOfWalkOne)).toEqual(['']);

    // Walk 1's leaf now finishes, with one request of walk 2 outstanding.
    parked.resolve();
    await leafOfWalkOne;
    expect(onCoveringEpochApplied).not.toHaveBeenCalled();

    await handler.handleORMapSyncRespBuckets(staged.bucketsFrame());
    expect(sorted(requestedPaths().slice(requestsOfWalkOne + 1))).toEqual(
      sorted([rootBucketOf(K), rootBucketOf(J)]),
    );
    await handler.handleORMapSyncRespLeaf(staged.leafFrame(J, 3));
    expect(onCoveringEpochApplied).not.toHaveBeenCalled();
    expect(map.get(K)).toEqual(['k-value']);

    await handler.handleORMapSyncRespLeaf(staged.leafFrame(K, 3));
    expect(onCoveringEpochApplied.mock.calls).toEqual([[MAP_NAME, 3]]);
    expect(map.get(K)).toEqual([]);
    expect(map.isTombstoned(tK)).toBe(true);
  });

  it('confirms at the end of a bucket response when that is the last response of the walk to finish', async () => {
    const staged = stageClientBehindServer({ localOnlyKey: true });
    const { handler, persistKeyTombstones, onCoveringEpochApplied, requestedPaths } = staged;

    await handler.handleORMapSyncRespRoot(staged.rootFrame(5));

    // The bucket response sends its two requests, then parks on the persist of
    // the attribution it cleared for the key only this client holds.
    const parked = deferred();
    persistKeyTombstones.mockImplementationOnce(() => parked.promise);
    const bucketResponse = handler.handleORMapSyncRespBuckets(staged.bucketsFrame());
    expect(persistKeyTombstones.mock.calls).toEqual([[MAP_NAME, [L]]]);
    expect(sorted(requestedPaths().slice(1))).toEqual(sorted([rootBucketOf(K), rootBucketOf(J)]));

    await handler.handleORMapSyncRespLeaf(staged.leafFrame(J, 5));
    await handler.handleORMapSyncRespLeaf(staged.leafFrame(K, 5));
    // Both leaves are applied, the bucket response has not finished.
    expect(onCoveringEpochApplied.mock.calls).toEqual([]);

    parked.resolve();
    await bucketResponse;
    expect(onCoveringEpochApplied.mock.calls).toEqual([[MAP_NAME, 5]]);
  });

  it('confirms nothing when the handling of a bucket response failed', async () => {
    const staged = stageClientBehindServer({ localOnlyKey: true });
    const { handler, persistKeyTombstones, onCoveringEpochApplied, requestedPaths } = staged;

    await handler.handleORMapSyncRespRoot(staged.rootFrame(5));

    persistKeyTombstones.mockRejectedValueOnce(new Error('simulated persist failure'));
    await expect(handler.handleORMapSyncRespBuckets(staged.bucketsFrame())).rejects.toThrow(
      'simulated persist failure',
    );
    expect(persistKeyTombstones.mock.calls).toEqual([[MAP_NAME, [L]]]);
    expect(sorted(requestedPaths().slice(1))).toEqual(sorted([rootBucketOf(K), rootBucketOf(J)]));

    await handler.handleORMapSyncRespLeaf(staged.leafFrame(J, 5));
    await handler.handleORMapSyncRespLeaf(staged.leafFrame(K, 5));

    expect(onCoveringEpochApplied.mock.calls).toEqual([]);
  });

  it('counts the epoch of the root that opened the walk: a lower one is what is confirmed, a missing one confirms nothing', async () => {
    const rootConveysTwo = stageClientBehindServer();
    await openTwoRequestWalk(rootConveysTwo, 2);
    await rootConveysTwo.handler.handleORMapSyncRespLeaf(rootConveysTwo.leafFrame(J, 5));
    await rootConveysTwo.handler.handleORMapSyncRespLeaf(rootConveysTwo.leafFrame(K, 3));

    const rootConveysNoEpoch = stageClientBehindServer();
    await openTwoRequestWalk(rootConveysNoEpoch);
    await rootConveysNoEpoch.handler.handleORMapSyncRespLeaf(rootConveysNoEpoch.leafFrame(J, 5));
    await rootConveysNoEpoch.handler.handleORMapSyncRespLeaf(rootConveysNoEpoch.leafFrame(K, 3));

    // One comparison for both halves, so that a failure shows what each did.
    expect({
      rootConveysTwo: rootConveysTwo.onCoveringEpochApplied.mock.calls,
      rootConveysNoEpoch: rootConveysNoEpoch.onCoveringEpochApplied.mock.calls,
    }).toEqual({
      rootConveysTwo: [[MAP_NAME, 2]],
      rootConveysNoEpoch: [],
    });
  });

  it('a full-resync root still discarding local state when the connection is lost opens no walk on the next connection', async () => {
    const staged = stageClientBehindServer();
    const { map, handler, onFullResync, onCoveringEpochApplied, requestedPaths } = staged;

    // The root of the first connection orders a full resync, and the discard of
    // the local state parks on storage.
    const parked = deferred();
    onFullResync.mockImplementationOnce(async () => {
      await parked.promise;
      map.clear();
    });
    const rootOfTheLostConnection = handler.handleORMapSyncRespRoot({
      ...staged.rootFrame(2),
      fullResync: true,
    });
    expect(onFullResync).toHaveBeenCalledTimes(1);

    // The connection drops and the discard finishes afterwards. A request sent
    // now would go out on the next connection ahead of its sync init, and its
    // answer would be taken for an answer of the walk that init opens.
    handler.onConnectionLost();
    parked.resolve();
    await rootOfTheLostConnection;
    expect(requestedPaths()).toEqual([]);

    // The next connection's own root opens the only walk, and that walk
    // confirms once, when its single request has been answered.
    handler.sendSyncInit(MAP_NAME, 0);
    await handler.handleORMapSyncRespRoot(staged.rootFrame(3));
    expect(requestedPaths()).toEqual(['']);
    expect(onCoveringEpochApplied).not.toHaveBeenCalled();
    await handler.handleORMapSyncRespLeaf(staged.leafFrameAt('', [K, J], 3));
    expect(onCoveringEpochApplied.mock.calls).toEqual([[MAP_NAME, 3]]);
  });
});
