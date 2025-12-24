/**
 * MigrationManager - Manages gradual partition rebalancing
 *
 * Phase 4 Task 03: Parallel Partition Sync
 *
 * Features:
 * - Gradual rebalancing with configurable batch size
 * - State machine for migration lifecycle
 * - Backpressure via chunk acknowledgments
 * - Retry logic for failed migrations
 * - Metrics and observability
 */

import { EventEmitter } from 'events';
import {
  PartitionState,
  PartitionMigration,
  MigrationConfig,
  MigrationStatus,
  MigrationMetrics,
  DEFAULT_MIGRATION_CONFIG,
  PartitionChange,
  serialize,
} from '@topgunbuild/core';
import { xxhash64AsNumber, createXxHash64State } from '@topgunbuild/native';
import { ClusterManager, ClusterMessage } from './ClusterManager';
import { PartitionService, PartitionDistribution } from './PartitionService';
import { logger } from '../utils/logger';

export interface IncomingMigration {
  sourceNode: string;
  chunks: Uint8Array[];
  expectedSize: number;
  receivedSize: number;
  startTime: number;
}

export interface MigrationManagerEvents {
  'migrationPlanned': (info: { total: number }) => void;
  'batchStarted': (info: { count: number; remaining: number }) => void;
  'migrationProgress': (migration: PartitionMigration) => void;
  'migrationComplete': (partitionId: number) => void;
  'migrationFailed': (partitionId: number, error: Error) => void;
  'error': (error: Error) => void;
}

export class MigrationManager extends EventEmitter {
  private readonly config: MigrationConfig;
  private readonly clusterManager: ClusterManager;
  private readonly partitionService: PartitionService;

  // Active outgoing migrations (this node is source)
  private activeMigrations: Map<number, PartitionMigration> = new Map();
  // Queue of migrations to process
  private migrationQueue: PartitionMigration[] = [];
  // Incoming migrations (this node is target)
  private incomingMigrations: Map<number, IncomingMigration> = new Map();
  // Pending chunk acknowledgments
  private pendingChunkAcks: Map<string, { resolve: () => void; reject: (err: Error) => void; timeout: ReturnType<typeof setTimeout> }> = new Map();
  // Pending verification results
  private pendingVerifications: Map<number, { resolve: (success: boolean) => void; timeout: ReturnType<typeof setTimeout> }> = new Map();

  // Metrics tracking
  private metrics: MigrationMetrics = {
    migrationsStarted: 0,
    migrationsCompleted: 0,
    migrationsFailed: 0,
    chunksTransferred: 0,
    bytesTransferred: 0,
    activeMigrations: 0,
    queuedMigrations: 0,
  };

  // Batch processing timer
  private batchTimer: ReturnType<typeof setInterval> | null = null;

  // Data collection callback (injected from ServerCoordinator)
  private dataCollector: ((partitionId: number) => Promise<Uint8Array[]>) | null = null;
  // Data storage callback (injected from ServerCoordinator)
  private dataStorer: ((partitionId: number, data: Uint8Array[]) => Promise<void>) | null = null;

  constructor(
    clusterManager: ClusterManager,
    partitionService: PartitionService,
    config: Partial<MigrationConfig> = {}
  ) {
    super();
    this.clusterManager = clusterManager;
    this.partitionService = partitionService;
    this.config = {
      ...DEFAULT_MIGRATION_CONFIG,
      ...config,
    };

    this.setupMessageHandlers();
  }

  // ============================================
  // Configuration
  // ============================================

  /**
   * Set the data collector callback
   * Called to collect all records for a partition before migration
   */
  public setDataCollector(collector: (partitionId: number) => Promise<Uint8Array[]>): void {
    this.dataCollector = collector;
  }

  /**
   * Set the data storer callback
   * Called to store received records after successful migration
   */
  public setDataStorer(storer: (partitionId: number, data: Uint8Array[]) => Promise<void>): void {
    this.dataStorer = storer;
  }

  // ============================================
  // Migration Planning
  // ============================================

