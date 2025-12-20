/**
 * Native Benchmark Harness
 *
 * Core benchmark engine using native MessagePack protocol.
 */

import WebSocket from 'ws';
import { serialize, deserialize } from '../../packages/core/src';
import { MetricsCollector } from './metrics';
import type {
  BenchmarkConfig,
  BenchmarkResult,
  ConnectionState,
  PendingOperation,
} from './types';

const OPERATION_TIMEOUT_MS = 5000;
const RECONNECT_DELAY_MS = 1000;

interface BenchmarkCallbacks {
  onProgress?: (sent: number, acked: number) => void;
  onError?: (error: Error) => void;
}

export class BenchmarkHarness {
  private config: BenchmarkConfig;
  private metrics: MetricsCollector;
  private connections: Map<string, ConnectionState> = new Map();
  private sockets: Map<string, WebSocket> = new Map();
  private isRunning = false;
  private isWarmup = true;
  private intervalTimers: NodeJS.Timeout[] = [];
  private timeoutChecker: NodeJS.Timeout | null = null;
  private startTime = 0;
  private callbacks: BenchmarkCallbacks;

  constructor(config: BenchmarkConfig, callbacks: BenchmarkCallbacks = {}) {
    this.config = config;
    this.metrics = new MetricsCollector();
    this.callbacks = callbacks;
  }

  /**
   * Run the benchmark
   */
  async run(scenarioName: string): Promise<BenchmarkResult> {
    const startTime = new Date().toISOString();
    this.startTime = Date.now();

    try {
      // Setup signal handlers
      this.setupSignalHandlers();

      // Create connections
      await this.createConnections();

      // Start warmup phase
      console.log(`Warming up for ${this.config.warmupMs / 1000} seconds...`);
      this.isWarmup = true;
      this.isRunning = true;

      // Start sending operations
      this.startSending();

      // Start timeout checker
      this.startTimeoutChecker();

      // Wait for warmup
      await this.sleep(this.config.warmupMs);

      // Start actual measurement
      console.log('Starting measurement...');
      this.isWarmup = false;
      this.metrics.start();

      // Wait for test duration
      const remainingTime = this.config.durationMs - this.config.warmupMs;
      await this.sleep(remainingTime);

      // Stop
      this.stop();

      // Build result
      return this.buildResult(scenarioName, startTime);
    } catch (error) {
      this.stop();
      throw error;
    }
  }

  /**
   * Create all WebSocket connections
   */
  private async createConnections(): Promise<void> {
    const promises: Promise<void>[] = [];

    for (let i = 0; i < this.config.connections; i++) {
      promises.push(this.createConnection(`conn-${i}`));
    }

    const results = await Promise.allSettled(promises);
    const failures = results.filter((r): r is PromiseRejectedResult => r.status === 'rejected');

    if (failures.length > 0) {
      console.warn(`${failures.length}/${this.config.connections} connections failed`);
    }

    const connected = this.connections.size;
    if (connected === 0) {
      // Throw the first connection error to provide useful error message
      const firstError = failures[0]?.reason;
      throw firstError || new Error('No connections established');
    }

    console.log(`Connected: ${connected}/${this.config.connections}`);
  }

  /**
   * Create a single WebSocket connection
   */
  private createConnection(id: string): Promise<void> {
    return new Promise((resolve, reject) => {
      const ws = new WebSocket(this.config.serverUrl);
      ws.binaryType = 'arraybuffer';

      const state: ConnectionState = {
        id,
        isAuthenticated: false,
        isConnected: false,
        pendingOps: new Map(),
        opCounter: 0,
        sentOps: 0,
        ackedOps: 0,
        errors: 0,
      };

      const timeout = setTimeout(() => {
        ws.close();
        reject(new Error(`Connection timeout: ${id}`));
      }, 10000);

      ws.on('open', () => {
        state.isConnected = true;
        this.sockets.set(id, ws);
        this.connections.set(id, state);
      });

      ws.on('message', (data: ArrayBuffer) => {
        try {
          const message = deserialize<any>(new Uint8Array(data));
          this.handleMessage(state, message, resolve, reject, timeout);
        } catch (err) {
          this.metrics.recordProtocolError();
        }
      });

      ws.on('error', (err: Error) => {
        state.errors++;
        this.metrics.recordConnectionError();
        clearTimeout(timeout);
        reject(err);
        this.callbacks.onError?.(err);
      });

      ws.on('close', () => {
        state.isConnected = false;
        state.isAuthenticated = false;
      });
    });
  }

  /**
   * Handle incoming message
   */
  private handleMessage(
    state: ConnectionState,
    message: any,
    resolve: () => void,
    reject: (err: Error) => void,
    timeout: NodeJS.Timeout
  ): void {
    switch (message.type) {
      case 'AUTH_REQUIRED':
        this.sendAuth(state);
        break;

      case 'AUTH_ACK':
        state.isAuthenticated = true;
        clearTimeout(timeout);
        resolve();
        break;

      case 'AUTH_ERROR':
        clearTimeout(timeout);
        reject(new Error(`Authentication failed: ${message.message}`));
        break;

      case 'OP_ACK':
        this.handleOpAck(state, message);
        break;

      case 'PONG':
        // Heartbeat response, ignore for now
        break;

      case 'ERROR':
        this.metrics.recordProtocolError();
        break;
    }
  }

