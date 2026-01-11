/**
 * DistributedSubscriptionCoordinator - Manages distributed live subscriptions
 *
 * Coordinates subscriptions across cluster nodes for both FTS (Search) and Query
 * predicates. When this node is the coordinator for a subscription, it:
 * 1. Registers the subscription on all nodes
 * 2. Collects initial results from all nodes
 * 3. Receives delta updates from nodes when data changes
 * 4. Forwards updates to the client
 *
 * @module subscriptions/DistributedSubscriptionCoordinator
 */

import { EventEmitter } from 'events';
import {
  ReciprocalRankFusion,
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
import { SearchCoordinator, type ServerSearchResult } from '../search/SearchCoordinator';
import { QueryRegistry } from '../query/QueryRegistry';
import { MetricsService } from '../monitoring/MetricsService';
import { logger } from '../utils/logger';
import { WebSocket } from 'ws';

/**
 * State for a distributed subscription where this node is the coordinator.
 */
export interface DistributedSubscription {
  /** Unique subscription ID */
  id: string;
  /** Subscription type */
  type: 'SEARCH' | 'QUERY';
  /** Node ID of the coordinator (this node) */
  coordinatorNodeId: string;
  /** Client WebSocket (for sending updates) */
  clientSocket: WebSocket;
  /** Map name being subscribed to */
  mapName: string;

  // For SEARCH subscriptions
  /** Search query string */
  searchQuery?: string;
  /** Search options */
  searchOptions?: SearchOptions;

  // For QUERY subscriptions
  /** Query predicate */
  queryPredicate?: Query;
  /** Sort order */
  querySort?: Record<string, 'asc' | 'desc'>;

  /** Nodes that have registered this subscription */
  registeredNodes: Set<string>;
  /** Pending initial results from nodes (before ACK complete) */
  pendingResults: Map<string, ClusterSubAckPayload>;
  /** When subscription was created */
  createdAt: number;
  /** Current merged result set (for delta computation) */
  currentResults: Map<string, { value: unknown; score?: number; sourceNode: string }>;
}

/**
 * Configuration for DistributedSubscriptionCoordinator.
 */
export interface DistributedSubscriptionConfig {
  /** Timeout for waiting on node ACKs (ms) */
  ackTimeoutMs?: number;
  /** RRF constant k for search result merging */
  rrfK?: number;
}

/**
 * Result of a distributed subscription registration.
 */
export interface DistributedSubscriptionResult {
  /** Subscription ID */
  subscriptionId: string;
  /** Initial merged results */
  results: Array<{ key: string; value: unknown; score?: number; matchedTerms?: string[] }>;
  /** Total hits across all nodes */
  totalHits: number;
  /** Nodes that registered the subscription */
  registeredNodes: string[];
  /** Nodes that failed to register */
  failedNodes: string[];
}

const DEFAULT_CONFIG: Required<DistributedSubscriptionConfig> = {
  ackTimeoutMs: 5000,
  rrfK: 60,
};

/**
 * DistributedSubscriptionCoordinator manages distributed live subscriptions.
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
  private readonly config: Required<DistributedSubscriptionConfig>;
  private readonly rrf: ReciprocalRankFusion;
  private readonly metricsService?: MetricsService;

  /**
   * Active subscriptions where this node is coordinator.
   * subscriptionId → SubscriptionState
   */
  private readonly subscriptions = new Map<string, DistributedSubscription>();

  /**
   * Track which nodes have acknowledged subscription registration.
   * subscriptionId → Set<nodeId>
   */
  private readonly nodeAcks = new Map<string, Set<string>>();

  /**
   * Pending ACK promises for subscription registration.
   * subscriptionId → { resolve, reject, timeout, startTime }
   */
  private readonly pendingAcks = new Map<string, {
    resolve: (result: DistributedSubscriptionResult) => void;
    reject: (error: Error) => void;
    timeoutHandle: NodeJS.Timeout;
    startTime: number;
  }>();

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
    this.config = { ...DEFAULT_CONFIG, ...config };
    this.rrf = new ReciprocalRankFusion({ k: this.config.rrfK });
    this.metricsService = metricsService;

    // Listen for cluster messages
    this.clusterManager.on('message', this.handleClusterMessage.bind(this));

    // Listen for local search updates (emitted by SearchCoordinator)
    this.localSearchCoordinator.on('distributedUpdate', this.handleLocalSearchUpdate.bind(this));

    // Listen for cluster node disconnect to cleanup subscriptions
    this.clusterManager.on('memberLeft', this.handleMemberLeft.bind(this));

    logger.debug('DistributedSubscriptionCoordinator initialized');
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
   * Unsubscribe from a distributed subscription.
   *
   * @param subscriptionId - Subscription ID to unsubscribe
   */
  async unsubscribe(subscriptionId: string): Promise<void> {
    const subscription = this.subscriptions.get(subscriptionId);
    if (!subscription) {
      logger.warn({ subscriptionId }, 'Attempt to unsubscribe from unknown subscription');
      return;
    }

    logger.debug({ subscriptionId }, 'Unsubscribing from distributed subscription');

    const myNodeId = this.clusterManager.config.nodeId;

    // Notify all nodes to remove subscription
    const payload: ClusterSubUnregisterPayload = { subscriptionId };

    for (const nodeId of subscription.registeredNodes) {
      if (nodeId !== myNodeId) {
        this.clusterManager.send(nodeId, 'CLUSTER_SUB_UNREGISTER', payload);
      }
    }

    // Cleanup local subscription
    this.unregisterLocalSubscription(subscription);

    // Cleanup coordinator state
    this.subscriptions.delete(subscriptionId);
    this.nodeAcks.delete(subscriptionId);

    // Cancel any pending ACK
    const pendingAck = this.pendingAcks.get(subscriptionId);
    if (pendingAck) {
      clearTimeout(pendingAck.timeoutHandle);
      this.pendingAcks.delete(subscriptionId);
    }

    // Record metrics
    this.metricsService?.incDistributedSubUnsubscribe(subscription.type);
    this.metricsService?.decDistributedSubActive(subscription.type);
    this.metricsService?.setDistributedSubPendingAcks(this.pendingAcks.size);
  }

  /**
   * Handle client disconnect - unsubscribe all their subscriptions.
   */
  unsubscribeClient(clientSocket: WebSocket): void {
    const subscriptionsToRemove: string[] = [];

    for (const [subId, sub] of this.subscriptions) {
      if (sub.clientSocket === clientSocket) {
        subscriptionsToRemove.push(subId);
      }
    }

    for (const subId of subscriptionsToRemove) {
      this.unsubscribe(subId);
    }
  }

  /**
   * Get active subscription count.
   */
  getActiveSubscriptionCount(): number {
    return this.subscriptions.size;
  }

  /**
   * Handle incoming cluster messages with Zod validation.
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
   * Handle cluster node disconnect - cleanup subscriptions involving this node.
   */
  private handleMemberLeft(nodeId: string): void {
    logger.debug({ nodeId }, 'Handling member left for distributed subscriptions');

    const subscriptionsToRemove: string[] = [];
    const myNodeId = this.clusterManager.config.nodeId;

    for (const [subId, subscription] of this.subscriptions) {
      // If we are the coordinator and this node was registered
      if (subscription.registeredNodes.has(nodeId)) {
        subscription.registeredNodes.delete(nodeId);

        // Remove any results from this node
        for (const [key, result] of subscription.currentResults) {
          if (result.sourceNode === nodeId) {
            subscription.currentResults.delete(key);
          }
        }

        logger.debug(
          { subscriptionId: subId, nodeId, remainingNodes: subscription.registeredNodes.size },
          'Removed disconnected node from subscription'
        );
      }
    }

    // Check for any pending ACKs that were waiting for this node
    for (const [subId, pending] of this.pendingAcks) {
      const acks = this.nodeAcks.get(subId);
      if (acks && !acks.has(nodeId)) {
        // This node hasn't ACK'd yet, treat as failed
        acks.add(nodeId); // Mark as "received" to complete the wait
        this.checkAcksComplete(subId);
      }
    }

    // Cleanup local subscriptions where the disconnected node was the coordinator
    // (SearchCoordinator and QueryRegistry handle their own cleanup)
    this.localSearchCoordinator.unsubscribeByCoordinator(nodeId);
    this.localQueryRegistry.unregisterByCoordinator(nodeId);

    // Record metrics
    this.metricsService?.incDistributedSubNodeDisconnect();
  }

  /**
   * Handle CLUSTER_SUB_REGISTER from coordinator (we are a data node).
   */
  private handleSubRegister(senderId: string, payload: ClusterSubRegisterPayload): void {
    const myNodeId = this.clusterManager.config.nodeId;

    logger.debug(
      { subscriptionId: payload.subscriptionId, coordinator: payload.coordinatorNodeId },
      'Received distributed subscription registration'
    );

    let ackPayload: ClusterSubAckPayload;

    try {
      if (payload.type === 'SEARCH') {
        // Register local search subscription
        const result = this.localSearchCoordinator.registerDistributedSubscription(
          payload.subscriptionId,
          payload.mapName,
          payload.searchQuery!,
          payload.searchOptions || {},
          payload.coordinatorNodeId
        );

        ackPayload = {
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
      } else {
        // Register local query subscription
        const results = this.localQueryRegistry.registerDistributed(
          payload.subscriptionId,
          payload.mapName,
          payload.queryPredicate!,
          payload.coordinatorNodeId
        );

        ackPayload = {
          subscriptionId: payload.subscriptionId,
          nodeId: myNodeId,
          success: true,
          initialResults: results.map(r => ({
            key: r.key,
            value: r.value,
          })),
          totalHits: results.length,
        };
      }
    } catch (error) {
      logger.error(
        { subscriptionId: payload.subscriptionId, error },
        'Failed to register distributed subscription locally'
      );

      ackPayload = {
        subscriptionId: payload.subscriptionId,
        nodeId: myNodeId,
        success: false,
        error: error instanceof Error ? error.message : 'Unknown error',
      };
    }

    // Send ACK back to coordinator
    this.clusterManager.send(payload.coordinatorNodeId, 'CLUSTER_SUB_ACK', ackPayload);
  }

  /**
   * Handle CLUSTER_SUB_ACK from a data node.
   */
  private handleSubAck(senderId: string, payload: ClusterSubAckPayload): void {
    const subscription = this.subscriptions.get(payload.subscriptionId);
    if (!subscription) {
      logger.warn(
        { subscriptionId: payload.subscriptionId, nodeId: payload.nodeId },
        'Received ACK for unknown subscription'
      );
      return;
    }

    const acks = this.nodeAcks.get(payload.subscriptionId);
    if (!acks) return;

    logger.debug(
      { subscriptionId: payload.subscriptionId, nodeId: payload.nodeId, success: payload.success },
      'Received subscription ACK'
    );

    if (payload.success) {
      subscription.registeredNodes.add(payload.nodeId);
      subscription.pendingResults.set(payload.nodeId, payload);
    }

    acks.add(payload.nodeId);

    // Check if all ACKs received
    this.checkAcksComplete(payload.subscriptionId);
  }

  /**
   * Handle CLUSTER_SUB_UPDATE from a data node.
   */
  private handleSubUpdate(senderId: string, payload: ClusterSubUpdatePayload): void {
    const subscription = this.subscriptions.get(payload.subscriptionId);
    if (!subscription) {
      logger.warn(
        { subscriptionId: payload.subscriptionId },
        'Update for unknown subscription'
      );
      return;
    }

    logger.debug(
      { subscriptionId: payload.subscriptionId, key: payload.key, changeType: payload.changeType },
      'Received subscription update'
    );

    // Update current results cache
    if (payload.changeType === 'LEAVE') {
      subscription.currentResults.delete(payload.key);
    } else {
      subscription.currentResults.set(payload.key, {
        value: payload.value,
        score: payload.score,
        sourceNode: payload.sourceNodeId,
      });
    }

    // Forward to client
    this.forwardUpdateToClient(subscription, payload);

    // Record metrics
    this.metricsService?.incDistributedSubUpdates('received', payload.changeType);

    // Calculate and record latency if timestamp is available
    if (payload.timestamp) {
      const latencyMs = Date.now() - payload.timestamp;
      this.metricsService?.recordDistributedSubUpdateLatency(subscription.type, latencyMs);
    }
  }

  /**
   * Handle CLUSTER_SUB_UNREGISTER from coordinator.
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
  private unregisterLocalSubscription(subscription: DistributedSubscription): void {
    if (subscription.type === 'SEARCH') {
      this.localSearchCoordinator.unsubscribe(subscription.id);
    } else {
      this.localQueryRegistry.unregister(subscription.id);
    }
  }

  /**
   * Handle local ACK (from this node's registration).
   */
  private handleLocalAck(
    subscriptionId: string,
    nodeId: string,
    result: { results?: ServerSearchResult[] | Array<{ key: string; value: unknown }>; totalHits?: number }
  ): void {
    const subscription = this.subscriptions.get(subscriptionId);
    if (!subscription) return;

    const acks = this.nodeAcks.get(subscriptionId);
    if (!acks) return;

    subscription.registeredNodes.add(nodeId);
    subscription.pendingResults.set(nodeId, {
      subscriptionId,
      nodeId,
      success: true,
      initialResults: (result.results || []).map((r: any) => ({
        key: r.key,
        value: r.value,
        score: r.score,
        matchedTerms: r.matchedTerms,
      })),
      totalHits: result.totalHits || (result.results?.length ?? 0),
    });

    acks.add(nodeId);
    this.checkAcksComplete(subscriptionId);
  }

  /**
   * Wait for all node ACKs with timeout.
   */
  private waitForAcks(
    subscriptionId: string,
    expectedNodes: Set<string>
  ): Promise<DistributedSubscriptionResult> {
    return new Promise((resolve, reject) => {
      const startTime = performance.now();
      const timeoutHandle = setTimeout(() => {
        this.resolveWithPartialAcks(subscriptionId);
      }, this.config.ackTimeoutMs);

      this.pendingAcks.set(subscriptionId, { resolve, reject, timeoutHandle, startTime });

      // Update pending ACKs metric
      this.metricsService?.setDistributedSubPendingAcks(this.pendingAcks.size);

      // Check if already complete (e.g., single-node cluster)
      this.checkAcksComplete(subscriptionId);
    });
  }

  /**
   * Check if all ACKs have been received.
   */
  private checkAcksComplete(subscriptionId: string): void {
    const subscription = this.subscriptions.get(subscriptionId);
    const acks = this.nodeAcks.get(subscriptionId);
    const pendingAck = this.pendingAcks.get(subscriptionId);

    if (!subscription || !acks || !pendingAck) return;

    const allNodes = new Set(this.clusterManager.getMembers());
    if (acks.size >= allNodes.size) {
      clearTimeout(pendingAck.timeoutHandle);
      this.pendingAcks.delete(subscriptionId);

      const duration = performance.now() - pendingAck.startTime;
      const result = this.mergeInitialResults(subscription);
      const hasFailures = result.failedNodes.length > 0;

      this.recordCompletionMetrics(
        subscription,
        result,
        duration,
        hasFailures ? 'timeout' : 'success'
      );

      pendingAck.resolve(result);
    }
  }

  /**
   * Resolve with partial ACKs (on timeout).
   */
  private resolveWithPartialAcks(subscriptionId: string): void {
    const subscription = this.subscriptions.get(subscriptionId);
    const pendingAck = this.pendingAcks.get(subscriptionId);

    if (!subscription || !pendingAck) return;

    this.pendingAcks.delete(subscriptionId);

    logger.warn(
      { subscriptionId, registeredNodes: Array.from(subscription.registeredNodes) },
      'Subscription ACK timeout, resolving with partial results'
    );

    const duration = performance.now() - pendingAck.startTime;
    const result = this.mergeInitialResults(subscription);

    this.recordCompletionMetrics(subscription, result, duration, 'timeout');

    pendingAck.resolve(result);
  }

  /**
   * Record metrics when subscription registration completes.
   */
  private recordCompletionMetrics(
    subscription: DistributedSubscription,
    result: DistributedSubscriptionResult,
    durationMs: number,
    status: 'success' | 'timeout'
  ): void {
    this.metricsService?.incDistributedSub(subscription.type, status);
    this.metricsService?.recordDistributedSubRegistration(subscription.type, durationMs);
    this.metricsService?.recordDistributedSubInitialResultsCount(subscription.type, result.results.length);
    this.metricsService?.setDistributedSubPendingAcks(this.pendingAcks.size);

    // Record ACK metrics
    this.metricsService?.incDistributedSubAck('success', subscription.registeredNodes.size);
    this.metricsService?.incDistributedSubAck('timeout', result.failedNodes.length);
  }

  /**
   * Merge initial results from all nodes.
   */
  private mergeInitialResults(subscription: DistributedSubscription): DistributedSubscriptionResult {
    const allNodes = new Set(this.clusterManager.getMembers());
    const failedNodes = Array.from(allNodes).filter(n => !subscription.registeredNodes.has(n));

    if (subscription.type === 'SEARCH') {
      return this.mergeSearchResults(subscription, failedNodes);
    } else {
      return this.mergeQueryResults(subscription, failedNodes);
    }
  }

  /**
   * Merge search results using RRF.
   */
  private mergeSearchResults(
    subscription: DistributedSubscription,
    failedNodes: string[]
  ): DistributedSubscriptionResult {
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
   * Merge query results (simple dedupe by key).
   */
  private mergeQueryResults(
    subscription: DistributedSubscription,
    failedNodes: string[]
  ): DistributedSubscriptionResult {
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
   * Forward update to client WebSocket.
   */
  private forwardUpdateToClient(
    subscription: DistributedSubscription,
    payload: ClusterSubUpdatePayload
  ): void {
    if (subscription.clientSocket.readyState !== WebSocket.OPEN) {
      logger.warn(
        { subscriptionId: subscription.id },
        'Cannot forward update, client socket not open'
      );
      return;
    }

    const message = subscription.type === 'SEARCH'
      ? {
          type: 'SEARCH_UPDATE',
          payload: {
            subscriptionId: payload.subscriptionId,
            key: payload.key,
            value: payload.value,
            score: payload.score,
            matchedTerms: payload.matchedTerms,
            type: payload.changeType,
          },
        }
      : {
          type: 'QUERY_UPDATE',
          payload: {
            queryId: payload.subscriptionId,
            key: payload.key,
            value: payload.value,
            type: payload.changeType,
          },
        };

    try {
      subscription.clientSocket.send(JSON.stringify(message));
    } catch (error) {
      logger.error(
        { subscriptionId: subscription.id, error },
        'Failed to send update to client'
      );
    }
  }

  /**
   * Get coordinator node for a subscription.
   * For remote subscriptions (where we are a data node), this returns the coordinator.
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
    // Unsubscribe all subscriptions
    for (const subscriptionId of this.subscriptions.keys()) {
      this.unsubscribe(subscriptionId);
    }

    // Clear pending ACKs
    for (const pending of this.pendingAcks.values()) {
      clearTimeout(pending.timeoutHandle);
    }
    this.pendingAcks.clear();

    this.removeAllListeners();
  }
}
