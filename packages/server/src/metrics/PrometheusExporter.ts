import {
  Registry,
  Counter,
  Histogram,
  Gauge,
  Summary,
  collectDefaultMetrics,
} from 'prom-client';

/**
 * PrometheusExporter - Extended metrics exporter for TopGun observability.
 *
 * This class provides comprehensive Prometheus metrics for:
 * - WebSocket connections
 * - CRDT operations and merges
 * - Sync operations
 * - Cluster state
 * - Storage operations
 * - Query execution
 * - Search operations
 *
 * @see PHASE_14C_OBSERVABILITY.md for specification
 */
export class PrometheusExporter {
  private registry: Registry;
  private nodeId: string;

  // ============================================================================
  // Connection Metrics
  // ============================================================================

  public readonly wsConnectionsTotal: Gauge;
  public readonly wsConnectionsActive: Gauge;
  public readonly wsMessagesSent: Counter;
  public readonly wsMessagesReceived: Counter;
  public readonly wsBytesTransferred: Counter;

  // ============================================================================
  // Operation Metrics
  // ============================================================================

  public readonly operationsTotal: Counter;
  public readonly operationDuration: Histogram;
  public readonly operationErrors: Counter;

  // ============================================================================
  // CRDT Metrics
  // ============================================================================

  public readonly crdtMergesTotal: Counter;
  public readonly crdtMergeDuration: Histogram;
  public readonly crdtConflictsResolved: Counter;
  public readonly crdtMapSize: Gauge;

  // ============================================================================
  // Sync Metrics
  // ============================================================================

  public readonly syncOperationsTotal: Counter;
  public readonly syncDuration: Histogram;
  public readonly syncBytesTransferred: Counter;
  public readonly merkleComparisons: Counter;
  public readonly deltasSent: Counter;

  // ============================================================================
  // Cluster Metrics
  // ============================================================================

  public readonly clusterNodesTotal: Gauge;
  public readonly clusterNodesHealthy: Gauge;
  public readonly partitionsOwned: Gauge;
  public readonly partitionRebalances: Counter;
  public readonly replicationLag: Gauge;

  // ============================================================================
  // Storage Metrics
  // ============================================================================

  public readonly storageOperations: Counter;
  public readonly storageDuration: Histogram;
  public readonly storageSize: Gauge;

  // ============================================================================
  // Query Engine Metrics
  // ============================================================================

  public readonly queryTotal: Counter;
  public readonly queryDuration: Histogram;
  public readonly queryResultSize: Summary;
  public readonly indexHits: Counter;
  public readonly indexMisses: Counter;

  // ============================================================================
  // Search Metrics
  // ============================================================================

  public readonly searchTotal: Counter;
  public readonly searchDuration: Histogram;
  public readonly searchResultCount: Summary;
  public readonly bm25Calculations: Counter;
  public readonly rrfFusions: Counter;

