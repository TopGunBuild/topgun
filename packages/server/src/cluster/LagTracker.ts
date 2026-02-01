/**
 * LagTracker - Monitors replication lag across cluster nodes
 *
 * Async Replication Pipeline
 *
 * Features:
 * - Tracks replication lag per node
 * - Maintains historical lag data for percentile calculations
 * - Identifies unhealthy and laggy nodes
 * - Provides health metrics for monitoring
 */

import { ReplicationLag, ReplicationHealth } from '@topgunbuild/core';

export interface LagInfo {
  current: number;
  history: number[];
  lastUpdate: number;
  pendingOps: number;
}

export interface LagTrackerConfig {
  /** Number of lag samples to keep in history (default: 100) */
  historySize: number;
  /** Threshold in ms for considering a node laggy (default: 5000) */
  laggyThresholdMs: number;
  /** Threshold in ms for considering a node unhealthy (default: 30000) */
  unhealthyThresholdMs: number;
}

export const DEFAULT_LAG_TRACKER_CONFIG: LagTrackerConfig = {
  historySize: 100,
  laggyThresholdMs: 5000,
  unhealthyThresholdMs: 30000,
};

export class LagTracker {
  private readonly config: LagTrackerConfig;
  private lagByNode: Map<string, LagInfo> = new Map();

  constructor(config: Partial<LagTrackerConfig> = {}) {
    this.config = {
      ...DEFAULT_LAG_TRACKER_CONFIG,
      ...config,
    };
  }

  /**
   * Update lag measurement for a node
   */
  public update(nodeId: string, lagMs: number): void {
    let info = this.lagByNode.get(nodeId);
    if (!info) {
      info = {
        current: 0,
        history: [],
        lastUpdate: Date.now(),
        pendingOps: 0,
      };
      this.lagByNode.set(nodeId, info);
    }

    info.current = lagMs;
    info.history.push(lagMs);

    // Trim history to configured size
    if (info.history.length > this.config.historySize) {
      info.history.shift();
    }

    info.lastUpdate = Date.now();
  }

  /**
   * Record acknowledgment from a node (lag effectively becomes 0)
   */
  public recordAck(nodeId: string): void {
    const info = this.lagByNode.get(nodeId);
    if (info) {
      info.current = 0;
      info.lastUpdate = Date.now();
      if (info.pendingOps > 0) {
        info.pendingOps--;
      }
    }
  }

  /**
   * Increment pending operations counter for a node
   */
  public incrementPending(nodeId: string): void {
    let info = this.lagByNode.get(nodeId);
    if (!info) {
      info = {
        current: 0,
        history: [],
        lastUpdate: Date.now(),
        pendingOps: 0,
      };
      this.lagByNode.set(nodeId, info);
    }
    info.pendingOps++;
  }

  /**
   * Get lag statistics for a specific node
   */
  public getLag(nodeId: string): ReplicationLag {
    const info = this.lagByNode.get(nodeId);
    if (!info || info.history.length === 0) {
      return { current: 0, avg: 0, max: 0, percentile99: 0 };
    }

    const sorted = [...info.history].sort((a, b) => a - b);
    const avg = sorted.reduce((a, b) => a + b, 0) / sorted.length;
    const max = sorted[sorted.length - 1] || 0;

    // Calculate 99th percentile
    const p99Index = Math.floor(sorted.length * 0.99);
    const percentile99 = sorted[p99Index] || max;

    return {
      current: info.current,
      avg: Math.round(avg * 100) / 100, // Round to 2 decimal places
      max,
      percentile99,
    };
  }

  /**
   * Get pending operations count for a node
   */
  public getPendingOps(nodeId: string): number {
    const info = this.lagByNode.get(nodeId);
    return info?.pendingOps ?? 0;
  }

