import type { ConflictResolverDef, MergeRejection, Timestamp } from '@topgunbuild/core';
import type { SyncEngine } from './SyncEngine';
import { logger } from './utils/logger';

/**
 * Registered resolver info returned from server.
 */
export interface ResolverInfo {
  mapName: string;
  name: string;
  priority?: number;
  keyPattern?: string;
}

/**
 * Registration result from server.
 */
export interface RegisterResult {
  success: boolean;
  error?: string;
}

/**
 * Client-side manager for conflict resolvers.
 *
 * Provides API for:
 * - Registering conflict resolvers on server
 * - Unregistering resolvers
 * - Listing registered resolvers
 * - Subscribing to merge rejection events
 */
export class ConflictResolverClient {
  private readonly syncEngine: SyncEngine;
  private readonly rejectionListeners: Set<(rejection: MergeRejection) => void> = new Set();
  private readonly pendingRequests: Map<string, {
    resolve: (result: any) => void;
    reject: (error: Error) => void;
    timeout: NodeJS.Timeout;
  }> = new Map();

  private static readonly REQUEST_TIMEOUT = 10000; // 10 seconds

  constructor(syncEngine: SyncEngine) {
    this.syncEngine = syncEngine;
  }

  /**
   * Register a conflict resolver on the server.
   *
   * @param mapName The map to register the resolver for
   * @param resolver The resolver definition
   * @returns Promise resolving to registration result
   *
   * @example
   * ```typescript
   * // Register a first-write-wins resolver for bookings
   * await client.resolvers.register('bookings', {
   *   name: 'first-write-wins',
   *   code: `
   *     if (context.localValue !== undefined) {
   *       return { action: 'reject', reason: 'Slot already booked' };
   *     }
   *     return { action: 'accept', value: context.remoteValue };
   *   `,
   *   priority: 100,
   * });
   * ```
   */
  async register<V>(
    mapName: string,
    resolver: Omit<ConflictResolverDef<V>, 'fn'>,
  ): Promise<RegisterResult> {
    const requestId = crypto.randomUUID();

    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pendingRequests.delete(requestId);
        reject(new Error('Register resolver request timed out'));
      }, ConflictResolverClient.REQUEST_TIMEOUT);

      this.pendingRequests.set(requestId, {
        resolve: (result: RegisterResult) => {
          clearTimeout(timeout);
          resolve(result);
        },
        reject,
        timeout,
      });

      try {
        this.syncEngine.send({
          type: 'REGISTER_RESOLVER',
          requestId,
          mapName,
          resolver: {
            name: resolver.name,
            code: resolver.code || '',
            priority: resolver.priority,
            keyPattern: resolver.keyPattern,
          },
        });
      } catch {
        this.pendingRequests.delete(requestId);
        clearTimeout(timeout);
        resolve({ success: false, error: 'Not connected to server' });
      }
    });
  }

  /**
   * Unregister a conflict resolver from the server.
   *
   * @param mapName The map the resolver is registered for
   * @param resolverName The name of the resolver to unregister
   * @returns Promise resolving to unregistration result
   */
  async unregister(mapName: string, resolverName: string): Promise<RegisterResult> {
    const requestId = crypto.randomUUID();

    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pendingRequests.delete(requestId);
        reject(new Error('Unregister resolver request timed out'));
      }, ConflictResolverClient.REQUEST_TIMEOUT);

      this.pendingRequests.set(requestId, {
        resolve: (result: RegisterResult) => {
          clearTimeout(timeout);
          resolve(result);
        },
        reject,
        timeout,
      });

      try {
        this.syncEngine.send({
          type: 'UNREGISTER_RESOLVER',
          requestId,
          mapName,
          resolverName,
        });
      } catch {
        this.pendingRequests.delete(requestId);
        clearTimeout(timeout);
        resolve({ success: false, error: 'Not connected to server' });
      }
    });
  }

  /**
   * List registered conflict resolvers on the server.
   *
   * @param mapName Optional - filter by map name
   * @returns Promise resolving to list of resolver info
   */
  async list(mapName?: string): Promise<ResolverInfo[]> {
    const requestId = crypto.randomUUID();

    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pendingRequests.delete(requestId);
        reject(new Error('List resolvers request timed out'));
      }, ConflictResolverClient.REQUEST_TIMEOUT);

      this.pendingRequests.set(requestId, {
        resolve: (result: { resolvers: ResolverInfo[] }) => {
          clearTimeout(timeout);
          resolve(result.resolvers);
        },
        reject,
        timeout,
      });

      try {
        this.syncEngine.send({
          type: 'LIST_RESOLVERS',
          requestId,
          mapName,
        });
      } catch {
        this.pendingRequests.delete(requestId);
        clearTimeout(timeout);
        resolve([]);
      }
    });
  }

  /**
   * Subscribe to merge rejection events.
   *
   * @param listener Callback for rejection events
   * @returns Unsubscribe function
   *
   * @example
   * ```typescript
   * const unsubscribe = client.resolvers.onRejection((rejection) => {
   *   console.log(`Merge rejected for ${rejection.key}: ${rejection.reason}`);
   *   // Optionally refresh the local value
   * });
   *
   * // Later...
   * unsubscribe();
   * ```
   */
  onRejection(listener: (rejection: MergeRejection) => void): () => void {
    this.rejectionListeners.add(listener);
    return () => this.rejectionListeners.delete(listener);
  }

  /**
   * Handle REGISTER_RESOLVER_RESPONSE from server.
   * Called by SyncEngine.
   */
  handleRegisterResponse(message: {
    requestId: string;
    success: boolean;
    error?: string;
  }): void {
    const pending = this.pendingRequests.get(message.requestId);
    if (pending) {
      this.pendingRequests.delete(message.requestId);
      pending.resolve({ success: message.success, error: message.error });
    }
  }

  /**
   * Handle UNREGISTER_RESOLVER_RESPONSE from server.
   * Called by SyncEngine.
   */
  handleUnregisterResponse(message: {
    requestId: string;
    success: boolean;
    error?: string;
  }): void {
    const pending = this.pendingRequests.get(message.requestId);
    if (pending) {
      this.pendingRequests.delete(message.requestId);
      pending.resolve({ success: message.success, error: message.error });
    }
  }

  /**
   * Handle LIST_RESOLVERS_RESPONSE from server.
   * Called by SyncEngine.
   */
  handleListResponse(message: {
    requestId: string;
    resolvers: ResolverInfo[];
  }): void {
    const pending = this.pendingRequests.get(message.requestId);
    if (pending) {
      this.pendingRequests.delete(message.requestId);
      pending.resolve({ resolvers: message.resolvers });
    }
  }

  /**
   * Handle MERGE_REJECTED from server.
   * Called by SyncEngine.
   */
  handleMergeRejected(message: {
    mapName: string;
    key: string;
    attemptedValue: unknown;
    reason: string;
    timestamp: Timestamp;
  }): void {
    const rejection: MergeRejection = {
      mapName: message.mapName,
      key: message.key,
      attemptedValue: message.attemptedValue,
      reason: message.reason,
      timestamp: message.timestamp,
      nodeId: '', // Not provided by server in this message
    };

    logger.debug({ rejection }, 'Merge rejected by server');

    for (const listener of this.rejectionListeners) {
      try {
        listener(rejection);
      } catch (e) {
        logger.error({ error: e }, 'Error in rejection listener');
      }
    }
  }

  /**
   * Clear all pending requests (e.g., on disconnect).
   */
  clearPending(): void {
    for (const [requestId, pending] of this.pendingRequests) {
      clearTimeout(pending.timeout);
      pending.reject(new Error('Connection lost'));
    }
    this.pendingRequests.clear();
  }

  /**
   * Get the number of registered rejection listeners.
   */
  get rejectionListenerCount(): number {
    return this.rejectionListeners.size;
  }
}
