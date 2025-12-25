import { useState, useCallback, useMemo } from 'react';
import { useClient } from './useClient';
import type { EntryProcessorDef, EntryProcessorResult } from '@topgunbuild/core';

/**
 * Options for the useEntryProcessor hook.
 */
export interface UseEntryProcessorOptions {
  /**
   * Number of retry attempts on failure.
   * Default: 0 (no retries)
   */
  retries?: number;

  /**
   * Delay between retries in milliseconds.
   * Default: 100ms, doubles with each retry (exponential backoff)
   */
  retryDelayMs?: number;
}

/**
 * Result type for useEntryProcessor hook.
 */
export interface UseEntryProcessorResult<R> {
  /**
   * Execute the processor on a key.
   * @param key The key to process
   * @param args Optional arguments to pass to the processor
   */
  execute: (key: string, args?: unknown) => Promise<EntryProcessorResult<R>>;

  /**
   * Execute the processor on multiple keys.
   * @param keys The keys to process
   * @param args Optional arguments to pass to the processor
   */
  executeMany: (keys: string[], args?: unknown) => Promise<Map<string, EntryProcessorResult<R>>>;

  /** True while a processor is executing */
  executing: boolean;

  /** Last execution result (single key) */
  lastResult: EntryProcessorResult<R> | null;

  /** Last error encountered */
  error: Error | null;

  /** Reset the hook state (clears lastResult and error) */
  reset: () => void;
}

/**
 * React hook for executing entry processors with loading and error states.
 *
 * Entry processors execute user-defined logic atomically on the server,
 * solving the read-modify-write race condition.
 *
 * @param mapName Name of the map to operate on
 * @param processorDef Processor definition (without args - args are passed per-execution)
 * @param options Optional configuration
 * @returns Execute function and state
 *
 * @example Basic increment
 * ```tsx
 * function LikeButton({ postId }: { postId: string }) {
 *   const { execute, executing } = useEntryProcessor<number>('likes', {
 *     name: 'increment',
 *     code: `
 *       const current = value ?? 0;
 *       return { value: current + 1, result: current + 1 };
 *     `,
 *   });
 *
 *   const handleLike = async () => {
 *     const result = await execute(postId);
 *     if (result.success) {
 *       console.log('New like count:', result.result);
 *     }
 *   };
 *
 *   return (
 *     <button onClick={handleLike} disabled={executing}>
 *       {executing ? '...' : 'Like'}
 *     </button>
 *   );
 * }
 * ```
 *
 * @example Inventory reservation with args
 * ```tsx
 * function ReserveButton({ productId }: { productId: string }) {
 *   const { execute, executing, error } = useEntryProcessor<
 *     { stock: number; reserved: string[] },
 *     { success: boolean; remaining: number }
 *   >('inventory', {
 *     name: 'reserve_item',
 *     code: `
 *       if (!value || value.stock <= 0) {
 *         return { value, result: { success: false, remaining: 0 } };
 *       }
 *       const newValue = {
 *         ...value,
 *         stock: value.stock - 1,
 *         reserved: [...value.reserved, args.userId],
 *       };
 *       return {
 *         value: newValue,
 *         result: { success: true, remaining: newValue.stock }
 *       };
 *     `,
 *   });
 *
 *   const handleReserve = async () => {
 *     const result = await execute(productId, { userId: currentUser.id });
 *     if (result.success && result.result?.success) {
 *       toast.success(`Reserved! ${result.result.remaining} left`);
 *     } else {
 *       toast.error('Out of stock');
 *     }
 *   };
 *
 *   return (
 *     <button onClick={handleReserve} disabled={executing}>
 *       {executing ? 'Reserving...' : 'Reserve'}
 *     </button>
 *   );
 * }
 * ```
 *
 * @example Using built-in processor
 * ```tsx
 * import { BuiltInProcessors } from '@topgunbuild/core';
 *
 * function DecrementStock({ productId }: { productId: string }) {
 *   const processorDef = useMemo(
 *     () => BuiltInProcessors.DECREMENT_FLOOR(1),
 *     []
 *   );
 *
 *   const { execute, executing, lastResult } = useEntryProcessor<
 *     number,
 *     { newValue: number; wasFloored: boolean }
 *   >('stock', processorDef);
 *
 *   const handleDecrement = async () => {
 *     const result = await execute(productId);
 *     if (result.result?.wasFloored) {
 *       alert('Stock is now at zero!');
 *     }
 *   };
 *
 *   return (
 *     <button onClick={handleDecrement} disabled={executing}>
 *       Decrease Stock
 *     </button>
 *   );
 * }
 * ```
 */
export function useEntryProcessor<V = unknown, R = V>(
  mapName: string,
  processorDef: Omit<EntryProcessorDef<V, R>, 'args'>,
  options: UseEntryProcessorOptions = {},
): UseEntryProcessorResult<R> {
  const client = useClient();
  const [executing, setExecuting] = useState(false);
  const [lastResult, setLastResult] = useState<EntryProcessorResult<R> | null>(null);
  const [error, setError] = useState<Error | null>(null);

  const { retries = 0, retryDelayMs = 100 } = options;

  const execute = useCallback(
    async (key: string, args?: unknown): Promise<EntryProcessorResult<R>> => {
      setExecuting(true);
      setError(null);

      const processor: EntryProcessorDef<V, R> = {
        ...processorDef,
        args,
      } as EntryProcessorDef<V, R>;

      let attempts = 0;
      let lastError: Error | null = null;

      while (attempts <= retries) {
        try {
          const result = await client.executeOnKey<V, R>(mapName, key, processor);
          setLastResult(result);
          setExecuting(false);
          return result;
        } catch (err) {
          lastError = err instanceof Error ? err : new Error(String(err));
          attempts++;

          if (attempts <= retries) {
            // Exponential backoff
            const delay = retryDelayMs * Math.pow(2, attempts - 1);
            await new Promise((r) => setTimeout(r, delay));
          }
        }
      }

      // All retries exhausted
      setError(lastError);
      setExecuting(false);
      throw lastError;
    },
    [client, mapName, processorDef, retries, retryDelayMs],
  );

  const executeMany = useCallback(
    async (keys: string[], args?: unknown): Promise<Map<string, EntryProcessorResult<R>>> => {
      setExecuting(true);
      setError(null);

      const processor: EntryProcessorDef<V, R> = {
        ...processorDef,
        args,
      } as EntryProcessorDef<V, R>;

      try {
        const results = await client.executeOnKeys<V, R>(mapName, keys, processor);
        setExecuting(false);
        return results;
      } catch (err) {
        const error = err instanceof Error ? err : new Error(String(err));
        setError(error);
        setExecuting(false);
        throw error;
      }
    },
    [client, mapName, processorDef],
  );

  const reset = useCallback(() => {
    setLastResult(null);
    setError(null);
  }, []);

  return useMemo(
    () => ({ execute, executeMany, executing, lastResult, error, reset }),
    [execute, executeMany, executing, lastResult, error, reset],
  );
}
