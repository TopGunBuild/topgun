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
  /** The Merkle walk in progress per map name; see {@link WalkState}. */
  private readonly walks = new Map<string, WalkState>();

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
        this.openWalk(mapName, coveringEpoch);
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
        const changedKeys: string[] = [];
        this.resetKeyTombstonesToPending(
          map,
          localKeysUnder(localTree, ''),
          this.pendingRemoveLookup(mapName),
          changedKeys,
        );
        await this.persistAttributionIfChanged(mapName, changedKeys);
      }
      const localRootHash = localTree.getRootHash();

      if (localRootHash !== rootHash) {
        logger.info(
          { mapName, localRootHash, remoteRootHash: rootHash },
          'ORMap root hash mismatch, requesting buckets',
        );
        this.openWalk(mapName, coveringEpoch);
        this.config.sendMessage({
          type: 'ORMAP_MERKLE_REQ_BUCKET',
          payload: { mapName, path: '' },
        });
        // Empty-diff liveness does NOT apply here: the roots differ, so the
        // client does not yet hold the covering-epoch tombstone set. It ACKs the
        // covering epoch only once the walk this request opens has drained.
      } else {
        logger.info({ mapName }, 'ORMap is in sync');
        // Empty diff: the roots match, and a root is a function of every key's
        // records and attributed tombstones, so this client holds the state the
        // server's walk snapshot holds. The epoch a response conveys never
        // postdates the snapshot its round's walk descends by, so every
        // tombstone stamped at or below it is in that state, and confirming it
        // cannot let the server prune one this client lacks (TG-MRK-002).
        // Confirm it now so an up-to-date client still advances its cursor
        // instead of pinning the server low-water-mark.
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

  private walkFor(mapName: string): WalkState {
    let walk = this.walks.get(mapName);
    if (!walk) {
      walk = newWalkState(0);
      this.walks.set(mapName, walk);
    }
    return walk;
  }

  /**
   * Capture, synchronously at handler entry, which walk a bucket or leaf
   * response belongs to.
   *
   * The transport does not await handlers and one handler serves every
   * connection, so a response parked on storage can outlive the connection it
   * arrived on and finish inside the walk of the next one. The generation read
   * here, before the invocation first yields, is the only thing that still
   * tells the two apart at that point.
   *
   * A response that arrives while nothing is outstanding answers no request of
   * the current walk (a late frame of a walk a new sync init has replaced, or
   * one after the walk already drained). What it carries cannot be placed in
   * the walk's accounting, so the generation is voided: nothing is confirmed
   * for the map until the next sync init.
   */
  private enterWalk(mapName: string): WalkEntry {
    const walk = this.walkFor(mapName);
    const counted = walk.outstanding > 0;
    if (!counted) {
      walk.voided = true;
    }
    return { generation: walk.generation, counted };
  }

  /** The walk an invocation may still account to, or nothing if it is stale or uncounted. */
  private walkOfEntry(mapName: string, entry: WalkEntry): WalkState | undefined {
    const walk = this.walkFor(mapName);
    // An invocation that entered under an earlier generation is applied as
    // data only: its epoch and its completion belong to a walk that no longer
    // exists, and must neither lower nor shorten the current one.
    if (walk.generation !== entry.generation) return undefined;
    return entry.counted ? walk : undefined;
  }

  /** One more ORMAP_MERKLE_REQ_BUCKET of the map's walk is out. */
  private countRequest(mapName: string): void {
    this.walkFor(mapName).outstanding += 1;
  }

  /**
   * Count the request that opens a walk and bound the walk by the epoch its
   * root conveyed. A root without a usable epoch voids the walk: there is then
   * no bound under which its leaves could be confirmed.
   */
  private openWalk(mapName: string, rootEpoch?: number): void {
    const walk = this.walkFor(mapName);
    walk.outstanding += 1;
    foldEpoch(walk, rootEpoch);
  }

  private foldLeafEpoch(mapName: string, entry: WalkEntry, coveringEpoch?: number): void {
    const walk = this.walkOfEntry(mapName, entry);
    if (!walk) return;
    foldEpoch(walk, coveringEpoch);
    walk.foldedLeaf = true;
  }

  /**
   * The end of a bucket or leaf invocation: its merges and persists are done
   * and every request it sent has been counted, so its own request is no
   * longer outstanding.
   */
  private leaveWalk(mapName: string, entry: WalkEntry, threw: boolean): void {
    const walk = this.walkOfEntry(mapName, entry);
    if (!walk) return;
    if (threw) {
      // Part of this response may not have reached disk, so the walk can never
      // claim to hold everything up to its epoch.
      walk.failed = true;
    }
    walk.outstanding -= 1;

    // Confirm only a drained walk. An epoch acknowledged while part of the
    // walk is still outstanding moves this client's cursor on the server, which
    // may then prune a tombstone this client has not applied yet; the value
    // that tombstone removed would come back on this client's next push
    // (TG-MRK-002).
    if (walk.outstanding !== 0 || walk.failed || walk.voided || !walk.foldedLeaf) return;
    const confirmed = walk.minEpoch;
    walk.minEpoch = undefined;
    walk.foldedLeaf = false;
    this.confirmCoveringEpoch(mapName, confirmed);
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
    // Read before the first await: see `enterWalk`.
    const entry = this.enterWalk(payload.mapName);
    let threw = false;
    try {
      await this.applyBucketsResponse(payload);
    } catch (error) {
      threw = true;
      throw error;
    } finally {
      this.leaveWalk(payload.mapName, entry, threw);
    }
  }

  private async applyBucketsResponse(payload: {
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
          this.countRequest(mapName);
          this.config.sendMessage({
            type: 'ORMAP_MERKLE_REQ_BUCKET',
            payload: { mapName, path: newPath },
          });
        }
      }

      // Also check for buckets that exist locally but not on remote
      const pendingFor = this.pendingRemoveLookup(mapName);
      const changedKeys: string[] = [];
      const localOnlyPaths: string[] = [];
      for (const [bucketKey, localHash] of Object.entries(localBuckets)) {
        if (!serverBuckets.has(bucketKey) && localHash !== 0) {
          const newPath = path + bucketKey;
          // The server holds no leaf under this child, so it attributes no
          // tombstone to any key there. Drop the mirrored attribution first: a
          // key held in the tree only by tombstones the server no longer has
          // would otherwise keep this bucket different on every later walk.
          this.resetKeyTombstonesToPending(
            map,
            localKeysUnder(tree, newPath),
            pendingFor,
            changedKeys,
          );
          localOnlyPaths.push(newPath);
        }
      }

      await this.persistAttributionIfChanged(mapName, changedKeys);

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
    // Read before the first await: see `enterWalk`.
    const entry = this.enterWalk(payload.mapName);
    let threw = false;
    try {
      await this.applyLeafResponse(payload, entry);
    } catch (error) {
      threw = true;
      throw error;
    } finally {
      this.leaveWalk(payload.mapName, entry, threw);
    }
  }

  private async applyLeafResponse(
    payload: {
      mapName: string;
      path?: string;
      coveringEpoch?: number;
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- records in the leaf entries are raw ORMapRecord objects decoded from msgpack; value type is erased at the sync protocol layer
      entries: Array<{ key: string; records: any[]; tombstones: string[] }>;
    },
    entry: WalkEntry,
  ): Promise<void> {
    const { mapName, path, coveringEpoch, entries } = payload;
    const map = this.config.getMap(mapName);
    if (map instanceof ORMap) {
      let totalAdded = 0;
      let totalUpdated = 0;
      const changedKeys: string[] = [];
      const pendingFor = this.pendingRemoveLookup(mapName);
      // Keys the server serves a record for under a tag this client has
      // tombstoned: each is pushed below even if it holds no live record here.
      const healedKeys = new Set<string>();

      // The keys this client holds under the leaf's path, taken before the
      // merge so that a key the response introduces is not mistaken for one
      // the server left out. Without a path the response's scope is unknown and
      // no absence can be inferred from it.
      const localKeysInScope =
        typeof path === 'string' ? localKeysUnder(map.getMerkleTree(), path) : [];

      for (const entry of entries) {
        const { key, records, tombstones } = entry;
        const mirrored = this.mirrorKeyTombstones(map, key, tombstones, pendingFor, records);
        if (mirrored.changed) changedKeys.push(key);
        if (mirrored.healed) healedKeys.add(key);
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
      this.resetKeyTombstonesToPending(map, omitted, pendingFor, changedKeys);

      await this.persistAttributionIfChanged(mapName, changedKeys);

      if (totalAdded > 0 || totalUpdated > 0) {
        await this.config.persistTombstones(mapName);
        logger.info(
          { mapName, added: totalAdded, updated: totalUpdated },
          'Synced ORMap records from server',
        );
      }

      // The leaf entries (including their tombstone tags) are now durably
      // applied. Their epoch is not confirmed here: it only lowers the walk's
      // bound, and the walk confirms once, when its last response has ended.
      this.foldLeafEpoch(mapName, entry, coveringEpoch);

      // Now push any local records that server might not have. A healed key is
      // one of the response's keys, so it goes out in this same single push,
      // once, whether or not it holds a live record.
      const keysToCheck = Array.from(listed);
      await this.pushORMapDiff(mapName, keysToCheck, map, healedKeys);
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
      const changedKeys: string[] = [];
      const pendingFor = this.pendingRemoveLookup(mapName);
      const healedKeys = new Set<string>();

      for (const entry of entries) {
        const { key, records, tombstones } = entry;
        const mirrored = this.mirrorKeyTombstones(map, key, tombstones, pendingFor, records);
        if (mirrored.changed) changedKeys.push(key);
        if (mirrored.healed) healedKeys.add(key);
        const result = map.mergeKey(key, records, tombstones);
        totalAdded += result.added;
        totalUpdated += result.updated;
        // Persist server-origin merge so it survives an offline reload (symmetric with LWW).
        await this.config.persistKey(mapName, key);
      }

      await this.persistAttributionIfChanged(mapName, changedKeys);

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

      // A diff response is otherwise the end of the exchange and nothing is
      // pushed after it. The one exception is a key the server still serves a
      // record for under a tag this client has tombstoned: only this client can
      // hand that tombstone back, so those keys, and no others, are pushed.
      if (healedKeys.size > 0) {
        await this.pushORMapDiff(mapName, Array.from(healedKeys), map, healedKeys);
      }
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
   * One kind of tag is kept although the server does not attribute it: the tag
   * of a record the server serves as LIVE for this key while this client already
   * holds that tag as a tombstone. A tag is created once, by one add, so a live
   * server record under a tag tombstoned here can only mean the server no longer
   * holds that tombstone: it acknowledged the remove and lost it (an unclean
   * shutdown before the remove reached disk), or it refused the remove. Nobody
   * but a client that still holds the tombstone can give it back, so the tag
   * stays attributed to the key that serves it live and the caller pushes that
   * key. This cannot make two different states compare equal: the client leaf
   * holds the tag as a tombstone where the server leaf holds it as a live
   * record, so the leaves differ until the push lands, and agree afterwards.
   *
   * The map-wide set is read BEFORE this key's set is replaced. Read afterwards
   * it would also hold the tags this very call attributes (the server's own and
   * the pending ones), and a remove the server simply has not received yet
   * would be mistaken for one it lost.
   *
   * @returns `changed`: whether the key's attributed set changed; `healed`:
   * whether a tag was kept because the server serves it live
   */
  private mirrorKeyTombstones(
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- ORMap value type is erased at the sync handler layer
    map: ORMap<any, any>,
    key: string,
    serverTombstones: Iterable<string>,
    pendingFor: PendingRemoveLookup,
    serverLiveRecords: ReadonlyArray<{ tag: string }> = [],
  ): { changed: boolean; healed: boolean } {
    const next = new Set(serverTombstones);
    for (const tag of pendingFor(key)) {
      next.add(tag);
    }
    let healed = false;
    for (const record of serverLiveRecords) {
      if (map.isTombstoned(record.tag)) {
        next.add(record.tag);
        healed = true;
      }
    }
    const previous = map.getKeyTombstones(key);
    map.setKeyTombstones(key, next);
    return { changed: !sameTags(previous, next), healed };
  }

  /**
   * The server's view shows no leaf for these keys, so it attributes no
   * tombstone to them: each key keeps only the tags of this client's own
   * not-yet-acknowledged removes. The map-wide suppression set is untouched, so
   * a tag dropped here stays removed.
   *
   * Every key whose attributed set changed is appended to `changedKeys`.
   */
  private resetKeyTombstonesToPending(
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- ORMap value type is erased at the sync handler layer
    map: ORMap<any, any>,
    keys: string[],
    pendingFor: PendingRemoveLookup,
    changedKeys: string[],
  ): void {
    for (const key of keys) {
      if (this.mirrorKeyTombstones(map, key, [], pendingFor).changed) {
        changedKeys.push(key);
      }
    }
  }

  /**
   * Persist the per-key attribution of the keys this invocation changed it for.
   * Separate from the record/tombstone persist because attribution changes on
   * responses that add and update nothing, and an unchanged walk must not write
   * at all. Only the changed keys are handed over, so that storage is rewritten
   * for them and not for the whole map.
   *
   * A rejection is not caught here: it propagates out of the handler exactly as
   * a failed record or tombstone persist does, so the covering epoch that
   * follows is not confirmed on top of attribution that never reached disk.
   */
  private async persistAttributionIfChanged(mapName: string, changedKeys: string[]): Promise<void> {
    if (changedKeys.length > 0) {
      await this.config.persistKeyTombstones(mapName, changedKeys);
    }
  }

  /**
   * Push local ORMap diff to server for the given keys.
   * Sends local records, and for each key only the tombstones attributed to it.
   *
   * A key with no live record is skipped unless it is in `alwaysPush`: its
   * entry then carries no record and only the key's attributed tombstones,
   * which is what lets the server drop a record it should no longer hold.
   */
  public async pushORMapDiff(
    mapName: string,
    keys: string[],
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- ORMap value type is erased at the sync handler layer; actual V type lives in the map instance generic at TopGunClient level
    map: ORMap<any, any>,
    alwaysPush?: ReadonlySet<string>,
  ): Promise<void> {
    const entries: Array<{
      key: string;
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- ORMapRecord value type is erased at the diff protocol layer; records are passed through to the server without inspection
      records: ORMapRecord<any>[];
      tombstones: string[];
    }> = [];

    for (const key of keys) {
      const recordsMap = map.getRecordsMap(key);
      const hasLiveRecords = recordsMap !== undefined && recordsMap.size > 0;
      if (hasLiveRecords || alwaysPush?.has(key)) {
        const records = recordsMap ? Array.from(recordsMap.values()) : [];

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
    // A sync init starts a new walk: nothing of an earlier connection or round
    // survives it, and a response still being handled from before it is stale.
    this.walks.set(mapName, newWalkState((this.walks.get(mapName)?.generation ?? 0) + 1));
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

/**
 * One map's Merkle walk: the requests sent since the last sync init and the
 * lowest covering epoch their responses (the opening root included) conveyed.
 */
interface WalkState {
  /** Raised by every sync init; tells a response of an earlier walk from one of this walk. */
  generation: number;
  /** ORMAP_MERKLE_REQ_BUCKET requests sent whose response has not finished being handled. */
  outstanding: number;
  minEpoch: number | undefined;
  /** Whether a leaf has contributed: a walk that applied no leaf has nothing to confirm. */
  foldedLeaf: boolean;
  /** A response of the walk threw while being handled. */
  failed: boolean;
  /** A response conveyed no usable epoch, or arrived answering no outstanding request. */
  voided: boolean;
}

/** What a bucket or leaf invocation read about its map's walk before first yielding. */
interface WalkEntry {
  generation: number;
  /** False when nothing was outstanding at entry: the invocation is not part of the count. */
  counted: boolean;
}

function newWalkState(generation: number): WalkState {
  return {
    generation,
    outstanding: 0,
    minEpoch: undefined,
    foldedLeaf: false,
    failed: false,
    voided: false,
  };
}

function foldEpoch(walk: WalkState, epoch: number | undefined): void {
  if (typeof epoch === 'number' && Number.isFinite(epoch) && epoch > 0) {
    walk.minEpoch = walk.minEpoch === undefined ? epoch : Math.min(walk.minEpoch, epoch);
  } else {
    walk.voided = true;
  }
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
