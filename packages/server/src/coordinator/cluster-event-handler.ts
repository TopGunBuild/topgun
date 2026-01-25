/**
 * ClusterEventHandler - Handles cluster message routing and event processing
 *
 * This handler manages inter-node communication including:
 * - Operation forwarding (OP_FORWARD)
 * - Cluster event replication (CLUSTER_EVENT)
 * - Distributed query execution (CLUSTER_QUERY_EXEC/RESP)
 * - GC consensus messages (CLUSTER_GC_REPORT/COMMIT)
 * - Distributed locks (CLUSTER_LOCK_*)
 * - Topic publication forwarding (CLUSTER_TOPIC_PUB)
 * - Anti-entropy repair (CLUSTER_MERKLE_*/CLUSTER_REPAIR_*)
 *
 * Extracted from ServerCoordinator as part of SPEC-003c refactoring.
 */

import { LWWMap, ORMap } from '@topgunbuild/core';
import { logger } from '../utils/logger';
import type { IClusterEventHandler, ClusterEventHandlerConfig } from './types';

export class ClusterEventHandler implements IClusterEventHandler {
    private readonly config: ClusterEventHandlerConfig;
    private messageHandler?: (msg: any) => void;
    private memberJoinedHandler?: () => void;
    private memberLeftHandler?: () => void;

    constructor(config: ClusterEventHandlerConfig) {
        this.config = config;
    }

    /**
     * Set up all cluster event listeners.
     */
    setupListeners(): void {
        // Member join/leave handlers for metrics
        this.memberJoinedHandler = () => {
            this.config.metricsService.setClusterMembers(this.getClusterMemberCount());
        };
        this.memberLeftHandler = () => {
            this.config.metricsService.setClusterMembers(this.getClusterMemberCount());
        };

        this.config.cluster.on('memberJoined', this.memberJoinedHandler);
        this.config.cluster.on('memberLeft', this.memberLeftHandler);

        // Main message handler
        this.messageHandler = (msg: any) => {
            this.handleMessage(msg);
        };
        this.config.cluster.on('message', this.messageHandler);
    }

    /**
     * Remove all cluster event listeners.
     */
    teardownListeners(): void {
        if (this.config.cluster.off) {
            if (this.messageHandler) {
                this.config.cluster.off('message', this.messageHandler);
            }
            if (this.memberJoinedHandler) {
                this.config.cluster.off('memberJoined', this.memberJoinedHandler);
            }
            if (this.memberLeftHandler) {
                this.config.cluster.off('memberLeft', this.memberLeftHandler);
            }
        }
        this.messageHandler = undefined;
        this.memberJoinedHandler = undefined;
        this.memberLeftHandler = undefined;
    }

    /**
     * Get cluster member count (internal helper).
     * ClusterManager exposes getMembers() which returns an array.
     */
    private getClusterMemberCount(): number {
        // Access via the cluster interface - need to check if getMembers exists
        const cluster = this.config.cluster as any;
        if (typeof cluster.getMembers === 'function') {
            return cluster.getMembers().length;
        }
        return 1; // Fallback to single node
    }

    /**
     * Route incoming cluster messages to appropriate handlers.
     */
    private handleMessage(msg: any): void {
        switch (msg.type) {
            case 'OP_FORWARD':
                this.handleOpForward(msg);
                break;

            case 'CLUSTER_EVENT':
                this.handleClusterEvent(msg.payload);
                break;

            case 'CLUSTER_QUERY_EXEC':
                this.handleClusterQueryExec(msg);
                break;

            case 'CLUSTER_QUERY_RESP':
                this.handleClusterQueryResp(msg);
                break;

            case 'CLUSTER_GC_REPORT':
                this.config.gcHandler.handleGcReport(msg.senderId, msg.payload.minHlc);
                break;

            case 'CLUSTER_GC_COMMIT':
                this.config.gcHandler.performGarbageCollection(msg.payload.safeTimestamp);
                break;

            case 'CLUSTER_LOCK_REQ':
                this.handleLockRequest(msg);
                break;

            case 'CLUSTER_LOCK_RELEASE':
                this.handleLockRelease(msg);
                break;

            case 'CLUSTER_LOCK_GRANTED':
                this.handleLockGranted(msg);
                break;

            case 'CLUSTER_LOCK_RELEASED':
                this.handleLockReleased(msg);
                break;

            case 'CLUSTER_CLIENT_DISCONNECTED':
                this.handleClientDisconnected(msg);
                break;

            case 'CLUSTER_TOPIC_PUB':
                this.handleTopicPub(msg);
                break;

            case 'CLUSTER_MERKLE_ROOT_REQ':
                this.handleMerkleRootRequest(msg);
                break;

            case 'CLUSTER_MERKLE_ROOT_RESP':
                this.handleMerkleRootResponse(msg);
                break;

            case 'CLUSTER_REPAIR_DATA_REQ':
                this.handleRepairDataRequest(msg);
                break;

            case 'CLUSTER_REPAIR_DATA_RESP':
                this.handleRepairDataResponse(msg);
                break;

            // Note: Messages with _replication or _migration flags are handled
            // by ReplicationPipeline and MigrationManager listeners respectively
        }
    }

