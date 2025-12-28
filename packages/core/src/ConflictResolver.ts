import { z } from 'zod';
import { Timestamp, HLC } from './HLC';

// --- Merge Context ---

/**
 * Context provided to a conflict resolver during merge operations.
 */
export interface MergeContext<V = unknown> {
  /** Map name being modified */
  mapName: string;

  /** Entry key being modified */
  key: string;

  /** Current server/local value (undefined if key doesn't exist) */
  localValue: V | undefined;

  /** Incoming client/remote value */
  remoteValue: V;

  /** Local HLC timestamp (undefined if key doesn't exist) */
  localTimestamp?: Timestamp;

  /** Remote HLC timestamp */
  remoteTimestamp: Timestamp;

  /** Client/node ID that sent the update */
  remoteNodeId: string;

  /** Authentication context (optional) */
  auth?: {
    userId?: string;
    roles?: string[];
    metadata?: Record<string, unknown>;
  };

  /** Read other entries for cross-key validation */
  readEntry: (key: string) => V | undefined;
}

// --- Merge Result ---

/**
 * Result of conflict resolution.
 */
export type MergeResult<V = unknown> =
  | { action: 'accept'; value: V }    // Accept remote value
  | { action: 'reject'; reason: string } // Reject with error
  | { action: 'merge'; value: V }     // Custom merged value
  | { action: 'local' };              // Keep local value

// --- Conflict Resolver Function ---

/**
 * Conflict resolver function signature.
 */
export type ConflictResolverFn<V = unknown> = (
  context: MergeContext<V>,
) => MergeResult<V> | Promise<MergeResult<V>>;

// --- Conflict Resolver Definition ---

/**
 * Conflict resolver definition (can be native function or sandboxed code).
 */
export interface ConflictResolverDef<V = unknown> {
  /** Unique resolver name */
  name: string;

  /** JavaScript function body as string (for sandboxed execution) */
  code?: string;

  /** Native function (for trusted server-side resolvers) */
  fn?: ConflictResolverFn<V>;

  /** Priority (higher = runs first, default 50) */
  priority?: number;

  /** Apply only to specific keys (glob pattern) */
  keyPattern?: string;
}

// --- Zod Schema for Wire Format ---

/**
 * Zod schema for validating conflict resolver definitions from network.
 */
export const ConflictResolverDefSchema = z.object({
  name: z.string().min(1).max(100),
  code: z.string().max(50000).optional(),
  priority: z.number().int().min(0).max(100).default(50),
  keyPattern: z.string().optional(),
});

// --- Security: Forbidden Patterns ---

/**
 * Patterns that are denied in resolver code for security reasons.
 * Same patterns as EntryProcessor for consistency.
 */
export const RESOLVER_FORBIDDEN_PATTERNS = [
  /\beval\b/,
  /\bFunction\b/,
  /\bprocess\b/,
  /\bglobal\b/,
  /\brequire\b/,
  /\bimport\b/,
  /\bfetch\b/,
  /\bXMLHttpRequest\b/,
  /\bsetTimeout\b/,
  /\bsetInterval\b/,
  /\bsetImmediate\b/,
];

/**
 * Validates resolver code against forbidden patterns.
 */
export function validateResolverCode(code: string): {
  valid: boolean;
  error?: string;
} {
  for (const pattern of RESOLVER_FORBIDDEN_PATTERNS) {
    if (pattern.test(code)) {
      return {
        valid: false,
        error: `Forbidden pattern detected: ${pattern.source}`,
      };
    }
  }
  return { valid: true };
}

// --- Rate Limiting Configuration ---

/**
 * Rate limiting configuration for resolver registration.
 */
export interface ResolverRateLimitConfig {
  /** Max resolver registrations per client */
  maxResolversPerClient: number;

  /** Max resolver code size in bytes */
  maxCodeSizeBytes: number;
}

/**
 * Default rate limit configuration.
 */
export const DEFAULT_RESOLVER_RATE_LIMITS: ResolverRateLimitConfig = {
  maxResolversPerClient: 50,
  maxCodeSizeBytes: 50000, // 50KB
};

// --- Helper Functions ---

/**
 * Compares two HLC timestamps.
 * Returns negative if a < b, positive if a > b, 0 if equal.
 */
export function compareHLCTimestamps(a: Timestamp, b: Timestamp): number {
  return HLC.compare(a, b);
}

/**
 * Deep merges two objects.
 * Remote values take precedence at each level.
 */
export function deepMerge<T extends object>(target: T, source: T): T {
  const result = { ...target };

  for (const key of Object.keys(source) as (keyof T)[]) {
    const sourceVal = source[key];
    const targetVal = target[key];

    if (
      typeof sourceVal === 'object' &&
      sourceVal !== null &&
      typeof targetVal === 'object' &&
      targetVal !== null &&
      !Array.isArray(sourceVal)
    ) {
      (result as Record<string, unknown>)[key as string] = deepMerge(
        targetVal as object,
        sourceVal as object,
      );
    } else {
      (result as Record<string, unknown>)[key as string] = sourceVal;
    }
  }

  return result;
}

// --- Built-in Resolvers ---

/**
 * Built-in conflict resolvers for common patterns.
 * These are type-safe and pre-validated.
 */