  /**
   * Plan migration for topology change
   */
  public planMigration(
    oldDistribution: Map<number, PartitionDistribution>,
    newDistribution: Map<number, PartitionDistribution>
  ): void {
    const migrations: PartitionMigration[] = [];

    for (const [partitionId, newDist] of newDistribution) {
      const oldDist = oldDistribution.get(partitionId);
      const oldOwner = oldDist?.owner ?? this.clusterManager.config.nodeId;
      const newOwner = newDist.owner;

      // Only plan migration if owner changed AND we are the source
      if (oldOwner !== newOwner && oldOwner === this.clusterManager.config.nodeId) {
        migrations.push({
          partitionId,
          state: PartitionState.STABLE,
          sourceNode: oldOwner,
          targetNode: newOwner,
          startTime: 0,
          bytesTransferred: 0,
          totalBytes: 0,
          retryCount: 0,
        });
      }
    }

    // Sort by partition ID for deterministic ordering
    migrations.sort((a, b) => a.partitionId - b.partitionId);

    this.migrationQueue = migrations;
    this.metrics.queuedMigrations = migrations.length;

    logger.info({ total: migrations.length }, 'Migration planned');
    this.emit('migrationPlanned', { total: migrations.length });

    // Start processing if we have migrations
    if (migrations.length > 0) {
      this.startBatchProcessing();
    }
  }

  /**
   * Start batch processing timer
   */
  private startBatchProcessing(): void {
    if (this.batchTimer) return;

    // Process first batch immediately
    this.startNextBatch().catch(err => {
      logger.error({ error: err }, 'Failed to start first migration batch');
      this.emit('error', err);
    });

    // Then process batches on interval
    this.batchTimer = setInterval(() => {
      this.startNextBatch().catch(err => {
        logger.error({ error: err }, 'Failed to start migration batch');
        this.emit('error', err);
      });
    }, this.config.batchIntervalMs);
  }

  /**
   * Stop batch processing
   */
  private stopBatchProcessing(): void {
    if (this.batchTimer) {
      clearInterval(this.batchTimer);
      this.batchTimer = null;
    }
  }

  /**
   * Start next batch of migrations
   */
  public async startNextBatch(): Promise<void> {
    // Check if we have capacity
    if (this.activeMigrations.size >= this.config.parallelTransfers) {
      return; // Wait for current batch to complete
    }

    const slotsAvailable = this.config.parallelTransfers - this.activeMigrations.size;
    const batch = this.migrationQueue.splice(0, Math.min(slotsAvailable, this.config.batchSize));

    if (batch.length === 0) {
      // No more migrations, stop timer
      if (this.migrationQueue.length === 0 && this.activeMigrations.size === 0) {
        this.stopBatchProcessing();
      }
      return;
    }

    for (const migration of batch) {
      migration.state = PartitionState.MIGRATING;
      migration.startTime = Date.now();
      this.activeMigrations.set(migration.partitionId, migration);
      this.metrics.migrationsStarted++;
      this.metrics.activeMigrations = this.activeMigrations.size;
      this.metrics.queuedMigrations = this.migrationQueue.length;

      // Start async migration
      this.startPartitionMigration(migration).catch(error => {
        this.onMigrationFailed(migration.partitionId, error);
      });
    }

    logger.info({ count: batch.length, remaining: this.migrationQueue.length }, 'Batch started');
    this.emit('batchStarted', { count: batch.length, remaining: this.migrationQueue.length });
  }

  // ============================================
  // Migration Execution
  // ============================================

  /**
   * Start migration for a single partition
   */
  private async startPartitionMigration(migration: PartitionMigration): Promise<void> {
    const { partitionId, targetNode } = migration;

    logger.info({ partitionId, targetNode }, 'Starting partition migration');

    // 1. Collect all records for partition
    let records: Uint8Array[];
    if (this.dataCollector) {
      records = await this.dataCollector(partitionId);
    } else {
      // No data collector set, send empty migration
      records = [];
    }

    migration.totalBytes = records.reduce((sum, r) => sum + r.length, 0);

    // 2. Send start message
    this.clusterManager.send(targetNode, 'OP_FORWARD', {
      _migration: {
        type: 'MIGRATION_START',
        payload: {
          partitionId,
          sourceNode: this.clusterManager.config.nodeId,
          estimatedSize: migration.totalBytes,
        },
      },
    });

    // 3. Stream chunks with backpressure
    const chunks = this.chunkify(records);

    for (let i = 0; i < chunks.length; i++) {
      const chunk = chunks[i];
      const checksum = this.calculateChecksum(chunk);

      this.clusterManager.send(targetNode, 'OP_FORWARD', {
        _migration: {
          type: 'MIGRATION_CHUNK',
          payload: {
            partitionId,
            chunkIndex: i,
            totalChunks: chunks.length,
            data: Array.from(chunk), // Convert Uint8Array to array for JSON serialization
            checksum,
          },
        },
      });

      // Wait for acknowledgment (backpressure)
      await this.waitForChunkAck(partitionId, i);

      migration.bytesTransferred += chunk.length;
      this.metrics.chunksTransferred++;
      this.metrics.bytesTransferred += chunk.length;
      this.emit('migrationProgress', migration);
    }

    // 4. Send completion and wait for verification
    const fullChecksum = this.calculatePartitionChecksum(records);

    migration.state = PartitionState.SYNC;

    this.clusterManager.send(targetNode, 'OP_FORWARD', {
      _migration: {
        type: 'MIGRATION_COMPLETE',
        payload: {
          partitionId,
          totalRecords: records.length,
          checksum: fullChecksum,
        },
      },
    });

    // 5. Wait for verification
    const verified = await this.waitForVerification(partitionId);

    if (verified) {
      await this.onMigrationComplete(partitionId);
    } else {
      throw new Error(`Migration verification failed for partition ${partitionId}`);
    }
  }

