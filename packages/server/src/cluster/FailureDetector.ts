/**
 * FailureDetector - Phi Accrual Failure Detector
 *
 * Implements the Phi Accrual Failure Detection algorithm for distributed systems.
 * Based on the paper: "The φ Accrual Failure Detector" by Hayashibara et al.
 *
 * The detector provides a suspicion level (phi) rather than binary alive/dead status,
 * allowing the application to make decisions based on configurable thresholds.
 *
 * Hazelcast equivalent: com.hazelcast.internal.cluster.fd.PhiAccrualFailureDetector
 */

import { EventEmitter } from 'events';
import { logger } from '../utils/logger';

export interface FailureDetectorConfig {
  /** Interval between heartbeat checks (ms). Default: 1000 */
  heartbeatIntervalMs: number;
  /** Time after which a node is suspected if no heartbeat received (ms). Default: 5000 */
  suspicionTimeoutMs: number;
  /** Time after suspicion before confirming failure (ms). Default: 10000 */
  confirmationTimeoutMs: number;
  /** Phi threshold above which a node is considered suspected. Default: 8 */
  phiThreshold: number;
  /** Minimum samples required for accurate phi calculation. Default: 10 */
  minSamples: number;
  /** Maximum samples to keep in history. Default: 100 */
  maxSamples: number;
  /** Initial heartbeat interval estimate (ms). Default: 1000 */
  initialHeartbeatIntervalMs: number;
}

export const DEFAULT_FAILURE_DETECTOR_CONFIG: FailureDetectorConfig = {
  heartbeatIntervalMs: 1000,
  suspicionTimeoutMs: 5000,
  confirmationTimeoutMs: 10000,
  phiThreshold: 8,
  minSamples: 10,
  maxSamples: 100,
  initialHeartbeatIntervalMs: 1000,
};

export interface NodeState {
  /** Last heartbeat timestamp */
  lastHeartbeat: number;
  /** Heartbeat interval history for phi calculation */
  intervalHistory: number[];
  /** Whether node is currently suspected */
  isSuspected: boolean;
  /** Timestamp when suspicion started */
  suspicionStartTime?: number;
  /** Whether failure has been confirmed */
  isConfirmedFailed: boolean;
}

export interface FailureDetectorEvents {
  nodeSuspected: { nodeId: string; phi: number };
  nodeRecovered: { nodeId: string };
  nodeConfirmedFailed: { nodeId: string };
}

export class FailureDetector extends EventEmitter {
  private config: FailureDetectorConfig;
  private nodeStates: Map<string, NodeState> = new Map();
  private monitoringNodes: Set<string> = new Set();
  private checkTimer?: ReturnType<typeof setInterval>;
  private confirmationTimers: Map<string, ReturnType<typeof setTimeout>> = new Map();
  private started = false;

  constructor(config: Partial<FailureDetectorConfig> = {}) {
    super();
    this.config = { ...DEFAULT_FAILURE_DETECTOR_CONFIG, ...config };
  }

  /**
   * Start the failure detector monitoring loop.
   */
  start(): void {
    if (this.started) return;
    this.started = true;

    this.checkTimer = setInterval(() => {
      this.checkAllNodes();
    }, this.config.heartbeatIntervalMs);

    logger.info({ config: this.config }, 'FailureDetector started');
  }

  /**
   * Stop the failure detector and clean up.
   */
  stop(): void {
    if (!this.started) return;
    this.started = false;

    if (this.checkTimer) {
      clearInterval(this.checkTimer);
      this.checkTimer = undefined;
    }

    // Clear all confirmation timers
    for (const timer of this.confirmationTimers.values()) {
      clearTimeout(timer);
    }
    this.confirmationTimers.clear();

    logger.info('FailureDetector stopped');
  }

  /**
   * Start monitoring a node.
   */
  startMonitoring(nodeId: string): void {
    if (this.monitoringNodes.has(nodeId)) return;

    this.monitoringNodes.add(nodeId);
    this.nodeStates.set(nodeId, {
      lastHeartbeat: Date.now(),
      intervalHistory: [],
      isSuspected: false,
      isConfirmedFailed: false,
    });

    logger.debug({ nodeId }, 'Started monitoring node');
  }

  /**
   * Stop monitoring a node.
   */
  stopMonitoring(nodeId: string): void {
    this.monitoringNodes.delete(nodeId);
    this.nodeStates.delete(nodeId);

    const timer = this.confirmationTimers.get(nodeId);
    if (timer) {
      clearTimeout(timer);
      this.confirmationTimers.delete(nodeId);
    }

    logger.debug({ nodeId }, 'Stopped monitoring node');
  }

  /**
   * Record a heartbeat from a node.
   * This updates the node's state and clears any suspicion.
   */
  recordHeartbeat(nodeId: string): void {
    const state = this.nodeStates.get(nodeId);
    if (!state) {
      // Auto-start monitoring if not already
      this.startMonitoring(nodeId);
      return;
    }

    const now = Date.now();
    const interval = now - state.lastHeartbeat;

    // Update interval history
    state.intervalHistory.push(interval);
    if (state.intervalHistory.length > this.config.maxSamples) {
      state.intervalHistory.shift();
    }

    state.lastHeartbeat = now;

    // If was suspected, clear suspicion
    if (state.isSuspected) {
      state.isSuspected = false;
      state.suspicionStartTime = undefined;
      state.isConfirmedFailed = false;

      // Cancel confirmation timer
      const timer = this.confirmationTimers.get(nodeId);
      if (timer) {
        clearTimeout(timer);
        this.confirmationTimers.delete(nodeId);
      }

      this.emit('nodeRecovered', { nodeId });
      logger.info({ nodeId }, 'Node recovered');
    }
  }

