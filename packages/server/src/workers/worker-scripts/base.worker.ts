/**
 * Base Worker Script
 * Phase 1.02: Worker Threads Implementation
 *
 * Main worker entry point that handles all task types.
 * Imports specialized workers to register their handlers.
 */

import { parentPort } from 'worker_threads';

interface TaskMessage {
  id: string;
  type: string;
  payload: unknown;
}

interface TaskResponse {
  id: string;
  success: boolean;
  result?: unknown;
  error?: string;
}

type TaskHandler = (payload: unknown) => unknown | Promise<unknown>;

// Handler registry
const handlers = new Map<string, TaskHandler>();

/**
 * Register a handler for a specific task type
 */
export function registerHandler(type: string, handler: TaskHandler): void {
  handlers.set(type, handler);
}

/**
 * Unregister a handler
 */
export function unregisterHandler(type: string): void {
  handlers.delete(type);
}

/**
 * Check if a handler is registered
 */
export function hasHandler(type: string): boolean {
  return handlers.has(type);
}

// Message loop
if (parentPort) {
  parentPort.on('message', async (task: TaskMessage) => {
    const { id, type, payload } = task;

    const response: TaskResponse = {
      id,
      success: false,
    };

    try {
      const handler = handlers.get(type);

      if (!handler) {
        throw new Error(`Unknown task type: ${type}`);
      }

      const result = await handler(payload);
      response.success = true;
      response.result = result;
    } catch (error) {
      response.success = false;
      response.error =
        error instanceof Error ? error.message : String(error);
    }

    parentPort!.postMessage(response);
  });

  // Signal ready (optional, for debugging)
  // parentPort.postMessage({ type: 'ready' });
}

// Export for testing
export { handlers };

// Load specialized workers to register their handlers
// Using require() to ensure side effects are executed (not tree-shaken)
// Each specialized worker calls registerHandler() when loaded
/* eslint-disable @typescript-eslint/no-require-imports */
require('./crdt.worker');
require('./merkle.worker');
require('./serialization.worker');