  /**
   * Split records into chunks
   */
  private chunkify(records: Uint8Array[]): Uint8Array[] {
    const chunks: Uint8Array[] = [];
    let currentChunk: number[] = [];
    let currentSize = 0;

    for (const record of records) {
      // Add record length prefix (4 bytes)
      const lengthPrefix = new Uint8Array(4);
      new DataView(lengthPrefix.buffer).setUint32(0, record.length, true);

      currentChunk.push(...lengthPrefix, ...record);
      currentSize += 4 + record.length;

      if (currentSize >= this.config.transferChunkSize) {
        chunks.push(new Uint8Array(currentChunk));
        currentChunk = [];
        currentSize = 0;
      }
    }

    // Add remaining data as last chunk
    if (currentChunk.length > 0) {
      chunks.push(new Uint8Array(currentChunk));
    }

    // Ensure at least one chunk (even if empty)
    if (chunks.length === 0) {
      chunks.push(new Uint8Array(0));
    }

    return chunks;
  }

  /**
   * Calculate checksum for a chunk using native xxhash
   */
  private calculateChecksum(data: Uint8Array): string {
    return String(xxhash64AsNumber(data));
  }

  /**
   * Calculate checksum for all partition records using streaming xxhash
   */
  private calculatePartitionChecksum(records: Uint8Array[]): string {
    const state = createXxHash64State();
    for (const record of records) {
      state.update(record);
    }
    return String(state.digestAsNumber());
  }

  /**
   * Wait for chunk acknowledgment
   */
  private waitForChunkAck(partitionId: number, chunkIndex: number): Promise<void> {
    return new Promise((resolve, reject) => {
      const key = `${partitionId}:${chunkIndex}`;

      const timeout = setTimeout(() => {
        this.pendingChunkAcks.delete(key);
        reject(new Error(`Chunk ack timeout for partition ${partitionId}, chunk ${chunkIndex}`));
      }, this.config.syncTimeoutMs);

      this.pendingChunkAcks.set(key, { resolve, reject, timeout });
    });
  }

  /**
   * Wait for migration verification
   */
  private waitForVerification(partitionId: number): Promise<boolean> {
    return new Promise((resolve) => {
      const timeout = setTimeout(() => {
        this.pendingVerifications.delete(partitionId);
        resolve(false); // Verification timed out
      }, this.config.syncTimeoutMs);

      this.pendingVerifications.set(partitionId, { resolve, timeout });
    });
  }

  // ============================================
  // Migration Completion
  // ============================================

  /**
   * Handle successful migration completion
   */
  private async onMigrationComplete(partitionId: number): Promise<void> {
    const migration = this.activeMigrations.get(partitionId);
    if (!migration) return;

    migration.state = PartitionState.STABLE;
    this.activeMigrations.delete(partitionId);

    this.metrics.migrationsCompleted++;
    this.metrics.activeMigrations = this.activeMigrations.size;

    logger.info({
      partitionId,
      duration: Date.now() - migration.startTime,
      bytesTransferred: migration.bytesTransferred,
    }, 'Migration completed');

    this.emit('migrationComplete', partitionId);
  }

  /**
   * Handle migration failure
   */
  private async onMigrationFailed(partitionId: number, error: Error): Promise<void> {
    const migration = this.activeMigrations.get(partitionId);
    if (!migration) return;

    migration.retryCount++;

    if (migration.retryCount <= this.config.maxRetries) {
      // Requeue for retry
      migration.state = PartitionState.STABLE;
      migration.bytesTransferred = 0;
      this.activeMigrations.delete(partitionId);
      this.migrationQueue.unshift(migration); // Add to front of queue
      this.metrics.queuedMigrations = this.migrationQueue.length;
      this.metrics.activeMigrations = this.activeMigrations.size;

      logger.warn({
        partitionId,
        retryCount: migration.retryCount,
        error: error.message,
      }, 'Migration failed, will retry');
    } else {
      // Max retries exceeded
      migration.state = PartitionState.FAILED;
      this.activeMigrations.delete(partitionId);
      this.metrics.migrationsFailed++;
      this.metrics.activeMigrations = this.activeMigrations.size;

      logger.error({
        partitionId,
        retryCount: migration.retryCount,
        error: error.message,
      }, 'Migration failed permanently');

      this.emit('migrationFailed', partitionId, error);
    }
  }

