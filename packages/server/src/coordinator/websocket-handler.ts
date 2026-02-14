import * as crypto from 'crypto';
import type { WebSocket } from 'ws';
import { serialize, deserialize, MessageSchema } from '@topgunbuild/core';
import type { ConnectionContext } from '../interceptor/IInterceptor';
import { logger } from '../utils/logger';
import { CoalescingWriter } from '../utils/CoalescingWriter';
import type { IWebSocketHandler, WebSocketHandlerConfig, ClientConnection } from './types';

/**
 * Handles WebSocket connection lifecycle and message routing.
 * Extracted from ServerCoordinator to reduce file size and improve modularity.
 */
export class WebSocketHandler implements IWebSocketHandler {
    private readonly config: WebSocketHandlerConfig;
    private messageRegistry: Record<string, (client: ClientConnection, message: any) => void | Promise<void>> = {};

    constructor(config: WebSocketHandlerConfig) {
        this.config = config;
        this.messageRegistry = config.messageRegistry || {};
    }

    /**
     * Set message registry after construction (for late binding when registry depends on other handlers).
     */
    setMessageRegistry(registry: Record<string, (client: ClientConnection, message: any) => void | Promise<void>>): void {
        this.messageRegistry = registry;
    }

    /**
     * Handle new WebSocket connection.
     * Manages rate limiting, client registration, interceptors, and event handlers.
     */
    async handleConnection(ws: WebSocket): Promise<void> {
        // Check rate limit before accepting connection
        if (this.config.rateLimitingEnabled && !this.config.rateLimiter.shouldAccept()) {
            logger.warn('Connection rate limit exceeded, rejecting');
            this.config.rateLimiter.onConnectionRejected();
            this.config.metricsService.incConnectionsRejected();
            ws.close(1013, 'Server overloaded'); // 1013 = Try Again Later
            return;
        }

        // Register connection attempt
        if (this.config.rateLimitingEnabled) {
            this.config.rateLimiter.onConnectionAttempt();
        }
        this.config.metricsService.incConnectionsAccepted();

        // Client ID is temporary until auth
        const clientId = crypto.randomUUID();
        logger.info({ clientId }, 'Client connected (pending auth)');

        // Create CoalescingWriter if enabled, otherwise create a pass-through writer
        const writer = new CoalescingWriter(ws, this.config.writeCoalescingEnabled ? this.config.writeCoalescingOptions : {
            maxBatchSize: 1, // Disable batching by flushing immediately
            maxDelayMs: 0,
            maxBatchBytes: 0,
        });

        // Register client connection via ConnectionManager
        const connection = this.config.connectionManager.registerClient(clientId, ws, writer);
        this.config.metricsService.setConnectedClients(this.config.connectionManager.getClientCount());

        // Run onConnection interceptors
        try {
            const context: ConnectionContext = {
                clientId: connection.id,
                socket: connection.socket,
                isAuthenticated: connection.isAuthenticated,
                principal: connection.principal
            };
            for (const interceptor of this.config.interceptors) {
                if (interceptor.onConnection) {
                    await interceptor.onConnection(context);
                }
            }
        } catch (err) {
            logger.error({ clientId, err }, 'Interceptor rejected connection');
            ws.close(4000, 'Connection Rejected');
            this.config.connectionManager.removeClient(clientId);
            return;
        }

        ws.on('message', (message) => {
            try {
                let data: any;
                let buf: Uint8Array;

                if (Buffer.isBuffer(message)) {
                    buf = message;
                } else if (message instanceof ArrayBuffer) {
                    buf = new Uint8Array(message);
                } else if (Array.isArray(message)) {
                    buf = Buffer.concat(message);
                } else {
                    // Fallback or unexpected type
                    buf = Buffer.from(message as any);
                }

                try {
                    data = deserialize(buf);
                } catch (e) {
                    // If msgpack fails, try JSON (legacy support)
                    try {
                        // Use Buffer.toString() or TextDecoder
                        const text = Buffer.isBuffer(buf) ? buf.toString() : new TextDecoder().decode(buf);
                        data = JSON.parse(text);
                    } catch (jsonErr) {
                        // Original error likely relevant
                        throw e;
                    }
                }

                this.handleMessage(connection, data);
            } catch (err) {
                logger.error({ err }, 'Invalid message format');
                ws.close(1002, 'Protocol Error');
            }
        });

        ws.on('close', () => {
            this.handleDisconnect(connection);
        });

        // Send Auth Challenge immediately
        ws.send(serialize({ type: 'AUTH_REQUIRED' }));
    }

