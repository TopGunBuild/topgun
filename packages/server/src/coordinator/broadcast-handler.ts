/**
 * BroadcastHandler - Handles broadcast operations
 *
 * This handler manages broadcasting messages to connected clients with
 * optimizations for subscription-based routing, role-based serialization
 * caching, and Field Level Security filtering.
 *
 * Extracted from ServerCoordinator.
 */

import { serialize } from '@topgunbuild/core';
import { logger } from '../utils/logger';
import type { IBroadcastHandler, ClientConnection, BroadcastHandlerConfig } from './types';

export class BroadcastHandler implements IBroadcastHandler {
    private readonly config: BroadcastHandlerConfig;

    constructor(config: BroadcastHandlerConfig) {
        this.config = config;
    }

    /**
     * Broadcast a single message to all relevant clients.
     * Uses subscription-based routing for SERVER_EVENT messages.
     */
    broadcast(message: any, excludeClientId?: string): void {
        const isServerEvent = message.type === 'SERVER_EVENT';

        if (isServerEvent) {
            const payload = message.payload;
            const mapName = payload.mapName;

            // === SUBSCRIPTION-BASED ROUTING ===
            // Only send to clients that have active subscriptions for this map
            const subscribedClientIds = this.config.queryRegistry.getSubscribedClientIds(mapName);

            // Track metrics
            this.config.metricsService.incEventsRouted();

            if (subscribedClientIds.size === 0) {
                // Early exit - no subscribers for this map!
                this.config.metricsService.incEventsFilteredBySubscription();
                return;
            }

            // Track average subscribers per event
            this.config.metricsService.recordSubscribersPerEvent(subscribedClientIds.size);

            // Send only to subscribed clients with FLS filtering
            for (const clientId of subscribedClientIds) {
                if (clientId === excludeClientId) continue;

                const client = this.config.connectionManager.getClient(clientId);
                if (!client || client.socket.readyState !== 1 || !client.isAuthenticated || !client.principal) {
                    continue;
                }

                // Shallow clone payload for FLS filtering
                const newPayload = { ...payload };

                if (newPayload.record) { // LWW
                    const newVal = this.config.securityManager.filterObject(newPayload.record.value, client.principal, mapName);
                    newPayload.record = { ...newPayload.record, value: newVal };
                }

                if (newPayload.orRecord) { // OR_ADD
                    const newVal = this.config.securityManager.filterObject(newPayload.orRecord.value, client.principal, mapName);
                    newPayload.orRecord = { ...newPayload.orRecord, value: newVal };
                }

                client.writer.write({ ...message, payload: newPayload });
            }
        } else {
            // Non-event messages (GC_PRUNE, SHUTDOWN_PENDING) still go to all clients
            const msgData = serialize(message);
            for (const [id, client] of this.config.connectionManager.getClients()) {
                if (id !== excludeClientId && client.socket.readyState === 1) { // 1 = OPEN
                    client.writer.writeRaw(msgData);
                }
            }
        }
    }

    /**
     * === OPTIMIZATION 2 & 3: Batched Broadcast with Serialization Caching ===
     * Groups clients by their permission roles and serializes once per group.
     * Also batches multiple events into a single SERVER_BATCH_EVENT message.
     * === OPTIMIZATION 4: Subscription-based Routing ===
     * Only sends events to clients with active subscriptions for affected maps.
     */
    broadcastBatch(events: any[], excludeClientId?: string): void {
        if (events.length === 0) return;

        // === SUBSCRIPTION-BASED ROUTING ===
        // Get unique map names from events
        const affectedMaps = new Set<string>();
        for (const event of events) {
            if (event.mapName) {
                affectedMaps.add(event.mapName);
            }
        }

        // Get all subscribed client IDs across all affected maps
        const subscribedClientIds = new Set<string>();
        for (const mapName of affectedMaps) {
            const mapSubscribers = this.config.queryRegistry.getSubscribedClientIds(mapName);
            for (const clientId of mapSubscribers) {
                subscribedClientIds.add(clientId);
            }
        }

        // Track metrics
        this.config.metricsService.incEventsRouted();

        if (subscribedClientIds.size === 0) {
            // Early exit - no subscribers for any of the affected maps!
            this.config.metricsService.incEventsFilteredBySubscription();
            return;
        }

        this.config.metricsService.recordSubscribersPerEvent(subscribedClientIds.size);

        // Group subscribed clients by their role signature for serialization caching
        const clientsByRoleSignature = new Map<string, ClientConnection[]>();

        for (const clientId of subscribedClientIds) {
            if (clientId === excludeClientId) continue;

            const client = this.config.connectionManager.getClient(clientId);
            if (!client || client.socket.readyState !== 1 || !client.isAuthenticated || !client.principal) {
                continue;
            }

            // Create a role signature for grouping (sorted roles joined)
            const roleSignature = this.getClientRoleSignature(client);

            if (!clientsByRoleSignature.has(roleSignature)) {
                clientsByRoleSignature.set(roleSignature, []);
            }
            clientsByRoleSignature.get(roleSignature)!.push(client);
        }

        // For each role group, filter events once and serialize once
        for (const [, clients] of clientsByRoleSignature) {
            if (clients.length === 0) continue;

            // Use first client as representative for filtering (same roles = same permissions)
            const representativeClient = clients[0];

            // Filter all events for this role group
            const filteredEvents = events.map(eventPayload => {
                const mapName = eventPayload.mapName;
                const newPayload = { ...eventPayload };

                if (newPayload.record) { // LWW
                    const newVal = this.config.securityManager.filterObject(
                        newPayload.record.value,
                        representativeClient.principal!,
                        mapName
                    );
                    newPayload.record = { ...newPayload.record, value: newVal };
                }

                if (newPayload.orRecord) { // OR_ADD
                    const newVal = this.config.securityManager.filterObject(
                        newPayload.orRecord.value,
                        representativeClient.principal!,
                        mapName
                    );
                    newPayload.orRecord = { ...newPayload.orRecord, value: newVal };
                }

                return newPayload;
            });

            // Serialize ONCE for this entire group
            const batchMessage = {
                type: 'SERVER_BATCH_EVENT',
                payload: { events: filteredEvents },
                timestamp: this.config.hlc.now()
            };
            const serializedBatch = serialize(batchMessage);

            // Send to all clients in this role group
            for (const client of clients) {
                try {
                    client.writer.writeRaw(serializedBatch);
                } catch (err) {
                    logger.error({ clientId: client.id, err }, 'Failed to send batch to client');
                }
            }
        }
    }

