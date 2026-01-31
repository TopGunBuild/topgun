import { TopGunClient, Predicates } from '@topgunbuild/client';
import type { BetterAuthOptions } from 'better-auth';
import type {
  DBAdapter,
  Where,
  DBAdapterInstance
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
}

export const topGunAdapter = (adapterOptions: TopGunAdapterOptions): DBAdapterInstance => {
  return (options: BetterAuthOptions): DBAdapter => {
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
     * Note: BetterAuth uses offset-based pagination, but TopGun uses cursor-based pagination.
     * For BetterAuth compatibility, we fetch limit+offset results and slice client-side.
     * This is acceptable for auth queries which typically have small result sets.
     */
    const runQuery = async <T extends AuthRecord>(model: string, where?: Where[], sort?: SortSpec, limit?: number, offset?: number): Promise<T[]> => {
      const mapName = getMapName(model);
      const predicate = where ? whereToPredicate(where) : undefined;

      // For BetterAuth offset compatibility, we request more results and slice
      const effectiveLimit = limit && offset ? limit + offset : limit;

      const filter = {
        predicate,
        sort,
        limit: effectiveLimit,
        // Note: TopGun uses cursor-based pagination (Phase 14.1)
        // offset is handled client-side for BetterAuth compatibility
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

        return record;
      },

      async findOne({ model, where, select, join }) {
        await ensureReady();
        const results = await runQuery<AuthRecord>(model, where, undefined, 1);
        
        if (results.length > 0) {
          const result = results[0];

          // Handle Join
          if (join) {
             for (const [joinModel, joinConfig] of Object.entries(join)) {
                 if (joinConfig === false) continue;
                 
                 // Assume standard relation on userId
                 // TODO: Handle custom foreign keys if Better Auth passes them or we infer them
                 const joinWhere: Where[] = [{ field: 'userId', value: result.id }];
                 
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
             return selected;
          }
          return result;
        }
        return null;
      },

      async findMany({ model, where, limit, offset, sortBy }) {
         await ensureReady();
         const results = await runQuery<AuthRecord>(model, where, sortBy ? {[sortBy.field]: sortBy.direction} : undefined, limit, offset);
         return results;
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

        return updatedItem;
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

      async transaction(callback) {
         // TopGun doesn't support atomic multi-map transactions yet.
         // We execute sequentially as per BetterAuth fallback.
         // But DBTransactionAdapter is Omit<DBAdapter, "transaction">.
         // We just pass 'this' as the transaction adapter (cast it).
         return callback(this as Omit<DBAdapter, 'transaction'>);
      }
    };
  };
};
