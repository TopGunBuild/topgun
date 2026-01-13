import {
  PrometheusExporter,
  getPrometheusExporter,
  resetPrometheusExporter,
} from '../metrics/PrometheusExporter';

describe('PrometheusExporter', () => {
  let exporter: PrometheusExporter;

  beforeEach(() => {
    resetPrometheusExporter();
    exporter = new PrometheusExporter('test-node');
  });

  describe('initialization', () => {
    it('should create exporter with nodeId', () => {
      expect(exporter.getNodeId()).toBe('test-node');
    });

    it('should have registry', () => {
      expect(exporter.getRegistry()).toBeDefined();
    });

    it('should have content type', () => {
      expect(exporter.contentType).toContain('text/plain');
    });
  });

  describe('connection metrics', () => {
    it('should track connection updates', async () => {
      exporter.updateConnection(1);
      exporter.updateConnection(1);
      exporter.updateConnection(-1);

      const metrics = await exporter.getMetrics();
      expect(metrics).toContain('topgun_ws_connections_total');
      expect(metrics).toContain('topgun_ws_connections_active');
    });

    it('should record messages', async () => {
      exporter.recordMessage('sent', 'PUT', 100);
      exporter.recordMessage('received', 'GET', 50);

      const metrics = await exporter.getMetrics();
      expect(metrics).toContain('topgun_ws_messages_sent_total');
      expect(metrics).toContain('topgun_ws_messages_received_total');
      expect(metrics).toContain('topgun_ws_bytes_total');
    });
  });

  describe('operation metrics', () => {
    it('should record operations', async () => {
      exporter.recordOperation('PUT', 5.0);
      exporter.recordOperation('GET', 2.0, 'success');
      exporter.recordOperation('DELETE', 10.0, 'error');

      const metrics = await exporter.getMetrics();
      expect(metrics).toContain('topgun_operations_total');
      expect(metrics).toContain('topgun_operation_duration_seconds');
    });

    it('should record operation errors', async () => {
      exporter.recordOperationError('PUT', 'timeout');
      exporter.recordOperationError('SYNC', 'network');

      const metrics = await exporter.getMetrics();
      expect(metrics).toContain('topgun_operation_errors_total');
    });
  });

  describe('CRDT metrics', () => {
    it('should record CRDT merges', async () => {
      exporter.recordCrdtMerge('LWWMap', 1.5, false);
      exporter.recordCrdtMerge('ORMap', 2.0, true);

      const metrics = await exporter.getMetrics();
      expect(metrics).toContain('topgun_crdt_merges_total');
      expect(metrics).toContain('topgun_crdt_merge_duration_seconds');
      expect(metrics).toContain('topgun_crdt_conflicts_resolved_total');
    });

    it('should update map sizes', async () => {
      exporter.updateMapSize('users', 100);
      exporter.updateMapSize('posts', 500);

      const metrics = await exporter.getMetrics();
      expect(metrics).toContain('topgun_crdt_map_entries');
    });
  });

  describe('sync metrics', () => {
    it('should record sync operations', async () => {
      exporter.recordSync('delta', 100, 'success');
      exporter.recordSync('full', 5000, 'error');

      const metrics = await exporter.getMetrics();
      expect(metrics).toContain('topgun_sync_operations_total');
      expect(metrics).toContain('topgun_sync_duration_seconds');
    });

    it('should record merkle operations', async () => {
      exporter.recordMerkleComparison();
      exporter.recordDeltaSent();

      const metrics = await exporter.getMetrics();
      expect(metrics).toContain('topgun_merkle_comparisons_total');
      expect(metrics).toContain('topgun_deltas_sent_total');
    });
  });

  describe('cluster metrics', () => {
    it('should update cluster state', async () => {
      exporter.updateClusterState(5, 4);
      exporter.updatePartitions(50);
      exporter.recordPartitionRebalance();

      const metrics = await exporter.getMetrics();
      expect(metrics).toContain('topgun_cluster_nodes_total');
      expect(metrics).toContain('topgun_cluster_nodes_healthy');
      expect(metrics).toContain('topgun_partitions_owned');
      expect(metrics).toContain('topgun_partition_rebalances_total');
    });

    it('should record replication lag', async () => {
      exporter.recordReplicationLag('node-2', 0.5);
      exporter.recordReplicationLag('node-3', 1.2);

      const metrics = await exporter.getMetrics();
      expect(metrics).toContain('topgun_replication_lag_seconds');
    });
  });

  describe('storage metrics', () => {
    it('should record storage operations', async () => {
      exporter.recordStorageOperation('write', 10);
      exporter.recordStorageOperation('read', 2, 'success');
      exporter.recordStorageOperation('delete', 5, 'error');
      exporter.setStorageSize(1024 * 1024 * 100);

      const metrics = await exporter.getMetrics();
      expect(metrics).toContain('topgun_storage_operations_total');
      expect(metrics).toContain('topgun_storage_duration_seconds');
      expect(metrics).toContain('topgun_storage_size_bytes');
    });
  });

  describe('query metrics', () => {
    it('should record queries', async () => {
      exporter.recordQuery('filter', 50, 10);
      exporter.recordQuery('scan', 200, 1000);

      const metrics = await exporter.getMetrics();
      expect(metrics).toContain('topgun_query_total');
      expect(metrics).toContain('topgun_query_duration_seconds');
      expect(metrics).toContain('topgun_query_result_size');
    });

    it('should record index hits/misses', async () => {
      exporter.recordIndexHit('hash');
      exporter.recordIndexHit('btree');
      exporter.recordIndexMiss();

      const metrics = await exporter.getMetrics();
      expect(metrics).toContain('topgun_index_hits_total');
      expect(metrics).toContain('topgun_index_misses_total');
    });
  });

  describe('search metrics', () => {
    it('should record searches', async () => {
      exporter.recordSearch('bm25', 25, 10);
      exporter.recordSearch('hybrid', 50, 20);
      exporter.recordBM25Calculation();
      exporter.recordRRFFusion();

      const metrics = await exporter.getMetrics();
      expect(metrics).toContain('topgun_search_total');
      expect(metrics).toContain('topgun_search_duration_seconds');
      expect(metrics).toContain('topgun_search_result_count');
      expect(metrics).toContain('topgun_bm25_calculations_total');
      expect(metrics).toContain('topgun_rrf_fusions_total');
    });
  });

  describe('metrics output', () => {
    it('should output valid Prometheus format', async () => {
      exporter.recordOperation('PUT', 5.0);
      exporter.updateConnection(1);

      const metrics = await exporter.getMetrics();

      // Should contain HELP and TYPE comments
      expect(metrics).toMatch(/# HELP/);
      expect(metrics).toMatch(/# TYPE/);

      // Should have valid metric lines
      expect(metrics).toMatch(/topgun_\w+\{[^}]*\}\s+\d+/);
    });

    it('should include default Node.js metrics', async () => {
      const metrics = await exporter.getMetrics();

      expect(metrics).toContain('topgun_nodejs_');
    });
  });

  describe('singleton', () => {
    it('should return same instance from getter', () => {
      const e1 = getPrometheusExporter('node1');
      const e2 = getPrometheusExporter('node2'); // should return same instance

      expect(e1).toBe(e2);
      expect(e1.getNodeId()).toBe('node1');
    });

    it('should reset singleton', () => {
      const e1 = getPrometheusExporter('node1');
      resetPrometheusExporter();
      const e2 = getPrometheusExporter('node2');

      expect(e1).not.toBe(e2);
      expect(e2.getNodeId()).toBe('node2');
    });
  });

  describe('nodeId', () => {
    it('should allow changing nodeId', () => {
      exporter.setNodeId('new-node');
      expect(exporter.getNodeId()).toBe('new-node');
    });
  });
});
