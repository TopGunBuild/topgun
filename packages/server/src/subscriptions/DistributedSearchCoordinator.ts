/**
 * DistributedSearchCoordinator - Distributed FTS subscription coordinator
 *
 * Handles distributed full-text search subscriptions using Reciprocal Rank Fusion
 * (RRF) for merging results from multiple cluster nodes.
 *
 * @module subscriptions/DistributedSearchCoordinator
 */

import {
  ReciprocalRankFusion,
  ClusterSubRegisterPayloadSchema,
  type ClusterSubRegisterPayload,
  type ClusterSubUpdatePayload,
  type SearchOptions,
} from '@topgunbuild/core';
import { ClusterManager } from '../cluster/ClusterManager';
import { SearchCoordinator, type ServerSearchResult } from '../search/SearchCoordinator';
import { MetricsService } from '../monitoring/MetricsService';
import { logger } from '../utils/logger';
import { WebSocket } from 'ws';
import {
  DistributedSubscriptionBase,
  type DistributedSubscription,
  type DistributedSubscriptionConfig,
  type DistributedSubscriptionResult,
} from './DistributedSubscriptionBase';

/**
 * DistributedSearchCoordinator manages distributed FTS subscriptions.
 *
 * Uses Reciprocal Rank Fusion (RRF) to merge search results from multiple nodes,
 * preserving relevance scores while eliminating duplicates.
 *
 * @example
 * ```typescript
 * const coordinator = new DistributedSearchCoordinator(
 *   clusterManager,
 *   searchCoordinator,
 *   { rrfK: 60 }
 * );
 *
 * const result = await coordinator.subscribeSearch(
 *   'sub-123',
 *   clientSocket,
 *   'articles',
 *   'machine learning',
 *   { limit: 10 }
 * );
 * ```
 */
export class DistributedSearchCoordinator extends DistributedSubscriptionBase {
  private readonly localSearchCoordinator: SearchCoordinator;
  private readonly rrf: ReciprocalRankFusion;

  constructor(
    clusterManager: ClusterManager,
    searchCoordinator: SearchCoordinator,
    config?: DistributedSubscriptionConfig,
    metricsService?: MetricsService,
    options?: { registerMemberLeftListener?: boolean }
  ) {
    super(clusterManager, config, metricsService, options);
    this.localSearchCoordinator = searchCoordinator;
    this.rrf = new ReciprocalRankFusion({ k: this.config.rrfK });

    // Listen for local search updates (emitted by SearchCoordinator)
    this.localSearchCoordinator.on('distributedUpdate', this.handleLocalSearchUpdate.bind(this));

    logger.debug('DistributedSearchCoordinator initialized');
  }

  /**
   * Get the subscription type handled by this coordinator.
   */
  getSubscriptionType(): 'SEARCH' | 'QUERY' {
    return 'SEARCH';
  }

  /**
   * Create a new distributed search subscription.
   *
   * @param subscriptionId - Unique subscription ID
   * @param clientSocket - Client WebSocket for sending updates
   * @param mapName - Map name to search
   * @param query - Search query string
   * @param options - Search options
   * @returns Promise resolving to initial results
   */
  async subscribeSearch(
    subscriptionId: string,
    clientSocket: WebSocket,
    mapName: string,
    query: string,
    options: SearchOptions = {}
  ): Promise<DistributedSubscriptionResult> {
    const myNodeId = this.clusterManager.config.nodeId;
    const allNodes = new Set(this.clusterManager.getMembers());

    logger.debug(
      { subscriptionId, mapName, query, nodes: Array.from(allNodes) },
      'Creating distributed search subscription'
    );

    // Create subscription state
    const subscription: DistributedSubscription = {
      id: subscriptionId,
      type: 'SEARCH',
      coordinatorNodeId: myNodeId,
      clientSocket,
      mapName,
      searchQuery: query,
      searchOptions: options,
      registeredNodes: new Set(),
      pendingResults: new Map(),
      createdAt: Date.now(),
      currentResults: new Map(),
    };

    this.subscriptions.set(subscriptionId, subscription);
    this.nodeAcks.set(subscriptionId, new Set());

    // Build register payload
    const registerPayload: ClusterSubRegisterPayload = {
      subscriptionId,
      coordinatorNodeId: myNodeId,
      mapName,
      type: 'SEARCH',
      searchQuery: query,
      searchOptions: {
        limit: options.limit,
        minScore: options.minScore,
        boost: options.boost,
      },
    };

    // Register locally first
    const localResult = this.registerLocalSearchSubscription(subscription);
    this.handleLocalAck(subscriptionId, myNodeId, localResult);

    // Register on remote nodes
    for (const nodeId of allNodes) {
      if (nodeId !== myNodeId) {
        this.clusterManager.send(nodeId, 'CLUSTER_SUB_REGISTER', registerPayload);
      }
    }

    // Wait for all nodes to acknowledge (with timeout)
    return this.waitForAcks(subscriptionId, allNodes);
  }

