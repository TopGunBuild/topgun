import { Registry, Gauge, Counter, Summary, Histogram, collectDefaultMetrics } from 'prom-client';

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

  // Distributed search metrics (Phase 14)
  private distributedSearchTotal: Counter;
  private distributedSearchDuration: Summary;
  private distributedSearchFailedNodes: Counter;
  private distributedSearchPartialResults: Counter;

  // Distributed subscription metrics (Phase 14.2)
  private distributedSubTotal: Counter;
  private distributedSubUnsubscribeTotal: Counter;
  private distributedSubActive: Gauge;
  private distributedSubPendingAcks: Gauge;
  private distributedSubUpdates: Counter;
  private distributedSubAckTotal: Counter;
  private distributedSubRegistrationDuration: Histogram;
  private distributedSubUpdateLatency: Histogram;
  private distributedSubInitialResultsCount: Histogram;

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

    // === Distributed search metrics (Phase 14) ===
    this.distributedSearchTotal = new Counter({
      name: 'topgun_distributed_search_total',
      help: 'Total number of distributed search requests',
      labelNames: ['map', 'status'],
      registers: [this.registry],
    });

    this.distributedSearchDuration = new Summary({
      name: 'topgun_distributed_search_duration_ms',
      help: 'Distribution of distributed search execution times in milliseconds',
      labelNames: ['map'],
      percentiles: [0.5, 0.9, 0.95, 0.99],
      registers: [this.registry],
    });

    this.distributedSearchFailedNodes = new Counter({
      name: 'topgun_distributed_search_failed_nodes_total',
      help: 'Total number of node failures during distributed search',
      registers: [this.registry],
    });

    this.distributedSearchPartialResults = new Counter({
      name: 'topgun_distributed_search_partial_results_total',
      help: 'Total number of searches that returned partial results due to node failures',
      registers: [this.registry],
    });

    // === Distributed subscription metrics (Phase 14.2) ===
    this.distributedSubTotal = new Counter({
      name: 'topgun_distributed_sub_total',
      help: 'Total distributed subscriptions created',
      labelNames: ['type', 'status'],
      registers: [this.registry],
    });

    this.distributedSubUnsubscribeTotal = new Counter({
      name: 'topgun_distributed_sub_unsubscribe_total',
      help: 'Total unsubscriptions from distributed subscriptions',
      labelNames: ['type'],
      registers: [this.registry],
    });

    this.distributedSubActive = new Gauge({
      name: 'topgun_distributed_sub_active',
      help: 'Currently active distributed subscriptions',
      labelNames: ['type'],
      registers: [this.registry],
    });

    this.distributedSubPendingAcks = new Gauge({
      name: 'topgun_distributed_sub_pending_acks',
      help: 'Subscriptions waiting for ACKs from cluster nodes',
      registers: [this.registry],
    });

    this.distributedSubUpdates = new Counter({
      name: 'topgun_distributed_sub_updates_total',
      help: 'Delta updates processed for distributed subscriptions',
      labelNames: ['direction', 'change_type'],
      registers: [this.registry],
    });

    this.distributedSubAckTotal = new Counter({
      name: 'topgun_distributed_sub_ack_total',
      help: 'Node ACK responses for distributed subscriptions',
      labelNames: ['status'],
      registers: [this.registry],
    });

    this.distributedSubRegistrationDuration = new Histogram({
      name: 'topgun_distributed_sub_registration_duration_ms',
      help: 'Time to register subscription on all nodes',
      labelNames: ['type'],
      buckets: [10, 50, 100, 250, 500, 1000, 2500],
      registers: [this.registry],
    });

    this.distributedSubUpdateLatency = new Histogram({
      name: 'topgun_distributed_sub_update_latency_ms',
      help: 'Latency from data change to client notification',
      labelNames: ['type'],
      buckets: [1, 5, 10, 25, 50, 100, 250],
      registers: [this.registry],
    });

    this.distributedSubInitialResultsCount = new Histogram({
      name: 'topgun_distributed_sub_initial_results_count',
      help: 'Initial result set size for distributed subscriptions',
      labelNames: ['type'],
      buckets: [0, 1, 5, 10, 25, 50, 100],
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

  // === Distributed search metric methods (Phase 14) ===

  /**
   * Record a distributed search request.
   * @param mapName - Name of the map being searched
   * @param status - 'success', 'partial', or 'error'
   */
  public incDistributedSearch(mapName: string, status: 'success' | 'partial' | 'error'): void {
    this.distributedSearchTotal.inc({ map: mapName, status });
  }

  /**
   * Record the duration of a distributed search.
   * @param mapName - Name of the map being searched
   * @param durationMs - Duration in milliseconds
   */
  public recordDistributedSearchDuration(mapName: string, durationMs: number): void {
    this.distributedSearchDuration.observe({ map: mapName }, durationMs);
  }

  /**
   * Increment counter for failed nodes during distributed search.
   * @param count - Number of nodes that failed (default 1)
   */
  public incDistributedSearchFailedNodes(count: number = 1): void {
    this.distributedSearchFailedNodes.inc(count);
  }

  /**
   * Increment counter for searches returning partial results.
   */
  public incDistributedSearchPartialResults(): void {
    this.distributedSearchPartialResults.inc();
  }

  // === Distributed subscription metric methods (Phase 14.2) ===

  /**
   * Record a distributed subscription creation.
   * @param type - Subscription type (SEARCH or QUERY)
   * @param status - Result status (success, failed, timeout)
   */
  public incDistributedSub(type: 'SEARCH' | 'QUERY', status: 'success' | 'failed' | 'timeout'): void {
    this.distributedSubTotal.inc({ type, status });
    if (status === 'success') {
      this.distributedSubActive.inc({ type });
    }
  }

  /**
   * Record a distributed subscription unsubscription.
   * @param type - Subscription type (SEARCH or QUERY)
   */
  public incDistributedSubUnsubscribe(type: 'SEARCH' | 'QUERY'): void {
    this.distributedSubUnsubscribeTotal.inc({ type });
  }

  /**
   * Decrement the active distributed subscriptions gauge.
   * @param type - Subscription type (SEARCH or QUERY)
   */
  public decDistributedSubActive(type: 'SEARCH' | 'QUERY'): void {
    this.distributedSubActive.dec({ type });
  }

  /**
   * Set the number of subscriptions waiting for ACKs.
   * @param count - Number of pending ACKs
   */
  public setDistributedSubPendingAcks(count: number): void {
    this.distributedSubPendingAcks.set(count);
  }

  /**
   * Record a delta update for distributed subscriptions.
   * @param direction - Direction of update (sent or received)
   * @param changeType - Type of change (ENTER, UPDATE, LEAVE)
   */
  public incDistributedSubUpdates(direction: 'sent' | 'received', changeType: 'ENTER' | 'UPDATE' | 'LEAVE'): void {
    this.distributedSubUpdates.inc({ direction, change_type: changeType });
  }

  /**
   * Record a node ACK response.
   * @param status - ACK status (success, failed, timeout)
   */
  public incDistributedSubAck(status: 'success' | 'failed' | 'timeout'): void {
    this.distributedSubAckTotal.inc({ status });
  }

  /**
   * Record the time to register a subscription on all nodes.
   * @param type - Subscription type (SEARCH or QUERY)
   * @param durationMs - Duration in milliseconds
   */
  public recordDistributedSubRegistration(type: 'SEARCH' | 'QUERY', durationMs: number): void {
    this.distributedSubRegistrationDuration.observe({ type }, durationMs);
  }

  /**
   * Record the latency from data change to client notification.
   * @param type - Subscription type (SEARCH or QUERY)
   * @param latencyMs - Latency in milliseconds
   */
  public recordDistributedSubUpdateLatency(type: 'SEARCH' | 'QUERY', latencyMs: number): void {
    this.distributedSubUpdateLatency.observe({ type }, latencyMs);
  }

  /**
   * Record the initial result set size for a subscription.
   * @param type - Subscription type (SEARCH or QUERY)
   * @param count - Number of initial results
   */
  public recordDistributedSubInitialResultsCount(type: 'SEARCH' | 'QUERY', count: number): void {
    this.distributedSubInitialResultsCount.observe({ type }, count);
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

