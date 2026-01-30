/**
 * MessageRouter - Routes incoming server messages to appropriate handlers.
 *
 * This module replaces the large switch statement in SyncEngine.handleServerMessage()
 * with a declarative, type-based routing system.
 *
 * Features:
 * - Register handlers for message types
 * - Route incoming messages to appropriate handlers
 * - Support async handlers
 * - Handle unregistered message types via fallback
 */

import { logger } from '../utils/logger';
import type { IMessageRouter, MessageHandler, MessageRouterConfig } from './types';

/**
 * MessageRouter implementation.
 * Routes server messages to registered handlers based on message type.
 */
export class MessageRouter implements IMessageRouter {
  private readonly handlers: Map<string, MessageHandler>;
  private readonly onUnhandled?: (message: any) => void;

  constructor(config: MessageRouterConfig = {}) {
    // Copy handlers from config or create new Map
    this.handlers = config.handlers
      ? new Map(config.handlers)
      : new Map();
    this.onUnhandled = config.onUnhandled;
  }

  /**
   * Register a handler for a message type.
   * @param type - Message type to handle
   * @param handler - Handler function
   */
  registerHandler(type: string, handler: MessageHandler): void {
    if (this.handlers.has(type)) {
      logger.warn({ type }, 'Overwriting existing handler for message type');
    }
    this.handlers.set(type, handler);
  }

  /**
   * Register multiple handlers at once.
   * @param handlers - Record of type -> handler
   */
  registerHandlers(handlers: Record<string, MessageHandler>): void {
    for (const [type, handler] of Object.entries(handlers)) {
      this.registerHandler(type, handler);
    }
  }

  /**
   * Route a message to its registered handler.
   * Returns true if handled, false if no handler found.
   * @param message - Message to route
   * @returns Promise resolving to true if handled
   */
  async route(message: any): Promise<boolean> {
    const type = message?.type;
    if (!type) {
      logger.warn({ message }, 'Cannot route message without type');
      return false;
    }

    const handler = this.handlers.get(type);
    if (!handler) {
      // Call fallback if provided
      if (this.onUnhandled) {
        this.onUnhandled(message);
      }
      return false;
    }

    try {
      // Await in case handler is async
      await handler(message);
      return true;
    } catch (error) {
      logger.error({ type, error }, 'Error in message handler');
      // Still return true since handler was found (just errored)
      return true;
    }
  }

  /**
   * Check if a handler is registered for a message type.
   * @param type - Message type to check
   * @returns true if handler exists
   */
  hasHandler(type: string): boolean {
    return this.handlers.has(type);
  }

  /**
   * Get the count of registered handlers.
   * Useful for debugging/testing.
   */
  get handlerCount(): number {
    return this.handlers.size;
  }

  /**
   * Get all registered message types.
   * Useful for debugging/testing.
   */
  getRegisteredTypes(): string[] {
    return Array.from(this.handlers.keys());
  }
}
