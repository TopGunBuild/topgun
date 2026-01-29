/**
 * GCHandler - Handles distributed garbage collection
 *
 * This handler manages garbage collection operations including:
 * - Periodic GC interval timer
 * - Distributed consensus for safe GC timestamp
 * - TTL expiration for both LWW and ORMap entries
 * - Tombstone pruning
 *
 * Extracted from ServerCoordinator as part of SPEC-003b refactoring.
 */

import { HLC, LWWMap, ORMap, Timestamp, LWWRecord, ORMapRecord } from '@topgunbuild/core';
import { logger } from '../utils/logger';
import { DEFAULT_GC_AGE_MS, type IGCHandler, type GCHandlerConfig } from './types';

const DEFAULT_GC_INTERVAL_MS = 60 * 60 * 1000; // 1 hour

export class GCHandler implements IGCHandler {
    private readonly config: GCHandlerConfig;
    private gcInterval?: NodeJS.Timeout;
    private gcReports: Map<string, Timestamp> = new Map();
    private readonly gcIntervalMs: number;
    private readonly gcAgeMs: number;
    private broadcastFn?: (message: any) => void;

    constructor(config: GCHandlerConfig) {
        this.config = config;
        this.gcIntervalMs = config.gcIntervalMs ?? DEFAULT_GC_INTERVAL_MS;
        this.gcAgeMs = config.gcAgeMs ?? DEFAULT_GC_AGE_MS;
        this.broadcastFn = config.broadcast;
    }

    /**
     * Set coordinator callbacks after construction (late binding pattern).
     * Used by ServerCoordinator to wire broadcast callback.
     */
    setCoordinatorCallbacks(callbacks: { broadcast: (message: any) => void }): void {
        this.broadcastFn = callbacks.broadcast;
    }

    /**
     * Broadcast message to all clients.
     * Uses late-bound callback from ServerCoordinator.
     */
    private broadcast(message: any): void {
        if (this.broadcastFn) {
            this.broadcastFn(message);
        }
    }

    /**
     * Start the GC interval timer.
     */
    start(): void {
        this.gcInterval = setInterval(() => {
            this.reportLocalHlc();
        }, this.gcIntervalMs);
    }

    /**
     * Stop the GC interval and clear pending reports.
     */
    stop(): void {
        if (this.gcInterval) {
            clearInterval(this.gcInterval);
            this.gcInterval = undefined;
        }
        this.gcReports.clear();
    }

    /**
     * Reports local minimum HLC to cluster leader for GC consensus.
     * Called periodically by the GC interval timer.
     */
    private reportLocalHlc(): void {
        // 1. Calculate Local Min HLC
        let minHlc = this.config.hlc.now();

        for (const client of this.config.connectionManager.getClients().values()) {
            if (HLC.compare(client.lastActiveHlc, minHlc) < 0) {
                minHlc = client.lastActiveHlc;
            }
        }

        const members = this.config.cluster.getMembers().sort();
        const leaderId = members[0];
        const myId = this.config.cluster.config.nodeId;

        if (leaderId === myId) {
            // I am Leader
            this.handleGcReport(myId, minHlc);
        } else {
            // Send to Leader
            this.config.cluster.send(leaderId, 'CLUSTER_GC_REPORT', { minHlc });
        }
    }

