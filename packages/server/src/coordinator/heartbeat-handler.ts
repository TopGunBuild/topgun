/**
 * HeartbeatHandler - Handles client heartbeat/ping operations
 *
 * This handler manages client liveness detection including:
 * - Periodic check for dead clients
 * - PING/PONG message handling
 * - Client idle time tracking
 * - Eviction of clients that exceed heartbeat timeout
 *
 * Extracted from ServerCoordinator.
 */

import { WebSocket } from 'ws';
import { logger } from '../utils/logger';
import type { IHeartbeatHandler, HeartbeatHandlerConfig, ClientConnection } from './types';

const DEFAULT_HEARTBEAT_TIMEOUT_MS = 20000; // 20 seconds
const DEFAULT_HEARTBEAT_CHECK_INTERVAL_MS = 5000; // 5 seconds

export class HeartbeatHandler implements IHeartbeatHandler {
    private readonly config: HeartbeatHandlerConfig;
    private readonly heartbeatTimeoutMs: number;
    private readonly heartbeatCheckIntervalMs: number;
    private heartbeatCheckInterval?: NodeJS.Timeout;

    constructor(config: HeartbeatHandlerConfig) {
        this.config = config;
        this.heartbeatTimeoutMs = config.heartbeatTimeoutMs ?? DEFAULT_HEARTBEAT_TIMEOUT_MS;
        this.heartbeatCheckIntervalMs = config.heartbeatCheckIntervalMs ?? DEFAULT_HEARTBEAT_CHECK_INTERVAL_MS;
    }

    /**
     * Start the periodic check for dead clients.
     */
    start(): void {
        this.heartbeatCheckInterval = setInterval(() => {
            this.evictDeadClients();
        }, this.heartbeatCheckIntervalMs);
    }

    /**
     * Stop the periodic heartbeat check.
     */
    stop(): void {
        if (this.heartbeatCheckInterval) {
            clearInterval(this.heartbeatCheckInterval);
            this.heartbeatCheckInterval = undefined;
        }
    }

    /**
     * Handle incoming PING message from client.
     * Responds with PONG immediately.
     */
    handlePing(client: ClientConnection, clientTimestamp: number): void {
        client.lastPingReceived = Date.now();

        const pongMessage = {
            type: 'PONG',
            timestamp: clientTimestamp,
            serverTime: Date.now(),
        };

        // PONG is urgent - bypass batching for accurate RTT measurement
        client.writer.write(pongMessage, true);
    }

    /**
     * Check if a client is still alive based on heartbeat.
     * Delegates to ConnectionManager.
     */
    isClientAlive(clientId: string): boolean {
        return this.config.connectionManager.isClientAlive(clientId);
    }

    /**
     * Get client idle time in ms.
     * Delegates to ConnectionManager.
     */
    getClientIdleTime(clientId: string): number {
        return this.config.connectionManager.getClientIdleTime(clientId);
    }

    /**
     * Evict clients that haven't sent a PING within the timeout period.
     */
    private evictDeadClients(): void {
        const now = Date.now();
        const deadClients: string[] = [];

        for (const [clientId, client] of this.config.connectionManager.getClients()) {
            // Only check authenticated clients (unauthenticated ones will timeout via auth mechanism)
            if (client.isAuthenticated) {
                const idleTime = now - client.lastPingReceived;
                if (idleTime > this.heartbeatTimeoutMs) {
                    deadClients.push(clientId);
                }
            }
        }

        for (const clientId of deadClients) {
            const client = this.config.connectionManager.getClient(clientId);
            if (client) {
                logger.warn({
                    clientId,
                    idleTime: now - client.lastPingReceived,
                    timeoutMs: this.heartbeatTimeoutMs,
                }, 'Evicting dead client (heartbeat timeout)');

                // Close the connection
                if (client.socket.readyState === WebSocket.OPEN) {
                    client.socket.close(4002, 'Heartbeat timeout');
                }
            }
        }
    }
}
