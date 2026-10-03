import { HLC, ORMap, ORMapMerkleTree, hashString } from '@topgunbuild/core';
import type { ORMapRecord, Timestamp } from '@topgunbuild/core';
import { ORMapSyncHandler } from '../sync/ORMapSyncHandler';
import { logger } from '../utils/logger';

/**
 * A client whose OR-Map leaves were hashed with the formula that preceded the
 * canonical leaf (tags and per-key tombstones only) still has to converge
 * against a server that reports one flat trie of canonical leaves: its root
 * can never match, so every sync is a full walk, and that walk has to end.
 */

const MAP_NAME = 'tags';
const COVERING_EPOCH = 3;

// The three helpers below reproduce, unchanged, the leaf hash a client computed
// before the canonical leaf existed. Source: `hashORMapEntry`, `stringifyValue`
// and `timestampToString` in packages/core/src/ORMapMerkle.ts as of commit
// eb03e214^1 (`git show eb03e214^1:packages/core/src/ORMapMerkle.ts`). That
// formula hashed values, timestamps and TTLs, which the server never did, so
// it is kept here only to stand in for a client that has not been upgraded.
function oldTimestampToString(ts: Timestamp): string {
  return `${ts.millis}:${ts.counter}:${ts.nodeId}`;
}

function oldStringifyValue(value: unknown): string {
  if (value === null || value === undefined) {
    return String(value);
  }
  if (typeof value === 'object') {
    return JSON.stringify(value, Object.keys(value as Record<string, unknown>).sort());
  }
  return String(value);
}

function oldHashORMapEntry<V>(key: string, records: Map<string, ORMapRecord<V>>): number {
  const sortedTags = Array.from(records.keys()).sort();
  const parts: string[] = [`key:${key}`];

  for (const tag of sortedTags) {
    const record = records.get(tag)!;
    const valuePart = oldStringifyValue(record.value);

    let recordStr = `${tag}:${valuePart}:${oldTimestampToString(record.timestamp)}`;
    if (record.ttlMs !== undefined) {
      recordStr += `:ttl=${record.ttlMs}`;
    }
    parts.push(recordStr);
  }

  return hashString(parts.join('|'));
}

/** The child of the root that holds `key` in a trie of canonical leaves. */
function rootBucketOf(key: string): string {
  const probe = new ORMapMerkleTree();
  probe.update(key, ['probe'], []);
  return Object.keys(probe.getBuckets(''))[0];
}

/** Two keys that sit under different children of the root. */
function twoKeysInDistinctRootBuckets(): [string, string] {
  const first = 'key-0';
  for (let i = 1; ; i++) {
    const candidate = `key-${i}`;
    if (rootBucketOf(candidate) !== rootBucketOf(first)) return [first, candidate];
  }
}

const [K1, K2] = twoKeysInDistinctRootBuckets();

function serverRecord(value: string, counter: number): ORMapRecord<string> {
  const timestamp: Timestamp = { millis: 1_700_000_000_000, counter, nodeId: 'server' };
  return { value, timestamp, tag: HLC.toString(timestamp) };
}

// What the server holds: one live record per key, no tombstones.
const SERVER_RECORDS: Record<string, ORMapRecord<string>> = {
  [K1]: serverRecord('k1-value', 0),
  [K2]: serverRecord('k2-value', 1),
};

/** The server's flat trie: one trie for the map, built from canonical leaves. */
function buildFlatServerTree(): ORMapMerkleTree {
  const tree = new ORMapMerkleTree();
  for (const [key, record] of Object.entries(SERVER_RECORDS)) {
    tree.update(key, [record.tag], []);
  }
  return tree;
}

/** The frame types the mocked server replies with. */
type ServerFrame =
  | {
      type: 'ORMAP_SYNC_RESP_ROOT';
      payload: { mapName: string; rootHash: number; coveringEpoch: number };
    }
  | {
      type: 'ORMAP_SYNC_RESP_BUCKETS';
      payload: { mapName: string; path: string; buckets: Record<string, number> };
    }
  | {
      type: 'ORMAP_SYNC_RESP_LEAF';
      payload: {
        mapName: string;
        path: string;
        coveringEpoch: number;
        entries: Array<{ key: string; records: ORMapRecord<string>[]; tombstones: string[] }>;
      };
    };

