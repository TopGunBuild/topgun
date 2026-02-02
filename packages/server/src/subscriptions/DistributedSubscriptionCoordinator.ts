/**
 * DistributedSubscriptionCoordinator - Facade for distributed live subscriptions
 *
 * Coordinates subscriptions across cluster nodes for both FTS (Search) and Query
 * predicates. Delegates to type-specific coordinators:
 * - DistributedSearchCoordinator for FTS subscriptions (RRF merging)
 * - DistributedQueryCoordinator for Query subscriptions (dedupe merging)
 *
 * Preserved as a facade for backward compatibility with existing consumers.
 *
 * @module subscriptions/DistributedSubscriptionCoordinator
 */

import { EventEmitter } from 'events';
import {
  ClusterSubRegisterPayloadSchema,
  ClusterSubAckPayloadSchema,
  ClusterSubUpdatePayloadSchema,
  ClusterSubUnregisterPayloadSchema,
  type ClusterSubRegisterPayload,
  type ClusterSubAckPayload,
  type ClusterSubUpdatePayload,
  type ClusterSubUnregisterPayload,
  type SearchOptions,
  type Query,
} from '@topgunbuild/core';
import { ClusterManager } from '../cluster/ClusterManager';
import { SearchCoordinator } from '../search/SearchCoordinator';
import { QueryRegistry } from '../query/QueryRegistry';
import { MetricsService } from '../monitoring/MetricsService';
import { logger } from '../utils/logger';
import { WebSocket } from 'ws';
import { DistributedSearchCoordinator } from './DistributedSearchCoordinator';
import { DistributedQueryCoordinator } from './DistributedQueryCoordinator';
import type {
  DistributedSubscription,
  DistributedSubscriptionConfig,
  DistributedSubscriptionResult,
} from './DistributedSubscriptionBase';

// Re-export interfaces for backward compatibility
export type { DistributedSubscription, DistributedSubscriptionConfig, DistributedSubscriptionResult };

/**
 * DistributedSubscriptionCoordinator - Facade for distributed live subscriptions.
 *
 * Delegates to type-specific coordinators while maintaining backward compatibility.
 *
 * @example
 * ```typescript
 * const coordinator = new DistributedSubscriptionCoordinator(
 *   clusterManager,
 *   queryRegistry,
 *   searchCoordinator
 * );
 *
 * // Subscribe to a distributed search
 * const result = await coordinator.subscribeSearch(
 *   'sub-123',
 *   clientSocket,
 *   'articles',
 *   'machine learning',
 *   { limit: 10 }
 * );
 *
 * // Later: unsubscribe
 * await coordinator.unsubscribe('sub-123');
 * ```
 */
export class DistributedSubscriptionCoordinator extends EventEmitter {
  private readonly clusterManager: ClusterManager;
  private readonly localQueryRegistry: QueryRegistry;
  private readonly localSearchCoordinator: SearchCoordinator;
  private readonly searchCoordinator: DistributedSearchCoordinator;
  private readonly queryCoordinator: DistributedQueryCoordinator;

  constructor(
    clusterManager: ClusterManager,
    queryRegistry: QueryRegistry,
    searchCoordinator: SearchCoordinator,
    config?: DistributedSubscriptionConfig,
    metricsService?: MetricsService
  ) {
    super();
    this.clusterManager = clusterManager;
    this.localQueryRegistry = queryRegistry;
    this.localSearchCoordinator = searchCoordinator;

    // Create type-specific coordinators
    this.searchCoordinator = new DistributedSearchCoordinator(
      clusterManager,
      searchCoordinator,
      config,
      metricsService
    );

    this.queryCoordinator = new DistributedQueryCoordinator(
      clusterManager,
      queryRegistry,
      config,
      metricsService
    );

    // Listen for cluster messages
    this.clusterManager.on('message', this.handleClusterMessage.bind(this));

    logger.debug('DistributedSubscriptionCoordinator (facade) initialized');
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
    return this.searchCoordinator.subscribeSearch(
      subscriptionId,
      clientSocket,
      mapName,
      query,
      options
    );
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
    return this.queryCoordinator.subscribeQuery(
      subscriptionId,
      clientSocket,
      mapName,
      query
    );
  }

  /**
   * Unsubscribe from a distributed subscription.
   * Routes to correct coordinator based on subscription type.
   *
   * @param subscriptionId - Subscription ID to unsubscribe
   */
  async unsubscribe(subscriptionId: string): Promise<void> {
    // Check search coordinator first
    if (this.searchCoordinator.hasSubscription(subscriptionId)) {
      return this.searchCoordinator.unsubscribe(subscriptionId);
    }

    // Check query coordinator
    if (this.queryCoordinator.hasSubscription(subscriptionId)) {
      return this.queryCoordinator.unsubscribe(subscriptionId);
    }

    logger.warn({ subscriptionId }, 'Attempt to unsubscribe from unknown subscription');
  }

  /**
   * Handle client disconnect - unsubscribe all their subscriptions.
   */
  unsubscribeClient(clientSocket: WebSocket): void {
    this.searchCoordinator.unsubscribeClient(clientSocket);
    this.queryCoordinator.unsubscribeClient(clientSocket);
  }

  /**
   * Get active subscription count (combined from both coordinators).
   */
  getActiveSubscriptionCount(): number {
    return (
      this.searchCoordinator.getActiveSubscriptionCount() +
      this.queryCoordinator.getActiveSubscriptionCount()
    );
  }

