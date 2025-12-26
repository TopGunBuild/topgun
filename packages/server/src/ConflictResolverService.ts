import {
  ConflictResolverDef,
  MergeContext,
  MergeResult,
  BuiltInResolvers,
  ConflictResolverDefSchema,
  validateResolverCode,
  MergeRejection,
  Timestamp,
} from '@topgunbuild/core';
import { ProcessorSandbox } from './ProcessorSandbox';
import { logger } from './utils/logger';

/**
 * Entry for storing registered resolvers.
 */
interface ResolverEntry<V = unknown> {
  resolver: ConflictResolverDef<V>;
  compiledFn?: (ctx: MergeContext<V>) => Promise<MergeResult<V>>;
  registeredBy?: string; // Client ID that registered this resolver
}

/**
 * Configuration for ConflictResolverService.
 */
export interface ConflictResolverServiceConfig {
  /** Maximum resolvers per map */
  maxResolversPerMap: number;

  /** Enable sandboxed code execution (requires isolated-vm) */
  enableSandboxedResolvers: boolean;

  /** Default timeout for resolver execution in milliseconds */
  resolverTimeoutMs: number;
}

/**
 * Default service configuration.
 */
export const DEFAULT_CONFLICT_RESOLVER_CONFIG: ConflictResolverServiceConfig = {
  maxResolversPerMap: 100,
  enableSandboxedResolvers: true,
  resolverTimeoutMs: 100,
};

/**
 * Service for managing and executing conflict resolvers.
 *
 * Resolvers are executed in priority order (highest first).
 * The first resolver that returns a non-'local' action wins.
 * If all resolvers return 'local', LWW is used as fallback.
 *
 * ## Design Decisions
 *
 * ### In-Memory Storage
 * Resolvers are stored in memory only (not persisted to database).
 * This is intentional - resolvers represent application logic that should
 * be registered by clients on connection. Benefits:
 * - Simpler architecture without resolver schema migrations
 * - Clients control their own conflict resolution logic
 * - Natural cleanup when client disconnects
 *
 * ### Permission Model
 * Resolver registration requires PUT permission on the target map.
 * This aligns with the principle that if you can write to a map,
 * you can define how your writes are resolved. For stricter control,
 * implement custom permission checks in ServerCoordinator.
 *
 * ### Deletion Handling
 * Deletions (tombstones with null value) are passed through resolvers
 * with `remoteValue: null`. This allows resolvers like IMMUTABLE or
 * OWNER_ONLY to protect against unauthorized deletions.
 */
export class ConflictResolverService {
  private resolvers: Map<string, ResolverEntry[]> = new Map();
  private sandbox: ProcessorSandbox;
  private config: ConflictResolverServiceConfig;
  private onRejectionCallback?: (rejection: MergeRejection) => void;
  private disposed = false;

  constructor(
    sandbox: ProcessorSandbox,
    config: Partial<ConflictResolverServiceConfig> = {},
  ) {
    this.sandbox = sandbox;
    this.config = { ...DEFAULT_CONFLICT_RESOLVER_CONFIG, ...config };
  }

  /**
   * Set callback for merge rejections.
   */
  onRejection(callback: (rejection: MergeRejection) => void): void {
    this.onRejectionCallback = callback;
  }

  /**
   * Register a resolver for a map.
   *
   * @param mapName The map this resolver applies to
   * @param resolver The resolver definition
   * @param registeredBy Optional client ID that registered this resolver
   */
  register<V>(
    mapName: string,
    resolver: ConflictResolverDef<V>,
    registeredBy?: string,
  ): void {
    if (this.disposed) {
      throw new Error('ConflictResolverService has been disposed');
    }

    // Validate resolver if it has code
    if (resolver.code) {
      // Validate against schema
      const parsed = ConflictResolverDefSchema.safeParse({
        name: resolver.name,
        code: resolver.code,
        priority: resolver.priority,
        keyPattern: resolver.keyPattern,
      });

      if (!parsed.success) {
        throw new Error(`Invalid resolver definition: ${parsed.error.message}`);
      }

      // Validate code for forbidden patterns
      const validation = validateResolverCode(resolver.code);
      if (!validation.valid) {
        throw new Error(`Invalid resolver code: ${validation.error}`);
      }
    }

    const entries = this.resolvers.get(mapName) ?? [];

    // Check max resolvers limit
    if (entries.length >= this.config.maxResolversPerMap) {
      throw new Error(
        `Maximum resolvers per map (${this.config.maxResolversPerMap}) exceeded`,
      );
    }

    // Remove existing with same name
    const filtered = entries.filter((e) => e.resolver.name !== resolver.name);

    // Create entry
    const entry: ResolverEntry<V> = {
      resolver,
      registeredBy,
    };

    // Pre-compile sandboxed code if needed
    if (resolver.code && !resolver.fn && this.config.enableSandboxedResolvers) {
      entry.compiledFn = this.compileSandboxed<V>(resolver.name, resolver.code);
    }

    filtered.push(entry as ResolverEntry);

    // Sort by priority (highest first)
    filtered.sort(
      (a, b) => (b.resolver.priority ?? 50) - (a.resolver.priority ?? 50),
    );

    this.resolvers.set(mapName, filtered);

    logger.debug(
      `Registered resolver '${resolver.name}' for map '${mapName}' with priority ${resolver.priority ?? 50}`,
    );
  }