    /**
     * Handle OP_FORWARD - forward write operation to partition owner.
     */
    private handleOpForward(msg: any): void {
        // OP_FORWARD is used for multiple purposes:
        // 1. Actual operation forwards (has key field) - route to partition owner
        // 2. Replication messages (has _replication field) - handled by ReplicationPipeline
        // 3. Migration messages (has _migration field) - handled by MigrationManager
        // Only validate key for actual operation forwards
        if (msg.payload._replication || msg.payload._migration) {
            // These are handled by ReplicationPipeline and MigrationManager listeners
            // No routing check needed
            return;
        }

        // Actual operation forward - validate key and route
        logger.info({ senderId: msg.senderId }, 'Received forwarded op');
        if (!msg.payload.key) {
            logger.warn({ senderId: msg.senderId }, 'OP_FORWARD missing key, dropping');
            return;
        }
        if (this.config.partitionService.isLocalOwner(msg.payload.key)) {
            this.config.processLocalOp(msg.payload, true, msg.senderId).catch(err => {
                logger.error({ err, senderId: msg.senderId }, 'Forwarded op failed');
            });
        } else {
            logger.warn({ key: msg.payload.key }, 'Received OP_FORWARD but not owner. Dropping.');
        }
    }

    /**
     * Handle CLUSTER_EVENT - replicate data and broadcast to local clients.
     */
    private handleClusterEvent(payload: any): void {
        // 1. Replication Logic: Am I a Backup?
        const { mapName, key, eventType } = payload;

        // Guard against undefined key (can happen with malformed cluster messages)
        if (!key) {
            logger.warn({ mapName, eventType }, 'Received cluster event with undefined key, ignoring');
            return;
        }

        const map = this.config.getMap(mapName, (eventType === 'OR_ADD' || eventType === 'OR_REMOVE') ? 'OR' : 'LWW');
        const oldRecord = (map instanceof LWWMap) ? map.getRecord(key) : null;

        // Only store if we are Owner (shouldn't receive event unless forwarded) or Backup
        if (this.config.partitionService.isRelated(key)) {
            if (map instanceof LWWMap && payload.record) {
                map.merge(key, payload.record);
            } else if (map instanceof ORMap) {
                if (eventType === 'OR_ADD' && payload.orRecord) {
                    map.apply(key, payload.orRecord);
                } else if (eventType === 'OR_REMOVE' && payload.orTag) {
                    map.applyTombstone(payload.orTag);
                }
            }
        }

        // 2. Notify Query Subscriptions
        this.config.queryRegistry.processChange(mapName, map, key, payload.record || payload.orRecord, oldRecord);

        // 3. Broadcast to local clients (Notification)
        this.config.broadcast({
            type: 'SERVER_EVENT',
            payload: payload,
            timestamp: this.config.hlc.now()
        });
    }

    /**
     * Handle CLUSTER_QUERY_EXEC - execute query locally and send response.
     */
    private handleClusterQueryExec(msg: any): void {
        const { requestId, mapName, query } = msg.payload;
        this.config.executeLocalQuery(mapName, query).then(results => {
            this.config.cluster.send(msg.senderId, 'CLUSTER_QUERY_RESP', {
                requestId,
                results
            });
        }).catch(err => {
            logger.error({ err, mapName }, 'Failed to execute cluster query');
            this.config.cluster.send(msg.senderId, 'CLUSTER_QUERY_RESP', {
                requestId,
                results: []
            });
        });
    }

    /**
     * Handle CLUSTER_QUERY_RESP - aggregate remote query results.
     */
    private handleClusterQueryResp(msg: any): void {
        const { requestId: reqId, results: remoteResults } = msg.payload;
        const pendingQuery = this.config.pendingClusterQueries.get(reqId);
        if (pendingQuery) {
            pendingQuery.results.push(...remoteResults);
            pendingQuery.respondedNodes.add(msg.senderId);

            if (pendingQuery.respondedNodes.size === pendingQuery.expectedNodes.size) {
                this.config.finalizeClusterQuery(reqId);
            }
        }
    }

