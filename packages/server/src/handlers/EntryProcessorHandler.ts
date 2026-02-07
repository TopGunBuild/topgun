import {
  HLC,
  LWWMap,
  EntryProcessorDef,
  EntryProcessorDefSchema,
  EntryProcessorResult,
  Timestamp,
} from '@topgunbuild/core';
import { ProcessorSandbox, ProcessorSandboxConfig } from '../ProcessorSandbox';
import { logger } from '../utils/logger';

/**
 * Configuration for the EntryProcessorHandler.
 */
export interface EntryProcessorHandlerConfig {
  /** HLC instance for timestamp generation */
  hlc: HLC;

  /** Optional sandbox configuration override */
  sandboxConfig?: Partial<ProcessorSandboxConfig>;
}

/**
 * Server-side handler for Entry Processor execution.
 *
 * Responsibilities:
 * - Validate incoming processor definitions
 * - Execute processors in sandboxed environment
 * - Update map state atomically
 * - Return results with new values for client cache sync
 */
export class EntryProcessorHandler {
  private sandbox: ProcessorSandbox;
  private hlc: HLC;

  // Per-key operation queue to serialize concurrent read-modify-write sequences
  private readonly keyQueues = new WeakMap<
    LWWMap<any, any>,
    Map<string, Promise<void>>
  >();

  constructor(config: EntryProcessorHandlerConfig) {
    this.hlc = config.hlc;
    this.sandbox = new ProcessorSandbox(config.sandboxConfig);
  }

  /**
   * Serialize async operations per key to prevent concurrent read-modify-write races.
   * Each key gets its own promise chain; operations on different keys run concurrently.
   */
  private withKeyLock<T>(
    map: LWWMap<any, any>,
    key: string,
    fn: () => Promise<T>,
  ): Promise<T> {
    let queues = this.keyQueues.get(map);
    if (!queues) {
      queues = new Map();
      this.keyQueues.set(map, queues);
    }

    const prev = queues.get(key) ?? Promise.resolve();

    let resolveDone!: () => void;
    const done = new Promise<void>((r) => {
      resolveDone = r;
    });
    queues.set(key, done);

    const execute = async (): Promise<T> => {
      await prev;
      try {
        return await fn();
      } finally {
        resolveDone();
        if (queues!.get(key) === done) {
          queues!.delete(key);
        }
      }
    };

    return execute();
  }

  /**
   * Execute a processor on a single key atomically.
   *
   * @param map The LWWMap to operate on
   * @param key The key to process
   * @param processorDef The processor definition (will be validated)
   * @returns Result with success status, processor result, and new value
   */
  async executeOnKey<V, R>(
    map: LWWMap<string, V>,
    key: string,
    processorDef: unknown,
  ): Promise<{ result: EntryProcessorResult<R>; timestamp?: Timestamp }> {
    // Validate processor definition
    const parseResult = EntryProcessorDefSchema.safeParse(processorDef);
    if (!parseResult.success) {
      logger.warn(
        { key, error: parseResult.error.message },
        'Invalid processor definition',
      );
      return {
        result: {
          success: false,
          error: `Invalid processor: ${parseResult.error.message}`,
        },
      };
    }

    const processor = parseResult.data as EntryProcessorDef<V, R>;

    // Serialize read-modify-write per key to prevent concurrent races
    return this.withKeyLock(map, key, async () => {
      // Get current value
      const currentValue = map.get(key);

      logger.debug(
        { key, processor: processor.name, hasValue: currentValue !== undefined },
        'Executing entry processor',
      );

      // Execute in sandbox
      const sandboxResult = await this.sandbox.execute(
        processor,
        currentValue,
        key,
      );

      if (!sandboxResult.success) {
        logger.warn(
          { key, processor: processor.name, error: sandboxResult.error },
          'Processor execution failed',
        );
        return { result: sandboxResult };
      }

      // Apply the change if value changed
      let timestamp: Timestamp | undefined;

      if (sandboxResult.newValue !== undefined) {
        // Set new value - map.set() generates timestamp internally
        const record = map.set(key, sandboxResult.newValue as V);
        timestamp = record.timestamp;

        logger.debug(
          { key, processor: processor.name, timestamp },
          'Processor updated value',
        );
      } else if (currentValue !== undefined) {
        // undefined newValue means delete
        const tombstone = map.remove(key);
        timestamp = tombstone.timestamp;

        logger.debug(
          { key, processor: processor.name, timestamp },
          'Processor deleted value',
        );
      }

      return {
        result: sandboxResult,
        timestamp,
      };
    });
  }

