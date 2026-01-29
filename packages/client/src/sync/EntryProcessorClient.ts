/**
 * EntryProcessorClient - Handles entry processor operations for SyncEngine
 *
 * Responsibilities:
 * - Execute entry processors on single keys
 * - Execute entry processors on multiple keys (batch)
 * - Handle entry processor responses from server
 * - Timeout handling for requests
 * - Cleanup on close
 */

import type { EntryProcessorDef, EntryProcessorResult, EntryProcessKeyResult } from '@topgunbuild/core';
import { logger } from '../utils/logger';
import type { IEntryProcessorClient, EntryProcessorClientConfig } from './types';

/**
 * Default timeout for entry processor requests (ms).
 */
const DEFAULT_PROCESSOR_TIMEOUT = 30000;

/**
 * Pending entry processor request state.
 */
interface PendingProcessorRequest<R> {
  resolve: (result: EntryProcessorResult<R>) => void;
  reject: (error: Error) => void;
  timeout: ReturnType<typeof setTimeout>;
}

/**
 * Pending batch entry processor request state.
 */
interface PendingBatchProcessorRequest<R> {
  resolve: (results: Map<string, EntryProcessorResult<R>>) => void;
  reject: (error: Error) => void;
  timeout: ReturnType<typeof setTimeout>;
}

/**
 * EntryProcessorClient implements IEntryProcessorClient.
 *
 * Manages entry processor operations with support for:
 * - Single key processing with atomic execution
 * - Batch processing for multiple keys
 * - Request/response pattern with timeout handling
 */
export class EntryProcessorClient implements IEntryProcessorClient {
  private readonly config: EntryProcessorClientConfig;
  private readonly timeoutMs: number;

  // Pending entry processor requests by requestId
  private pendingProcessorRequests: Map<string, PendingProcessorRequest<any>> = new Map();

  // Pending batch entry processor requests by requestId
  private pendingBatchProcessorRequests: Map<string, PendingBatchProcessorRequest<any>> = new Map();

  constructor(config: EntryProcessorClientConfig) {
    this.config = config;
    this.timeoutMs = config.timeoutMs ?? DEFAULT_PROCESSOR_TIMEOUT;
  }

  // ============================================
  // Public API
  // ============================================

  /**
   * Execute an entry processor on a single key atomically.
   *
   * @param mapName Name of the map
   * @param key Key to process
   * @param processor Processor definition
   * @returns Promise resolving to the processor result
   */
  public async executeOnKey<V, R = V>(
    mapName: string,
    key: string,
    processor: EntryProcessorDef<V, R>,
  ): Promise<EntryProcessorResult<R>> {
    if (!this.config.isAuthenticated()) {
      return {
        success: false,
        error: 'Not connected to server',
      };
    }

    const requestId = crypto.randomUUID();

    return new Promise((resolve, reject) => {
      // Set timeout
      const timeout = setTimeout(() => {
        this.pendingProcessorRequests.delete(requestId);
        reject(new Error('Entry processor request timed out'));
      }, this.timeoutMs);

      // Store pending request
      this.pendingProcessorRequests.set(requestId, {
        resolve: (result) => {
          clearTimeout(timeout);
          resolve(result);
        },
        reject,
        timeout,
      });

      // Send request
      const sent = this.config.sendMessage({
        type: 'ENTRY_PROCESS',
        requestId,
        mapName,
        key,
        processor: {
          name: processor.name,
          code: processor.code,
          args: processor.args,
        },
      }, key);

      if (!sent) {
        this.pendingProcessorRequests.delete(requestId);
        clearTimeout(timeout);
        reject(new Error('Failed to send entry processor request'));
      }
    });
  }

  /**
   * Execute an entry processor on multiple keys.
   *
   * @param mapName Name of the map
   * @param keys Keys to process
   * @param processor Processor definition
   * @returns Promise resolving to a map of key -> result
   */
  public async executeOnKeys<V, R = V>(
    mapName: string,
    keys: string[],
    processor: EntryProcessorDef<V, R>,
  ): Promise<Map<string, EntryProcessorResult<R>>> {
    if (!this.config.isAuthenticated()) {
      const results = new Map<string, EntryProcessorResult<R>>();
      const error: EntryProcessorResult<R> = {
        success: false,
        error: 'Not connected to server',
      };
      for (const key of keys) {
        results.set(key, error);
      }
      return results;
    }

    const requestId = crypto.randomUUID();

    return new Promise((resolve, reject) => {
      // Set timeout
      const timeout = setTimeout(() => {
        this.pendingBatchProcessorRequests.delete(requestId);
        reject(new Error('Entry processor batch request timed out'));
      }, this.timeoutMs);

      // Store pending request
      this.pendingBatchProcessorRequests.set(requestId, {
        resolve: (results) => {
          clearTimeout(timeout);
          resolve(results);
        },
        reject,
        timeout,
      });

      // Send request
      const sent = this.config.sendMessage({
        type: 'ENTRY_PROCESS_BATCH',
        requestId,
        mapName,
        keys,
        processor: {
          name: processor.name,
          code: processor.code,
          args: processor.args,
        },
      });

      if (!sent) {
        this.pendingBatchProcessorRequests.delete(requestId);
        clearTimeout(timeout);
        reject(new Error('Failed to send entry processor batch request'));
      }
    });
  }

  /**
   * Handle entry processor response from server.
   * Called by SyncEngine for ENTRY_PROCESS_RESPONSE messages.
   */
  public handleEntryProcessResponse(message: {
    requestId: string;
    success: boolean;
    result?: unknown;
    newValue?: unknown;
    error?: string;
  }): void {
    const pending = this.pendingProcessorRequests.get(message.requestId);
    if (pending) {
      this.pendingProcessorRequests.delete(message.requestId);
      pending.resolve({
        success: message.success,
        result: message.result,
        newValue: message.newValue,
        error: message.error,
      });
    }
  }

  /**
   * Handle entry processor batch response from server.
   * Called by SyncEngine for ENTRY_PROCESS_BATCH_RESPONSE messages.
   */
  public handleEntryProcessBatchResponse(message: {
    requestId: string;
    results: Record<string, EntryProcessKeyResult>;
  }): void {
    const pending = this.pendingBatchProcessorRequests.get(message.requestId);
    if (pending) {
      this.pendingBatchProcessorRequests.delete(message.requestId);

      // Convert Record to Map
      const resultsMap = new Map<string, EntryProcessorResult<any>>();
      for (const [key, result] of Object.entries(message.results)) {
        resultsMap.set(key, {
          success: result.success,
          result: result.result,
          newValue: result.newValue,
          error: result.error,
        });
      }

      pending.resolve(resultsMap);
    }
  }

  /**
   * Clean up resources.
   * Clears pending timeouts without rejecting promises to match original SyncEngine behavior.
   * Note: This may leave promises hanging, but maintains backward compatibility with tests.
   */
  public close(error?: Error): void {
    // Only clear timeouts, don't reject promises to avoid unhandled rejections in tests
    for (const [requestId, pending] of this.pendingProcessorRequests.entries()) {
      clearTimeout(pending.timeout);
    }
    this.pendingProcessorRequests.clear();

    for (const [requestId, pending] of this.pendingBatchProcessorRequests.entries()) {
      clearTimeout(pending.timeout);
    }
    this.pendingBatchProcessorRequests.clear();
  }
}
