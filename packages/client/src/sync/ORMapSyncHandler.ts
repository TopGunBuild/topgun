import { ORMap } from '@topgunbuild/core';
import type { ORMapMerkleTree, ORMapRecord } from '@topgunbuild/core';
import type { IORMapSyncHandler, ORMapSyncHandlerConfig } from './types';
import { logger } from '../utils/logger';

/**
 * ORMapSyncHandler
 *
 * Handles Merkle tree synchronization protocol messages for ORMap.
 * Manages sync state, root hash comparison, bucket traversal, leaf merging,
 * and bidirectional diff exchange.
 */
export class ORMapSyncHandler implements IORMapSyncHandler {
  private readonly config: ORMapSyncHandlerConfig;
  private lastSyncTimestamp: number = 0;

  constructor(config: ORMapSyncHandlerConfig) {
    this.config = config;
  }

  /**
   * Handle ORMAP_SYNC_RESP_ROOT message from server.
   * Compares root hashes and requests buckets if mismatch detected.
   */
  public async handleORMapSyncRespRoot(payload: {
    mapName: string;
    rootHash: number;
    coveringEpoch?: number;
    fullResync?: boolean;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- server timestamp shape is implementation-defined (HLC or raw ms); passed through to onTimestampUpdate without inspection
    timestamp?: any;
  }): Promise<void> {
    const { mapName, rootHash, coveringEpoch, fullResync, timestamp } = payload;
    const map = this.config.getMap(mapName);
    if (map instanceof ORMap) {
      if (fullResync) {
        // Authoritative REPLACE resync: the server has FORGOTTEN or found this
        // client REGRESSED, so an incremental delta could re-admit a record whose
        // tombstone was already pruned (the server no longer holds the per-tag
        // epoch to suppress it). DISCARD the materialized local OR-Map AND every
        // pending pre-snapshot op, then pull the full server snapshot: the now-empty
        // local tree makes the merkle walk transfer the server's entire state. The
        // covering epoch is deliberately NOT confirmed here — it is confirmed only
        // after the snapshot leaves are durably applied, which is what re-enables
        // this client's ACKs server-side (delivered_conn set on resync completion).
        await this.config.onFullResync(mapName, timestamp);
        logger.info(
          { mapName },
          'ORMap full-resync REPLACE: discarded local state, pulling snapshot',
        );
        this.config.sendMessage({
          type: 'ORMAP_MERKLE_REQ_BUCKET',
          payload: { mapName, path: '' },
        });
        if (timestamp) {
          await this.config.onTimestampUpdate(timestamp);
        }
        return;
      }
      const localTree = map.getMerkleTree();
      if (rootHash === 0) {
        // A zero root means the server holds no leaf for this map at all, so it
        // attributes no tombstone to any key. Mirror that before comparing: a
        // key kept in the local tree only by stale attribution would otherwise
        // hold the roots apart, and the bucket request a mismatch sends gets no
        // answer from an empty server tree, so nothing would ever clear it.
        const changed = this.resetKeyTombstonesToPending(
          map,
          localKeysUnder(localTree, ''),
          this.pendingRemoveLookup(mapName),
        );
        await this.persistAttributionIfChanged(mapName, changed);
      }
      const localRootHash = localTree.getRootHash();

      if (localRootHash !== rootHash) {
        logger.info(
          { mapName, localRootHash, remoteRootHash: rootHash },
          'ORMap root hash mismatch, requesting buckets',
        );
        this.config.sendMessage({
          type: 'ORMAP_MERKLE_REQ_BUCKET',
          payload: { mapName, path: '' },
        });
        // Empty-diff liveness does NOT apply here: the roots differ, so the
        // client does not yet hold the covering-epoch tombstone set. It ACKs the
        // covering epoch only AFTER applying the leaves/diff that follow.
      } else {
        logger.info({ mapName }, 'ORMap is in sync');
        // Empty diff: the roots match, so the client demonstrably already holds
        // the full tombstone set up to the covering epoch (the OR-Map leaf hash
        // covers the tombstone tags). Confirm it now so an up-to-date client
        // still advances its cursor instead of pinning the server low-water-mark.
        this.confirmCoveringEpoch(mapName, coveringEpoch);
      }
    }
    // Update HLC with server timestamp
    if (timestamp) {
      await this.config.onTimestampUpdate(timestamp);
    }
  }

