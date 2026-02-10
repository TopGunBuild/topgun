/**
 * QueryHandler - Handles QUERY_SUB, QUERY_UNSUB messages
 *
 * This handler manages query subscriptions with support for both
 * local and distributed subscriptions.
 *
 * Extracted from ServerCoordinator .
 */

import * as crypto from 'crypto';
import { logger } from '../utils/logger';
import type { IQueryHandler, ClientConnection, QueryHandlerConfig } from './types';

export class QueryHandler implements IQueryHandler {
    private readonly config: QueryHandlerConfig;

    constructor(config: QueryHandlerConfig) {
        this.config = config;
    }

    /**
     * Handle QUERY_SUB message.
     * Creates a query subscription for live updates.
     */
    async handleQuerySub(client: ClientConnection, message: any): Promise<void> {
        const { queryId, mapName, query } = message.payload;

        // Check READ permission
        if (!this.config.securityManager.checkPermission(client.principal!, mapName, 'READ')) {
            logger.warn({ clientId: client.id, mapName }, 'Access Denied: QUERY_SUB');
            client.writer.write({
                type: 'ERROR',
                payload: { code: 403, message: `Access Denied for map ${mapName}` }
            }, true);
            return;
        }

        logger.info({ clientId: client.id, mapName, query }, 'Client subscribed');
        this.config.metricsService.incOp('SUBSCRIBE', mapName);

        // Use distributed subscription if cluster mode with multiple nodes
        if (this.config.distributedSubCoordinator && this.config.cluster && this.config.cluster.getMembers().length > 1) {
            // Distributed query subscription
            try {
                const result = await this.config.distributedSubCoordinator.subscribeQuery(
                    queryId,
                    client.socket,
                    mapName,
                    query
                );

                // Apply Field Level Security to results
                const filteredResults = result.results.map((res: any) => {
                    const filteredValue = this.config.securityManager.filterObject(res.value, client.principal!, mapName);
                    return { ...res, value: filteredValue };
                });

                client.writer.write({
                    type: 'QUERY_RESP',
                    payload: {
                        queryId,
                        results: filteredResults,
                    },
                });

                // Track subscription on client
                client.subscriptions.add(queryId);

                logger.debug({
                    clientId: client.id,
                    queryId,
                    mapName,
                    resultCount: result.results.length,
                    totalHits: result.totalHits,
                    nodes: result.registeredNodes,
                }, 'Distributed query subscription created');
            } catch (err) {
                logger.error({ err, queryId }, 'Distributed query subscription failed');
                client.writer.write({
                    type: 'QUERY_RESP',
                    payload: {
                        queryId,
                        results: [],
                        error: 'Failed to create distributed subscription',
                    },
                });
            }
        } else {
            // Single-node fallback: use existing logic
            // Identify all relevant nodes
            const allMembers = this.config.cluster.getMembers();
            let remoteMembers: string[];

            // Partition pruning: narrow the node set to only nodes owning relevant partitions
            const partitionIds = this.config.partitionService?.getRelevantPartitions(query) ?? null;
            if (partitionIds !== null && this.config.partitionService) {
                const targetNodes = this.config.partitionService.getOwnerNodesForPartitions(partitionIds);
                remoteMembers = targetNodes.filter(id => !this.config.cluster.isLocal(id));
                logger.debug({ queryId, partitionCount: partitionIds.length, targetNodeCount: targetNodes.length, remoteCount: remoteMembers.length }, 'Partition pruning applied to query');
            } else {
                remoteMembers = allMembers.filter(id => !this.config.cluster.isLocal(id));
            }

            // Read-from-Replica Optimization
            // If query targets a specific key, we can optimize by routing to a specific replica
            // instead of broadcasting to the entire cluster.
            const queryKey = (query as any)._id || (query as any).where?._id;

            if (queryKey && typeof queryKey === 'string' && this.config.readReplicaHandler) {
                try {
                    const targetNode = this.config.readReplicaHandler.selectReadNode({
                        mapName,
                        key: queryKey,
                        options: {
                            // Default to EVENTUAL for read scaling unless specified otherwise
                            // In future, we could extract consistency from query options if available
                            consistency: this.config.ConsistencyLevel.EVENTUAL
                        }
                    });

                    if (targetNode) {
                        if (this.config.cluster.isLocal(targetNode)) {
                            // Serve locally only
                            remoteMembers = [];
                            logger.debug({ clientId: client.id, mapName, key: queryKey }, 'Read optimization: Serving locally');
                        } else if (remoteMembers.includes(targetNode)) {
                            // Serve from specific remote replica
                            remoteMembers = [targetNode];
                            logger.debug({ clientId: client.id, mapName, key: queryKey, targetNode }, 'Read optimization: Routing to replica');
                        }
                    }
                } catch (e) {
                    logger.warn({ err: e }, 'Error in ReadReplicaHandler selection');
                }
            }

            const requestId = crypto.randomUUID();

            const pending = {
                requestId,
                client,
                queryId,
                mapName,
                query,
                results: [] as { key: string; value: any }[], // Will populate with local results first
                expectedNodes: new Set(remoteMembers),
                respondedNodes: new Set<string>(),
                timer: setTimeout(() => this.config.finalizeClusterQuery(requestId, true), 5000) // 5s timeout
            };

            this.config.pendingClusterQueries.set(requestId, pending);

            // Execute Locally (async - wait for map to load from storage)
            // [FIX] Using await ensures handleMessage completes only after query execution
            // This is important for:
            // 1. Tests that need to verify results immediately after handleMessage
            // 2. Ensuring storage is loaded before returning results
            try {
                const localResults = await this.config.executeLocalQuery(mapName, query);
                pending.results.push(...localResults);

                // Scatter: Send to other nodes
                if (remoteMembers.length > 0) {
                    for (const nodeId of remoteMembers) {
                        this.config.cluster.send(nodeId, 'CLUSTER_QUERY_EXEC', {
                            requestId,
                            mapName,
                            query
                        });
                    }
                } else {
                    // Single node cluster: finalize immediately
                    await this.config.finalizeClusterQuery(requestId);
                }
            } catch (err) {
                logger.error({ err, mapName }, 'Failed to execute local query');
                // Finalize with empty results on error
                await this.config.finalizeClusterQuery(requestId);
            }
        }
    }

    /**
     * Handle QUERY_UNSUB message.
     * Removes a query subscription.
     */
    async handleQueryUnsub(client: ClientConnection, message: any): Promise<void> {
        const { queryId: unsubId } = message.payload;

        // Unsubscribe from distributed coordinator if in cluster mode
        if (this.config.distributedSubCoordinator && this.config.cluster && this.config.cluster.getMembers().length > 1) {
            try {
                await this.config.distributedSubCoordinator.unsubscribe(unsubId);
            } catch (err) {
                logger.warn({ err, queryId: unsubId }, 'Failed to unsubscribe from distributed coordinator');
            }
        }

        this.config.queryRegistry.unregister(unsubId);
        client.subscriptions.delete(unsubId);
    }
}