  constructor(nodeId: string = 'default') {
    this.registry = new Registry();
    this.nodeId = nodeId;

    // Collect default Node.js metrics
    collectDefaultMetrics({
      register: this.registry,
      prefix: 'topgun_nodejs_',
    });

    // Connection metrics
    this.wsConnectionsTotal = new Gauge({
      name: 'topgun_ws_connections_total',
      help: 'Total WebSocket connections since startup',
      labelNames: ['node_id'],
      registers: [this.registry],
    });

    this.wsConnectionsActive = new Gauge({
      name: 'topgun_ws_connections_active',
      help: 'Currently active WebSocket connections',
      labelNames: ['node_id'],
      registers: [this.registry],
    });

    this.wsMessagesSent = new Counter({
      name: 'topgun_ws_messages_sent_total',
      help: 'Total WebSocket messages sent',
      labelNames: ['node_id', 'type'],
      registers: [this.registry],
    });

    this.wsMessagesReceived = new Counter({
      name: 'topgun_ws_messages_received_total',
      help: 'Total WebSocket messages received',
      labelNames: ['node_id', 'type'],
      registers: [this.registry],
    });

    this.wsBytesTransferred = new Counter({
      name: 'topgun_ws_bytes_total',
      help: 'Total bytes transferred via WebSocket',
      labelNames: ['node_id', 'direction'],
      registers: [this.registry],
    });

    // Operation metrics
    this.operationsTotal = new Counter({
      name: 'topgun_operations_total',
      help: 'Total operations processed',
      labelNames: ['node_id', 'operation', 'status'],
      registers: [this.registry],
    });

    this.operationDuration = new Histogram({
      name: 'topgun_operation_duration_seconds',
      help: 'Operation duration in seconds',
      labelNames: ['node_id', 'operation'],
      buckets: [0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10],
      registers: [this.registry],
    });

    this.operationErrors = new Counter({
      name: 'topgun_operation_errors_total',
      help: 'Total operation errors',
      labelNames: ['node_id', 'operation', 'error_type'],
      registers: [this.registry],
    });

    // CRDT metrics
    this.crdtMergesTotal = new Counter({
      name: 'topgun_crdt_merges_total',
      help: 'Total CRDT merge operations',
      labelNames: ['node_id', 'crdt_type'],
      registers: [this.registry],
    });

    this.crdtMergeDuration = new Histogram({
      name: 'topgun_crdt_merge_duration_seconds',
      help: 'CRDT merge duration in seconds',
      labelNames: ['node_id', 'crdt_type'],
      buckets: [0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1],
      registers: [this.registry],
    });

    this.crdtConflictsResolved = new Counter({
      name: 'topgun_crdt_conflicts_resolved_total',
      help: 'Total CRDT conflicts resolved by LWW',
      labelNames: ['node_id'],
      registers: [this.registry],
    });

    this.crdtMapSize = new Gauge({
      name: 'topgun_crdt_map_entries',
      help: 'Number of entries in CRDT maps',
      labelNames: ['node_id', 'map_name'],
      registers: [this.registry],
    });

    // Sync metrics
    this.syncOperationsTotal = new Counter({
      name: 'topgun_sync_operations_total',
      help: 'Total sync operations',
      labelNames: ['node_id', 'type', 'status'],
      registers: [this.registry],
    });

    this.syncDuration = new Histogram({
      name: 'topgun_sync_duration_seconds',
      help: 'Sync operation duration',
      labelNames: ['node_id', 'type'],
      buckets: [0.01, 0.05, 0.1, 0.5, 1, 2, 5, 10, 30],
      registers: [this.registry],
    });

    this.syncBytesTransferred = new Counter({
      name: 'topgun_sync_bytes_total',
      help: 'Bytes transferred during sync',
      labelNames: ['node_id', 'direction'],
      registers: [this.registry],
    });

    this.merkleComparisons = new Counter({
      name: 'topgun_merkle_comparisons_total',
      help: 'Total Merkle tree comparisons',
      labelNames: ['node_id'],
      registers: [this.registry],
    });

    this.deltasSent = new Counter({
      name: 'topgun_deltas_sent_total',
      help: 'Total deltas sent during sync',
      labelNames: ['node_id'],
      registers: [this.registry],
    });

    // Cluster metrics
    this.clusterNodesTotal = new Gauge({
      name: 'topgun_cluster_nodes_total',
      help: 'Total nodes in cluster',
      labelNames: ['node_id'],
      registers: [this.registry],
    });

    this.clusterNodesHealthy = new Gauge({
      name: 'topgun_cluster_nodes_healthy',
      help: 'Healthy nodes in cluster',
      labelNames: ['node_id'],
      registers: [this.registry],
    });

    this.partitionsOwned = new Gauge({
      name: 'topgun_partitions_owned',
      help: 'Number of partitions owned by this node',
      labelNames: ['node_id'],
      registers: [this.registry],
    });

    this.partitionRebalances = new Counter({
      name: 'topgun_partition_rebalances_total',
      help: 'Total partition rebalance operations',
      labelNames: ['node_id'],
      registers: [this.registry],
    });

    this.replicationLag = new Gauge({
      name: 'topgun_replication_lag_seconds',
      help: 'Replication lag in seconds',
      labelNames: ['node_id', 'peer_id'],
      registers: [this.registry],
    });

    // Storage metrics
    this.storageOperations = new Counter({
      name: 'topgun_storage_operations_total',
      help: 'Total storage operations',
      labelNames: ['node_id', 'operation', 'status'],
      registers: [this.registry],
    });

    this.storageDuration = new Histogram({
      name: 'topgun_storage_duration_seconds',
      help: 'Storage operation duration',
      labelNames: ['node_id', 'operation'],
      buckets: [0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1],
      registers: [this.registry],
    });

    this.storageSize = new Gauge({
      name: 'topgun_storage_size_bytes',
      help: 'Storage size in bytes',
      labelNames: ['node_id'],
      registers: [this.registry],
    });

    // Query Engine metrics
    this.queryTotal = new Counter({
      name: 'topgun_query_total',
      help: 'Total queries executed',
      labelNames: ['node_id', 'query_type'],
      registers: [this.registry],
    });

    this.queryDuration = new Histogram({
      name: 'topgun_query_duration_seconds',
      help: 'Query execution duration',
      labelNames: ['node_id', 'query_type'],
      buckets: [0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1, 5],
      registers: [this.registry],
    });

    this.queryResultSize = new Summary({
      name: 'topgun_query_result_size',
      help: 'Query result set size',
      labelNames: ['node_id'],
      percentiles: [0.5, 0.9, 0.99],
      registers: [this.registry],
    });

    this.indexHits = new Counter({
      name: 'topgun_index_hits_total',
      help: 'Index hits during query execution',
      labelNames: ['node_id', 'index_type'],
      registers: [this.registry],
    });

    this.indexMisses = new Counter({
      name: 'topgun_index_misses_total',
      help: 'Index misses (full scan required)',
      labelNames: ['node_id'],
      registers: [this.registry],
    });

    // Search metrics
    this.searchTotal = new Counter({
      name: 'topgun_search_total',
      help: 'Total search operations',
      labelNames: ['node_id', 'search_type'],
      registers: [this.registry],
    });

    this.searchDuration = new Histogram({
      name: 'topgun_search_duration_seconds',
      help: 'Search operation duration',
      labelNames: ['node_id', 'search_type'],
      buckets: [0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1],
      registers: [this.registry],
    });

    this.searchResultCount = new Summary({
      name: 'topgun_search_result_count',
      help: 'Number of search results',
      labelNames: ['node_id'],
      percentiles: [0.5, 0.9, 0.99],
      registers: [this.registry],
    });

    this.bm25Calculations = new Counter({
      name: 'topgun_bm25_calculations_total',
      help: 'Total BM25 score calculations',
      labelNames: ['node_id'],
      registers: [this.registry],
    });

    this.rrfFusions = new Counter({
      name: 'topgun_rrf_fusions_total',
      help: 'Total RRF fusion operations',
      labelNames: ['node_id'],
      registers: [this.registry],
    });
  }