  /**
   * ACK the conveyed covering epoch after the client has durably applied the
   * matching OR-Map sync data for `mapName`. A no-op when the server conveyed no
   * epoch (nothing stamped yet). The underlying device-wide ACK is
   * cumulative-monotonic AND gated by the cross-map min-barrier (see
   * `SyncEngine.applyMapCoverage`) — this call reports only THIS map's coverage,
   * it does not by itself guarantee a CLIENT_APPLY_ACK is sent.
   */
  private confirmCoveringEpoch(mapName: string, coveringEpoch?: number): void {
    if (typeof coveringEpoch === 'number' && Number.isFinite(coveringEpoch) && coveringEpoch > 0) {
      this.config.onCoveringEpochApplied(mapName, coveringEpoch);
    }
  }

  /**
   * Handle ORMAP_SYNC_RESP_BUCKETS message from server.
   * Compares bucket hashes and requests mismatched buckets.
   * Also pushes local data that server doesn't have.
   */
  public async handleORMapSyncRespBuckets(payload: {
    mapName: string;
    path: string;
    buckets: Record<string, number>;
  }): Promise<void> {
    const { mapName, path, buckets } = payload;
    const map = this.config.getMap(mapName);
    if (map instanceof ORMap) {
      const tree = map.getMerkleTree();
      const localBuckets = tree.getBuckets(path);

      // A child the server lists with hash 0 is the residue of an emptied
      // bucket and means exactly what an unlisted child means: the server holds
      // nothing there. Both loops read this one normalised view, so such a
      // child is never requested (the server answers a request for an empty
      // path with no message at all) and is treated as absent below.
      const serverBuckets = new Map<string, number>();
      for (const [bucketKey, remoteHash] of Object.entries(buckets)) {
        if (remoteHash !== 0) serverBuckets.set(bucketKey, remoteHash);
      }

      for (const [bucketKey, remoteHash] of serverBuckets) {
        const localHash = localBuckets[bucketKey] || 0;
        if (localHash !== remoteHash) {
          const newPath = path + bucketKey;
          this.config.sendMessage({
            type: 'ORMAP_MERKLE_REQ_BUCKET',
            payload: { mapName, path: newPath },
          });
        }
      }

      // Also check for buckets that exist locally but not on remote
      const pendingFor = this.pendingRemoveLookup(mapName);
      let attributionChanged = false;
      const localOnlyPaths: string[] = [];
      for (const [bucketKey, localHash] of Object.entries(localBuckets)) {
        if (!serverBuckets.has(bucketKey) && localHash !== 0) {
          const newPath = path + bucketKey;
          // The server holds no leaf under this child, so it attributes no
          // tombstone to any key there. Drop the mirrored attribution first: a
          // key held in the tree only by tombstones the server no longer has
          // would otherwise keep this bucket different on every later walk.
          if (this.resetKeyTombstonesToPending(map, localKeysUnder(tree, newPath), pendingFor)) {
            attributionChanged = true;
          }
          localOnlyPaths.push(newPath);
        }
      }

      await this.persistAttributionIfChanged(mapName, attributionChanged);

      for (const newPath of localOnlyPaths) {
        // Local has data that remote doesn't - need to push
        const keys = tree.getKeysInBucket(newPath);
        if (keys.length > 0) {
          await this.pushORMapDiff(mapName, keys, map);
        }
      }
    }
  }

  /**
   * Handle ORMAP_SYNC_RESP_LEAF message from server.
   * Merges leaf entries into local map and pushes local diff back.
   */
  public async handleORMapSyncRespLeaf(payload: {
    mapName: string;
    path?: string;
    coveringEpoch?: number;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- records in the leaf entries are raw ORMapRecord objects decoded from msgpack; value type is erased at the sync protocol layer
    entries: Array<{ key: string; records: any[]; tombstones: string[] }>;
  }): Promise<void> {
    const { mapName, path, coveringEpoch, entries } = payload;
    const map = this.config.getMap(mapName);
    if (map instanceof ORMap) {
      let totalAdded = 0;
      let totalUpdated = 0;
      let attributionChanged = false;
      const pendingFor = this.pendingRemoveLookup(mapName);

      // The keys this client holds under the leaf's path, taken before the
      // merge so that a key the response introduces is not mistaken for one
      // the server left out. Without a path the response's scope is unknown and
      // no absence can be inferred from it.
      const localKeysInScope =
        typeof path === 'string' ? localKeysUnder(map.getMerkleTree(), path) : [];

      for (const entry of entries) {
        const { key, records, tombstones } = entry;
        if (this.mirrorKeyTombstones(map, key, tombstones, pendingFor)) {
          attributionChanged = true;
        }
        const result = map.mergeKey(key, records, tombstones);
        totalAdded += result.added;
        totalUpdated += result.updated;
        // Persist server-origin merge so it survives an offline reload (symmetric with LWW).
        await this.config.persistKey(mapName, key);
      }

      // A leaf response lists every key the server holds under its path, so a
      // local key it leaves out has no leaf on the server and therefore no
      // server-attributed tombstone.
      const listed = new Set(entries.map((e: { key: string }) => e.key));
      const omitted = localKeysInScope.filter((key) => !listed.has(key));
      if (this.resetKeyTombstonesToPending(map, omitted, pendingFor)) {
        attributionChanged = true;
      }

      await this.persistAttributionIfChanged(mapName, attributionChanged);

      if (totalAdded > 0 || totalUpdated > 0) {
        await this.config.persistTombstones(mapName);
        logger.info(
          { mapName, added: totalAdded, updated: totalUpdated },
          'Synced ORMap records from server',
        );
      }

      // The leaf entries (including their tombstone tags) are now durably
      // applied — confirm the covering epoch so the server's cursor advances.
      this.confirmCoveringEpoch(mapName, coveringEpoch);

      // Now push any local records that server might not have
      const keysToCheck = entries.map((e: { key: string }) => e.key);
      await this.pushORMapDiff(mapName, keysToCheck, map);
    }
  }

