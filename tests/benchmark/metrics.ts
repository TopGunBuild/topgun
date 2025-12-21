/**
 * Metrics Collection with HDR Histogram
 *
 * High-precision latency tracking with minimal overhead.
 */

import * as hdr from 'hdr-histogram-js';
import type { LatencyMetrics, ThroughputMetrics, ReliabilityMetrics } from './types';

export class MetricsCollector {
  private histogram: hdr.Histogram;
  private opsSent = 0;
  private opsAcked = 0;
  private connectionErrors = 0;
  private timeoutErrors = 0;
  private protocolErrors = 0;
  private startTime = 0;
  private isRecording = false;

  // For peak throughput calculation
  private opsPerSecond: number[] = [];
  private lastSecondOps = 0;
  private lastSecondTime = 0;

  constructor() {
    this.histogram = hdr.build({
      lowestDiscernibleValue: 1,        // 1 microsecond
      highestTrackableValue: 60_000_000, // 60 seconds in microseconds
      numberOfSignificantValueDigits: 3,
    });
  }

  /**
   * Start recording metrics (called after warmup)
   */
  start(): void {
    this.isRecording = true;
    this.startTime = Date.now();
    this.lastSecondTime = this.startTime;
    this.histogram.reset();
    this.opsSent = 0;
    this.opsAcked = 0;
    this.connectionErrors = 0;
    this.timeoutErrors = 0;
    this.protocolErrors = 0;
    this.opsPerSecond = [];
    this.lastSecondOps = 0;
  }

  /**
   * Stop recording metrics
   */
  stop(): void {
    this.isRecording = false;
    // Record final partial second if any ops
    if (this.lastSecondOps > 0) {
      this.opsPerSecond.push(this.lastSecondOps);
    }
  }

  /**
   * Record a latency measurement (in milliseconds)
   */
  recordLatency(latencyMs: number): void {
    if (!this.isRecording) return;

    // Convert to microseconds for better precision
    const latencyUs = Math.round(latencyMs * 1000);
    if (latencyUs > 0 && latencyUs <= 60_000_000) {
      this.histogram.recordValue(latencyUs);
    }
  }

  /**
   * Record operations sent
   */
  recordSent(count: number): void {
    if (!this.isRecording) return;
    this.opsSent += count;
  }

  /**
   * Record operations acknowledged
   */
  recordAcked(count: number): void {
    if (!this.isRecording) return;
    this.opsAcked += count;
    this.lastSecondOps += count;

    // Check if a second (or more) has passed
    const now = Date.now();
    const elapsed = now - this.lastSecondTime;

    if (elapsed >= 1000) {
      // Normalize to ops per second
      const rate = this.lastSecondOps / (elapsed / 1000);
      this.opsPerSecond.push(rate);

      this.lastSecondOps = 0;
      this.lastSecondTime = now;
    }
  }

  /**
   * Record connection error
   */
  recordConnectionError(): void {
    this.connectionErrors++;
  }

  /**
   * Record timeout error
   */
  recordTimeoutError(): void {
    if (!this.isRecording) return;
    this.timeoutErrors++;
  }

  /**
   * Record protocol error
   */
  recordProtocolError(): void {
    if (!this.isRecording) return;
    this.protocolErrors++;
  }

  /**
   * Get elapsed time in seconds
   */
  getElapsedSeconds(): number {
    if (this.startTime === 0) return 0;
    return (Date.now() - this.startTime) / 1000;
  }

  /**
   * Get throughput metrics
   */
  getThroughput(): ThroughputMetrics {
    const durationSec = this.getElapsedSeconds();
    const opsPerSec = durationSec > 0 ? this.opsAcked / durationSec : 0;
    const peakOpsPerSec = this.opsPerSecond.length > 0
      ? Math.max(...this.opsPerSecond)
      : opsPerSec;

    return {
      totalOpsSent: this.opsSent,
      totalOpsAcked: this.opsAcked,
      opsPerSec: Math.round(opsPerSec),
      peakOpsPerSec: Math.round(peakOpsPerSec),
    };
  }

  /**
   * Get latency metrics (all values in milliseconds)
   */
  getLatency(): LatencyMetrics {
    const toMs = (us: number) => us / 1000;

    return {
      min: toMs(this.histogram.minNonZeroValue),
      max: toMs(this.histogram.maxValue),
      mean: toMs(this.histogram.mean),
      p50: toMs(this.histogram.getValueAtPercentile(50)),
      p95: toMs(this.histogram.getValueAtPercentile(95)),
      p99: toMs(this.histogram.getValueAtPercentile(99)),
      p999: toMs(this.histogram.getValueAtPercentile(99.9)),
      stdDev: toMs(this.histogram.stdDeviation),
    };
  }

  /**
   * Get reliability metrics
   */
  getReliability(): ReliabilityMetrics {
    const total = this.opsSent;
    const failed = total - this.opsAcked;
    const successRate = total > 0 ? this.opsAcked / total : 1;
    const errorRate = total > 0 ? failed / total : 0;

    return {
      successRate,
      errorRate,
      connectionErrors: this.connectionErrors,
      timeoutErrors: this.timeoutErrors,
      protocolErrors: this.protocolErrors,
    };
  }

  /**
   * Get total recorded values count
   */
  getRecordedCount(): number {
    return this.histogram.totalCount;
  }

  /**
   * Reset all metrics
   */
  reset(): void {
    this.histogram.reset();
    this.opsSent = 0;
    this.opsAcked = 0;
    this.connectionErrors = 0;
    this.timeoutErrors = 0;
    this.protocolErrors = 0;
    this.startTime = 0;
    this.isRecording = false;
    this.opsPerSecond = [];
    this.lastSecondOps = 0;
    this.lastSecondTime = 0;
  }
}
