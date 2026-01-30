import { ORMap } from '@topgunbuild/core';
import type { ORMapRecord } from '@topgunbuild/core';
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
  public async handleORMapSyncRespRoot(payload: { mapName: string; rootHash: number; timestamp?: any }): Promise<void> {
    const { mapName, rootHash, timestamp } = payload;
    const map = this.config.getMap(mapName);
    if (map instanceof ORMap) {
      const localTree = map.getMerkleTree();
      const localRootHash = localTree.getRootHash();

      if (localRootHash !== rootHash) {
        logger.info({ mapName, localRootHash, remoteRootHash: rootHash }, 'ORMap root hash mismatch, requesting buckets');
        this.config.sendMessage({
          type: 'ORMAP_MERKLE_REQ_BUCKET',
          payload: { mapName, path: '' }
        });
      } else {
        logger.info({ mapName }, 'ORMap is in sync');
      }
    }
    // Update HLC with server timestamp
    if (timestamp) {
      await this.config.onTimestampUpdate(timestamp);
    }
  }

  /**
   * Handle ORMAP_SYNC_RESP_BUCKETS message from server.
   * Compares bucket hashes and requests mismatched buckets.
   * Also pushes local data that server doesn't have.
   */
  public async handleORMapSyncRespBuckets(payload: { mapName: string; path: string; buckets: Record<string, number> }): Promise<void> {
    const { mapName, path, buckets } = payload;
    const map = this.config.getMap(mapName);
    if (map instanceof ORMap) {
      const tree = map.getMerkleTree();
      const localBuckets = tree.getBuckets(path);

      for (const [bucketKey, remoteHash] of Object.entries(buckets)) {
        const localHash = localBuckets[bucketKey] || 0;
        if (localHash !== remoteHash) {
          const newPath = path + bucketKey;
          this.config.sendMessage({
            type: 'ORMAP_MERKLE_REQ_BUCKET',
            payload: { mapName, path: newPath }
          });
        }
      }

      // Also check for buckets that exist locally but not on remote
      for (const [bucketKey, localHash] of Object.entries(localBuckets)) {
        if (!(bucketKey in buckets) && localHash !== 0) {
          // Local has data that remote doesn't - need to push
          const newPath = path + bucketKey;
          const keys = tree.getKeysInBucket(newPath);
          if (keys.length > 0) {
            await this.pushORMapDiff(mapName, keys, map);
          }
        }
      }
    }
  }

  /**
   * Handle ORMAP_SYNC_RESP_LEAF message from server.
   * Merges leaf entries into local map and pushes local diff back.
   */
  public async handleORMapSyncRespLeaf(payload: { mapName: string; entries: Array<{ key: string; records: any[]; tombstones: string[] }> }): Promise<void> {
    const { mapName, entries } = payload;
    const map = this.config.getMap(mapName);
    if (map instanceof ORMap) {
      let totalAdded = 0;
      let totalUpdated = 0;

      for (const entry of entries) {
        const { key, records, tombstones } = entry;
        const result = map.mergeKey(key, records, tombstones);
        totalAdded += result.added;
        totalUpdated += result.updated;
      }

      if (totalAdded > 0 || totalUpdated > 0) {
        logger.info({ mapName, added: totalAdded, updated: totalUpdated }, 'Synced ORMap records from server');
      }

      // Now push any local records that server might not have
      const keysToCheck = entries.map((e: { key: string }) => e.key);
      await this.pushORMapDiff(mapName, keysToCheck, map);
    }
  }

  /**
   * Handle ORMAP_DIFF_RESPONSE message from server.
   * Merges diff entries into local map.
   */
  public async handleORMapDiffResponse(payload: { mapName: string; entries: Array<{ key: string; records: any[]; tombstones: string[] }> }): Promise<void> {
    const { mapName, entries } = payload;
    const map = this.config.getMap(mapName);
    if (map instanceof ORMap) {
      let totalAdded = 0;
      let totalUpdated = 0;

      for (const entry of entries) {
        const { key, records, tombstones } = entry;
        const result = map.mergeKey(key, records, tombstones);
        totalAdded += result.added;
        totalUpdated += result.updated;
      }

      if (totalAdded > 0 || totalUpdated > 0) {
        logger.info({ mapName, added: totalAdded, updated: totalUpdated }, 'Merged ORMap diff from server');
      }
    }
  }

  /**
   * Push local ORMap diff to server for the given keys.
   * Sends local records and tombstones that the server might not have.
   */
  public async pushORMapDiff(
    mapName: string,
    keys: string[],
    map: ORMap<any, any>
  ): Promise<void> {
    const entries: Array<{
      key: string;
      records: ORMapRecord<any>[];
      tombstones: string[];
    }> = [];

    const snapshot = map.getSnapshot();

    for (const key of keys) {
      const recordsMap = map.getRecordsMap(key);
      if (recordsMap && recordsMap.size > 0) {
        // Get records as array
        const records = Array.from(recordsMap.values());

        // Get tombstones relevant to this key's records
        // (tombstones that match tags that were in this key)
        const tombstones: string[] = [];
        for (const tag of snapshot.tombstones) {
          // Include all tombstones - server will filter
          tombstones.push(tag);
        }

        entries.push({
          key,
          records,
          tombstones
        });
      }
    }

    if (entries.length > 0) {
      this.config.sendMessage({
        type: 'ORMAP_PUSH_DIFF',
        payload: {
          mapName,
          entries
        }
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
        lastSyncTimestamp
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
