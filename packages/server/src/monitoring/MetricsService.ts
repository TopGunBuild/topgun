import { Registry, Gauge, Counter, Summary, collectDefaultMetrics } from 'prom-client';

export class MetricsService {
  public readonly registry: Registry;

  // Metrics
  private connectedClients: Gauge;
  private mapSizeItems: Gauge;
  private opsTotal: Counter;
  private memoryUsage: Gauge;
  private clusterMembers: Gauge;

  // Subscription-based routing metrics
  private eventsRoutedTotal: Counter;
  private eventsFilteredBySubscription: Counter;
  private subscribersPerEvent: Summary;

  // Bounded event queue metrics
  private eventQueueSize: Gauge;
  private eventQueueEnqueued: Counter;
  private eventQueueDequeued: Counter;
  private eventQueueRejected: Counter;

  // Backpressure metrics
  private backpressureSyncForcedTotal: Counter;
  private backpressurePendingOps: Gauge;
  private backpressureWaitsTotal: Counter;
  private backpressureTimeoutsTotal: Counter;

  // Connection scaling metrics
  private connectionsAcceptedTotal: Counter;
  private connectionsRejectedTotal: Counter;
  private connectionsPending: Gauge;
  private connectionRatePerSecond: Gauge;

  constructor() {
    this.registry = new Registry();

    // Enable default nodejs metrics (cpu, memory, etc.)
    collectDefaultMetrics({ register: this.registry, prefix: 'topgun_' });

    this.connectedClients = new Gauge({
      name: 'topgun_connected_clients',
      help: 'Number of currently connected clients',
      registers: [this.registry],
    });

    this.mapSizeItems = new Gauge({
      name: 'topgun_map_size_items',
      help: 'Number of items in a map',
      labelNames: ['map'],
      registers: [this.registry],
    });

    this.opsTotal = new Counter({
      name: 'topgun_ops_total',
      help: 'Total number of operations',
      labelNames: ['type', 'map'],
      registers: [this.registry],
    });

    this.memoryUsage = new Gauge({
      name: 'topgun_memory_usage_bytes',
      help: 'Current memory usage in bytes',
      registers: [this.registry],
      collect() {
        this.set(process.memoryUsage().heapUsed);
      }
    });

    this.clusterMembers = new Gauge({
      name: 'topgun_cluster_members',
      help: 'Number of active cluster members',
      registers: [this.registry],
    });

    // === Subscription-based routing metrics ===
    this.eventsRoutedTotal = new Counter({
      name: 'topgun_events_routed_total',
      help: 'Total number of events processed for routing',
      registers: [this.registry],
    });

    this.eventsFilteredBySubscription = new Counter({
      name: 'topgun_events_filtered_by_subscription',
      help: 'Events NOT sent due to no active subscriptions',
      registers: [this.registry],
    });

    this.subscribersPerEvent = new Summary({
      name: 'topgun_subscribers_per_event',
      help: 'Distribution of subscribers per event',
      percentiles: [0.5, 0.9, 0.99],
      registers: [this.registry],
    });

    // === Bounded event queue metrics ===
    this.eventQueueSize = new Gauge({
      name: 'topgun_event_queue_size',
      help: 'Current size of the event queue across all stripes',
      labelNames: ['stripe'],
      registers: [this.registry],
    });

    this.eventQueueEnqueued = new Counter({
      name: 'topgun_event_queue_enqueued_total',
      help: 'Total number of events enqueued',
      registers: [this.registry],
    });

    this.eventQueueDequeued = new Counter({
      name: 'topgun_event_queue_dequeued_total',
      help: 'Total number of events dequeued',
      registers: [this.registry],
    });

    this.eventQueueRejected = new Counter({
      name: 'topgun_event_queue_rejected_total',
      help: 'Total number of events rejected due to queue capacity',
      registers: [this.registry],
    });

    // === Backpressure metrics ===
    this.backpressureSyncForcedTotal = new Counter({
      name: 'topgun_backpressure_sync_forced_total',
      help: 'Total number of times sync processing was forced',
      registers: [this.registry],
    });

    this.backpressurePendingOps = new Gauge({
      name: 'topgun_backpressure_pending_ops',
      help: 'Current number of pending async operations',
      registers: [this.registry],
    });

    this.backpressureWaitsTotal = new Counter({
      name: 'topgun_backpressure_waits_total',
      help: 'Total number of times had to wait for capacity',
      registers: [this.registry],
    });

    this.backpressureTimeoutsTotal = new Counter({
      name: 'topgun_backpressure_timeouts_total',
      help: 'Total number of backpressure timeouts',
      registers: [this.registry],
    });

    // === Connection scaling metrics ===
    this.connectionsAcceptedTotal = new Counter({
      name: 'topgun_connections_accepted_total',
      help: 'Total number of connections accepted',
      registers: [this.registry],
    });

    this.connectionsRejectedTotal = new Counter({
      name: 'topgun_connections_rejected_total',
      help: 'Total number of connections rejected due to rate limiting',
      registers: [this.registry],
    });

    this.connectionsPending = new Gauge({
      name: 'topgun_connections_pending',
      help: 'Number of connections currently pending (handshake in progress)',
      registers: [this.registry],
    });

    this.connectionRatePerSecond = new Gauge({
      name: 'topgun_connection_rate_per_second',
      help: 'Current connection rate per second',
      registers: [this.registry],
    });
  }

