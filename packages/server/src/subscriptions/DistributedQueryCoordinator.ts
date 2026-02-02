/**
 * DistributedQueryCoordinator - Distributed Query subscription coordinator
 *
 * Handles distributed query subscriptions using simple deduplication
 * for merging results from multiple cluster nodes.
 *
 * @module subscriptions/DistributedQueryCoordinator
 */

import {
  ClusterSubRegisterPayloadSchema,
  type ClusterSubRegisterPayload,
  type ClusterSubUpdatePayload,
  type Query,
} from '@topgunbuild/core';
import { ClusterManager } from '../cluster/ClusterManager';
import { QueryRegistry } from '../query/QueryRegistry';
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
 * DistributedQueryCoordinator manages distributed query subscriptions.
 *
 * Uses simple key-based deduplication to merge query results from multiple nodes,
 * keeping the first occurrence of each key.
 *
 * @example
 * ```typescript
 * const coordinator = new DistributedQueryCoordinator(
 *   clusterManager,
 *   queryRegistry
 * );
 *
 * const result = await coordinator.subscribeQuery(
 *   'sub-456',
 *   clientSocket,
 *   'users',
 *   { field: 'status', op: 'eq', value: 'active' }
 * );
 * ```
 */
export class DistributedQueryCoordinator extends DistributedSubscriptionBase {
  private readonly localQueryRegistry: QueryRegistry;

  constructor(
    clusterManager: ClusterManager,
    queryRegistry: QueryRegistry,
    config?: DistributedSubscriptionConfig,
    metricsService?: MetricsService
  ) {
    super(clusterManager, config, metricsService);
    this.localQueryRegistry = queryRegistry;

    logger.debug('DistributedQueryCoordinator initialized');
  }

  /**
   * Get the subscription type handled by this coordinator.
   */
  getSubscriptionType(): 'SEARCH' | 'QUERY' {
    return 'QUERY';
  }

  /**
   * Create a new distributed query subscription.
   *
   * @param subscriptionId - Unique subscription ID
   * @param clientSocket - Client WebSocket for sending updates
   * @param mapName - Map name to query
   * @param query - Query predicate
   * @returns Promise resolving to initial results
   */
  async subscribeQuery(
    subscriptionId: string,
    clientSocket: WebSocket,
    mapName: string,
    query: Query
  ): Promise<DistributedSubscriptionResult> {
    const myNodeId = this.clusterManager.config.nodeId;
    const allNodes = new Set(this.clusterManager.getMembers());

    logger.debug(
      { subscriptionId, mapName, nodes: Array.from(allNodes) },
      'Creating distributed query subscription'
    );

    // Create subscription state
    const subscription: DistributedSubscription = {
      id: subscriptionId,
      type: 'QUERY',
      coordinatorNodeId: myNodeId,
      clientSocket,
      mapName,
      queryPredicate: query,
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
      type: 'QUERY',
      queryPredicate: query,
    };

    // Register locally first
    const localResults = this.registerLocalQuerySubscription(subscription);
    this.handleLocalAck(subscriptionId, myNodeId, { results: localResults, totalHits: localResults.length });

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
   * Handle CLUSTER_SUB_REGISTER for QUERY type registrations.
   */
  handleSubRegister(senderId: string, payload: ClusterSubRegisterPayload): void {
    if (payload.type !== 'QUERY') {
      logger.warn(
        { subscriptionId: payload.subscriptionId, type: payload.type },
        'DistributedQueryCoordinator received non-QUERY registration'
      );
      return;
    }

    const myNodeId = this.clusterManager.config.nodeId;

    logger.debug(
      { subscriptionId: payload.subscriptionId, coordinator: payload.coordinatorNodeId },
      'Received distributed query subscription registration'
    );

    try {
      // Register local query subscription
      const results = this.localQueryRegistry.registerDistributed(
        payload.subscriptionId,
        payload.mapName,
        payload.queryPredicate!,
        payload.coordinatorNodeId
      );

      const ackPayload = {
        subscriptionId: payload.subscriptionId,
        nodeId: myNodeId,
        success: true,
        initialResults: results.map(r => ({
          key: r.key,
          value: r.value,
        })),
        totalHits: results.length,
      };

      // Send ACK back to coordinator
      this.clusterManager.send(payload.coordinatorNodeId, 'CLUSTER_SUB_ACK', ackPayload);
    } catch (error) {
      logger.error(
        { subscriptionId: payload.subscriptionId, error },
        'Failed to register distributed query subscription locally'
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
   * Register a local query subscription for a distributed coordinator.
   */
  private registerLocalQuerySubscription(
    subscription: DistributedSubscription
  ): Array<{ key: string; value: unknown }> {
    return this.localQueryRegistry.registerDistributed(
      subscription.id,
      subscription.mapName,
      subscription.queryPredicate!,
      subscription.coordinatorNodeId
    );
  }

  /**
   * Unregister local subscription.
   */
  protected unregisterLocalSubscription(subscription: DistributedSubscription): void {
    this.localQueryRegistry.unregister(subscription.id);
  }

  /**
   * Cleanup local subscriptions when a coordinator node disconnects.
   */
  cleanupByCoordinator(nodeId: string): void {
    this.localQueryRegistry.unregisterByCoordinator(nodeId);
  }

  /**
   * Get coordinator node for a subscription.
   */
  private getCoordinatorForSubscription(subscriptionId: string): string | null {
    // Check if we are the coordinator
    if (this.subscriptions.has(subscriptionId)) {
      return this.clusterManager.config.nodeId;
    }

    // Check local query registry for distributed subscriptions
    const querySub = this.localQueryRegistry.getDistributedSubscription(subscriptionId);
    if (querySub?.coordinatorNodeId) {
      return querySub.coordinatorNodeId;
    }

    return null;
  }

  /**
   * Merge initial results from all nodes (simple dedupe by key).
   */
  mergeInitialResults(subscription: DistributedSubscription): DistributedSubscriptionResult {
    const allNodes = new Set(this.clusterManager.getMembers());
    const failedNodes = Array.from(allNodes).filter(n => !subscription.registeredNodes.has(n));

    const resultMap = new Map<string, { key: string; value: unknown }>();

    for (const [nodeId, ack] of subscription.pendingResults) {
      if (ack.success && ack.initialResults) {
        for (const result of ack.initialResults) {
          if (!resultMap.has(result.key)) {
            resultMap.set(result.key, { key: result.key, value: result.value });
            subscription.currentResults.set(result.key, {
              value: result.value,
              sourceNode: nodeId,
            });
          }
        }
      }
    }

    const results = Array.from(resultMap.values());

    return {
      subscriptionId: subscription.id,
      results,
      totalHits: results.length,
      registeredNodes: Array.from(subscription.registeredNodes),
      failedNodes,
    };
  }

  /**
   * Build QUERY_UPDATE message for client.
   */
  buildUpdateMessage(
    subscription: DistributedSubscription,
    payload: ClusterSubUpdatePayload
  ): { type: string; payload: unknown } {
    return {
      type: 'QUERY_UPDATE',
      payload: {
        queryId: payload.subscriptionId,
        key: payload.key,
        value: payload.value,
        type: payload.changeType,
      },
    };
  }
}