  /**
   * Check all monitored nodes for failure.
   */
  private checkAllNodes(): void {
    for (const nodeId of this.monitoringNodes) {
      const phi = this.calculatePhi(nodeId);
      const state = this.nodeStates.get(nodeId);

      if (!state) continue;

      if (phi > this.config.phiThreshold) {
        if (!state.isSuspected) {
          state.isSuspected = true;
          state.suspicionStartTime = Date.now();

          this.emit('nodeSuspected', { nodeId, phi });
          logger.warn({ nodeId, phi }, 'Node suspected');

          // Schedule confirmation
          this.scheduleConfirmation(nodeId);
        }
      }
    }
  }

  /**
   * Schedule failure confirmation after suspicion timeout.
   */
  private scheduleConfirmation(nodeId: string): void {
    // Cancel existing timer if any
    const existingTimer = this.confirmationTimers.get(nodeId);
    if (existingTimer) {
      clearTimeout(existingTimer);
    }

    const timer = setTimeout(() => {
      this.confirmFailure(nodeId);
    }, this.config.confirmationTimeoutMs);

    this.confirmationTimers.set(nodeId, timer);
  }

  /**
   * Confirm node failure after confirmation timeout.
   */
  private confirmFailure(nodeId: string): void {
    const state = this.nodeStates.get(nodeId);
    if (!state) return;

    // Only confirm if still suspected
    if (state.isSuspected && !state.isConfirmedFailed) {
      state.isConfirmedFailed = true;
      this.emit('nodeConfirmedFailed', { nodeId });
      logger.error({ nodeId }, 'Node failure confirmed');
    }

    this.confirmationTimers.delete(nodeId);
  }

  /**
   * Calculate the phi value for a node using the Phi Accrual algorithm.
   *
   * Phi = -log10(P_later(t_now - t_last))
   *
   * where P_later is the probability that a heartbeat will arrive later than expected.
   */
  calculatePhi(nodeId: string): number {
    const state = this.nodeStates.get(nodeId);
    if (!state) return 0;

    const now = Date.now();
    const timeSinceLastHeartbeat = now - state.lastHeartbeat;

    // If we don't have enough samples, use simple timeout-based detection
    if (state.intervalHistory.length < this.config.minSamples) {
      // Simple fallback: phi increases linearly with time since last heartbeat
      const expectedInterval = this.config.initialHeartbeatIntervalMs;
      return timeSinceLastHeartbeat / expectedInterval;
    }

    // Calculate mean and variance of intervals
    const mean = this.calculateMean(state.intervalHistory);
    const variance = this.calculateVariance(state.intervalHistory, mean);
    const stdDev = Math.sqrt(variance);

    // Phi Accrual formula using normal distribution approximation
    // Phi = -log10(1 - CDF(timeSinceLastHeartbeat))
    // For simplicity, we use: phi ≈ (t - μ) / σ for t > μ
    if (timeSinceLastHeartbeat <= mean) {
      return 0; // No suspicion if within expected interval
    }

    // Calculate how many standard deviations away we are
    const deviations = stdDev > 0 ? (timeSinceLastHeartbeat - mean) / stdDev : 0;

    // Convert to phi using exponential distribution approximation
    // This gives phi values in a similar range to Hazelcast (0-16+)
    const phi = Math.max(0, deviations);

    return phi;
  }

  /**
   * Calculate mean of an array of numbers.
   */
  private calculateMean(values: number[]): number {
    if (values.length === 0) return 0;
    return values.reduce((sum, v) => sum + v, 0) / values.length;
  }

  /**
   * Calculate variance of an array of numbers.
   */
  private calculateVariance(values: number[], mean: number): number {
    if (values.length < 2) return 0;
    return values.reduce((sum, v) => sum + Math.pow(v - mean, 2), 0) / values.length;
  }

  /**
   * Get list of currently suspected nodes.
   */
  getSuspectedNodes(): string[] {
    const suspected: string[] = [];
    for (const [nodeId, state] of this.nodeStates) {
      if (state.isSuspected) {
        suspected.push(nodeId);
      }
    }
    return suspected;
  }

  /**
   * Get list of confirmed failed nodes.
   */
  getConfirmedFailedNodes(): string[] {
    const failed: string[] = [];
    for (const [nodeId, state] of this.nodeStates) {
      if (state.isConfirmedFailed) {
        failed.push(nodeId);
      }
    }
    return failed;
  }

  /**
   * Check if a specific node is suspected.
   */
  isSuspected(nodeId: string): boolean {
    return this.nodeStates.get(nodeId)?.isSuspected ?? false;
  }

  /**
   * Check if a specific node's failure is confirmed.
   */
  isConfirmedFailed(nodeId: string): boolean {
    return this.nodeStates.get(nodeId)?.isConfirmedFailed ?? false;
  }

  /**
   * Get the current phi value for a node.
   */
  getPhi(nodeId: string): number {
    return this.calculatePhi(nodeId);
  }

  /**
   * Get all monitored nodes.
   */
  getMonitoredNodes(): string[] {
    return Array.from(this.monitoringNodes);
  }

  /**
   * Get metrics for monitoring.
   */
  getMetrics(): {
    monitoredNodes: number;
    suspectedNodes: number;
    confirmedFailedNodes: number;
  } {
    let suspectedCount = 0;
    let confirmedCount = 0;

    for (const state of this.nodeStates.values()) {
      if (state.isSuspected) suspectedCount++;
      if (state.isConfirmedFailed) confirmedCount++;
    }

    return {
      monitoredNodes: this.monitoringNodes.size,
      suspectedNodes: suspectedCount,
      confirmedFailedNodes: confirmedCount,
    };
  }
}
