import { z } from 'zod';

// --- Entry Processor Types ---

/**
 * Function executed on the server against a single entry.
 * Receives the current value (or undefined if key doesn't exist).
 * Returns the new value (or undefined to delete the entry).
 */
export type EntryProcessorFn<V, R = V> = (
  value: V | undefined,
  key: string,
  args?: unknown,
) => { value: V | undefined; result?: R };

/**
 * Zod schema for entry processor definition validation.
 */
export const EntryProcessorDefSchema = z.object({
  name: z.string().min(1).max(100),
  code: z.string().min(1).max(10000), // Max 10KB code
  args: z.unknown().optional(),
});

/**
 * Serializable entry processor definition.
 * Code is sent as string and executed in isolated sandbox.
 */
export interface EntryProcessorDef<V = unknown, R = V> {
  /** Unique processor name for caching compiled code */
  name: string;

  /** JavaScript function body as string */
  code: string;

  /** Optional arguments passed to the processor */
  args?: unknown;

  /** Type markers (not serialized) */
  __valueType?: V;
  __resultType?: R;
}

/**
 * Result of entry processor execution.
 */
export interface EntryProcessorResult<R = unknown> {
  /** Whether the operation succeeded */
  success: boolean;

  /** Custom result returned by processor */
  result?: R;

  /** Error message if failed */
  error?: string;

  /** New value after processing (for client cache update) */
  newValue?: unknown;
}

// --- Security: Forbidden Patterns ---

/**
 * Patterns that are denied in processor code for security reasons.
 */
