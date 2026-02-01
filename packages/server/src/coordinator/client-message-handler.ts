/**
 * ClientMessageHandler - Handles client-specific messaging operations
 *
 * This handler manages:
 * - Client HLC timestamp updates from incoming messages
 * - Partition map broadcasting to authenticated clients
 * - Merge rejection notifications to clients
 *
 * Extracted from ServerCoordinator.
 */

import { WebSocket } from 'ws';
import { HLC, Timestamp } from '@topgunbuild/core';
import { logger } from '../utils/logger';
import type { IClientMessageHandler, ClientMessageHandlerConfig, ClientConnection } from './types';

export class ClientMessageHandler implements IClientMessageHandler {
    private readonly config: ClientMessageHandlerConfig;

    constructor(config: ClientMessageHandlerConfig) {
        this.config = config;
    }

    /**
     * Update client's HLC timestamp from incoming message.
     * Extracts timestamp from message payload and updates both server and client clocks.
     */
    updateClientHlc(client: ClientConnection, message: any): void {
        // Try to extract timestamp from message if available
        // This is heuristic based on typical message structure
        let ts: Timestamp | undefined;

        if (message.type === 'CLIENT_OP') {
            const op = message.payload;
            if (op.record && op.record.timestamp) {
                ts = op.record.timestamp;
            } else if (op.orRecord && op.orRecord.timestamp) {
                // orRecord usually has entries which have timestamps, or value itself is decorated?
                // Depends on implementation.
            } else if (op.orTag) {
                try {
                    ts = HLC.parse(op.orTag);
                } catch (e) {
                    logger.debug({ orTag: op.orTag, error: e }, 'Failed to parse HLC from orTag');
                }
            }
        }

        if (ts) {
            // Client sent an explicit timestamp, update their HLC
            this.config.hlc.update(ts); // Also update server clock
            // Client HLC is at least this
            client.lastActiveHlc = ts;
        } else {
            // Just bump to current server time if no explicit TS
            // This assumes client is "alive" at this moment.
            client.lastActiveHlc = this.config.hlc.now();
        }
    }

    /**
     * Broadcast partition map to all connected and authenticated clients.
     * Called when partition topology changes (node join/leave/failover).
     */
    broadcastPartitionMap(partitionMap: any): void {
        const message = {
            type: 'PARTITION_MAP',
            payload: partitionMap
        };

        let broadcastCount = 0;
        for (const client of this.config.connectionManager.getClients().values()) {
            if (client.isAuthenticated && client.socket.readyState === WebSocket.OPEN && client.writer) {
                client.writer.write(message);
                broadcastCount++;
            }
        }

        logger.info({
            version: partitionMap.version,
            clientCount: broadcastCount
        }, 'Broadcast partition map to clients');
    }

    /**
     * Notify a client about a merge rejection .
     * Finds the client by node ID and sends MERGE_REJECTED message.
     */
    notifyMergeRejection(rejection: any): void {
        // Find client by node ID
        // Node ID format: "client-{uuid}" - we need to find matching client
        for (const [clientId, client] of this.config.connectionManager.getClients()) {
            // Check if this client sent the rejected operation
            // The nodeId in rejection matches the remoteNodeId from the operation
            if (clientId === rejection.nodeId || rejection.nodeId.includes(clientId)) {
                client.writer.write({
                    type: 'MERGE_REJECTED',
                    mapName: rejection.mapName,
                    key: rejection.key,
                    attemptedValue: rejection.attemptedValue,
                    reason: rejection.reason,
                    timestamp: rejection.timestamp,
                }, true); // urgent - bypass batching
                return;
            }
        }

        // If no matching client found, broadcast to all clients subscribed to this map
        const subscribedClientIds = this.config.queryRegistry.getSubscribedClientIds(rejection.mapName);
        for (const clientId of subscribedClientIds) {
            const client = this.config.connectionManager.getClient(clientId);
            if (client) {
                client.writer.write({
                    type: 'MERGE_REJECTED',
                    mapName: rejection.mapName,
                    key: rejection.key,
                    attemptedValue: rejection.attemptedValue,
                    reason: rejection.reason,
                    timestamp: rejection.timestamp,
                });
            }
        }
    }
}