  public destroy() {
    this.registry.clear();
  }

  public setConnectedClients(count: number) {
    this.connectedClients.set(count);
  }

  public setMapSize(mapName: string, size: number) {
    this.mapSizeItems.set({ map: mapName }, size);
  }

  public incOp(type: 'PUT' | 'GET' | 'DELETE' | 'SUBSCRIBE', mapName: string) {
    this.opsTotal.inc({ type, map: mapName });
  }

  public setClusterMembers(count: number) {
    this.clusterMembers.set(count);
  }

  // === Subscription-based routing metric methods ===

  /**
   * Increment counter for total events processed for routing.
   */
  public incEventsRouted(): void {
    this.eventsRoutedTotal.inc();
  }

  /**
   * Increment counter for events filtered out due to no subscribers.
   */
  public incEventsFilteredBySubscription(): void {
    this.eventsFilteredBySubscription.inc();
  }

  /**
   * Record the number of subscribers for an event (for average calculation).
   */
  public recordSubscribersPerEvent(count: number): void {
    this.subscribersPerEvent.observe(count);
  }

  // === Bounded event queue metric methods ===

  /**
   * Set the current size of a specific queue stripe.
   */
  public setEventQueueSize(stripe: number, size: number): void {
    this.eventQueueSize.set({ stripe: String(stripe) }, size);
  }

  /**
   * Increment counter for events enqueued.
   */
  public incEventQueueEnqueued(): void {
    this.eventQueueEnqueued.inc();
  }

  /**
   * Increment counter for events dequeued.
   */
  public incEventQueueDequeued(): void {
    this.eventQueueDequeued.inc();
  }

  /**
   * Increment counter for events rejected due to queue capacity.
   */
  public incEventQueueRejected(): void {
    this.eventQueueRejected.inc();
  }

  // === Backpressure metric methods ===

  /**
   * Increment counter for forced sync operations.
   */
  public incBackpressureSyncForced(): void {
    this.backpressureSyncForcedTotal.inc();
  }

  /**
   * Set the current number of pending async operations.
   */
  public setBackpressurePendingOps(count: number): void {
    this.backpressurePendingOps.set(count);
  }

  /**
   * Increment counter for times had to wait for capacity.
   */
  public incBackpressureWaits(): void {
    this.backpressureWaitsTotal.inc();
  }

  /**
   * Increment counter for backpressure timeouts.
   */
  public incBackpressureTimeouts(): void {
    this.backpressureTimeoutsTotal.inc();
  }

  // === Connection scaling metric methods ===

  /**
   * Increment counter for accepted connections.
   */
  public incConnectionsAccepted(): void {
    this.connectionsAcceptedTotal.inc();
  }

  /**
   * Increment counter for rejected connections.
   */
  public incConnectionsRejected(): void {
    this.connectionsRejectedTotal.inc();
  }

  /**
   * Set the current number of pending connections.
   */
  public setConnectionsPending(count: number): void {
    this.connectionsPending.set(count);
  }

  /**
   * Set the current connection rate per second.
   */
  public setConnectionRatePerSecond(rate: number): void {
    this.connectionRatePerSecond.set(rate);
  }

  public async getMetrics(): Promise<string> {
    return this.registry.metrics();
  }

  public async getMetricsJson(): Promise<Record<string, any>> {
    const metrics = await this.registry.getMetricsAsJSON();
    // Flatten or simplify for dashboard if needed, but raw JSON is fine for now
    const result: Record<string, any> = {};
    for (const metric of metrics) {
      // Simple flattening: name -> value (if single value)
      // metric.type is an enum/number in some versions or string in others. 
      // To be safe and avoid type errors, we just check values length.
      if (metric.values.length === 1) {
        result[metric.name] = metric.values[0].value;
      } else {
        // Complex metrics
        result[metric.name] = metric.values;
      }
    }
    return result;
  }

  public getContentType(): string {
    return this.registry.contentType;
  }
}