export const FORBIDDEN_PATTERNS = [
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
 * Validates processor code against forbidden patterns.
 * Returns true if code is safe, false otherwise.
 */
export function validateProcessorCode(code: string): {
  valid: boolean;
  error?: string;
} {
  for (const pattern of FORBIDDEN_PATTERNS) {
    if (pattern.test(code)) {
      return {
        valid: false,
        error: `Forbidden pattern detected: ${pattern.source}`,
      };
    }
  }
  return { valid: true };
}

// --- Built-in Processors ---

/**
 * Built-in processors for common operations.
 * These are type-safe and pre-validated.
 */
export const BuiltInProcessors = {
  /**
   * Increment numeric value by delta.
   * If value doesn't exist, starts from 0.
   */
  INCREMENT: (delta: number = 1): EntryProcessorDef<number, number> => ({
    name: 'builtin:increment',
    code: `
      const current = value ?? 0;
      const newValue = current + args;
      return { value: newValue, result: newValue };
    `,
    args: delta,
  }),

  /**
   * Decrement numeric value by delta.
   * If value doesn't exist, starts from 0.
   */
  DECREMENT: (delta: number = 1): EntryProcessorDef<number, number> => ({
    name: 'builtin:decrement',
    code: `
      const current = value ?? 0;
      const newValue = current - args;
      return { value: newValue, result: newValue };
    `,
    args: delta,
  }),

  /**
   * Decrement with floor (won't go below 0).
   * Returns both the new value and whether it was floored.
   */
  DECREMENT_FLOOR: (
    delta: number = 1,
  ): EntryProcessorDef<number, { newValue: number; wasFloored: boolean }> => ({
    name: 'builtin:decrement_floor',
    code: `
      const current = value ?? 0;
      const target = current - args;
      const newValue = Math.max(0, target);
      return {
        value: newValue,
        result: { newValue, wasFloored: target < 0 }
      };
    `,
    args: delta,
  }),

  /**
   * Multiply numeric value by factor.
   * If value doesn't exist, starts from 1.
   */
  MULTIPLY: (factor: number): EntryProcessorDef<number, number> => ({
    name: 'builtin:multiply',
    code: `
      const current = value ?? 1;
      const newValue = current * args;
      return { value: newValue, result: newValue };
    `,
    args: factor,
  }),

  /**
   * Set value only if key doesn't exist.
   * Returns true if value was set, false if key already existed.
   */
  PUT_IF_ABSENT: <V>(newValue: V): EntryProcessorDef<V, boolean> => ({
    name: 'builtin:put_if_absent',
    code: `
      if (value !== undefined) {
        return { value, result: false };
      }
      return { value: args, result: true };
    `,
    args: newValue,
  }),

  /**
   * Replace value only if key exists.
   * Returns the old value if replaced, undefined otherwise.
   */
  REPLACE: <V>(newValue: V): EntryProcessorDef<V, V | undefined> => ({
    name: 'builtin:replace',
    code: `
      if (value === undefined) {
        return { value: undefined, result: undefined };
      }
      return { value: args, result: value };
    `,
    args: newValue,
  }),

  /**
   * Replace value only if it matches expected value.
   * Returns true if replaced, false otherwise.
   */
  REPLACE_IF_EQUALS: <V>(
    expectedValue: V,
    newValue: V,
  ): EntryProcessorDef<V, boolean> => ({
    name: 'builtin:replace_if_equals',
    code: `
      if (JSON.stringify(value) === JSON.stringify(args.expected)) {
        return { value: args.newValue, result: true };
      }
      return { value, result: false };
    `,
    args: { expected: expectedValue, newValue },
  }),

  /**
   * Delete entry only if value matches.
   * Returns true if deleted, false otherwise.
   */
  DELETE_IF_EQUALS: <V>(expectedValue: V): EntryProcessorDef<V, boolean> => ({
    name: 'builtin:delete_if_equals',
    code: `
      if (JSON.stringify(value) === JSON.stringify(args)) {
        return { value: undefined, result: true };
      }
      return { value, result: false };
    `,
    args: expectedValue,
  }),

  /**
   * Append item to array.
   * Creates array if it doesn't exist.
   * Returns new array length.
   */
  ARRAY_PUSH: <T>(item: T): EntryProcessorDef<T[], number> => ({
    name: 'builtin:array_push',
    code: `
      const arr = value ?? [];
      arr.push(args);
      return { value: arr, result: arr.length };
    `,
    args: item,
  }),

  /**
   * Remove last item from array.
   * Returns the removed item or undefined.
   */
  ARRAY_POP: <T>(): EntryProcessorDef<T[], T | undefined> => ({
    name: 'builtin:array_pop',
    code: `
      if (!value || value.length === 0) {
        return { value: value ?? [], result: undefined };
      }
      const removed = value.pop();
      return { value, result: removed };
    `,
  }),

  /**
   * Remove item from array by value (first occurrence).
   * Returns true if item was found and removed.
   */
  ARRAY_REMOVE: <T>(item: T): EntryProcessorDef<T[], boolean> => ({
    name: 'builtin:array_remove',
    code: `
      if (!value) {
        return { value: [], result: false };
      }
      const idx = value.findIndex(v => JSON.stringify(v) === JSON.stringify(args));
      if (idx === -1) {
        return { value, result: false };
      }
      value.splice(idx, 1);
      return { value, result: true };
    `,
    args: item,
  }),

  /**
   * Update nested property using dot notation path.
   * Creates intermediate objects if they don't exist.
   */
  SET_PROPERTY: <V>(
    path: string,
    propValue: unknown,
  ): EntryProcessorDef<V, V> => ({
    name: 'builtin:set_property',
    code: `
      const obj = value ?? {};
      const parts = args.path.split('.');
      let current = obj;
      for (let i = 0; i < parts.length - 1; i++) {
        current[parts[i]] = current[parts[i]] ?? {};
        current = current[parts[i]];
      }
      current[parts[parts.length - 1]] = args.value;
      return { value: obj, result: obj };
    `,
    args: { path, value: propValue },
  }),

  /**
   * Delete nested property using dot notation path.
   * Returns the deleted value or undefined.
   */
  DELETE_PROPERTY: <V>(path: string): EntryProcessorDef<V, unknown> => ({
    name: 'builtin:delete_property',
    code: `
      if (!value) {
        return { value, result: undefined };
      }
      const parts = args.split('.');
      let current = value;
      for (let i = 0; i < parts.length - 1; i++) {
        if (!current[parts[i]]) {
          return { value, result: undefined };
        }
        current = current[parts[i]];
      }
      const lastKey = parts[parts.length - 1];
      const deleted = current[lastKey];
      delete current[lastKey];
      return { value, result: deleted };
    `,
    args: path,
  }),

  /**
   * Get current value without modifying it.
   * Useful for conditional reads.
   */
  GET: <V>(): EntryProcessorDef<V, V | undefined> => ({
    name: 'builtin:get',
    code: `
      return { value, result: value };
    `,
  }),

  /**
   * Conditional update based on version/timestamp.
   * Only updates if current version matches expected.
   * Useful for optimistic locking.
   */
  CONDITIONAL_UPDATE: <V extends { version?: number }>(
    expectedVersion: number,
    newData: Partial<V>,
  ): EntryProcessorDef<V, { updated: boolean; conflict: boolean }> => ({
    name: 'builtin:conditional_update',
    code: `
      if (!value || value.version !== args.expectedVersion) {
        return {
          value,
          result: { updated: false, conflict: true }
        };
      }
      const updated = {
        ...value,
        ...args.newData,
        version: (value.version ?? 0) + 1,
      };
      return {
        value: updated,
        result: { updated: true, conflict: false }
      };
    `,
    args: { expectedVersion, newData },
  }),

  /**
   * Merge object properties into existing value.
   * Shallow merge only.
   */
  MERGE: <V extends Record<string, unknown>>(
    properties: Partial<V>,
  ): EntryProcessorDef<V, V> => ({
    name: 'builtin:merge',
    code: `
      const merged = { ...(value ?? {}), ...args };
      return { value: merged, result: merged };
    `,
    args: properties,
  }),
};

// --- Rate Limiting Configuration ---

/**
 * Rate limiting configuration for processor execution.
 */
export interface ProcessorRateLimitConfig {
  /** Max processor executions per second per client */
  maxExecutionsPerSecond: number;

  /** Max processor code size in bytes */
  maxCodeSizeBytes: number;

  /** Max args size in bytes (JSON stringified) */
  maxArgsSizeBytes: number;
}

/**
 * Default rate limit configuration.
 */
export const DEFAULT_PROCESSOR_RATE_LIMITS: ProcessorRateLimitConfig = {
  maxExecutionsPerSecond: 100,
  maxCodeSizeBytes: 10240, // 10KB
  maxArgsSizeBytes: 1048576, // 1MB
};