  // ============================================================================
  // Helper Methods
  // ============================================================================

  recordOperation(
    operation: string,
    durationMs: number,
    status: 'success' | 'error' = 'success'
  ): void {
    this.operationsTotal.inc({ node_id: this.nodeId, operation, status });
    this.operationDuration.observe(
      { node_id: this.nodeId, operation },
      durationMs / 1000
    );
  }

  recordSync(type: string, durationMs: number, status: 'success' | 'error'): void {
    this.syncOperationsTotal.inc({ node_id: this.nodeId, type, status });
    this.syncDuration.observe({ node_id: this.nodeId, type }, durationMs / 1000);
  }

  recordCrdtMerge(crdtType: string, durationMs: number, hadConflict: boolean = false): void {
    this.crdtMergesTotal.inc({ node_id: this.nodeId, crdt_type: crdtType });
    this.crdtMergeDuration.observe(
      { node_id: this.nodeId, crdt_type: crdtType },
      durationMs / 1000
    );
    if (hadConflict) {
      this.crdtConflictsResolved.inc({ node_id: this.nodeId });
    }
  }

  recordQuery(queryType: string, durationMs: number, resultSize: number): void {
    this.queryTotal.inc({ node_id: this.nodeId, query_type: queryType });
    this.queryDuration.observe(
      { node_id: this.nodeId, query_type: queryType },
      durationMs / 1000
    );
    this.queryResultSize.observe({ node_id: this.nodeId }, resultSize);
  }