  /**
   * Handle ORMAP_DIFF_RESPONSE message from server.
   * Merges diff entries into local map.
   */
  public async handleORMapDiffResponse(payload: {
    mapName: string;
    coveringEpoch?: number;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- records in the diff response are raw ORMapRecord objects decoded from msgpack; value type is erased at the sync protocol layer
    entries: Array<{ key: string; records: any[]; tombstones: string[] }>;
  }): Promise<void> {
    const { mapName, coveringEpoch, entries } = payload;
    const map = this.config.getMap(mapName);
    if (map instanceof ORMap) {
      let totalAdded = 0;
      let totalUpdated = 0;
      let attributionChanged = false;
      const pendingFor = this.pendingRemoveLookup(mapName);

      for (const entry of entries) {
        const { key, records, tombstones } = entry;
        if (this.mirrorKeyTombstones(map, key, tombstones, pendingFor)) {
          attributionChanged = true;
        }
        const result = map.mergeKey(key, records, tombstones);
        totalAdded += result.added;
        totalUpdated += result.updated;
        // Persist server-origin merge so it survives an offline reload (symmetric with LWW).
        await this.config.persistKey(mapName, key);
      }

      await this.persistAttributionIfChanged(mapName, attributionChanged);

      if (totalAdded > 0 || totalUpdated > 0) {
        await this.config.persistTombstones(mapName);
        logger.info(
          { mapName, added: totalAdded, updated: totalUpdated },
          'Merged ORMap diff from server',
        );
      }

      // The diff entries (including tombstone tags) are now durably applied —
      // confirm the covering epoch so the server's cursor advances.
      this.confirmCoveringEpoch(mapName, coveringEpoch);
    }
  }

  /**
   * The un-acknowledged local remove tags per key for one handler invocation.
   *
   * The op log is read at most once per invocation, on the first key that asks,
   * however many keys the response covers: a reset can span every key of the
   * map, and a read per key would make it keys times op-log entries. The result
   * is held only by the returned closure, so the next invocation reads the op
   * log again and an op acknowledged in between is no longer pending.
   */
  private pendingRemoveLookup(mapName: string): PendingRemoveLookup {
    let byKey: Map<string, string[]> | undefined;
    return (key) => {
      byKey ??= this.config.getPendingRemoveTagsByKey(mapName);
      return byKey.get(key) ?? NO_TAGS;
    };
  }

  /**
   * Make the server's tombstone set for `key` this client's attributed set,
   * keeping the tags of this client's removes the server has not acknowledged
   * yet (it cannot report a remove it has not applied).
   *
   * Must run BEFORE the entry's `mergeKey`: every attributed tag is then already
   * suppressed map-wide when the records are merged, so a record carrying one is
   * never admitted and no subscriber is shown a value that is being removed.
   *
   * This is a replace, not a union: a tag the server no longer attributes to the
   * key leaves the attributed set (it stays suppressed map-wide), which is what
   * lets the key's leaf match the server's (TG-MRK-001).
   *
   * @returns whether the key's attributed set changed
   */
  private mirrorKeyTombstones(
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- ORMap value type is erased at the sync handler layer
    map: ORMap<any, any>,
    key: string,
    serverTombstones: Iterable<string>,
    pendingFor: PendingRemoveLookup,
  ): boolean {
    const next = new Set(serverTombstones);
    for (const tag of pendingFor(key)) {
      next.add(tag);
    }
    const previous = map.getKeyTombstones(key);
    map.setKeyTombstones(key, next);
    return !sameTags(previous, next);
  }

