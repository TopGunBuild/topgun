import { TopGunClient, Predicates } from '@topgunbuild/client';
import type { BetterAuthOptions } from 'better-auth';
import type {
  DBAdapter,
  Where,
} from 'better-auth/adapters';
import type { PredicateNode } from '@topgunbuild/core';

/**
 * Base interface for all BetterAuth records stored in TopGun.
 * Allows string-indexed properties for flexibility with different model types.
 */
interface AuthRecord {
  id: string;
  [key: string]: unknown;
}

/**
 * Sort direction for query ordering.
 */
type SortDirection = 'asc' | 'desc';

/**
 * Sort specification mapping field names to sort directions.
 */
type SortSpec = Record<string, SortDirection>;

export interface TopGunAdapterOptions {
  client: TopGunClient;
  /**
   * Map model names to TopGun map names.
   * Default: "auth_user", "auth_session", etc.
   */
  modelMap?: Record<string, string>;
  /** Wait for client storage to be ready before accepting requests (default: true) */
  waitForReady?: boolean;
  /**
   * Map model names to their foreign key field for join operations.
   * Default: "userId" for all models.
   * Example: { account: "ownerId", session: "userId" }
   */
  foreignKeyMap?: Record<string, string>;
}

/**
 * Options for cursor-based pagination via findManyWithCursor.
 * Cursor pagination always uses sort by id asc for stable page results.
 */
export interface TopGunAdapterCursorOptions {
  /** ID of the last record from the previous page. Results will start after this record. */
  afterCursor?: string;
}

/**
 * Extended adapter type that includes the TopGun-specific findManyWithCursor method.
 * This type exposes the cursor-based pagination extension without modifying BetterAuth's
 * DBAdapter interface contract.
 *
 * The sortBy parameter is intentionally absent from findManyWithCursor — cursor pagination
 * always enforces sort by id asc to guarantee stable pages with string IDs.
 */
export type TopGunDBAdapter = DBAdapter & {
  findManyWithCursor(params: {
    model: string;
    where?: Where[];
    limit?: number;
    cursor?: TopGunAdapterCursorOptions['afterCursor'];
  }): Promise<{ data: Record<string, unknown>[]; nextCursor: string | null }>;
};

