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

