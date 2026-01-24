/**
 * LockHandler - Handles LOCK_REQUEST, LOCK_RELEASE messages
 *
 * This handler manages distributed lock acquisition and release
 * with partition-aware routing.
 *
 * Extracted from ServerCoordinator as part of Phase 4 refactoring.
 */

import { logger } from '../utils/logger';
import type { ILockHandler, ClientConnection, LockHandlerConfig } from './types';

export class LockHandler implements ILockHandler {
    private readonly config: LockHandlerConfig;

    constructor(config: LockHandlerConfig) {
        this.config = config;
    }

    /**
     * Handle LOCK_REQUEST message.
     * Acquires a lock locally or forwards to partition owner.
     */
    handleLockRequest(client: ClientConnection, message: any): void {
        const { requestId, name, ttl } = message.payload;

        // 1. Access Control
        // Define a convention: lock names are resources.
        // Check if user has 'WRITE' permission on "locks" map or specific lock name.
        // Since locks are ephemeral, we might treat them as a special resource "sys:locks".
        // Or just check against the lock name itself.
        // If we use just name, it might conflict with map names if policies are strict.
        // Assuming for now that lock name represents the resource being protected.
        if (!this.config.securityManager.checkPermission(client.principal!, name, 'PUT')) {
            client.writer.write({
                // We don't have LOCK_DENIED type in schema yet?
                // Using LOCK_RELEASED with success=false as a hack or ERROR.
                // Ideally ERROR.
                type: 'ERROR',
                payload: { code: 403, message: `Access Denied for lock ${name}` }
            }, true);
            return;
        }

        if (this.config.partitionService.isLocalOwner(name)) {
            const result = this.config.lockManager.acquire(name, client.id, requestId, ttl || 10000);
            if (result.granted) {
                client.writer.write({
                    type: 'LOCK_GRANTED',
                    payload: { requestId, name, fencingToken: result.fencingToken }
                });
            }
            // If not granted, it is queued. Response sent later via event.
        } else {
            const owner = this.config.partitionService.getOwner(name);
            // 2. Cluster Reliability Check
            if (!this.config.cluster.getMembers().includes(owner)) {
                client.writer.write({
                    type: 'ERROR',
                    payload: { code: 503, message: `Lock owner ${owner} is unavailable` }
                }, true);
                return;
            }

            this.config.cluster.send(owner, 'CLUSTER_LOCK_REQ', {
                originNodeId: this.config.cluster.config.nodeId,
                clientId: client.id,
                requestId,
                name,
                ttl
            });
        }
    }

    /**
     * Handle LOCK_RELEASE message.
     * Releases a lock locally or forwards to partition owner.
     */
    handleLockRelease(client: ClientConnection, message: any): void {
        const { requestId, name, fencingToken } = message.payload;

        if (this.config.partitionService.isLocalOwner(name)) {
            const success = this.config.lockManager.release(name, client.id, fencingToken);
            client.writer.write({
                type: 'LOCK_RELEASED',
                payload: { requestId, name, success }
            });
        } else {
            const owner = this.config.partitionService.getOwner(name);
            this.config.cluster.send(owner, 'CLUSTER_LOCK_RELEASE', {
                originNodeId: this.config.cluster.config.nodeId,
                clientId: client.id,
                requestId,
                name,
                fencingToken
            });
        }
    }
}