export const BuiltInResolvers = {
  /**
   * Standard Last-Write-Wins - accept if remote timestamp is newer.
   */
  LWW: <V>(): ConflictResolverDef<V> => ({
    name: 'builtin:lww',
    fn: (ctx) => {
      if (!ctx.localTimestamp) {
        return { action: 'accept', value: ctx.remoteValue };
      }

      const cmp = compareHLCTimestamps(ctx.remoteTimestamp, ctx.localTimestamp);
      if (cmp > 0) {
        return { action: 'accept', value: ctx.remoteValue };
      }
      return { action: 'local' };
    },
    priority: 0, // Lowest priority - fallback
  }),

  /**
   * First-Write-Wins - reject if local value exists.
   * Useful for booking systems, unique constraints.
   */
  FIRST_WRITE_WINS: <V>(): ConflictResolverDef<V> => ({
    name: 'builtin:first_write_wins',
    fn: (ctx) => {
      if (ctx.localValue !== undefined) {
        return { action: 'reject', reason: 'Entry already exists' };
      }
      return { action: 'accept', value: ctx.remoteValue };
    },
    priority: 100,
  }),

  /**
   * Numeric minimum - keep lowest value.
   * Useful for auction systems (lowest bid wins).
   */
  NUMERIC_MIN: (): ConflictResolverDef<number> => ({
    name: 'builtin:numeric_min',
    fn: (ctx) => {
      const local = ctx.localValue ?? Infinity;
      const remote = ctx.remoteValue;
      return { action: 'merge', value: Math.min(local, remote) };
    },
    priority: 50,
  }),

  /**
   * Numeric maximum - keep highest value.
   * Useful for high score tracking.
   */
  NUMERIC_MAX: (): ConflictResolverDef<number> => ({
    name: 'builtin:numeric_max',
    fn: (ctx) => {
      const local = ctx.localValue ?? -Infinity;
      const remote = ctx.remoteValue;
      return { action: 'merge', value: Math.max(local, remote) };
    },
    priority: 50,
  }),

  /**
   * Non-negative - reject if value would be negative.
   * Useful for inventory systems.
   */
  NON_NEGATIVE: (): ConflictResolverDef<number> => ({
    name: 'builtin:non_negative',
    fn: (ctx) => {
      if (typeof ctx.remoteValue !== 'number' || ctx.remoteValue < 0) {
        return { action: 'reject', reason: 'Value cannot be negative' };
      }
      return { action: 'accept', value: ctx.remoteValue };
    },
    priority: 90,
  }),

  /**
   * Array union - merge arrays by taking union of elements.
   * Useful for tags, categories.
   */
  ARRAY_UNION: <T>(): ConflictResolverDef<T[]> => ({
    name: 'builtin:array_union',
    fn: (ctx) => {
      const local = ctx.localValue ?? [];
      const remote = ctx.remoteValue ?? [];
      const merged = [...new Set([...local, ...remote])];
      return { action: 'merge', value: merged };
    },
    priority: 50,
  }),

  /**
   * Deep merge - recursively merge objects.
   * Remote values take precedence at leaf level.
   */
  DEEP_MERGE: <V extends object>(): ConflictResolverDef<V> => ({
    name: 'builtin:deep_merge',
    fn: (ctx) => {
      const local = (ctx.localValue ?? {}) as V;
      const remote = ctx.remoteValue;
      const merged = deepMerge(local, remote);
      return { action: 'merge', value: merged };
    },
    priority: 50,
  }),

  /**
   * Server-only - reject all client writes.
   * Useful for server-controlled state.
   */
  SERVER_ONLY: <V>(): ConflictResolverDef<V> => ({
    name: 'builtin:server_only',
    fn: (ctx) => {
      // Server writes have a special node ID or auth role
      if (ctx.auth?.roles?.includes('server') || ctx.remoteNodeId.startsWith('server:')) {
        return { action: 'accept', value: ctx.remoteValue };
      }
      return { action: 'reject', reason: 'Only server can write to this entry' };
    },
    priority: 100,
  }),

  /**
   * Owner-only - only the original creator can modify.
   * Requires value to have an `ownerId` property.
   */
  OWNER_ONLY: <V extends { ownerId?: string }>(): ConflictResolverDef<V> => ({
    name: 'builtin:owner_only',
    fn: (ctx) => {
      // If no local value, accept (first write sets owner)
      if (!ctx.localValue) {
        return { action: 'accept', value: ctx.remoteValue };
      }

      // Check if remote is from owner
      const ownerId = ctx.localValue.ownerId;
      if (ownerId && ctx.auth?.userId !== ownerId) {
        return { action: 'reject', reason: 'Only owner can modify this entry' };
      }

      return { action: 'accept', value: ctx.remoteValue };
    },
    priority: 95,
  }),

  /**
   * Immutable - reject any modifications after initial write.
   */
  IMMUTABLE: <V>(): ConflictResolverDef<V> => ({
    name: 'builtin:immutable',
    fn: (ctx) => {
      if (ctx.localValue !== undefined) {
        return { action: 'reject', reason: 'Entry is immutable' };
      }
      return { action: 'accept', value: ctx.remoteValue };
    },
    priority: 100,
  }),

  /**
   * Version check - only accept if version increments by 1.
   * Useful for optimistic locking.
   */
  VERSION_INCREMENT: <V extends { version?: number }>(): ConflictResolverDef<V> => ({
    name: 'builtin:version_increment',
    fn: (ctx) => {
      const localVersion = ctx.localValue?.version ?? 0;
      const remoteVersion = ctx.remoteValue?.version ?? 0;

      if (remoteVersion !== localVersion + 1) {
        return {
          action: 'reject',
          reason: `Version conflict: expected ${localVersion + 1}, got ${remoteVersion}`,
        };
      }
      return { action: 'accept', value: ctx.remoteValue };
    },
    priority: 90,
  }),
};

// --- Merge Rejection Event ---

/**
 * Event emitted when a merge is rejected.
 */
export interface MergeRejection {
  mapName: string;
  key: string;
  attemptedValue: unknown;
  reason: string;
  timestamp: Timestamp;
  nodeId: string;
}