  // ============================================
  // Incoming Migration Handlers (Target Node)
  // ============================================

  /**
   * Handle MIGRATION_START message
   */
  private handleMigrationStart(payload: { partitionId: number; sourceNode: string; estimatedSize: number }): void {
    const { partitionId, sourceNode, estimatedSize } = payload;

    logger.info({ partitionId, sourceNode, estimatedSize }, 'Receiving migration');

    this.incomingMigrations.set(partitionId, {
      sourceNode,
      chunks: [],
      expectedSize: estimatedSize,
      receivedSize: 0,
      startTime: Date.now(),
    });
  }

  /**
   * Handle MIGRATION_CHUNK message
   */
  private handleMigrationChunk(payload: {
    partitionId: number;
    chunkIndex: number;
    totalChunks: number;
    data: number[];
    checksum: string;
  }): void {
    const { partitionId, chunkIndex, data, checksum } = payload;
    const incoming = this.incomingMigrations.get(partitionId);

    if (!incoming) {
      logger.warn({ partitionId, chunkIndex }, 'Received chunk for unknown migration');
      return;
    }

    const chunkData = new Uint8Array(data);

    // Verify chunk checksum
    const actualChecksum = this.calculateChecksum(chunkData);
    const success = actualChecksum === checksum;

    if (success) {
      // Store chunk
      incoming.chunks[chunkIndex] = chunkData;
      incoming.receivedSize += chunkData.length;
    } else {
      logger.warn({ partitionId, chunkIndex, expected: checksum, actual: actualChecksum }, 'Chunk checksum mismatch');
    }

    // Send acknowledgment
    this.clusterManager.send(incoming.sourceNode, 'OP_FORWARD', {
      _migration: {
        type: 'MIGRATION_CHUNK_ACK',
        payload: {
          partitionId,
          chunkIndex,
          success,
        },
      },
    });
  }

  /**
   * Handle MIGRATION_COMPLETE message
   */
  private async handleMigrationComplete(payload: {
    partitionId: number;
    totalRecords: number;
    checksum: string;
  }): Promise<void> {
    const { partitionId, totalRecords, checksum } = payload;
    const incoming = this.incomingMigrations.get(partitionId);

    if (!incoming) {
      logger.warn({ partitionId }, 'Received complete for unknown migration');
      return;
    }

    // Reassemble data
    const allData = this.reassemble(incoming.chunks);
    const records = this.deserializeRecords(allData);

    // Verify checksum
    const actualChecksum = this.calculatePartitionChecksum(records);
    const checksumMatch = actualChecksum === checksum;
    const success = checksumMatch && records.length === totalRecords;

    if (success && this.dataStorer) {
      // Store records
      await this.dataStorer(partitionId, records);
    }

    logger.info({
      partitionId,
      duration: Date.now() - incoming.startTime,
      records: records.length,
      checksumMatch,
    }, 'Migration received');

    // Send verification result
    this.clusterManager.send(incoming.sourceNode, 'OP_FORWARD', {
      _migration: {
        type: 'MIGRATION_VERIFY',
        payload: {
          partitionId,
          success,
          checksumMatch,
        },
      },
    });

    this.incomingMigrations.delete(partitionId);
  }

  /**
   * Handle MIGRATION_CHUNK_ACK message
   */
  private handleMigrationChunkAck(payload: {
    partitionId: number;
    chunkIndex: number;
    success: boolean;
  }): void {
    const { partitionId, chunkIndex, success } = payload;
    const key = `${partitionId}:${chunkIndex}`;
    const pending = this.pendingChunkAcks.get(key);

    if (pending) {
      clearTimeout(pending.timeout);
      this.pendingChunkAcks.delete(key);

      if (success) {
        pending.resolve();
      } else {
        pending.reject(new Error(`Chunk ${chunkIndex} rejected by target`));
      }
    }
  }

  /**
   * Handle MIGRATION_VERIFY message
   */
  private handleMigrationVerify(payload: {
    partitionId: number;
    success: boolean;
    checksumMatch: boolean;
  }): void {
    const { partitionId, success } = payload;
    const pending = this.pendingVerifications.get(partitionId);

    if (pending) {
      clearTimeout(pending.timeout);
      this.pendingVerifications.delete(partitionId);
      pending.resolve(success);
    }
  }