    /**
     * Leader processes GC reports from nodes.
     * When all nodes have reported, calculates safe GC timestamp and broadcasts commit.
     */
    handleGcReport(nodeId: string, minHlc: Timestamp): void {
        this.gcReports.set(nodeId, minHlc);

        const members = this.config.cluster.getMembers();

        // Check if we have reports from ALL members
        // (Including self, which is inserted directly)
        const allReported = members.every(m => this.gcReports.has(m));

        if (allReported) {
            // Calculate Global Safe Timestamp
            let globalSafe = this.config.hlc.now(); // Start high
            let initialized = false;

            for (const ts of this.gcReports.values()) {
                if (!initialized || HLC.compare(ts, globalSafe) < 0) {
                    globalSafe = ts;
                    initialized = true;
                }
            }

            // Add safety buffer (e.g. GC_AGE)
            // prune(timestamp) removes items OLDER than timestamp.
            // We want to remove items OLDER than (GlobalMin - GC_AGE).

            const olderThanMillis = globalSafe.millis - this.gcAgeMs;
            const safeTimestamp: Timestamp = {
                millis: olderThanMillis,
                counter: 0,
                nodeId: globalSafe.nodeId // Doesn't matter much for comparison if millis match, but best effort
            };

            logger.info({
                globalMinHlc: globalSafe.millis,
                safeGcTimestamp: olderThanMillis,
                reportsCount: this.gcReports.size
            }, 'GC Consensus Reached. Broadcasting Commit.');

            // Broadcast Commit to other nodes
            for (const member of members) {
                if (!this.config.cluster.isLocal(member)) {
                    this.config.cluster.send(member, 'CLUSTER_GC_COMMIT', { safeTimestamp });
                }
            }

            // Execute Locally
            this.performGarbageCollection(safeTimestamp);

            // Clear reports for next round
            this.gcReports.clear();
        }
    }

    /**
     * Performs garbage collection: TTL expiration + tombstone pruning.
     * Note: This is intentionally synchronous to match the original implementation.
     * Storage operations are fire-and-forget with error logging.
     */
    performGarbageCollection(olderThan: Timestamp): void {
        logger.info({ olderThanMillis: olderThan.millis }, 'Performing Garbage Collection');
        const now = Date.now();

        for (const [name, map] of this.config.storageManager.getMaps()) {
            if (map instanceof LWWMap) {
                this.gcLWWMap(name, map, olderThan, now);
            } else if (map instanceof ORMap) {
                this.gcORMap(name, map, olderThan, now);
            }
        }

        // Broadcast to clients
        this.broadcast({
            type: 'GC_PRUNE',
            payload: {
                olderThan
            }
        });
    }

    /**
     * GC for LWWMap: handle TTL expiration and prune old tombstones.
     */
    private gcLWWMap(
        name: string,
        map: LWWMap<string, any>,
        olderThan: Timestamp,
        now: number
    ): void {
        // 1. Check for active expired records (TTL)
        for (const key of map.allKeys()) {
            const record = map.getRecord(key);
            if (record && record.value !== null && record.ttlMs) {
                const expirationTime = record.timestamp.millis + record.ttlMs;
                if (expirationTime < now) {
                    this.expireLWWRecord(name, map, key, record, expirationTime);
                }
            }
        }

        // 2. Prune old tombstones
        const removedKeys = map.prune(olderThan);
        if (removedKeys.length > 0) {
            logger.info({ mapName: name, count: removedKeys.length }, 'Pruned records from LWW map');
            if (this.config.storage) {
                this.config.storage.deleteAll(name, removedKeys).catch(err => {
                    logger.error({ mapName: name, err }, 'Failed to delete pruned keys from storage');
                });
            }
        }
    }

    /**
     * Expire a single LWW record by converting it to a tombstone.
     */
    private expireLWWRecord(
        mapName: string,
        map: LWWMap<string, any>,
        key: string,
        record: LWWRecord<any>,
        expirationTime: number
    ): void {
        logger.info({ mapName, key }, 'Record expired (TTL). Converting to tombstone.');

        // Create Tombstone at expiration time to handle "Resurrection" correctly
        const tombstoneTimestamp: Timestamp = {
            millis: expirationTime,
            counter: 0, // Reset counter for expiration time
            nodeId: this.config.hlc.getNodeId // Use our ID
        };

        const tombstone: LWWRecord<any> = { value: null, timestamp: tombstoneTimestamp };

        // Apply locally
        const changed = map.merge(key, tombstone);

        if (changed) {
            // Persist
            if (this.config.storage) {
                this.config.storage.store(mapName, key, tombstone).catch(err =>
                    logger.error({ mapName, key, err }, 'Failed to persist expired tombstone')
                );
            }

            const eventPayload = {
                mapName: mapName,
                key: key,
                eventType: 'UPDATED',
                record: tombstone
            };

            // Broadcast to local clients
            this.broadcast({
                type: 'SERVER_EVENT',
                payload: eventPayload,
                timestamp: this.config.hlc.now()
            });

            // Notify query subscriptions (handles both local and distributed via CLUSTER_SUB_UPDATE)
            this.config.queryRegistry.processChange(mapName, map, key, tombstone, record);

            // Replicate to backup nodes via partition-aware ReplicationPipeline
            if (this.config.replicationPipeline) {
                const op = {
                    opType: 'set',
                    mapName: mapName,
                    key: key,
                    record: tombstone
                };
                const opId = `ttl:${mapName}:${key}:${Date.now()}`;
                this.config.replicationPipeline.replicate(op, opId, key).catch(err => {
                    logger.warn({ opId, key, err }, 'TTL expiration replication failed (non-fatal)');
                });
            }
        }
    }

