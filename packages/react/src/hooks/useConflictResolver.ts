import { useState, useCallback, useEffect } from 'react';
import { useClient } from './useClient';
import type { ConflictResolverDef } from '@topgunbuild/core';
import type { ResolverInfo, RegisterResult } from '@topgunbuild/client';

/**
 * Options for useConflictResolver hook.
 */
export interface UseConflictResolverOptions {
  /** Auto-unregister resolver on unmount (default: true) */
  autoUnregister?: boolean;
}

/**
 * Result type for useConflictResolver hook.
 */
export interface UseConflictResolverResult {
  /**
   * Register a conflict resolver on the server.
   * @param resolver The resolver definition
   */
  register: (resolver: Omit<ConflictResolverDef, 'fn'>) => Promise<RegisterResult>;

  /**
   * Unregister a resolver by name.
   * @param resolverName Name of the resolver to unregister
   */
  unregister: (resolverName: string) => Promise<RegisterResult>;

  /**
   * List all registered resolvers for this map.
   */
  list: () => Promise<ResolverInfo[]>;

  /** True while a registration/unregistration is in progress */
  loading: boolean;

  /** Last error encountered */
  error: Error | null;

  /** List of resolvers registered by this hook instance */
  registered: string[];
}

/**
 * React hook for managing conflict resolvers on a specific map.
 *
 * Conflict resolvers allow you to customize how merge conflicts are handled
 * on the server. This hook provides a convenient way to:
 * - Register custom resolvers
 * - Auto-unregister on component unmount
 * - Track registration state
 *
 * @param mapName Name of the map to manage resolvers for
 * @param options Optional configuration
 * @returns Resolver management functions and state
 *
 * @example First-write-wins for bookings
 * ```tsx
 * function BookingManager() {
 *   const { register, registered, loading, error } = useConflictResolver('bookings');
 *
 *   useEffect(() => {
 *     // Register resolver on mount
 *     register({
 *       name: 'first-write-wins',
 *       code: `
 *         if (context.localValue !== undefined) {
 *           return { action: 'reject', reason: 'Already booked' };
 *         }
 *         return { action: 'accept', value: context.remoteValue };
 *       `,
 *       priority: 100,
 *     });
 *   }, []);
 *
 *   return (
 *     <div>
 *       {loading && <span>Registering...</span>}
 *       {error && <span>Error: {error.message}</span>}
 *       <ul>
 *         {registered.map(name => <li key={name}>{name}</li>)}
 *       </ul>
 *     </div>
 *   );
 * }
 * ```
 *
 * @example Numeric constraints
 * ```tsx
 * function InventorySettings() {
 *   const { register } = useConflictResolver('inventory');
 *
 *   const enableNonNegative = async () => {
 *     await register({
 *       name: 'non-negative',
 *       code: `
 *         if (context.remoteValue < 0) {
 *           return { action: 'reject', reason: 'Stock cannot be negative' };
 *         }
 *         return { action: 'accept', value: context.remoteValue };
 *       `,
 *       priority: 90,
 *       keyPattern: 'stock:*',
 *     });
 *   };
 *
 *   return <button onClick={enableNonNegative}>Enable Stock Protection</button>;
 * }
 * ```
 */
export function useConflictResolver(
  mapName: string,
  options: UseConflictResolverOptions = {},
): UseConflictResolverResult {
  const client = useClient();
  const { autoUnregister = true } = options;

  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<Error | null>(null);
  const [registered, setRegistered] = useState<string[]>([]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      if (autoUnregister && registered.length > 0) {
        const resolvers = client.getConflictResolvers();
        // Fire-and-forget unregistration
        for (const name of registered) {
          resolvers.unregister(mapName, name).catch(() => {
            // Ignore errors on cleanup
          });
        }
      }
    };
  }, [client, mapName, autoUnregister, registered]);

  const register = useCallback(
    async (resolver: Omit<ConflictResolverDef, 'fn'>): Promise<RegisterResult> => {
      setLoading(true);
      setError(null);

      try {
        const resolvers = client.getConflictResolvers();
        const result = await resolvers.register(mapName, resolver);

        if (result.success) {
          setRegistered((prev) => {
            if (prev.includes(resolver.name)) {
              return prev;
            }
            return [...prev, resolver.name];
          });
        } else if (result.error) {
          setError(new Error(result.error));
        }

        return result;
      } catch (e) {
        const err = e instanceof Error ? e : new Error(String(e));
        setError(err);
        return { success: false, error: err.message };
      } finally {
        setLoading(false);
      }
    },
    [client, mapName],
  );

  const unregister = useCallback(
    async (resolverName: string): Promise<RegisterResult> => {
      setLoading(true);
      setError(null);

      try {
        const resolvers = client.getConflictResolvers();
        const result = await resolvers.unregister(mapName, resolverName);

        if (result.success) {
          setRegistered((prev) => prev.filter((n) => n !== resolverName));
        } else if (result.error) {
          setError(new Error(result.error));
        }

        return result;
      } catch (e) {
        const err = e instanceof Error ? e : new Error(String(e));
        setError(err);
        return { success: false, error: err.message };
      } finally {
        setLoading(false);
      }
    },
    [client, mapName],
  );

  const list = useCallback(async (): Promise<ResolverInfo[]> => {
    try {
      const resolvers = client.getConflictResolvers();
      return await resolvers.list(mapName);
    } catch (e) {
      const err = e instanceof Error ? e : new Error(String(e));
      setError(err);
      return [];
    }
  }, [client, mapName]);

  return {
    register,
    unregister,
    list,
    loading,
    error,
    registered,
  };
}