  /**
   * Reassemble chunks into continuous data
   */
  private reassemble(chunks: Uint8Array[]): Uint8Array {
    const totalLength = chunks.reduce((sum, c) => sum + (c?.length ?? 0), 0);
    const result = new Uint8Array(totalLength);
    let offset = 0;

    for (const chunk of chunks) {
      if (chunk) {
        result.set(chunk, offset);
        offset += chunk.length;
      }
    }

    return result;
  }

  /**
   * Deserialize records from chunk data
   */
  private deserializeRecords(data: Uint8Array): Uint8Array[] {
    const records: Uint8Array[] = [];
    let offset = 0;

    while (offset < data.length) {
      if (offset + 4 > data.length) break;

      const length = new DataView(data.buffer, data.byteOffset + offset, 4).getUint32(0, true);
      offset += 4;

      if (offset + length > data.length) break;

      records.push(data.slice(offset, offset + length));
      offset += length;
    }

    return records;
  }

  // ============================================
  // Message Handling
  // ============================================

  /**
   * Setup cluster message handlers
   */
  private setupMessageHandlers(): void {
    this.clusterManager.on('message', (msg: ClusterMessage) => {
      if (msg.payload?._migration) {
        const migration = msg.payload._migration;

        switch (migration.type) {
          case 'MIGRATION_START':
            this.handleMigrationStart(migration.payload);
            break;
          case 'MIGRATION_CHUNK':
            this.handleMigrationChunk(migration.payload);
            break;
          case 'MIGRATION_COMPLETE':
            this.handleMigrationComplete(migration.payload).catch(err => {
              logger.error({ error: err }, 'Error handling migration complete');
            });
            break;
          case 'MIGRATION_CHUNK_ACK':
            this.handleMigrationChunkAck(migration.payload);
            break;
          case 'MIGRATION_VERIFY':
            this.handleMigrationVerify(migration.payload);
            break;
        }
      }
    });
  }

  // ============================================
  // Status and Metrics
  // ============================================

  /**
   * Check if a partition is currently migrating
   */
  public isActive(partitionId: number): boolean {
    return this.activeMigrations.has(partitionId) || this.incomingMigrations.has(partitionId);
  }

  /**
   * Get migration status
   */
  public getStatus(): MigrationStatus {
    const avgMigrationTime = this.metrics.migrationsCompleted > 0
      ? (Date.now() - (this.activeMigrations.values().next().value?.startTime ?? Date.now()))
      : 0;

    const estimatedTimeRemainingMs =
      (this.migrationQueue.length + this.activeMigrations.size) *
      (avgMigrationTime || 1000); // Default to 1s if no data

    return {
      inProgress: this.activeMigrations.size > 0 || this.migrationQueue.length > 0,
      active: Array.from(this.activeMigrations.values()),
      queued: this.migrationQueue.length,
      completed: this.metrics.migrationsCompleted,
      failed: this.metrics.migrationsFailed,
      estimatedTimeRemainingMs,
    };
  }

  /**
   * Get migration metrics
   */
  public getMetrics(): MigrationMetrics {
    return { ...this.metrics };
  }

  /**
   * Cancel all active and queued migrations
   */
  public async cancelAll(): Promise<void> {
    this.stopBatchProcessing();

    // Clear queued migrations
    this.migrationQueue = [];
    this.metrics.queuedMigrations = 0;

    // Mark active migrations as failed
    for (const [partitionId, migration] of this.activeMigrations) {
      migration.state = PartitionState.FAILED;
      this.metrics.migrationsFailed++;
      this.emit('migrationFailed', partitionId, new Error('Migration cancelled'));
    }

    this.activeMigrations.clear();
    this.metrics.activeMigrations = 0;

    // Clear pending acks
    for (const pending of this.pendingChunkAcks.values()) {
      clearTimeout(pending.timeout);
      pending.reject(new Error('Migration cancelled'));
    }
    this.pendingChunkAcks.clear();

    // Clear pending verifications
    for (const pending of this.pendingVerifications.values()) {
      clearTimeout(pending.timeout);
      pending.resolve(false);
    }
    this.pendingVerifications.clear();

    // Clear incoming migrations
    this.incomingMigrations.clear();

    logger.info('All migrations cancelled');
  }

  /**
   * Cleanup resources (sync version for backwards compatibility)
   */
  public close(): void {
    this.cancelAll();
  }

  /**
   * Async cleanup - waits for cancellation to complete
   */
  public async closeAsync(): Promise<void> {
    await this.cancelAll();
    this.removeAllListeners();
  }
}