    /**
     * === BACKPRESSURE: Synchronous Broadcast ===
     * Same as broadcastBatch but waits for all sends to complete.
     * Used when backpressure forces sync processing to drain the pipeline.
     */
    async broadcastBatchSync(events: any[], excludeClientId?: string): Promise<void> {
        if (events.length === 0) return;

        // Get unique map names from events
        const affectedMaps = new Set<string>();
        for (const event of events) {
            if (event.mapName) {
                affectedMaps.add(event.mapName);
            }
        }

        // Get all subscribed client IDs across all affected maps
        const subscribedClientIds = new Set<string>();
        for (const mapName of affectedMaps) {
            const mapSubscribers = this.config.queryRegistry.getSubscribedClientIds(mapName);
            for (const clientId of mapSubscribers) {
                subscribedClientIds.add(clientId);
            }
        }

        if (subscribedClientIds.size === 0) {
            return;
        }

        // Group subscribed clients by their role signature
        const clientsByRoleSignature = new Map<string, ClientConnection[]>();

        for (const clientId of subscribedClientIds) {
            if (clientId === excludeClientId) continue;

            const client = this.config.connectionManager.getClient(clientId);
            if (!client || client.socket.readyState !== 1 || !client.isAuthenticated || !client.principal) {
                continue;
            }

            const roleSignature = this.getClientRoleSignature(client);

            if (!clientsByRoleSignature.has(roleSignature)) {
                clientsByRoleSignature.set(roleSignature, []);
            }
            clientsByRoleSignature.get(roleSignature)!.push(client);
        }

        // Collect all send promises
        const sendPromises: Promise<void>[] = [];

        for (const [, clients] of clientsByRoleSignature) {
            if (clients.length === 0) continue;

            const representativeClient = clients[0];

            // Filter all events for this role group
            const filteredEvents = events.map(eventPayload => {
                const mapName = eventPayload.mapName;
                const newPayload = { ...eventPayload };

                if (newPayload.record) {
                    const newVal = this.config.securityManager.filterObject(
                        newPayload.record.value,
                        representativeClient.principal!,
                        mapName
                    );
                    newPayload.record = { ...newPayload.record, value: newVal };
                }

                if (newPayload.orRecord) {
                    const newVal = this.config.securityManager.filterObject(
                        newPayload.orRecord.value,
                        representativeClient.principal!,
                        mapName
                    );
                    newPayload.orRecord = { ...newPayload.orRecord, value: newVal };
                }

                return newPayload;
            });

            const batchMessage = {
                type: 'SERVER_BATCH_EVENT',
                payload: { events: filteredEvents },
                timestamp: this.config.hlc.now()
            };
            const serializedBatch = serialize(batchMessage);

            // Send to all clients and collect promises
            for (const client of clients) {
                sendPromises.push(new Promise<void>((resolve, reject) => {
                    try {
                        client.socket.send(serializedBatch, (err) => {
                            if (err) {
                                logger.error({ clientId: client.id, err }, 'Failed to send sync batch to client');
                                reject(err);
                            } else {
                                resolve();
                            }
                        });
                    } catch (err) {
                        logger.error({ clientId: client.id, err }, 'Exception sending sync batch to client');
                        reject(err);
                    }
                }));
            }
        }

        // Wait for all sends to complete (ignore individual failures)
        await Promise.allSettled(sendPromises);
    }

    /**
     * Helper method to get role signature for a client (for caching key)
     */
    private getClientRoleSignature(client: ClientConnection): string {
        if (!client.principal || !client.principal.roles) {
            return 'USER';
        }
        return client.principal.roles.sort().join(',');
    }
}