export const topGunAdapter = (adapterOptions: TopGunAdapterOptions): ((options: BetterAuthOptions) => TopGunDBAdapter) => {
  return (options: BetterAuthOptions): TopGunDBAdapter => {
    const { client, modelMap = {} } = adapterOptions;

    const getMapName = (model: string) => {
      return modelMap[model] || `auth_${model}`;
    };

    // Ready state tracking for cold start race condition fix
    const shouldWaitForReady = adapterOptions.waitForReady ?? true;
    let isReady = false;
    let readyPromise: Promise<void> | null = null;

    const ensureReady = async (): Promise<void> => {
      if (!shouldWaitForReady || isReady) return;
      if (!readyPromise) {
        // client.start() ensures storage is initialized and loaded
        readyPromise = client.start().then(() => {
          isReady = true;
        });
      }
      await readyPromise;
    };

    const whereToPredicate = (where: Where[]): PredicateNode | undefined => {
      if (!where || where.length === 0) return undefined;

      const predicates: PredicateNode[] = where.map(w => {
        const field = w.field;
        const value = w.value;
        
        switch (w.operator) {
          case 'eq': return Predicates.equal(field, value);
          case 'ne': return Predicates.notEqual(field, value);
          case 'lt': return Predicates.lessThan(field, value);
          case 'lte': return Predicates.lessThanOrEqual(field, value);
          case 'gt': return Predicates.greaterThan(field, value);
          case 'gte': return Predicates.greaterThanOrEqual(field, value);
          case 'contains': return Predicates.like(field, `%${value}%`);
          case 'starts_with': return Predicates.like(field, `${value}%`);
          case 'ends_with': return Predicates.like(field, `%${value}`);
          case 'in': 
            if (Array.isArray(value)) {
              return Predicates.or(...value.map(v => Predicates.equal(field, v)));
            }
            return Predicates.equal(field, value);
          case 'not_in':
             if (Array.isArray(value)) {
               return Predicates.and(...value.map(v => Predicates.notEqual(field, v)));
             }
             return Predicates.notEqual(field, value);
          default: return Predicates.equal(field, value);
        }
      });

      // BetterAuth Where[] implies AND
      if (predicates.length === 1) return predicates[0];
      return Predicates.and(...predicates);
    };

    /**
     * Run a query against TopGun.
     *
     * When cursor is provided, cursor-based pagination is used:
     * - Sort is forced to { id: 'asc' } for stable page results
     * - A greaterThan('id', cursor) predicate is added
     * - limit is used directly without over-fetching
     *
     * Without cursor (default path): BetterAuth offset-based pagination is emulated
     * by fetching limit+offset records and slicing client-side. Acceptable for auth
     * queries which typically have small result sets.
     */
    const runQuery = async <T extends AuthRecord>(model: string, where?: Where[], sort?: SortSpec, limit?: number, offset?: number, cursor?: string): Promise<T[]> => {
      const mapName = getMapName(model);

      if (cursor !== undefined) {
        // Cursor-based pagination: enforce id asc sort, add id > cursor predicate
        const cursorPredicate = Predicates.greaterThan('id', cursor);
        const wherePredicate = where ? whereToPredicate(where) : undefined;
        const predicate = wherePredicate
          ? Predicates.and(wherePredicate, cursorPredicate)
          : cursorPredicate;

        const filter = {
          predicate,
          sort: { id: 'asc' } as SortSpec,
          limit,
        };

        return new Promise((resolve) => {
          const handle = client.query<T>(mapName, filter);
          const unsubscribe = handle.subscribe((results: T[]) => {
            unsubscribe();
            // Apply limit client-side — the query engine may not enforce it in all environments
            const limited = limit !== undefined ? results.slice(0, limit) : results;
            resolve(limited);
          });
        });
      }

      const predicate = where ? whereToPredicate(where) : undefined;

      // For BetterAuth offset compatibility, we request more results and slice
      const effectiveLimit = limit && offset ? limit + offset : limit;

      const filter = {
        predicate,
        sort,
        limit: effectiveLimit,
        // TopGun uses cursor-based pagination; offset is handled client-side for BetterAuth compatibility
      };

      // We use client.query which subscribes. We wait for the first result.
      // TopGun QueryHandle is reactive. We need a one-shot fetch.

      return new Promise((resolve) => {
        const handle = client.query<T>(mapName, filter);

        // Subscribe returns an unsubscribe function
        const unsubscribe = handle.subscribe((results: T[]) => {
           unsubscribe();
           // Apply offset client-side for BetterAuth compatibility
           const sliced = offset ? results.slice(offset, offset + (limit || results.length)) : results;
           resolve(sliced);
        });
      });
    };

    // Type assertion needed because BetterAuth's DBAdapter uses method-level generics
    // that TypeScript can't verify at compile time. Our AuthRecord constraint provides
    // internal type safety while the adapter boundary requires runtime type flexibility.
    return {
      id: 'topgun-adapter',

      async create({ model, data }) {
        await ensureReady();
        const mapName = getMapName(model);
        const dataWithId = data as Partial<AuthRecord> & Record<string, unknown>;
        const id = dataWithId.id || crypto.randomUUID();
        const record: AuthRecord = { ...data, id };

        // Use LWWMap for standard records
        const map = client.getMap<string, AuthRecord>(mapName);
        map.set(id, record);

        // map.set is optimistic and writes to local storage/sync engine.
        // Ideally we wait for confirmation? TopGun doesn't expose Promise for set completion easily
        // (it returns the record). But SyncEngine queues it.

        // Type assertion needed to match DBAdapter's generic return type
        return record as unknown as typeof data & { id: string };
      },

      async findOne({ model, where, select, join }) {
        await ensureReady();
        const results = await runQuery<AuthRecord>(model, where, undefined, 1);
        
        if (results.length > 0) {
          const result = results[0];

          /**
           * Join implementation: N+1 query pattern.
           *
           * For each requested join model, a separate query is executed against
           * that model's map with a foreign key filter. This means:
           * - 1 join model = 2 total queries (main + 1 join)
           * - 2 join models = 3 total queries (main + 2 joins)
           *
           * Performance characteristics:
           * - Each query runs against in-memory CRDT maps (sub-millisecond)
           * - Auth workloads typically join 1-2 models (account, session)
           * - Result sets are small (1-5 records per join)
           * - Network cost: zero (all reads are local)
           *
           * For workloads with >5 join models or >100 records per join,
           * consider a direct TopGun query instead of the BetterAuth adapter.
           */
          if (join) {
             for (const [joinModel, joinConfig] of Object.entries(join)) {
                 if (joinConfig === false) continue;

                 const foreignKey = adapterOptions.foreignKeyMap?.[joinModel] ?? 'userId';
                 const joinWhere: Where[] = [{ field: foreignKey, value: result.id }];
                 
                 const limit = typeof joinConfig === 'object' ? joinConfig.limit : undefined;
                 
                 const joinResults = await runQuery(joinModel, joinWhere, undefined, limit);
                 
                 // Attach to result using pluralized name (simple heuristic)
                 const pluralName = joinModel.endsWith('s') ? joinModel : joinModel + 's';
                 result[pluralName] = joinResults;
             }
          }

          // console.log(`[Adapter] findOne final result:`, result);
          
          // Ensure Dates are Date objects if they are strings (basic fix for JSON/serialization issues)
          const fixDates = (obj: Record<string, unknown>): Record<string, unknown> => {
              if (!obj) return obj;
              for (const key in obj) {
                  const value = obj[key];
                  if (typeof value === 'string' && /^\d{4}-\d{2}-\d{2}T/.test(value)) {
                      obj[key] = new Date(value);
                  } else if (typeof value === 'object' && value !== null) {
                      if (Array.isArray(value)) {
                          value.forEach((item: unknown) => {
                              if (typeof item === 'object' && item !== null) {
                                  fixDates(item as Record<string, unknown>);
                              }
                          });
                      }
                  }
              }
              return obj;
          };
          fixDates(result);

          if (select) {
             const selected: Partial<AuthRecord> = {};
             select.forEach(field => selected[field] = result[field]);
             // Ensure joined props are kept if they are not in select?
             // Usually select applies to the main model fields.
             // If join is requested, it implies we want those too.
             if (join) {
                 for (const joinModel of Object.keys(join)) {
                     const propName = joinModel.endsWith('s') ? joinModel : joinModel + 's';
                     if (result[propName]) {
                         selected[propName] = result[propName];
                     }
                 }
             }
             // Type assertion needed to match DBAdapter's generic return type
             return selected as unknown as Record<string, unknown>;
          }
          // Type assertion needed to match DBAdapter's generic return type
          return result as unknown as Record<string, unknown>;
        }
        return null;
      },

      async findMany({ model, where, limit, offset, sortBy }) {
         await ensureReady();
         const results = await runQuery<AuthRecord>(model, where, sortBy ? {[sortBy.field]: sortBy.direction} : undefined, limit, offset);
         // Type assertion needed to match DBAdapter's generic return type
         return results as unknown as Record<string, unknown>[];
      },

      async update({ model, where, update }) {
        await ensureReady();
        // We need to find the records first to update them
        const results = await runQuery<AuthRecord>(model, where);
        if (results.length === 0) return null;

        const mapName = getMapName(model);
        const map = client.getMap<string, AuthRecord>(mapName);

        // Update implies modifying existing.
        // If multiple matches, update only first? The interface says "Update may not return the updated data if multiple where clauses are provided".
        // Usually update finds one.
        // But if where is implicit AND, it finds specific set.
        // Standard behavior for 'update' (singular) is update ONE.

        const item = results[0];
        const updatedItem = { ...item, ...update };
        map.set(item.id, updatedItem);

        // Type assertion needed to match DBAdapter's generic return type
        return updatedItem as unknown as Record<string, unknown>;
      },

      async updateMany({ model, where, update }) {
        await ensureReady();
        const results = await runQuery<AuthRecord>(model, where);
        const mapName = getMapName(model);
        const map = client.getMap<string, AuthRecord>(mapName);

        for (const item of results) {
           map.set(item.id, { ...item, ...update });
        }
        return results.length;
      },

      async delete({ model, where }) {
         await ensureReady();
         const results = await runQuery<AuthRecord>(model, where);
         const mapName = getMapName(model);
         const map = client.getMap<string, AuthRecord>(mapName);

         if (results.length > 0) {
            map.remove(results[0].id);
         }
      },

      async deleteMany({ model, where }) {
         await ensureReady();
         const results = await runQuery<AuthRecord>(model, where);
         const mapName = getMapName(model);
         const map = client.getMap<string, AuthRecord>(mapName);

         for (const item of results) {
            map.remove(item.id);
         }
         return results.length;
      },
      
      async count({ model, where }) {
         await ensureReady();
         const results = await runQuery<AuthRecord>(model, where);
         return results.length;
      },

      /**
       * WARNING: Not atomic. TopGun uses CRDTs (conflict-free replicated data types)
       * which do not support traditional ACID transactions. This method executes
       * the callback sequentially — if an operation fails mid-callback, earlier
       * operations are NOT rolled back.
       *
       * Why this is acceptable for BetterAuth:
       * - BetterAuth uses transactions for user+account creation during signup
       * - CRDT LWW semantics ensure eventual consistency even on partial failure
       * - Worst case: orphaned account record (no user), which is harmless
       *
       * For deployments requiring strict atomicity, use a traditional SQL adapter.
       */
      async transaction(callback) {
         return callback(this as Omit<DBAdapter, 'transaction'>);
      },

      /**
       * Cursor-based pagination for TopGun callers who need efficient large-result iteration.
       *
       * Sort order is always id asc — this is not configurable. Cursor pagination requires
       * a stable, deterministic sort to guarantee non-overlapping pages. String IDs sorted
       * lexicographically with greaterThan(id, cursor) provide this guarantee.
       *
       * Callers who need a different sort order should use the standard findMany with offset.
       *
       * This method is NOT part of BetterAuth's DBAdapter interface. It is a TopGun extension
       * accessible to callers who use topGunAdapter directly and type the result as TopGunDBAdapter.
       */
      async findManyWithCursor({ model, where, limit, cursor }) {
        await ensureReady();
        // Pass empty string as cursor for first page — empty string sorts before all IDs lexicographically,
        // so greaterThan('id', '') returns all records. This ensures the cursor code path always enforces
        // id asc sort and limit enforcement regardless of whether a cursor was provided.
        const effectiveCursor = cursor !== undefined ? cursor : '';
        const results = await runQuery<AuthRecord>(model, where, undefined, limit, undefined, effectiveCursor);
        const data = results as unknown as Record<string, unknown>[];
        // nextCursor is null when: no results, fewer results than limit (last page),
        // or limit is undefined (pagination without a page size is meaningless)
        const nextCursor = limit === undefined
          ? null
          : results.length < limit
            ? null
            : String(results[results.length - 1].id);
        return { data, nextCursor };
      },
    } as TopGunDBAdapter;
  };
};