  /**
   * Send authentication message
   */
  private sendAuth(state: ConnectionState): void {
    const ws = this.sockets.get(state.id);
    if (!ws || ws.readyState !== WebSocket.OPEN) return;

    const message = {
      type: 'AUTH',
      token: this.config.token,
    };

    ws.send(serialize(message));
  }

  /**
   * Handle operation acknowledgment
   */
  private handleOpAck(state: ConnectionState, message: any): void {
    const lastId = message.payload?.lastId;
    if (!lastId) return;

    const pending = state.pendingOps.get(lastId);
    if (!pending) return;

    const latencyMs = Date.now() - pending.sendTime;

    if (!this.isWarmup) {
      this.metrics.recordLatency(latencyMs);
      this.metrics.recordAcked(pending.batchSize);
      state.ackedOps += pending.batchSize;
    }

    state.pendingOps.delete(lastId);
  }

  /**
   * Start sending operations on all connections
   */
  private startSending(): void {
    this.connections.forEach((state, id) => {
      const timer = setInterval(() => {
        if (!this.isRunning) return;
        this.sendBatch(state);
      }, this.config.intervalMs);

      this.intervalTimers.push(timer);
    });
  }

  /**
   * Send a batch of operations
   */
  private sendBatch(state: ConnectionState): void {
    const ws = this.sockets.get(state.id);
    if (!ws || ws.readyState !== WebSocket.OPEN || !state.isAuthenticated) {
      return;
    }

    const ops = [];
    const sendTime = Date.now();

    for (let i = 0; i < this.config.batchSize; i++) {
      state.opCounter++;
      const opId = `${state.id}-op-${state.opCounter}`;
      const mapIndex = state.opCounter % this.config.mapCount;

      ops.push({
        id: opId,
        mapName: `benchmark-map-${mapIndex}`,
        key: `key-${state.opCounter}`,
        opType: 'PUT',
        record: {
          value: {
            counter: state.opCounter,
            timestamp: sendTime,
            data: `benchmark-${Math.random().toString(36).substring(7)}`,
          },
          timestamp: {
            millis: sendTime,
            counter: state.opCounter,
            nodeId: state.id,
          },
        },
      });
    }

    const lastOpId = ops[ops.length - 1].id;
    const message = {
      type: 'OP_BATCH',
      payload: { ops },
    };

    try {
      ws.send(serialize(message));

      state.pendingOps.set(lastOpId, {
        id: lastOpId,
        sendTime,
        batchSize: this.config.batchSize,
      });

      state.sentOps += this.config.batchSize;

      if (!this.isWarmup) {
        this.metrics.recordSent(this.config.batchSize);
      }

      this.callbacks.onProgress?.(
        this.getTotalSent(),
        this.getTotalAcked()
      );
    } catch (err) {
      this.metrics.recordProtocolError();
    }
  }

  /**
   * Start timeout checker for pending operations
   */
  private startTimeoutChecker(): void {
    this.timeoutChecker = setInterval(() => {
      const now = Date.now();

      this.connections.forEach((state) => {
        state.pendingOps.forEach((pending, opId) => {
          if (now - pending.sendTime > OPERATION_TIMEOUT_MS) {
            if (!this.isWarmup) {
              this.metrics.recordTimeoutError();
            }
            state.pendingOps.delete(opId);
          }
        });
      });
    }, 1000);
  }

  /**
   * Stop the benchmark
   */
  private stop(): void {
    this.isRunning = false;
    this.metrics.stop();

    // Clear interval timers
    for (const timer of this.intervalTimers) {
      clearInterval(timer);
    }
    this.intervalTimers = [];

    // Clear timeout checker
    if (this.timeoutChecker) {
      clearInterval(this.timeoutChecker);
      this.timeoutChecker = null;
    }

    // Close all connections
    this.sockets.forEach((ws) => {
      try {
        ws.close();
      } catch {}
    });
    this.sockets.clear();
    this.connections.clear();
  }

  /**
   * Setup signal handlers for graceful shutdown
   */
  private setupSignalHandlers(): void {
    const handler = () => {
      console.log('\nGracefully shutting down...');
      this.stop();
      process.exit(0);
    };

    process.on('SIGINT', handler);
    process.on('SIGTERM', handler);
  }

  /**
   * Build benchmark result
   */
  private buildResult(scenarioName: string, startTime: string): BenchmarkResult {
    const throughput = this.metrics.getThroughput();
    const latency = this.metrics.getLatency();
    const reliability = this.metrics.getReliability();
    const durationSec = this.metrics.getElapsedSeconds();

    // Get version from package.json
    let version = 'unknown';
    try {
      const pkg = require('../../package.json');
      version = pkg.version;
    } catch {}

    return {
      scenario: scenarioName,
      config: this.config,
      startTime,
      durationSec,
      throughput,
      latency,
      reliability,
      version,
      passed: true, // Will be updated by threshold check
      failureReasons: [],
    };
  }

  /**
   * Get total operations sent
   */
  private getTotalSent(): number {
    let total = 0;
    this.connections.forEach((state) => {
      total += state.sentOps;
    });
    return total;
  }

  /**
   * Get total operations acknowledged
   */
  private getTotalAcked(): number {
    let total = 0;
    this.connections.forEach((state) => {
      total += state.ackedOps;
    });
    return total;
  }

  /**
   * Sleep helper
   */
  private sleep(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
  }
}