describe('a client with the previous OR-Map leaf formula against a flat server', () => {
  let errorSpy: jest.SpyInstance;

  beforeEach(() => {
    errorSpy = jest.spyOn(logger, 'error');
  });

  afterEach(() => {
    errorSpy.mockRestore();
  });

  test('old-formula client converges through exactly one full walk against a flat server', async () => {
    const flat = buildFlatServerTree();

    // The client holds the same two records as the server, but its tree
    // carries the previous formula's leaves, put there through the seam that
    // stores an already-computed leaf hash.
    const map = new ORMap<string, string>(new HLC('old-client'));
    for (const [key, record] of Object.entries(SERVER_RECORDS)) {
      map.apply(key, record);
    }
    for (const key of Object.keys(SERVER_RECORDS)) {
      map.getMerkleTree().updateLeafHash(key, oldHashORMapEntry(key, map.getRecordsMap(key)!));
    }
    expect(flat.getRootHash()).not.toBe(0);
    expect(map.getMerkleTree().getRootHash()).not.toBe(flat.getRootHash());

    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- captured sent messages have no fixed shape at the test boundary
    const sent: any[] = [];
    const onCoveringEpochApplied = jest.fn<void, [string, number]>();
    const handler = new ORMapSyncHandler({
      getMap: () => map,
      sendMessage: (msg) => {
        sent.push(msg);
        return true;
      },
      hlc: new HLC('old-client'),
      onTimestampUpdate: async () => {},
      persistKey: async () => {},
      persistTombstones: async () => {},
      getPendingRemoveTagsByKey: () => new Map(),
      persistKeyTombstones: async () => {},
      onCoveringEpochApplied,
      onFullResync: async () => {},
      getClaimedEpoch: () => 0,
    });

    /**
     * The flat server's reply to one outbound frame, or undefined when it
     * sends none. A root reply carries no `fullResync` key: the server leaves
     * the field out unless a full resync is needed.
     */
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the outbound frame is whatever the handler sent
    const replyTo = (msg: any): ServerFrame | undefined => {
      if (msg.type === 'ORMAP_SYNC_INIT') {
        return {
          type: 'ORMAP_SYNC_RESP_ROOT',
          payload: {
            mapName: MAP_NAME,
            rootHash: flat.getRootHash(),
            coveringEpoch: COVERING_EPOCH,
          },
        };
      }
      if (msg.type !== 'ORMAP_MERKLE_REQ_BUCKET') return undefined;
      const path: string = msg.payload.path;
      if (flat.isLeaf(path)) {
        return {
          type: 'ORMAP_SYNC_RESP_LEAF',
          payload: {
            mapName: MAP_NAME,
            path,
            coveringEpoch: COVERING_EPOCH,
            entries: flat.getKeysInBucket(path).map((key) => ({
              key,
              records: [SERVER_RECORDS[key]],
              tombstones: [],
            })),
          },
        };
      }
      if (flat.getNode(path) === undefined) {
        throw new Error(`the client requested a path the flat server does not hold: "${path}"`);
      }
      return {
        type: 'ORMAP_SYNC_RESP_BUCKETS',
        payload: { mapName: MAP_NAME, path, buckets: flat.getBuckets(path) },
      };
    };

    const countSent = (type: string): number => sent.filter((msg) => msg.type === type).length;

    const rejections: unknown[] = [];
    const leafPathsHandled: string[] = [];
    /** How many confirms had been made when each leaf response was handed over. */
    const confirmsSeenAtLeafEntry: number[] = [];
    let walkFramesWhenLastLeafResolved = { inits: 0, bucketRequests: 0 };

    handler.sendSyncInit(MAP_NAME, 0);

    // The mock answers every frame the client sends, in the order sent, and
    // nothing else. The loop ends when the client has stopped asking.
    for (let next = 0; next < sent.length; next++) {
      const reply = replyTo(sent[next]);
      if (reply === undefined) continue;
      try {
        if (reply.type === 'ORMAP_SYNC_RESP_ROOT') {
          expect('fullResync' in reply.payload).toBe(false);
          await handler.handleORMapSyncRespRoot(reply.payload);
        } else if (reply.type === 'ORMAP_SYNC_RESP_BUCKETS') {
          await handler.handleORMapSyncRespBuckets(reply.payload);
        } else {
          confirmsSeenAtLeafEntry.push(onCoveringEpochApplied.mock.calls.length);
          await handler.handleORMapSyncRespLeaf(reply.payload);
          leafPathsHandled.push(reply.payload.path);
          walkFramesWhenLastLeafResolved = {
            inits: countSent('ORMAP_SYNC_INIT'),
            bucketRequests: countSent('ORMAP_MERKLE_REQ_BUCKET'),
          };
        }
      } catch (error) {
        rejections.push(error);
      }
    }
    // Anything the last handler left queued runs before the frames are counted.
    await new Promise<void>((resolve) => setImmediate(resolve));

    // No handler promise rejected and the handler logged no error.
    expect(rejections).toEqual([]);
    expect(errorSpy).not.toHaveBeenCalled();

    // Exactly one walk: one init, a descent that reached both leaves, and
    // nothing asked again once the last leaf response had been handled.
    expect(countSent('ORMAP_SYNC_INIT')).toBe(1);
    expect(countSent('ORMAP_MERKLE_REQ_BUCKET')).toBeGreaterThanOrEqual(1);
    expect(leafPathsHandled).toHaveLength(2);
    expect({
      inits: countSent('ORMAP_SYNC_INIT'),
      bucketRequests: countSent('ORMAP_MERKLE_REQ_BUCKET'),
    }).toEqual(walkFramesWhenLastLeafResolved);

    // The map ends holding both keys' values.
    expect(map.get(K1)).toEqual(['k1-value']);
    expect(map.get(K2)).toEqual(['k2-value']);

    // The walk is confirmed once, with the conveyed epoch, and not before its
    // last leaf response was handed over.
    expect(onCoveringEpochApplied.mock.calls).toEqual([[MAP_NAME, COVERING_EPOCH]]);
    expect(confirmsSeenAtLeafEntry).toEqual([0, 0]);
  });
});
