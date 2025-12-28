import { useState, useEffect, useCallback, useMemo } from 'react';
import { useClient } from './useClient';

/**
 * Result type for usePNCounter hook.
 */
export interface UsePNCounterResult {
  /** Current counter value */
  value: number;
  /** Increment the counter by 1 */
  increment: () => void;
  /** Decrement the counter by 1 */
  decrement: () => void;
  /** Add delta (positive or negative) to the counter */
  add: (delta: number) => void;
  /** Loading state (true until first value received) */
  loading: boolean;
}

/**
 * React hook for using a PN Counter with real-time updates.
 *
 * PN Counters support increment and decrement operations that work offline
 * and sync to server when connected. They guarantee convergence across
 * distributed nodes without coordination.
 *
 * @param name The counter name (e.g., 'likes:post-123')
 * @returns Counter value and methods
 *
 * @example Basic usage
 * ```tsx
 * function LikeButton({ postId }: { postId: string }) {
 *   const { value, increment } = usePNCounter(`likes:${postId}`);
 *
 *   return (
 *     <button onClick={increment}>
 *       ❤️ {value}
 *     </button>
 *   );
 * }
 * ```
 *
 * @example Inventory control
 * ```tsx
 * function InventoryControl({ productId }: { productId: string }) {
 *   const { value, increment, decrement } = usePNCounter(`inventory:${productId}`);
 *
 *   return (
 *     <div>
 *       <span>Stock: {value}</span>
 *       <button onClick={decrement} disabled={value <= 0}>-</button>
 *       <button onClick={increment}>+</button>
 *     </div>
 *   );
 * }
 * ```
 *
 * @example Bulk operations
 * ```tsx
 * function BulkAdd({ counterId }: { counterId: string }) {
 *   const { value, add } = usePNCounter(counterId);
 *   const [amount, setAmount] = useState(10);
 *
 *   return (
 *     <div>
 *       <span>Value: {value}</span>
 *       <input
 *         type="number"
 *         value={amount}
 *         onChange={(e) => setAmount(parseInt(e.target.value))}
 *       />
 *       <button onClick={() => add(amount)}>Add {amount}</button>
 *       <button onClick={() => add(-amount)}>Subtract {amount}</button>
 *     </div>
 *   );
 * }
 * ```
 */
export function usePNCounter(name: string): UsePNCounterResult {
  const client = useClient();
  const [value, setValue] = useState(0);
  const [loading, setLoading] = useState(true);

  // Get or create counter handle - memoized by name
  const counter = useMemo(() => {
    return client.getPNCounter(name);
  }, [client, name]);

  useEffect(() => {
    // Reset state when counter changes
    setLoading(true);

    const unsubscribe = counter.subscribe((newValue) => {
      setValue(newValue);
      setLoading(false);
    });

    return unsubscribe;
  }, [counter]);

  const increment = useCallback(() => {
    counter.increment();
  }, [counter]);

  const decrement = useCallback(() => {
    counter.decrement();
  }, [counter]);

  const add = useCallback((delta: number) => {
    counter.addAndGet(delta);
  }, [counter]);

  return useMemo(
    () => ({ value, increment, decrement, add, loading }),
    [value, increment, decrement, add, loading]
  );
}
