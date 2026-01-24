/**
 * SearchHandler - Handles SEARCH, SEARCH_SUB, SEARCH_UNSUB messages
 *
 * This handler manages full-text search operations with support for
 * both local and distributed search.
 *
 * Extracted from ServerCoordinator as part of Phase 4 refactoring.
 */

import { logger } from '../utils/logger';
import type { ISearchHandler, ClientConnection, SearchHandlerConfig } from './types';

export class SearchHandler implements ISearchHandler {
    private readonly config: SearchHandlerConfig;

    constructor(config: SearchHandlerConfig) {
        this.config = config;
    }

    /**
     * Handle SEARCH message.
     * Executes a full-text search, using distributed search if cluster mode.
     */
    async handleSearch(client: ClientConnection, message: any): Promise<void> {
        const { requestId: searchReqId, mapName: searchMapName, query: searchQuery, options: searchOptions } = message.payload;

        // Check READ permission
        if (!this.config.securityManager.checkPermission(client.principal!, searchMapName, 'READ')) {
            logger.warn({ clientId: client.id, mapName: searchMapName }, 'Access Denied: SEARCH');
            client.writer.write({
                type: 'SEARCH_RESP',
                payload: {
                    requestId: searchReqId,
                    results: [],
                    totalCount: 0,
                    error: `Access denied for map: ${searchMapName}`,
                }
            });
            return;
        }

        // Check if FTS is enabled for this map
        if (!this.config.searchCoordinator.isSearchEnabled(searchMapName)) {
            client.writer.write({
                type: 'SEARCH_RESP',
                payload: {
                    requestId: searchReqId,
                    results: [],
                    totalCount: 0,
                    error: `Full-text search not enabled for map: ${searchMapName}`,
                }
            });
            return;
        }

        // Phase 14: Use distributed search if ClusterSearchCoordinator is available
        // and we have more than one node in the cluster
        if (this.config.clusterSearchCoordinator && this.config.cluster.getMembers().length > 1) {
            // Execute distributed search across cluster
            try {
                const distributedResult = await this.config.clusterSearchCoordinator.search(searchMapName, searchQuery, {
                    limit: searchOptions?.limit ?? 10,
                    minScore: searchOptions?.minScore,
                    boost: searchOptions?.boost,
                });

                logger.debug({
                    clientId: client.id,
                    mapName: searchMapName,
                    query: searchQuery,
                    resultCount: distributedResult.results.length,
                    totalHits: distributedResult.totalHits,
                    respondedNodes: distributedResult.respondedNodes.length,
                    failedNodes: distributedResult.failedNodes.length,
                    executionTimeMs: distributedResult.executionTimeMs,
                }, 'Distributed search executed');

                client.writer.write({
                    type: 'SEARCH_RESP',
                    payload: {
                        requestId: searchReqId,
                        results: distributedResult.results,
                        totalCount: distributedResult.totalHits,
                        // Include cursor for pagination if available
                        nextCursor: distributedResult.nextCursor,
                    },
                });
            } catch (err: any) {
                logger.error({ err, mapName: searchMapName, query: searchQuery }, 'Distributed search failed');
                client.writer.write({
                    type: 'SEARCH_RESP',
                    payload: {
                        requestId: searchReqId,
                        results: [],
                        totalCount: 0,
                        error: `Distributed search failed: ${err.message}`,
                    },
                });
            }
        } else {
            // Execute local search (single node or no cluster)
            const searchResult = this.config.searchCoordinator.search(searchMapName, searchQuery, searchOptions);
            searchResult.requestId = searchReqId;

            logger.debug({
                clientId: client.id,
                mapName: searchMapName,
                query: searchQuery,
                resultCount: searchResult.results.length
            }, 'Local search executed');

            client.writer.write({
                type: 'SEARCH_RESP',
                payload: searchResult,
            });
        }
    }

    /**
     * Handle SEARCH_SUB message.
     * Creates a live search subscription.
     */
    async handleSearchSub(client: ClientConnection, message: any): Promise<void> {
        const { subscriptionId, mapName: subMapName, query: subQuery, options: subOptions } = message.payload;

        // Check READ permission
        if (!this.config.securityManager.checkPermission(client.principal!, subMapName, 'READ')) {
            logger.warn({ clientId: client.id, mapName: subMapName }, 'Access Denied: SEARCH_SUB');
            client.writer.write({
                type: 'SEARCH_RESP',
                payload: {
                    requestId: subscriptionId,
                    results: [],
                    totalCount: 0,
                    error: `Access denied for map: ${subMapName}`,
                }
            });
            return;
        }

        // Check if FTS is enabled for this map
        if (!this.config.searchCoordinator.isSearchEnabled(subMapName)) {
            client.writer.write({
                type: 'SEARCH_RESP',
                payload: {
                    requestId: subscriptionId,
                    results: [],
                    totalCount: 0,
                    error: `Full-text search not enabled for map: ${subMapName}`,
                }
            });
            return;
        }

        // Phase 14.2: Use distributed subscription if cluster mode with multiple nodes
        if (this.config.distributedSubCoordinator && this.config.cluster.getMembers().length > 1) {
            // Distributed search subscription
            try {
                const result = await this.config.distributedSubCoordinator.subscribeSearch(
                    subscriptionId,
                    client.socket,
                    subMapName,
                    subQuery,
                    subOptions || {}
                );

                client.writer.write({
                    type: 'SEARCH_RESP',
                    payload: {
                        requestId: subscriptionId,
                        results: result.results,
                        totalCount: result.totalHits,
                    },
                });

                logger.debug({
                    clientId: client.id,
                    subscriptionId,
                    mapName: subMapName,
                    query: subQuery,
                    resultCount: result.results.length,
                    totalHits: result.totalHits,
                    nodes: result.registeredNodes,
                }, 'Distributed search subscription created');
            } catch (err) {
                logger.error({ err, subscriptionId }, 'Distributed search subscription failed');
                client.writer.write({
                    type: 'SEARCH_RESP',
                    payload: {
                        requestId: subscriptionId,
                        results: [],
                        totalCount: 0,
                        error: 'Failed to create distributed subscription',
                    },
                });
            }
        } else {
            // Single-node fallback: use local SearchCoordinator
            const initialResults = this.config.searchCoordinator.subscribe(
                client.id,
                subscriptionId,
                subMapName,
                subQuery,
                subOptions
            );

            logger.debug({
                clientId: client.id,
                subscriptionId,
                mapName: subMapName,
                query: subQuery,
                resultCount: initialResults.length
            }, 'Search subscription created (local)');

            // Send initial snapshot as SEARCH_RESP
            client.writer.write({
                type: 'SEARCH_RESP',
                payload: {
                    requestId: subscriptionId,
                    results: initialResults,
                    totalCount: initialResults.length,
                },
            });
        }
    }

    /**
     * Handle SEARCH_UNSUB message.
     * Removes a search subscription.
     */
    handleSearchUnsub(client: ClientConnection, message: any): void {
        const { subscriptionId: unsubId } = message.payload;
        // Unsubscribe from both local and distributed
        this.config.searchCoordinator.unsubscribe(unsubId);
        if (this.config.distributedSubCoordinator) {
            this.config.distributedSubCoordinator.unsubscribe(unsubId);
        }
        logger.debug({ clientId: client.id, subscriptionId: unsubId }, 'Search unsubscription');
    }
}