  /**
   * Get overall replication health status
   */
  public getHealth(): ReplicationHealth {
    const unhealthyNodes: string[] = [];
    const laggyNodes: string[] = [];
    let totalLag = 0;
    let nodeCount = 0;

    const now = Date.now();

    for (const [nodeId, info] of this.lagByNode) {
      const timeSinceUpdate = now - info.lastUpdate;

      // Check if node is unhealthy (no updates for too long)
      if (timeSinceUpdate > this.config.unhealthyThresholdMs) {
        unhealthyNodes.push(nodeId);
      }
      // Check if node is laggy (current lag exceeds threshold)
      else if (info.current > this.config.laggyThresholdMs) {
        laggyNodes.push(nodeId);
      }

      totalLag += info.current;
      nodeCount++;
    }

    const avgLagMs = nodeCount > 0 ? totalLag / nodeCount : 0;

    return {
      healthy: unhealthyNodes.length === 0,
      unhealthyNodes,
      laggyNodes,
      avgLagMs: Math.round(avgLagMs * 100) / 100,
    };
  }

  /**
   * Get average lag across all tracked nodes
   */
  public getAverageLag(): number {
    let total = 0;
    let count = 0;

    for (const info of this.lagByNode.values()) {
      total += info.current;
      count++;
    }

    return count > 0 ? total / count : 0;
  }

  /**
   * Check if a specific node is considered healthy
   */
  public isNodeHealthy(nodeId: string): boolean {
    const info = this.lagByNode.get(nodeId);
    if (!info) return true; // Unknown nodes are considered healthy

    const timeSinceUpdate = Date.now() - info.lastUpdate;
    return timeSinceUpdate < this.config.unhealthyThresholdMs;
  }

  /**
   * Check if a specific node is considered laggy
   */
  public isNodeLaggy(nodeId: string): boolean {
    const info = this.lagByNode.get(nodeId);
    if (!info) return false;

    return info.current > this.config.laggyThresholdMs;
  }

  /**
   * Remove a node from tracking
   */
  public removeNode(nodeId: string): void {
    this.lagByNode.delete(nodeId);
  }

  /**
   * Get all tracked node IDs
   */
  public getTrackedNodes(): string[] {
    return Array.from(this.lagByNode.keys());
  }

  /**
   * Get raw lag info for a node (for advanced monitoring)
   */
  public getRawLagInfo(nodeId: string): LagInfo | undefined {
    return this.lagByNode.get(nodeId);
  }

  /**
   * Clear all tracking data
   */
  public clear(): void {
    this.lagByNode.clear();
  }

  /**
   * Export metrics in Prometheus format
   */
  public toPrometheusMetrics(): string {
    const lines: string[] = [
      '# HELP topgun_replication_lag_ms Current replication lag in milliseconds',
      '# TYPE topgun_replication_lag_ms gauge',
    ];

    for (const [nodeId, info] of this.lagByNode) {
      lines.push(`topgun_replication_lag_ms{node="${nodeId}"} ${info.current}`);
    }

    lines.push('');
    lines.push('# HELP topgun_replication_pending_ops Pending replication operations');
    lines.push('# TYPE topgun_replication_pending_ops gauge');

    for (const [nodeId, info] of this.lagByNode) {
      lines.push(`topgun_replication_pending_ops{node="${nodeId}"} ${info.pendingOps}`);
    }

    const health = this.getHealth();
    lines.push('');
    lines.push('# HELP topgun_replication_healthy Cluster replication health (1=healthy, 0=unhealthy)');
    lines.push('# TYPE topgun_replication_healthy gauge');
    lines.push(`topgun_replication_healthy ${health.healthy ? 1 : 0}`);

    lines.push('');
    lines.push('# HELP topgun_replication_avg_lag_ms Average replication lag across all nodes');
    lines.push('# TYPE topgun_replication_avg_lag_ms gauge');
    lines.push(`topgun_replication_avg_lag_ms ${health.avgLagMs}`);

    return lines.join('\n');
  }
}