    /**
     * Handle incoming message from client.
     * Validates message, handles auth, and routes to appropriate handler.
     */
    async handleMessage(client: ClientConnection, rawMessage: any): Promise<void> {
        // Validation with Zod
        const parseResult = MessageSchema.safeParse(rawMessage);
        if (!parseResult.success) {
            this.config.rateLimitedLogger.error(
                `invalid-message:${client.id}`,
                { clientId: client.id, errorCode: parseResult.error.issues[0]?.code },
                'Invalid message format from client'
            );
            client.writer.write({
                type: 'ERROR',
                payload: { code: 400, message: 'Invalid message format', details: (parseResult.error as any).errors }
            }, true); // urgent
            return;
        }
        const message = parseResult.data;

        // Handle PING immediately (even before auth check for authenticated clients)
        if (message.type === 'PING') {
            this.config.heartbeatHandler.handlePing(client, message.timestamp);
            return;
        }

        // Update client's last active HLC
        this.config.clientMessageHandler.updateClientHlc(client, message);

        // Handshake / Auth handling
        if (!client.isAuthenticated) {
            if (message.type === 'AUTH') {
                const token = message.token;
                const result = await this.config.authHandler.handleAuth(client, token);
                if (result.success) {
                    client.writer.write({ type: 'AUTH_ACK', protocolVersion: 1 }, true); // urgent: bypass batching
                } else {
                    client.writer.write({ type: 'AUTH_FAIL', error: result.error || 'Invalid token' }, true); // urgent
                    client.socket.close(4001, 'Unauthorized');
                }
                return;
            } else {
                // Reject any other message before auth
                client.socket.close(4001, 'Auth required');
            }
            return;
        }

        // Standard Protocol Handling (Authenticated)
        // All message types are routed through MessageRegistry
        const registryHandler = this.messageRegistry?.[message.type];
        if (registryHandler) {
            await registryHandler(client, message);
            return;
        }

        // Only AUTH for already-authenticated clients remains (duplicate AUTH handling)
        if (message.type === 'AUTH') {
            // Client already authenticated, ignore duplicate AUTH messages
            logger.debug({ clientId: client.id }, 'Ignoring duplicate AUTH from already authenticated client');
            return;
        }

        logger.warn({ type: message.type }, 'Unknown message type');
    }

    /**
     * Handle client disconnect - cleanup all subscriptions and notify cluster.
     */
    private handleDisconnect(connection: ClientConnection): void {
        const clientId = connection.id;
        logger.info({ clientId }, 'Client disconnected');

        // If connection was still pending (not authenticated), mark as failed
        if (this.config.rateLimitingEnabled && !connection.isAuthenticated) {
            this.config.rateLimiter.onPendingConnectionFailed();
        }

        // Close the CoalescingWriter to flush any pending messages
        connection.writer.close();

        // Run onDisconnect interceptors
        const context: ConnectionContext = {
            clientId: connection.id,
            socket: connection.socket,
            isAuthenticated: connection.isAuthenticated,
            principal: connection.principal
        };
        for (const interceptor of this.config.interceptors) {
            if (interceptor.onDisconnect) {
                interceptor.onDisconnect(context).catch((err: unknown) => {
                    logger.error({ clientId, err }, 'Error in onDisconnect interceptor');
                });
            }
        }

        // Cleanup subscriptions
        for (const subId of connection.subscriptions) {
            this.config.queryRegistry.unregister(subId);
        }

        // Cleanup Locks (Local)
        this.config.lockManager.handleClientDisconnect(clientId);

        // Cleanup Topics (Local)
        this.config.topicManager.unsubscribeAll(clientId);

        // Cleanup Counters (Local)
        this.config.counterHandler.unsubscribeAll(clientId);

        // Cleanup Search Subscriptions
        this.config.searchCoordinator.unsubscribeClient(clientId);

        // Cleanup distributed subscriptions for this client
        if (this.config.distributedSubCoordinator && connection) {
            this.config.distributedSubCoordinator.unsubscribeClient(connection.socket);
        }

        // Notify Cluster to Cleanup Locks (Remote)
        const members = this.config.cluster.getMembers();
        for (const memberId of members) {
            if (!this.config.cluster.isLocal(memberId)) {
                this.config.cluster.send(memberId, 'CLUSTER_CLIENT_DISCONNECTED', {
                    originNodeId: this.config.cluster.config.nodeId,
                    clientId
                });
            }
        }

        this.config.connectionManager.removeClient(clientId);
        this.config.metricsService.setConnectedClients(this.config.connectionManager.getClientCount());
    }
}