  /**
   * The server's view shows no leaf for these keys, so it attributes no
   * tombstone to them: each key keeps only the tags of this client's own
   * not-yet-acknowledged removes. The map-wide suppression set is untouched, so
   * a tag dropped here stays removed.
   *
   * @returns whether any key's attributed set changed
   */
  private resetKeyTombstonesToPending(
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- ORMap value type is erased at the sync handler layer
    map: ORMap<any, any>,
    keys: string[],
    pendingFor: PendingRemoveLookup,
  ): boolean {
    let changed = false;
    for (const key of keys) {
      if (this.mirrorKeyTombstones(map, key, [], pendingFor)) {
        changed = true;
      }
    }
    return changed;
  }

  /**
   * Persist the per-key attribution when this invocation changed it. Separate
   * from the record/tombstone persist because attribution changes on responses
   * that add and update nothing, and an unchanged walk must not write at all.
   *
   * A rejection is not caught here: it propagates out of the handler exactly as
   * a failed record or tombstone persist does, so the covering epoch that
   * follows is not confirmed on top of attribution that never reached disk.
   */
  private async persistAttributionIfChanged(mapName: string, changed: boolean): Promise<void> {
    if (changed) {
      await this.config.persistKeyTombstones(mapName);
    }
  }

  /**
   * Push local ORMap diff to server for the given keys.
   * Sends local records, and for each key only the tombstones attributed to it.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- ORMap value type is erased at the sync handler layer; actual V type lives in the map instance generic at TopGunClient level
  public async pushORMapDiff(mapName: string, keys: string[], map: ORMap<any, any>): Promise<void> {
    const entries: Array<{
      key: string;
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- ORMapRecord value type is erased at the diff protocol layer; records are passed through to the server without inspection
      records: ORMapRecord<any>[];
      tombstones: string[];
    }> = [];

    for (const key of keys) {
      const recordsMap = map.getRecordsMap(key);
      if (recordsMap && recordsMap.size > 0) {
        // Get records as array
        const records = Array.from(recordsMap.values());

        // Only the tombstones attributed to this key. The server unions whatever
        // an entry carries into that key's own set without filtering, so sending
        // the map-wide set would make every key's set depend on push history,
        // which no client can reproduce.
        const tombstones = Array.from(map.getKeyTombstones(key));

        entries.push({
          key,
          records,
          tombstones,
        });
      }
    }

    if (entries.length > 0) {
      this.config.sendMessage({
        type: 'ORMAP_PUSH_DIFF',
        payload: {
          mapName,
          entries,
        },
      });
      logger.debug({ mapName, keyCount: entries.length }, 'Pushed ORMap diff to server');
    }
  }

  /**
   * Send ORMAP_SYNC_INIT message to server to start sync.
   * Encapsulates sync init message construction.
   */
  public sendSyncInit(mapName: string, lastSyncTimestamp: number): void {
    this.lastSyncTimestamp = lastSyncTimestamp;
    const map = this.config.getMap(mapName);
    if (map instanceof ORMap) {
      logger.info({ mapName }, 'Starting Merkle sync for ORMap');
      const tree = map.getMerkleTree();
      const rootHash = tree.getRootHash();

      // Build bucket hashes for all non-empty buckets at depth 0
      const bucketHashes: Record<string, number> = tree.getBuckets('');

      this.config.sendMessage({
        type: 'ORMAP_SYNC_INIT',
        mapName,
        rootHash,
        bucketHashes,
        lastSyncTimestamp,
        // Report the client's confirmed-apply cursor so the server can detect a
        // REGRESSED replica (claim < stored cursor) and route it to a full resync.
        claimedEpoch: this.config.getClaimedEpoch(),
      });
    }
  }

  /**
   * Get the last sync timestamp for debugging/testing.
   */
  public getLastSyncTimestamp(): number {
    return this.lastSyncTimestamp;
  }
}

/**
 * Every key the tree holds at or below `path`.
 *
 * The tree, not the map's key list, is the source: a key with no records left
 * but tombstones still attributed to it lives only in the tree, and those are
 * exactly the keys whose attribution an absence must be able to clear.
 */
function localKeysUnder(tree: ORMapMerkleTree, path: string): string[] {
  const start = tree.getNode(path);
  if (!start) return [];

  const keys: string[] = [];
  const pending = [start];
  for (let node = pending.pop(); node !== undefined; node = pending.pop()) {
    if (node.entries) {
      for (const key of node.entries.keys()) keys.push(key);
    }
    if (node.children) {
      for (const child of Object.values(node.children)) pending.push(child);
    }
  }
  return keys;
}

/** The un-acknowledged local remove tags of one key, for one handler invocation. */
type PendingRemoveLookup = (key: string) => readonly string[];

const NO_TAGS: readonly string[] = [];

function sameTags(a: ReadonlySet<string>, b: ReadonlySet<string>): boolean {
  if (a.size !== b.size) return false;
  for (const tag of a) {
    if (!b.has(tag)) return false;
  }
  return true;
}
