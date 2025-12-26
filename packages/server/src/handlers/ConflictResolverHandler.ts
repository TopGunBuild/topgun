import {
  ConflictResolverDef,
  MergeContext,
  MergeResult,
  MergeRejection,
  LWWMap,
  LWWRecord,
} from '@topgunbuild/core';
import {
  ConflictResolverService,
  ConflictResolverServiceConfig,
} from '../ConflictResolverService';
import { ProcessorSandbox, ProcessorSandboxConfig } from '../ProcessorSandbox';
import { logger } from '../utils/logger';

/**
 * Configuration for ConflictResolverHandler.
 */
export interface ConflictResolverHandlerConfig {
  /** Node ID for identifying server-side resolvers */
  nodeId: string;

  /** Optional sandbox configuration override */
  sandboxConfig?: Partial<ProcessorSandboxConfig>;

  /** Optional resolver service configuration */
  resolverConfig?: Partial<ConflictResolverServiceConfig>;
}

/**
 * Result of merge operation with resolver.
 */
export interface MergeWithResolverResult<V> {
  /** Whether the merge was applied */
  applied: boolean;

  /** The merge result details */
  result: MergeResult<V>;

  /** The final record if applied */
  record?: LWWRecord<V>;

  /** Rejection details if rejected */
  rejection?: MergeRejection;
}

/**
 * Server-side handler for Conflict Resolver operations.
 *
 * Responsibilities:
 * - Manage conflict resolver registrations
 * - Execute resolvers during merge operations
 * - Provide merge rejection notifications
 */
export class ConflictResolverHandler {
  private sandbox: ProcessorSandbox;
  private resolverService: ConflictResolverService;
  /** Reserved for future use (server-side resolver identification) */
  private readonly nodeId: string;
  private rejectionListeners: Set<(rejection: MergeRejection) => void> =
    new Set();

  constructor(config: ConflictResolverHandlerConfig) {
    this.nodeId = config.nodeId;
    this.sandbox = new ProcessorSandbox(config.sandboxConfig);
    this.resolverService = new ConflictResolverService(
      this.sandbox,
      config.resolverConfig,
    );

    // Wire up rejection handler
    this.resolverService.onRejection((rejection) => {
      for (const listener of this.rejectionListeners) {
        try {
          listener(rejection);
        } catch (e) {
          logger.error({ error: e }, 'Error in rejection listener');
        }
      }
    });
  }

  /**
   * Register a conflict resolver for a map.
   *
   * @param mapName The map name
   * @param resolver The resolver definition
   * @param clientId Optional client ID that registered this resolver
   */
  registerResolver<V>(
    mapName: string,
    resolver: ConflictResolverDef<V>,
    clientId?: string,
  ): void {
    this.resolverService.register(mapName, resolver, clientId);
    logger.info(
      {
        mapName,
        resolverName: resolver.name,
        priority: resolver.priority,
        clientId,
      },
      'Resolver registered',
    );
  }

  /**
   * Unregister a conflict resolver.
   *
   * @param mapName The map name
   * @param resolverName The resolver name
   * @param clientId Optional - only unregister if registered by this client
   */
  unregisterResolver(
    mapName: string,
    resolverName: string,
    clientId?: string,
  ): boolean {
    const removed = this.resolverService.unregister(
      mapName,
      resolverName,
      clientId,
    );
    if (removed) {
      logger.info({ mapName, resolverName, clientId }, 'Resolver unregistered');
    }
    return removed;
  }

  /**
   * List registered resolvers.
   *
   * @param mapName Optional - filter by map name
   */
  listResolvers(mapName?: string): Array<{
    mapName: string;
    name: string;
    priority?: number;
    keyPattern?: string;
  }> {
    return this.resolverService.list(mapName);
  }

  /**
   * Apply a merge with conflict resolution.
   *
   * Deletions (tombstones) are also passed through resolvers to allow
   * protection via IMMUTABLE, OWNER_ONLY, or similar resolvers.
   * If no custom resolvers are registered, deletions use standard LWW.
   *
   * @param map The LWWMap to merge into
   * @param mapName The map name (for resolver lookup)
   * @param key The key being merged
   * @param record The incoming record
   * @param remoteNodeId The source node ID
   * @param auth Optional authentication context
   */
  async mergeWithResolver<V>(
    map: LWWMap<string, V>,
    mapName: string,
    key: string,
    record: LWWRecord<V>,
    remoteNodeId: string,
    auth?: MergeContext['auth'],
  ): Promise<MergeWithResolverResult<V>> {
    const isDeletion = record.value === null;
    const localRecord = map.getRecord(key);

    // Build merge context (works for both updates and deletions)
    const context: MergeContext<V> = {
      mapName,
      key,
      localValue: localRecord?.value ?? undefined,
      // For deletions, remoteValue is null - resolvers can check this
      remoteValue: record.value as V,
      localTimestamp: localRecord?.timestamp,
      remoteTimestamp: record.timestamp,
      remoteNodeId,
      auth,
      readEntry: (k: string) => map.get(k) as V | undefined,
    };

    // Resolve conflict (applies to both updates and deletions)
    const result = await this.resolverService.resolve(context);

    // Apply result
    switch (result.action) {
      case 'accept':
      case 'merge': {
        // For deletions, use the original null value regardless of merge result
        // For updates, use the resolved value
        const finalValue = isDeletion ? null : result.value;
        const finalRecord: LWWRecord<V> = {
          value: finalValue as V,
          timestamp: record.timestamp,
          ttlMs: record.ttlMs,
        };
        map.merge(key, finalRecord);
        return { applied: true, result, record: finalRecord };
      }

      case 'reject': {
        const rejection: MergeRejection = {
          mapName,
          key,
          attemptedValue: record.value,
          reason: result.reason,
          timestamp: record.timestamp,
          nodeId: remoteNodeId,
        };
        return { applied: false, result, rejection };
      }

      case 'local':
      default:
        return { applied: false, result };
    }
  }

  /**
   * Check if a map has custom resolvers registered.
   */
  hasResolvers(mapName: string): boolean {
    return this.resolverService.hasResolvers(mapName);
  }

  /**
   * Add a listener for merge rejections.
   */
  onRejection(listener: (rejection: MergeRejection) => void): () => void {
    this.rejectionListeners.add(listener);
    return () => this.rejectionListeners.delete(listener);
  }

  /**
   * Clear resolvers registered by a specific client.
   */
  clearByClient(clientId: string): number {
    return this.resolverService.clearByClient(clientId);
  }

  /**
   * Get the number of registered resolvers.
   */
  get resolverCount(): number {
    return this.resolverService.size;
  }

  /**
   * Check if sandbox is in secure mode.
   */
  isSecureMode(): boolean {
    return this.sandbox.isSecureMode();
  }

  /**
   * Dispose of the handler.
   */
  dispose(): void {
    this.resolverService.dispose();
    this.sandbox.dispose();
    this.rejectionListeners.clear();
    logger.debug('ConflictResolverHandler disposed');
  }
}
