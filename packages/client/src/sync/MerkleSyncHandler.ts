import { LWWMap } from '@topgunbuild/core';
import type { IMerkleSyncHandler, MerkleSyncHandlerConfig } from './types';
import { logger } from '../utils/logger';

/**
 * MerkleSyncHandler
 *
 * Handles Merkle tree synchronization protocol messages for LWWMap.
 * Manages sync state, root hash comparison, bucket traversal, and leaf merging.
 */
export class MerkleSyncHandler implements IMerkleSyncHandler {
  private readonly config: MerkleSyncHandlerConfig;
  private lastSyncTimestamp: number = 0;

  constructor(config: MerkleSyncHandlerConfig) {
    this.config = config;
  }

  /**
   * Handle SYNC_RESET_REQUIRED message from server.
   * Resets the map and triggers a fresh sync.
   */
  public async handleSyncResetRequired(payload: { mapName: string }): Promise<void> {
    const { mapName } = payload;
    logger.warn({ mapName }, 'Sync Reset Required due to GC Age');
    await this.config.resetMap(mapName);
    // Trigger re-sync as fresh
    this.config.sendMessage({
      type: 'SYNC_INIT',
      mapName,
      lastSyncTimestamp: 0
    });
  }

  /**
   * Handle SYNC_RESP_ROOT message from server.
   * Compares root hashes and requests buckets if mismatch detected.
   */
  public async handleSyncRespRoot(payload: { mapName: string; rootHash: number; timestamp?: any }): Promise<void> {
    const { mapName, rootHash, timestamp } = payload;
    const map = this.config.getMap(mapName);
    if (map instanceof LWWMap) {
      const localRootHash = map.getMerkleTree().getRootHash();
      if (localRootHash !== rootHash) {
        logger.info({ mapName, localRootHash, remoteRootHash: rootHash }, 'Root hash mismatch, requesting buckets');
        this.config.sendMessage({
          type: 'MERKLE_REQ_BUCKET',
          payload: { mapName, path: '' }
        });
      } else {
        logger.info({ mapName }, 'Map is in sync');
      }
    }
    // Update HLC with server timestamp
    if (timestamp) {
      await this.config.onTimestampUpdate(timestamp);
    }
  }

  /**
   * Handle SYNC_RESP_BUCKETS message from server.
   * Compares bucket hashes and requests mismatched buckets.
   */
  public handleSyncRespBuckets(payload: { mapName: string; path: string; buckets: Record<string, number> }): void {
    const { mapName, path, buckets } = payload;
    const map = this.config.getMap(mapName);
    if (map instanceof LWWMap) {
      const tree = map.getMerkleTree();
      const localBuckets = tree.getBuckets(path);

      for (const [bucketKey, remoteHash] of Object.entries(buckets)) {
        const localHash = localBuckets[bucketKey] || 0;
        if (localHash !== remoteHash) {
          const newPath = path + bucketKey;
          this.config.sendMessage({
            type: 'MERKLE_REQ_BUCKET',
            payload: { mapName, path: newPath }
          });
        }
      }
    }
  }

  /**
   * Handle SYNC_RESP_LEAF message from server.
   * Merges leaf records into local map and persists to storage.
   */
  public async handleSyncRespLeaf(payload: { mapName: string; records: Array<{ key: string; record: any }> }): Promise<void> {
    const { mapName, records } = payload;
    const map = this.config.getMap(mapName);
    if (map instanceof LWWMap) {
      let updateCount = 0;
      for (const { key, record } of records) {
        // Merge into local map
        const updated = map.merge(key, record);
        if (updated) {
          updateCount++;
          // Persist to storage
          await this.config.storageAdapter.put(`${mapName}:${key}`, record);
        }
      }
      if (updateCount > 0) {
        logger.info({ mapName, count: updateCount }, 'Synced records from server');
      }
    }
  }

  /**
   * Send SYNC_INIT message to server to start sync.
   * Encapsulates sync init message construction.
   */
  public sendSyncInit(mapName: string, lastSyncTimestamp: number): void {
    this.lastSyncTimestamp = lastSyncTimestamp;
    logger.info({ mapName }, 'Starting Merkle sync for LWWMap');
    this.config.sendMessage({
      type: 'SYNC_INIT',
      mapName,
      lastSyncTimestamp
    });
  }

  /**
   * Get the last sync timestamp for debugging/testing.
   */
  public getLastSyncTimestamp(): number {
    return this.lastSyncTimestamp;
  }
}