    /**
     * Handle CLUSTER_LOCK_REQ - process lock acquisition request.
     */
    private handleLockRequest(msg: any): void {
        const { originNodeId, clientId, requestId, name, ttl } = msg.payload;
        const compositeId = `${originNodeId}:${clientId}`;
        const result = this.config.lockManager.acquire(name, compositeId, requestId, ttl || 10000);
        if (result.granted) {
            this.config.cluster.send(originNodeId, 'CLUSTER_LOCK_GRANTED', {
                clientId,
                requestId,
                name,
                fencingToken: result.fencingToken
            });
        }
    }

    /**
     * Handle CLUSTER_LOCK_RELEASE - process lock release request.
     */
    private handleLockRelease(msg: any): void {
        const { originNodeId, clientId, requestId, name, fencingToken } = msg.payload;
        const compositeId = `${originNodeId}:${clientId}`;
        const success = this.config.lockManager.release(name, compositeId, fencingToken);
        this.config.cluster.send(originNodeId, 'CLUSTER_LOCK_RELEASED', {
            clientId, requestId, name, success
        });
    }

    /**
     * Handle CLUSTER_LOCK_GRANTED - notify client that lock was granted.
     */
    private handleLockGranted(msg: any): void {
        const { clientId, requestId, name, fencingToken } = msg.payload;
        const client = this.config.connectionManager.getClient(clientId);
        if (client) {
            client.writer.write({
                type: 'LOCK_GRANTED',
                payload: { requestId, name, fencingToken }
            });
        }
    }

    /**
     * Handle CLUSTER_LOCK_RELEASED - notify client that lock was released.
     */
    private handleLockReleased(msg: any): void {
        const { clientId, requestId, name, success } = msg.payload;
        const client = this.config.connectionManager.getClient(clientId);
        if (client) {
            client.writer.write({
                type: 'LOCK_RELEASED',
                payload: { requestId, name, success }
            });
        }
    }

    /**
     * Handle CLUSTER_CLIENT_DISCONNECTED - clean up locks for disconnected client.
     */
    private handleClientDisconnected(msg: any): void {
        const { clientId, originNodeId } = msg.payload;
        const compositeId = `${originNodeId}:${clientId}`;
        this.config.lockManager.handleClientDisconnect(compositeId);
    }

    /**
     * Handle CLUSTER_TOPIC_PUB - forward topic publication to local subscribers.
     */
    private handleTopicPub(msg: any): void {
        const { topic, data, originalSenderId } = msg.payload;
        this.config.topicManager.publish(topic, data, originalSenderId, true);
    }

    /**
     * Handle CLUSTER_MERKLE_ROOT_REQ - respond with local Merkle root hash.
     */
    private handleMerkleRootRequest(msg: any): void {
        const { partitionId, requestId } = msg.payload;
        const rootHash = this.config.merkleTreeManager?.getRootHash(partitionId) ?? 0;
        this.config.cluster.send(msg.senderId, 'CLUSTER_MERKLE_ROOT_RESP', {
            requestId,
            partitionId,
            rootHash
        });
    }

    /**
     * Handle CLUSTER_MERKLE_ROOT_RESP - forward to RepairScheduler.
     */
    private handleMerkleRootResponse(msg: any): void {
        // Response handled by RepairScheduler via event
        if (this.config.repairScheduler) {
            this.config.repairScheduler.emit('merkleRootResponse', {
                nodeId: msg.senderId,
                ...msg.payload
            });
        }
    }

    /**
     * Handle CLUSTER_REPAIR_DATA_REQ - respond with requested data records.
     */
    private handleRepairDataRequest(msg: any): void {
        const { partitionId, keys, requestId } = msg.payload;
        const records: Record<string, any> = {};
        for (const key of keys) {
            const record = this.config.getLocalRecord(key);
            if (record) {
                records[key] = record;
            }
        }
        this.config.cluster.send(msg.senderId, 'CLUSTER_REPAIR_DATA_RESP', {
            requestId,
            partitionId,
            records
        });
    }

    /**
     * Handle CLUSTER_REPAIR_DATA_RESP - forward to RepairScheduler.
     */
    private handleRepairDataResponse(msg: any): void {
        if (this.config.repairScheduler) {
            this.config.repairScheduler.emit('repairDataResponse', {
                nodeId: msg.senderId,
                ...msg.payload
            });
        }
    }
}
