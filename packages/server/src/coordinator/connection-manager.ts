import type { WebSocket } from 'ws';
import type { HLC, Principal } from '@topgunbuild/core';
import { CoalescingWriter } from '../utils/CoalescingWriter';
import { logger } from '../utils/logger';
import type { ClientConnection, ConnectionManagerConfig, IConnectionManager } from './types';

const DEFAULT_CLIENT_HEARTBEAT_TIMEOUT_MS = 20000;

/**
 * ConnectionManager owns the clients Map and handles connection lifecycle.
 * This is the single source of truth for connected clients.
 */
export class ConnectionManager implements IConnectionManager {
    private clients: Map<string, ClientConnection> = new Map();
    private readonly hlc: HLC;
    private readonly clientHeartbeatTimeoutMs: number;
    private readonly onClientRegistered?: (client: ClientConnection) => void;
    private readonly onClientRemoved?: (clientId: string) => void;

    constructor(config: ConnectionManagerConfig) {
        this.hlc = config.hlc;
        this.clientHeartbeatTimeoutMs = config.clientHeartbeatTimeoutMs ?? DEFAULT_CLIENT_HEARTBEAT_TIMEOUT_MS;
        this.onClientRegistered = config.onClientRegistered;
        this.onClientRemoved = config.onClientRemoved;
    }

    /**
     * Get all connected clients (read-only access).
     */
    getClients(): Map<string, ClientConnection> {
        return this.clients;
    }

    /**
     * Get a specific client by ID.
     */
    getClient(clientId: string): ClientConnection | undefined {
        return this.clients.get(clientId);
    }

    /**
     * Register a new client connection.
     * Creates the ClientConnection object with initial state.
     */
    registerClient(clientId: string, socket: WebSocket, writer: CoalescingWriter): ClientConnection {
        const connection: ClientConnection = {
            id: clientId,
            socket,
            writer,
            isAuthenticated: false,
            subscriptions: new Set(),
            lastActiveHlc: this.hlc.now(),
            lastPingReceived: Date.now(),
        };

        this.clients.set(clientId, connection);
        logger.debug({ clientId, totalClients: this.clients.size }, 'Client registered');

        if (this.onClientRegistered) {
            this.onClientRegistered(connection);
        }

        return connection;
    }

    /**
     * Remove client and return the removed connection (for cleanup).
     */
    removeClient(clientId: string): ClientConnection | undefined {
        const connection = this.clients.get(clientId);
        if (connection) {
            this.clients.delete(clientId);
            logger.debug({ clientId, totalClients: this.clients.size }, 'Client removed');

            if (this.onClientRemoved) {
                this.onClientRemoved(clientId);
            }
        }
        return connection;
    }

    /**
     * Update client's authenticated state.
     */
    setClientAuthenticated(clientId: string, principal: Principal): void {
        const client = this.clients.get(clientId);
        if (client) {
            client.isAuthenticated = true;
            client.principal = principal;
            logger.debug({ clientId, userId: principal.userId }, 'Client authenticated');
        }
    }

    /**
     * Broadcast message to all clients.
     * Note: This is a simple broadcast. For subscription-based routing with
     * security filtering, use ServerCoordinator's broadcast methods.
     */
    broadcast(message: any, excludeClientId?: string): void {
        for (const [id, client] of this.clients) {
            if (id !== excludeClientId && client.socket.readyState === 1) {
                try {
                    client.writer.write(message);
                } catch (err) {
                    logger.error({ clientId: id, err }, 'Failed to broadcast to client');
                }
            }
        }
    }

    /**
     * Broadcast batch of events to all clients.
     * Note: This is a simple broadcast. For subscription-based routing with
     * security filtering, use ServerCoordinator's broadcastBatch methods.
     */
    broadcastBatch(events: any[], excludeClientId?: string): void {
        if (events.length === 0) return;

        const batchMessage = {
            type: 'SERVER_BATCH_EVENT',
            payload: { events },
            timestamp: this.hlc.now(),
        };

        for (const [id, client] of this.clients) {
            if (id !== excludeClientId && client.socket.readyState === 1) {
                try {
                    client.writer.write(batchMessage);
                } catch (err) {
                    logger.error({ clientId: id, err }, 'Failed to broadcast batch to client');
                }
            }
        }
    }

    /**
     * Check if client is alive based on heartbeat.
     */
    isClientAlive(clientId: string): boolean {
        const client = this.clients.get(clientId);
        if (!client) return false;

        const idleTime = Date.now() - client.lastPingReceived;
        return idleTime < this.clientHeartbeatTimeoutMs;
    }

    /**
     * Get client idle time in ms.
     */
    getClientIdleTime(clientId: string): number {
        const client = this.clients.get(clientId);
        if (!client) return Infinity;

        return Date.now() - client.lastPingReceived;
    }

    /**
     * Update client's last ping timestamp.
     */
    updateLastPing(clientId: string): void {
        const client = this.clients.get(clientId);
        if (client) {
            client.lastPingReceived = Date.now();
        }
    }

    /**
     * Get total client count.
     */
    getClientCount(): number {
        return this.clients.size;
    }
}