  /**
   * Handle CLUSTER_SUB_REGISTER for SEARCH type registrations.
   */
  handleSubRegister(senderId: string, payload: ClusterSubRegisterPayload): void {
    if (payload.type !== 'SEARCH') {
      logger.warn(
        { subscriptionId: payload.subscriptionId, type: payload.type },
        'DistributedSearchCoordinator received non-SEARCH registration'
      );
      return;
    }

    const myNodeId = this.clusterManager.config.nodeId;

    logger.debug(
      { subscriptionId: payload.subscriptionId, coordinator: payload.coordinatorNodeId },
      'Received distributed search subscription registration'
    );

    try {
      // Register local search subscription
      const result = this.localSearchCoordinator.registerDistributedSubscription(
        payload.subscriptionId,
        payload.mapName,
        payload.searchQuery!,
        payload.searchOptions || {},
        payload.coordinatorNodeId
      );

      const ackPayload = {
        subscriptionId: payload.subscriptionId,
        nodeId: myNodeId,
        success: true,
        initialResults: result.results.map(r => ({
          key: r.key,
          value: r.value,
          score: r.score,
          matchedTerms: r.matchedTerms,
        })),
        totalHits: result.totalHits,
      };

      // Send ACK back to coordinator
      this.clusterManager.send(payload.coordinatorNodeId, 'CLUSTER_SUB_ACK', ackPayload);
    } catch (error) {
      logger.error(
        { subscriptionId: payload.subscriptionId, error },
        'Failed to register distributed search subscription locally'
      );

      const ackPayload = {
        subscriptionId: payload.subscriptionId,
        nodeId: myNodeId,
        success: false,
        error: error instanceof Error ? error.message : 'Unknown error',
      };

      this.clusterManager.send(payload.coordinatorNodeId, 'CLUSTER_SUB_ACK', ackPayload);
    }
  }

  /**
   * Register a local search subscription for a distributed coordinator.
   */
  private registerLocalSearchSubscription(
    subscription: DistributedSubscription
  ): { results: ServerSearchResult[]; totalHits: number } {
    return this.localSearchCoordinator.registerDistributedSubscription(
      subscription.id,
      subscription.mapName,
      subscription.searchQuery!,
      subscription.searchOptions || {},
      subscription.coordinatorNodeId
    );
  }

  /**
   * Unregister local subscription.
   */
  protected unregisterLocalSubscription(subscription: DistributedSubscription): void {
    this.localSearchCoordinator.unsubscribe(subscription.id);
  }

  /**
   * Cleanup local subscriptions when a coordinator node disconnects.
   */
  cleanupByCoordinator(nodeId: string): void {
    this.localSearchCoordinator.unsubscribeByCoordinator(nodeId);
  }