  recordSearch(
    searchType: string,
    durationMs: number,
    resultCount: number
  ): void {
    this.searchTotal.inc({ node_id: this.nodeId, search_type: searchType });
    this.searchDuration.observe(
      { node_id: this.nodeId, search_type: searchType },
      durationMs / 1000
    );
    this.searchResultCount.observe({ node_id: this.nodeId }, resultCount);
  }

  recordStorageOperation(
    operation: string,
    durationMs: number,
    status: 'success' | 'error' = 'success'
  ): void {
    this.storageOperations.inc({ node_id: this.nodeId, operation, status });
    this.storageDuration.observe(
      { node_id: this.nodeId, operation },
      durationMs / 1000
    );
  }

  updateConnection(delta: 1 | -1): void {
    if (delta === 1) {
      this.wsConnectionsTotal.inc({ node_id: this.nodeId });
    }
    this.wsConnectionsActive.inc({ node_id: this.nodeId }, delta);
  }

  updateClusterState(totalNodes: number, healthyNodes: number): void {
    this.clusterNodesTotal.set({ node_id: this.nodeId }, totalNodes);
    this.clusterNodesHealthy.set({ node_id: this.nodeId }, healthyNodes);
  }

  updatePartitions(count: number): void {
    this.partitionsOwned.set({ node_id: this.nodeId }, count);
  }

  updateMapSize(mapName: string, size: number): void {
    this.crdtMapSize.set({ node_id: this.nodeId, map_name: mapName }, size);
  }

  recordMessage(direction: 'sent' | 'received', type: string, bytes: number): void {
    if (direction === 'sent') {
      this.wsMessagesSent.inc({ node_id: this.nodeId, type });
    } else {
      this.wsMessagesReceived.inc({ node_id: this.nodeId, type });
    }
    this.wsBytesTransferred.inc({ node_id: this.nodeId, direction }, bytes);
  }

  recordIndexHit(indexType: string): void {
    this.indexHits.inc({ node_id: this.nodeId, index_type: indexType });
  }

  recordIndexMiss(): void {
    this.indexMisses.inc({ node_id: this.nodeId });
  }

  recordBM25Calculation(): void {
    this.bm25Calculations.inc({ node_id: this.nodeId });
  }

  recordRRFFusion(): void {
    this.rrfFusions.inc({ node_id: this.nodeId });
  }

  recordReplicationLag(peerId: string, lagSeconds: number): void {
    this.replicationLag.set({ node_id: this.nodeId, peer_id: peerId }, lagSeconds);
  }

  recordMerkleComparison(): void {
    this.merkleComparisons.inc({ node_id: this.nodeId });
  }

  recordDeltaSent(): void {
    this.deltasSent.inc({ node_id: this.nodeId });
  }

  recordPartitionRebalance(): void {
    this.partitionRebalances.inc({ node_id: this.nodeId });
  }

  recordOperationError(operation: string, errorType: string): void {
    this.operationErrors.inc({ node_id: this.nodeId, operation, error_type: errorType });
  }

  setStorageSize(bytes: number): void {
    this.storageSize.set({ node_id: this.nodeId }, bytes);
  }

  // ============================================================================
  // Export
  // ============================================================================

  async getMetrics(): Promise<string> {
    return this.registry.metrics();
  }

  get contentType(): string {
    return this.registry.contentType;
  }

  getRegistry(): Registry {
    return this.registry;
  }

  getNodeId(): string {
    return this.nodeId;
  }

  setNodeId(nodeId: string): void {
    this.nodeId = nodeId;
  }
}

// Singleton instance for global access
let globalExporter: PrometheusExporter | null = null;

export function getPrometheusExporter(nodeId?: string): PrometheusExporter {
  if (!globalExporter) {
    globalExporter = new PrometheusExporter(nodeId);
  }
  return globalExporter;
}

export function resetPrometheusExporter(): void {
  globalExporter = null;
}