    /**
     * GC for ORMap: handle TTL expiration and prune old tombstones.
     */
    private gcORMap(
        name: string,
        map: ORMap<string, any>,
        olderThan: Timestamp,
        now: number
    ): void {
        // ORMap Expiration
        // We need to check all active records in the ORMap
        const items = (map as any).items as Map<string, Map<string, ORMapRecord<any>>>;
        const tombstonesSet = (map as any).tombstones as Set<string>;

        const tagsToExpire: { key: string; tag: string }[] = [];

        for (const [key, keyMap] of items) {
            for (const [tag, record] of keyMap) {
                if (!tombstonesSet.has(tag)) {
                    if (record.ttlMs) {
                        const expirationTime = record.timestamp.millis + record.ttlMs;
                        if (expirationTime < now) {
                            tagsToExpire.push({ key, tag });
                        }
                    }
                }
            }
        }

        for (const { key, tag } of tagsToExpire) {
            this.expireORMapRecord(name, map, key, tag);
        }

        // 2. Prune old tombstones
        const removedTags = map.prune(olderThan);
        if (removedTags.length > 0) {
            logger.info({ mapName: name, count: removedTags.length }, 'Pruned tombstones from OR map');
            // We need to update __tombstones__ in storage
            if (this.config.storage) {
                const currentTombstones = map.getTombstones();
                this.config.storage.store(name, '__tombstones__', {
                    type: 'OR_TOMBSTONES',
                    tags: currentTombstones
                }).catch(err => {
                    logger.error({ mapName: name, err }, 'Failed to update tombstones');
                });
            }
        }
    }

    /**
     * Expire a single ORMap record by adding it to tombstones.
     */
    private expireORMapRecord(
        mapName: string,
        map: ORMap<string, any>,
        key: string,
        tag: string
    ): void {
        logger.info({ mapName, key, tag }, 'ORMap Record expired (TTL). Removing.');

        // Get old records for processChange before modification
        const oldRecords = map.getRecords(key);

        // Remove by adding tag to tombstones
        map.applyTombstone(tag);

        // Persist change
        if (this.config.storage) {
            // We need to update the key's record list and tombstones
            const records = map.getRecords(key);
            if (records.length > 0) {
                this.config.storage.store(mapName, key, { type: 'OR', records });
            } else {
                this.config.storage.delete(mapName, key);
            }

            const currentTombstones = map.getTombstones();
            this.config.storage.store(mapName, '__tombstones__', {
                type: 'OR_TOMBSTONES',
                tags: currentTombstones
            });
        }

        // Broadcast
        const eventPayload = {
            mapName: mapName,
            key: key,
            eventType: 'OR_REMOVE',
            orTag: tag
        };

        // Broadcast to local clients
        this.broadcast({
            type: 'SERVER_EVENT',
            payload: eventPayload,
            timestamp: this.config.hlc.now()
        });

        // Notify query subscriptions (handles both local and distributed via CLUSTER_SUB_UPDATE)
        const newRecords = map.getRecords(key);
        this.config.queryRegistry.processChange(mapName, map, key, newRecords, oldRecords);

        // Replicate to backup nodes via partition-aware ReplicationPipeline
        if (this.config.replicationPipeline) {
            const op = {
                opType: 'OR_REMOVE',
                mapName: mapName,
                key: key,
                orTag: tag
            };
            const opId = `ttl:${mapName}:${key}:${tag}:${Date.now()}`;
            this.config.replicationPipeline.replicate(op, opId, key).catch(err => {
                logger.warn({ opId, key, err }, 'ORMap TTL expiration replication failed (non-fatal)');
            });
        }
    }
}
