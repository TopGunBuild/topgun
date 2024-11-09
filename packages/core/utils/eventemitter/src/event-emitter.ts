export type EventHandler<T = any> = (data: T) => void;

export class EventEmitter<EventMap extends Record<string, any> = Record<string, any>> {
  private handlers: Map<keyof EventMap, Set<EventHandler>> = new Map();

  /**
   * Subscribe to an event
   * @param event The event name to subscribe to
   * @param handler The callback function to be called when the event is emitted
   * @returns An unsubscribe function
   */
  public on<K extends keyof EventMap>(event: K, handler: EventHandler<EventMap[K]>): () => void {
    if (!this.handlers.has(event)) {
      this.handlers.set(event, new Set());
    }

    this.handlers.get(event)!.add(handler);

    // Return unsubscribe function
    return () => {
      this.off(event, handler);
    };
  }

  /**
   * Subscribe to an event and unsubscribe after the first emission
   * @param event The event name to subscribe to
   * @param handler The callback function to be called when the event is emitted
   * @returns An unsubscribe function
   */
  public once<K extends keyof EventMap>(event: K, handler: EventHandler<EventMap[K]>): () => void {
    const wrappedHandler: EventHandler<EventMap[K]> = (data) => {
      this.off(event, wrappedHandler);
      handler(data);
    };

    return this.on(event, wrappedHandler);
  }

  /**
   * Wait for the first emission of an event and return its data
   * @param event The event name to wait for
   * @returns Promise that resolves with the event data
   */
  public waitFor<K extends keyof EventMap>(event: K): Promise<EventMap[K]> {
    return new Promise((resolve) => {
      this.once(event, resolve);
    });
  }

  /**
   * Unsubscribe from an event
   * @param event The event name to unsubscribe from
   * @param handler The handler to remove
   */
  public off<K extends keyof EventMap>(event: K, handler: EventHandler<EventMap[K]>): void {
    const handlers = this.handlers.get(event);
    if (handlers) {
      handlers.delete(handler);
      if (handlers.size === 0) {
        this.handlers.delete(event);
      }
    }
  }

  /**
   * Remove all event handlers
   */
  public removeAllListeners(): void {
    this.handlers.clear();
  }

  /**
   * Emit an event with data
   * @param event The event name to emit
   * @param data The data to pass to handlers
   */
  public emit<K extends keyof EventMap>(event: K, data: EventMap[K]): void {
    const handlers = this.handlers.get(event);
    if (handlers) {
      handlers.forEach((handler) => {
        try {
          handler(data);
        } catch (error) {
          console.error(`Error in event handler for ${String(event)}:`, error);
        }
      });
    }
  }

  /**
   * Get the number of listeners for an event
   * @param event The event name
   * @returns The number of listeners
   */
  public listenerCount(event: keyof EventMap): number {
    return this.handlers.get(event)?.size ?? 0;
  }
} 