  /**
   * Execute a processor on multiple keys.
   *
   * Each key is processed sequentially to ensure atomicity per-key.
   * For parallel execution across keys, use multiple calls.
   *
   * @param map The LWWMap to operate on
   * @param keys The keys to process
   * @param processorDef The processor definition
   * @returns Map of key -> result
   */
  async executeOnKeys<V, R>(
    map: LWWMap<string, V>,
    keys: string[],
    processorDef: unknown,
  ): Promise<{
    results: Map<string, EntryProcessorResult<R>>;
    timestamps: Map<string, Timestamp>;
  }> {
    const results = new Map<string, EntryProcessorResult<R>>();
    const timestamps = new Map<string, Timestamp>();

    // Validate once before processing
    const parseResult = EntryProcessorDefSchema.safeParse(processorDef);
    if (!parseResult.success) {
      const errorResult: EntryProcessorResult<R> = {
        success: false,
        error: `Invalid processor: ${parseResult.error.message}`,
      };
      for (const key of keys) {
        results.set(key, errorResult);
      }
      return { results, timestamps };
    }

    // Execute for each key
    for (const key of keys) {
      const { result, timestamp } = await this.executeOnKey<V, R>(
        map,
        key,
        processorDef,
      );
      results.set(key, result);
      if (timestamp) {
        timestamps.set(key, timestamp);
      }
    }

    return { results, timestamps };
  }

  /**
   * Execute a processor on all entries matching a predicate.
   *
   * WARNING: This can be expensive for large maps.
   *
   * @param map The LWWMap to operate on
   * @param processorDef The processor definition
   * @param predicateCode Optional predicate code to filter entries
   * @returns Map of key -> result for processed entries
   */
  async executeOnEntries<V, R>(
    map: LWWMap<string, V>,
    processorDef: unknown,
    predicateCode?: string,
  ): Promise<{
    results: Map<string, EntryProcessorResult<R>>;
    timestamps: Map<string, Timestamp>;
  }> {
    const results = new Map<string, EntryProcessorResult<R>>();
    const timestamps = new Map<string, Timestamp>();

    // Validate processor
    const parseResult = EntryProcessorDefSchema.safeParse(processorDef);
    if (!parseResult.success) {
      return { results, timestamps };
    }

    const entries = map.entries();

    for (const [key, value] of entries) {
      // Apply predicate if provided
      if (predicateCode) {
        const predicateResult = await this.sandbox.execute(
          {
            name: '_predicate',
            code: `return { value, result: (function() { ${predicateCode} })() };`,
          },
          value,
          key,
        );

        if (!predicateResult.success || !predicateResult.result) {
          continue; // Skip this entry
        }
      }

      const { result, timestamp } = await this.executeOnKey<V, R>(
        map,
        key,
        processorDef,
      );
      results.set(key, result);
      if (timestamp) {
        timestamps.set(key, timestamp);
      }
    }

    return { results, timestamps };
  }

  /**
   * Check if sandbox is in secure mode (using isolated-vm).
   */
  isSecureMode(): boolean {
    return this.sandbox.isSecureMode();
  }

  /**
   * Get sandbox cache statistics.
   */
  getCacheStats(): { isolates: number; scripts: number; fallbackScripts: number } {
    return this.sandbox.getCacheStats();
  }

  /**
   * Clear sandbox cache.
   */
  clearCache(processorName?: string): void {
    this.sandbox.clearCache(processorName);
  }

  /**
   * Dispose of the handler and its sandbox.
   */
  dispose(): void {
    this.sandbox.dispose();
    logger.debug('EntryProcessorHandler disposed');
  }
}