  /**
   * Unregister a resolver.
   *
   * @param mapName The map name
   * @param resolverName The resolver name to unregister
   * @param clientId Optional - only unregister if registered by this client
   */
  unregister(
    mapName: string,
    resolverName: string,
    clientId?: string,
  ): boolean {
    const entries = this.resolvers.get(mapName);
    if (!entries) return false;

    const entryIndex = entries.findIndex(
      (e) =>
        e.resolver.name === resolverName &&
        (!clientId || e.registeredBy === clientId),
    );

    if (entryIndex === -1) return false;

    entries.splice(entryIndex, 1);

    if (entries.length === 0) {
      this.resolvers.delete(mapName);
    }

    logger.debug(`Unregistered resolver '${resolverName}' from map '${mapName}'`);
    return true;
  }

  /**
   * Resolve a merge conflict using registered resolvers.
   *
   * @param context The merge context
   * @returns The merge result
   */
  async resolve<V>(context: MergeContext<V>): Promise<MergeResult<V>> {
    if (this.disposed) {
      return { action: 'accept', value: context.remoteValue };
    }

    const entries = this.resolvers.get(context.mapName) ?? [];

    // Always add LWW as fallback (lowest priority)
    const allEntries: ResolverEntry[] = [
      ...entries,
      { resolver: BuiltInResolvers.LWW() },
    ];

    for (const entry of allEntries) {
      const { resolver } = entry;

      // Check key pattern if specified
      if (resolver.keyPattern && !this.matchKeyPattern(context.key, resolver.keyPattern)) {
        continue;
      }

      try {
        let result: MergeResult<V>;

        if (resolver.fn) {
          // Native function execution - cast context to unknown since resolvers are stored with unknown type
          const fn = resolver.fn as (ctx: MergeContext<V>) => MergeResult<V> | Promise<MergeResult<V>>;
          const maybePromise = fn(context);
          result = maybePromise instanceof Promise ? await maybePromise : maybePromise;
        } else if (entry.compiledFn) {
          // Sandboxed code execution
          const compiledFn = entry.compiledFn as (ctx: MergeContext<V>) => Promise<MergeResult<V>>;
          result = await compiledFn(context);
        } else {
          // Skip resolvers without executable code
          continue;
        }

        // Only 'local' allows falling through to next resolver
        if (result.action !== 'local') {
          // Log rejections
          if (result.action === 'reject') {
            logger.debug(
              `Resolver '${resolver.name}' rejected merge for key '${context.key}' in map '${context.mapName}': ${result.reason}`,
            );

            // Emit rejection event
            if (this.onRejectionCallback) {
              this.onRejectionCallback({
                mapName: context.mapName,
                key: context.key,
                attemptedValue: context.remoteValue,
                reason: result.reason,
                timestamp: context.remoteTimestamp,
                nodeId: context.remoteNodeId,
              });
            }
          }

          return result;
        }
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        logger.error(`Resolver '${resolver.name}' threw error: ${message}`);
        // Continue to next resolver on error
      }
    }

    // Fallback: accept remote value (LWW should have handled this, but just in case)
    return { action: 'accept', value: context.remoteValue };
  }

