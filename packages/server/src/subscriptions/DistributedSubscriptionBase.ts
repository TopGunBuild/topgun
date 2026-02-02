/**
 * DistributedSubscriptionBase - Abstract base class for distributed subscription coordinators
 *
 * Contains shared ACK management, timeout handling, and WebSocket communication logic
 * used by both DistributedSearchCoordinator and DistributedQueryCoordinator.
 *
 * @module subscriptions/DistributedSubscriptionBase
 */

import { EventEmitter } from 'events';
import {
  ClusterSubAckPayloadSchema,
  type ClusterSubAckPayload,
  type ClusterSubUpdatePayload,
} from '@topgunbuild/core';
import { ClusterManager } from '../cluster/ClusterManager';
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
  searchOptions?: import('@topgunbuild/core').SearchOptions;

  // For QUERY subscriptions
  /** Query predicate */
  queryPredicate?: import('@topgunbuild/core').Query;
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
 * Configuration for distributed subscription coordinators.
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
 * Abstract base class for distributed subscription coordinators.
 *
 * Provides shared functionality for:
 * - ACK tracking and timeout management
 * - Cluster member disconnect handling
 * - WebSocket communication helpers
 * - Metrics recording
 *
 * Subclasses implement type-specific subscription registration and result merging.
 */
export abstract class DistributedSubscriptionBase extends EventEmitter {
  protected readonly clusterManager: ClusterManager;
  protected readonly config: Required<DistributedSubscriptionConfig>;
  protected readonly metricsService?: MetricsService;

  /**
   * Active subscriptions where this node is coordinator.
   * subscriptionId -> SubscriptionState
   */
  protected readonly subscriptions = new Map<string, DistributedSubscription>();

  /**
   * Track which nodes have acknowledged subscription registration.
   * subscriptionId -> Set<nodeId>
   */
  protected readonly nodeAcks = new Map<string, Set<string>>();

  /**
   * Pending ACK promises for subscription registration.
   * subscriptionId -> { resolve, reject, timeout, startTime }
   */
  protected readonly pendingAcks = new Map<string, {
    resolve: (result: DistributedSubscriptionResult) => void;
    reject: (error: Error) => void;
    timeoutHandle: NodeJS.Timeout;
    startTime: number;
  }>();

  constructor(
    clusterManager: ClusterManager,
    config?: DistributedSubscriptionConfig,
    metricsService?: MetricsService
  ) {
    super();
    this.clusterManager = clusterManager;
    this.config = { ...DEFAULT_CONFIG, ...config };
    this.metricsService = metricsService;

    // Listen for cluster node disconnect to cleanup subscriptions
    this.clusterManager.on('memberLeft', this.handleMemberLeft.bind(this));
  }

  /**
   * Get the subscription type handled by this coordinator.
   */
  abstract getSubscriptionType(): 'SEARCH' | 'QUERY';

  /**
   * Merge initial results from all nodes.
   */
  abstract mergeInitialResults(subscription: DistributedSubscription): DistributedSubscriptionResult;

  /**
   * Build type-specific update message for client.
   */
  abstract buildUpdateMessage(
    subscription: DistributedSubscription,
    payload: ClusterSubUpdatePayload
  ): { type: string; payload: unknown };

  /**
   * Cleanup type-specific local subscriptions when a coordinator node disconnects.
   * Called by handleMemberLeft for type-specific cleanup.
   */
  abstract cleanupByCoordinator(nodeId: string): void;

  /**
   * Get active subscription count.
   */
  getActiveSubscriptionCount(): number {
    return this.subscriptions.size;
  }

  /**
   * Check if a subscription exists.
   */
  hasSubscription(subscriptionId: string): boolean {
    return this.subscriptions.has(subscriptionId);
  }

  /**
   * Get a subscription by ID.
   */
  getSubscription(subscriptionId: string): DistributedSubscription | undefined {
    return this.subscriptions.get(subscriptionId);
  }

  /**
   * Handle CLUSTER_SUB_ACK from a data node.
   */
  handleSubAck(senderId: string, payload: ClusterSubAckPayload): void {
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
   * Handle cluster node disconnect - cleanup subscriptions involving this node.
   */
  protected handleMemberLeft(nodeId: string): void {
    logger.debug({ nodeId }, `Handling member left for ${this.getSubscriptionType()} subscriptions`);

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
    this.cleanupByCoordinator(nodeId);

    // Record metrics
    this.metricsService?.incDistributedSubNodeDisconnect();
  }

  /**
   * Unsubscribe from a distributed subscription.
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
    const payload = { subscriptionId };

    for (const nodeId of subscription.registeredNodes) {
      if (nodeId !== myNodeId) {
        this.clusterManager.send(nodeId, 'CLUSTER_SUB_UNREGISTER', payload);
      }
    }

    // Cleanup local subscription (implemented by subclass)
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
   * Unregister local subscription - implemented by subclass.
   */
  protected abstract unregisterLocalSubscription(subscription: DistributedSubscription): void;

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
   * Wait for all node ACKs with timeout.
   */
  protected waitForAcks(
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
  protected checkAcksComplete(subscriptionId: string): void {
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
  protected resolveWithPartialAcks(subscriptionId: string): void {
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
  protected recordCompletionMetrics(
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
   * Handle local ACK (from this node's registration).
   */
  protected handleLocalAck(
    subscriptionId: string,
    nodeId: string,
    result: { results?: Array<{ key: string; value: unknown; score?: number; matchedTerms?: string[] }>; totalHits?: number }
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
      initialResults: (result.results || []).map((r) => ({
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
   * Send update message to client WebSocket with validation and error handling.
   */
  protected sendToClient(
    subscription: DistributedSubscription,
    message: { type: string; payload: unknown }
  ): boolean {
    if (subscription.clientSocket.readyState !== WebSocket.OPEN) {
      logger.warn(
        { subscriptionId: subscription.id },
        'Cannot forward update, client socket not open'
      );
      return false;
    }

    try {
      subscription.clientSocket.send(JSON.stringify(message));
      return true;
    } catch (error) {
      logger.error(
        { subscriptionId: subscription.id, error },
        'Failed to send update to client'
      );
      return false;
    }
  }

  /**
   * Forward update to client using type-specific message format.
   */
  protected forwardUpdateToClient(
    subscription: DistributedSubscription,
    payload: ClusterSubUpdatePayload
  ): void {
    const message = this.buildUpdateMessage(subscription, payload);
    this.sendToClient(subscription, message);
  }

  /**
   * Handle CLUSTER_SUB_UPDATE from a data node.
   */
  handleSubUpdate(senderId: string, payload: ClusterSubUpdatePayload): void {
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
