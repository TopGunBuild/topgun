/**
 * SerializationWorker - High-level API for serialization operations in worker threads
 * Phase 1.07: SerializationWorker Implementation
 *
 * Provides a clean interface for CPU-intensive serialization/deserialization.
 * Delegates actual work to worker threads via WorkerPool for large payloads.
 *
 * Uses base64 encoding to transfer binary data through postMessage.
 * This adds ~33% overhead but is necessary since Uint8Array cannot be
 * directly transferred through structured clone algorithm for our use case.
 */

import { join } from 'path';
import { serialize as coreSerialize, deserialize as coreDeserialize } from '@topgunbuild/core';
import { WorkerPool } from './WorkerPool';
import type { WorkerTask, WorkerTaskType } from './types';
import type {
  SerializeBatchPayload,
  SerializeBatchResult,
  DeserializeBatchPayload,
  DeserializeBatchResult,
} from './serialization-types';

// Threshold: use worker only if batch exceeds this count OR total size is large
const WORKER_BATCH_THRESHOLD = 10;
const WORKER_SIZE_THRESHOLD = 50 * 1024; // 50 KB estimated payload size

let taskIdCounter = 0;

function generateTaskId(): string {
  return `ser-${Date.now()}-${++taskIdCounter}`;
}

/**
 * Convert Uint8Array to base64 string
 */
function uint8ArrayToBase64(bytes: Uint8Array): string {
  let binary = '';
  for (let i = 0; i < bytes.length; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary);
}

/**
 * Convert base64 string back to Uint8Array
 */
function base64ToUint8Array(base64: string): Uint8Array {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

/**
 * Estimate serialized size of an object (rough heuristic)
 * MessagePack typically produces smaller output than JSON.stringify
 */
function estimateSize(obj: unknown): number {
  if (obj === null || obj === undefined) return 1;
  if (typeof obj === 'boolean') return 1;
  if (typeof obj === 'number') return 9; // worst case: double
  if (typeof obj === 'string') return (obj as string).length + 5;
  if (Array.isArray(obj)) {
    let size = 5;
    for (const item of obj) {
      size += estimateSize(item);
    }
    return size;
  }
  if (typeof obj === 'object') {
    let size = 5;
    for (const [key, value] of Object.entries(obj as Record<string, unknown>)) {
      size += key.length + 5 + estimateSize(value);
    }
    return size;
  }
  return 10;
}

/**
 * SerializationWorker provides methods for serialization operations.
 * Automatically decides whether to use worker threads based on payload size.
 */
export class SerializationWorker {
  private readonly pool: WorkerPool;
  private readonly workerScript: string;

  /** Threshold for using worker (items below this go to main thread) */
  static readonly BATCH_THRESHOLD = WORKER_BATCH_THRESHOLD;
  /** Size threshold for using worker (bytes) */
  static readonly SIZE_THRESHOLD = WORKER_SIZE_THRESHOLD;

  constructor(pool: WorkerPool) {
    this.pool = pool;
    this.workerScript = this.resolveWorkerScript();
  }

  private resolveWorkerScript(): string {
    // When running via ts-jest, __dirname is src/workers
    // When running compiled, __dirname is dist/workers
    const directJsPath = join(__dirname, 'worker-scripts', 'serialization.worker.js');
    const distJsPath = join(__dirname, '..', '..', 'dist', 'workers', 'worker-scripts', 'serialization.worker.js');
    const tsPath = join(__dirname, 'worker-scripts', 'serialization.worker.ts');

    try {
      require.resolve(directJsPath);
      return directJsPath;
    } catch {
      // Direct .js not found, try dist path
    }

    try {
      require.resolve(distJsPath);
      return distJsPath;
    } catch {
      return tsPath;
    }
  }

  /**
   * Decide if batch should go to worker based on count or size
   */
  shouldUseWorker(items: unknown[]): boolean {
    if (items.length >= WORKER_BATCH_THRESHOLD) {
      return true;
    }

    // Estimate total size
    let totalSize = 0;
    for (const item of items) {
      totalSize += estimateSize(item);
      if (totalSize >= WORKER_SIZE_THRESHOLD) {
        return true;
      }
    }

    return false;
  }

  /**
   * Serialize multiple objects to MessagePack binary format.
   * Uses worker thread for large batches.
   *
   * @param items - Objects to serialize
   * @returns Array of Uint8Array containing serialized data
   */
  async serializeBatch(items: unknown[]): Promise<Uint8Array[]> {
    if (items.length === 0) {
      return [];
    }

    if (!this.shouldUseWorker(items)) {
      return this.serializeBatchInline(items);
    }

    const payload: SerializeBatchPayload = { items };
    const task: WorkerTask<SerializeBatchPayload, SerializeBatchResult> = {
      id: generateTaskId(),
      type: 'serialize' as WorkerTaskType,
      payload,
      priority: 'normal',
    };

    const result = await this.pool.submit(task);

    // Convert base64 strings back to Uint8Array
    return result.serialized.map(base64ToUint8Array);
  }

  /**
   * Deserialize multiple MessagePack binary payloads to objects.
   * Uses worker thread for large batches.
   *
   * @param items - Binary data to deserialize
   * @returns Array of deserialized objects
   */
  async deserializeBatch<T = unknown>(items: Uint8Array[]): Promise<T[]> {
    if (items.length === 0) {
      return [];
    }

    if (items.length < WORKER_BATCH_THRESHOLD) {
      return this.deserializeBatchInline(items);
    }

    // Convert to base64 for worker transfer
    const base64Items = items.map(uint8ArrayToBase64);

    const payload: DeserializeBatchPayload = { items: base64Items };
    const task: WorkerTask<DeserializeBatchPayload, DeserializeBatchResult> = {
      id: generateTaskId(),
      type: 'deserialize' as WorkerTaskType,
      payload,
      priority: 'normal',
    };

    const result = await this.pool.submit(task);
    return result.deserialized as T[];
  }

  /**
   * Serialize a single object (always inline, too small for worker)
   */
  serialize(data: unknown): Uint8Array {
    return coreSerialize(data);
  }

  /**
   * Deserialize a single payload (always inline, too small for worker)
   */
  deserialize<T = unknown>(data: Uint8Array | ArrayBuffer): T {
    return coreDeserialize(data) as T;
  }

  // ============ Inline implementations for small batches ============

  private serializeBatchInline(items: unknown[]): Uint8Array[] {
    const results: Uint8Array[] = [];
    for (const item of items) {
      results.push(coreSerialize(item));
    }
    return results;
  }

  private deserializeBatchInline<T>(items: Uint8Array[]): T[] {
    const results: T[] = [];
    for (const item of items) {
      results.push(coreDeserialize(item) as T);
    }
    return results;
  }
}