  /**
   * List registered resolvers.
   *
   * @param mapName Optional - filter by map name
   */
  list(mapName?: string): Array<{
    mapName: string;
    name: string;
    priority?: number;
    keyPattern?: string;
    registeredBy?: string;
  }> {
    const result: Array<{
      mapName: string;
      name: string;
      priority?: number;
      keyPattern?: string;
      registeredBy?: string;
    }> = [];

    if (mapName) {
      const entries = this.resolvers.get(mapName) ?? [];
      for (const entry of entries) {
        result.push({
          mapName,
          name: entry.resolver.name,
          priority: entry.resolver.priority,
          keyPattern: entry.resolver.keyPattern,
          registeredBy: entry.registeredBy,
        });
      }
    } else {
      for (const [map, entries] of this.resolvers.entries()) {
        for (const entry of entries) {
          result.push({
            mapName: map,
            name: entry.resolver.name,
            priority: entry.resolver.priority,
            keyPattern: entry.resolver.keyPattern,
            registeredBy: entry.registeredBy,
          });
        }
      }
    }

    return result;
  }

  /**
   * Check if a map has any registered resolvers.
   */
  hasResolvers(mapName: string): boolean {
    const entries = this.resolvers.get(mapName);
    return entries !== undefined && entries.length > 0;
  }

  /**
   * Get the number of registered resolvers.
   */
  get size(): number {
    let count = 0;
    for (const entries of this.resolvers.values()) {
      count += entries.length;
    }
    return count;
  }

  /**
   * Clear all registered resolvers.
   *
   * @param mapName Optional - only clear resolvers for specific map
   */
  clear(mapName?: string): void {
    if (mapName) {
      this.resolvers.delete(mapName);
    } else {
      this.resolvers.clear();
    }
  }

  /**
   * Clear resolvers registered by a specific client.
   */
  clearByClient(clientId: string): number {
    let removed = 0;

    for (const [mapName, entries] of this.resolvers.entries()) {
      const before = entries.length;
      const filtered = entries.filter((e) => e.registeredBy !== clientId);
      removed += before - filtered.length;

      if (filtered.length === 0) {
        this.resolvers.delete(mapName);
      } else if (filtered.length !== before) {
        this.resolvers.set(mapName, filtered);
      }
    }

    return removed;
  }

  /**
   * Dispose the service.
   */
  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.resolvers.clear();
    logger.debug('ConflictResolverService disposed');
  }

  /**
   * Match a key against a glob-like pattern.
   * Supports * (any chars) and ? (single char).
   */
  private matchKeyPattern(key: string, pattern: string): boolean {
    // Convert glob pattern to regex
    const regexPattern = pattern
      .replace(/[.+^${}()|[\]\\]/g, '\\$&') // Escape special regex chars
      .replace(/\*/g, '.*') // * -> .*
      .replace(/\?/g, '.'); // ? -> .

    const regex = new RegExp(`^${regexPattern}$`);
    return regex.test(key);
  }

  /**
   * Compile sandboxed resolver code.
   */
  private compileSandboxed<V>(
    name: string,
    code: string,
  ): (ctx: MergeContext<V>) => Promise<MergeResult<V>> {
    return async (ctx: MergeContext<V>) => {
      // Build the resolver code wrapper
      const wrappedCode = `
        const context = {
          mapName: ${JSON.stringify(ctx.mapName)},
          key: ${JSON.stringify(ctx.key)},
          localValue: ${JSON.stringify(ctx.localValue)},
          remoteValue: ${JSON.stringify(ctx.remoteValue)},
          localTimestamp: ${JSON.stringify(ctx.localTimestamp)},
          remoteTimestamp: ${JSON.stringify(ctx.remoteTimestamp)},
          remoteNodeId: ${JSON.stringify(ctx.remoteNodeId)},
          auth: ${JSON.stringify(ctx.auth)},
        };

        function resolve(context) {
          ${code}
        }

        const result = resolve(context);
        return { value: result, result };
      `;

      // Execute using ProcessorSandbox
      const result = await this.sandbox.execute(
        {
          name: `resolver:${name}`,
          code: wrappedCode,
        },
        null, // value parameter unused for resolvers
        'resolver',
      );

      if (!result.success) {
        throw new Error(result.error || 'Resolver execution failed');
      }

      // Extract result from sandbox
      const resolverResult = (result as { result?: MergeResult<V> }).result;

      if (!resolverResult || typeof resolverResult !== 'object') {
        throw new Error('Resolver must return a result object');
      }

      // Validate result format
      const action = (resolverResult as MergeResult<V>).action;
      if (!['accept', 'reject', 'merge', 'local'].includes(action)) {
        throw new Error(`Invalid resolver action: ${action}`);
      }

      return resolverResult as MergeResult<V>;
    };
  }
}
