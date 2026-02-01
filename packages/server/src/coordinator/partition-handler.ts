/**
 * PartitionHandler - Handles PARTITION_MAP_REQUEST messages
 *
 * This handler provides partition map information to clients for
 * cluster-aware routing.
 *
 * Extracted from ServerCoordinator .
 */

import { logger } from '../utils/logger';
import type { IPartitionHandler, ClientConnection, PartitionHandlerConfig } from './types';

export class PartitionHandler implements IPartitionHandler {
    private readonly config: PartitionHandlerConfig;

    constructor(config: PartitionHandlerConfig) {
        this.config = config;
    }

    /**
     * Handle PARTITION_MAP_REQUEST message.
     * Returns the current partition map if client has a stale version.
     */
    handlePartitionMapRequest(client: ClientConnection, message: any): void {
        // Client is requesting the current partition map
        // This is used for cluster-aware routing
        const clientVersion = message.payload?.currentVersion ?? 0;
        const currentMap = this.config.partitionService.getPartitionMap();

        // Only send if client has stale version or no version
        if (clientVersion < currentMap.version) {
            client.writer.write({
                type: 'PARTITION_MAP',
                payload: currentMap
            });
            logger.debug({
                clientId: client.id,
                clientVersion,
                serverVersion: currentMap.version
            }, 'Sent partition map to client');
        }
    }
}