  /**
   * Handle incoming cluster messages with Zod validation.
   * Routes to appropriate coordinator based on message type and subscription type.
   */
  private handleClusterMessage(msg: { type: string; senderId: string; payload: unknown }): void {
    switch (msg.type) {
      case 'CLUSTER_SUB_REGISTER': {
        const parsed = ClusterSubRegisterPayloadSchema.safeParse(msg.payload);
        if (!parsed.success) {
          logger.warn(
            { senderId: msg.senderId, error: parsed.error.message },
            'Invalid CLUSTER_SUB_REGISTER payload'
          );
          return;
        }
        this.handleSubRegister(msg.senderId, parsed.data);
        break;
      }
      case 'CLUSTER_SUB_ACK': {
        const parsed = ClusterSubAckPayloadSchema.safeParse(msg.payload);
        if (!parsed.success) {
          logger.warn(
            { senderId: msg.senderId, error: parsed.error.message },
            'Invalid CLUSTER_SUB_ACK payload'
          );
          return;
        }
        this.handleSubAck(msg.senderId, parsed.data);
        break;
      }
      case 'CLUSTER_SUB_UPDATE': {
        const parsed = ClusterSubUpdatePayloadSchema.safeParse(msg.payload);
        if (!parsed.success) {
          logger.warn(
            { senderId: msg.senderId, error: parsed.error.message },
            'Invalid CLUSTER_SUB_UPDATE payload'
          );
          return;
        }
        this.handleSubUpdate(msg.senderId, parsed.data);
        break;
      }
      case 'CLUSTER_SUB_UNREGISTER': {
        const parsed = ClusterSubUnregisterPayloadSchema.safeParse(msg.payload);
        if (!parsed.success) {
          logger.warn(
            { senderId: msg.senderId, error: parsed.error.message },
            'Invalid CLUSTER_SUB_UNREGISTER payload'
          );
          return;
        }
        this.handleSubUnregister(msg.senderId, parsed.data);
        break;
      }
    }
  }

  /**
   * Handle CLUSTER_SUB_REGISTER from coordinator (we are a data node).
   * Routes to appropriate coordinator based on subscription type.
   */
  private handleSubRegister(senderId: string, payload: ClusterSubRegisterPayload): void {
    if (payload.type === 'SEARCH') {
      this.searchCoordinator.handleSubRegister(senderId, payload);
    } else {
      this.queryCoordinator.handleSubRegister(senderId, payload);
    }
  }

  /**
   * Handle CLUSTER_SUB_ACK from a data node.
   * Routes to appropriate coordinator based on subscription lookup.
   */
  private handleSubAck(senderId: string, payload: ClusterSubAckPayload): void {
    // Route to coordinator that has this subscription
    if (this.searchCoordinator.hasSubscription(payload.subscriptionId)) {
      this.searchCoordinator.handleSubAck(senderId, payload);
    } else if (this.queryCoordinator.hasSubscription(payload.subscriptionId)) {
      this.queryCoordinator.handleSubAck(senderId, payload);
    } else {
      logger.warn(
        { subscriptionId: payload.subscriptionId, nodeId: payload.nodeId },
        'Received ACK for unknown subscription'
      );
    }
  }

  /**
   * Handle CLUSTER_SUB_UPDATE from a data node.
   * Routes to appropriate coordinator based on subscription lookup.
   */
  private handleSubUpdate(senderId: string, payload: ClusterSubUpdatePayload): void {
    // Route to coordinator that has this subscription
    if (this.searchCoordinator.hasSubscription(payload.subscriptionId)) {
      this.searchCoordinator.handleSubUpdate(senderId, payload);
    } else if (this.queryCoordinator.hasSubscription(payload.subscriptionId)) {
      this.queryCoordinator.handleSubUpdate(senderId, payload);
    } else {
      logger.warn(
        { subscriptionId: payload.subscriptionId },
        'Update for unknown subscription'
      );
    }
  }

  /**
   * Handle CLUSTER_SUB_UNREGISTER from coordinator.
   * Routes to both local coordinators for cleanup.
   */
  private handleSubUnregister(senderId: string, payload: ClusterSubUnregisterPayload): void {
    logger.debug(
      { subscriptionId: payload.subscriptionId },
      'Received subscription unregister request'
    );

    // Unregister from local SearchCoordinator
    this.localSearchCoordinator.unsubscribe(payload.subscriptionId);

    // Unregister from local QueryRegistry
    this.localQueryRegistry.unregister(payload.subscriptionId);
  }

  /**
   * Get coordinator node for a subscription.
   * For remote subscriptions (where we are a data node), this returns the coordinator.
   */
  private getCoordinatorForSubscription(subscriptionId: string): string | null {
    // Check if we are the coordinator via search coordinator
    if (this.searchCoordinator.hasSubscription(subscriptionId)) {
      return this.clusterManager.config.nodeId;
    }

    // Check if we are the coordinator via query coordinator
    if (this.queryCoordinator.hasSubscription(subscriptionId)) {
      return this.clusterManager.config.nodeId;
    }

    // Check local search coordinator for distributed subscriptions
    const searchSub = this.localSearchCoordinator.getDistributedSubscription(subscriptionId);
    if (searchSub?.coordinatorNodeId) {
      return searchSub.coordinatorNodeId;
    }

    // Check local query registry for distributed subscriptions
    const querySub = this.localQueryRegistry.getDistributedSubscription(subscriptionId);
    if (querySub?.coordinatorNodeId) {
      return querySub.coordinatorNodeId;
    }

    return null;
  }

  /**
   * Cleanup on destroy.
   */
  destroy(): void {
    this.searchCoordinator.destroy();
    this.queryCoordinator.destroy();
    this.removeAllListeners();
  }
}