  /**
   * Handle local search update (emitted by SearchCoordinator for distributed subscriptions).
   */
  private handleLocalSearchUpdate(payload: ClusterSubUpdatePayload): void {
    // This is called when SearchCoordinator detects a change affecting a distributed subscription
    // We need to send this to the coordinator
    const coordinatorNodeId = this.getCoordinatorForSubscription(payload.subscriptionId);
    if (!coordinatorNodeId) return;

    const myNodeId = this.clusterManager.config.nodeId;

    if (coordinatorNodeId === myNodeId) {
      // We are the coordinator, handle locally
      this.handleSubUpdate(myNodeId, payload);
    } else {
      // Send to remote coordinator
      this.clusterManager.send(coordinatorNodeId, 'CLUSTER_SUB_UPDATE', payload);
    }

    // Record metrics for sent updates
    this.metricsService?.incDistributedSubUpdates('sent', payload.changeType);
  }

  /**
   * Get coordinator node for a subscription.
   */
  private getCoordinatorForSubscription(subscriptionId: string): string | null {
    // Check if we are the coordinator
    if (this.subscriptions.has(subscriptionId)) {
      return this.clusterManager.config.nodeId;
    }

    // Check local search coordinator for distributed subscriptions
    const searchSub = this.localSearchCoordinator.getDistributedSubscription(subscriptionId);
    if (searchSub?.coordinatorNodeId) {
      return searchSub.coordinatorNodeId;
    }

    return null;
  }

  /**
   * Merge initial results from all nodes using RRF.
   */
  mergeInitialResults(subscription: DistributedSubscription): DistributedSubscriptionResult {
    const allNodes = new Set(this.clusterManager.getMembers());
    const failedNodes = Array.from(allNodes).filter(n => !subscription.registeredNodes.has(n));

    // Build result sets for RRF (each node's results as a separate set)
    const resultSets: Array<Array<{ docId: string; score: number; source: string }>> = [];
    // Keep original result data for lookup
    const resultDataMap = new Map<string, { key: string; value: unknown; score: number; matchedTerms?: string[]; nodeId: string }>();

    for (const [nodeId, ack] of subscription.pendingResults) {
      if (ack.success && ack.initialResults) {
        const rankedResults: Array<{ docId: string; score: number; source: string }> = [];

        for (const r of ack.initialResults) {
          rankedResults.push({
            docId: r.key,
            score: r.score ?? 0,
            source: nodeId,
          });
          // Store original data if not already present (first node wins)
          if (!resultDataMap.has(r.key)) {
            resultDataMap.set(r.key, {
              key: r.key,
              value: r.value,
              score: r.score ?? 0,
              matchedTerms: r.matchedTerms,
              nodeId,
            });
          }
        }

        if (rankedResults.length > 0) {
          resultSets.push(rankedResults);
        }
      }
    }

    // Use RRF to merge results
    const mergedResults = this.rrf.merge(resultSets);
    const limit = subscription.searchOptions?.limit ?? 10;

    // Build final results with original data, applying limit
    const results: Array<{ key: string; value: unknown; score?: number; matchedTerms?: string[] }> = [];
    for (const merged of mergedResults.slice(0, limit)) {
      const original = resultDataMap.get(merged.docId);
      if (original) {
        results.push({
          key: original.key,
          value: original.value,
          score: merged.score, // Use RRF score
          matchedTerms: original.matchedTerms,
        });
        // Update current results cache
        subscription.currentResults.set(original.key, {
          value: original.value,
          score: merged.score,
          sourceNode: original.nodeId,
        });
      }
    }

    // Calculate total hits
    let totalHits = 0;
    for (const ack of subscription.pendingResults.values()) {
      totalHits += ack.totalHits ?? 0;
    }

    return {
      subscriptionId: subscription.id,
      results,
      totalHits,
      registeredNodes: Array.from(subscription.registeredNodes),
      failedNodes,
    };
  }

  /**
   * Build SEARCH_UPDATE message for client.
   */
  buildUpdateMessage(
    subscription: DistributedSubscription,
    payload: ClusterSubUpdatePayload
  ): { type: string; payload: unknown } {
    return {
      type: 'SEARCH_UPDATE',
      payload: {
        subscriptionId: payload.subscriptionId,
        key: payload.key,
        value: payload.value,
        score: payload.score,
        matchedTerms: payload.matchedTerms,
        changeType: payload.changeType,
      },
    };
  }
}
