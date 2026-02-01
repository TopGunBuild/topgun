import { MetricsService } from '../monitoring/MetricsService';
import { register } from 'prom-client';

describe('MetricsService', () => {
  let metrics: MetricsService;

  beforeEach(() => {
    register.clear();
    metrics = new MetricsService();
  });

  afterEach(() => {
    metrics.destroy();
    register.clear();
  });

  test('should register default metrics', async () => {
    const output = await metrics.getMetrics();
    expect(output).toContain('topgun_process_cpu_user_seconds_total');
    expect(output).toContain('topgun_process_resident_memory_bytes');
  });

  test('should track connected clients', async () => {
    metrics.setConnectedClients(5);
    const output = await metrics.getMetrics();
    expect(output).toContain('topgun_connected_clients 5');
  });

  test('should track map size', async () => {
    metrics.setMapSize('users', 100);
    metrics.setMapSize('posts', 50);
    
    const output = await metrics.getMetrics();
    expect(output).toContain('topgun_map_size_items{map="users"} 100');
    expect(output).toContain('topgun_map_size_items{map="posts"} 50');
  });

  test('should track operations', async () => {
    metrics.incOp('PUT', 'users');
    metrics.incOp('PUT', 'users');
    metrics.incOp('GET', 'posts');

    const output = await metrics.getMetrics();
    expect(output).toContain('topgun_ops_total{type="PUT",map="users"} 2');
    expect(output).toContain('topgun_ops_total{type="GET",map="posts"} 1');
  });

  test('should track cluster members', async () => {
    metrics.setClusterMembers(3);
    const output = await metrics.getMetrics();
    expect(output).toContain('topgun_cluster_members 3');
  });

  test('should return correct content type', () => {
    expect(metrics.getContentType()).toBe('text/plain; version=0.0.4; charset=utf-8');
  });

  // === Distributed Subscription Metrics ===

  describe('Distributed Subscription Metrics', () => {
    test('should increment subscription counter on success', async () => {
      metrics.incDistributedSub('SEARCH', 'success');
      const output = await metrics.getMetrics();
      expect(output).toContain('topgun_distributed_sub_total{type="SEARCH",status="success"} 1');
    });

    test('should track active subscriptions gauge on success', async () => {
      metrics.incDistributedSub('SEARCH', 'success');
      metrics.incDistributedSub('SEARCH', 'success');
      const output = await metrics.getMetrics();
      expect(output).toContain('topgun_distributed_sub_active{type="SEARCH"} 2');
    });

    test('should not increment active gauge on failed subscription', async () => {
      metrics.incDistributedSub('QUERY', 'failed');
      const output = await metrics.getMetrics();
      expect(output).toContain('topgun_distributed_sub_total{type="QUERY",status="failed"} 1');
      // Active gauge should not be incremented for failed
      expect(output).not.toContain('topgun_distributed_sub_active{type="QUERY"} 1');
    });

    test('should decrement active subscriptions gauge', async () => {
      metrics.incDistributedSub('SEARCH', 'success');
      metrics.incDistributedSub('SEARCH', 'success');
      metrics.decDistributedSubActive('SEARCH');
      const output = await metrics.getMetrics();
      expect(output).toContain('topgun_distributed_sub_active{type="SEARCH"} 1');
    });

    test('should track unsubscriptions', async () => {
      metrics.incDistributedSubUnsubscribe('QUERY');
      metrics.incDistributedSubUnsubscribe('QUERY');
      const output = await metrics.getMetrics();
      expect(output).toContain('topgun_distributed_sub_unsubscribe_total{type="QUERY"} 2');
    });

    test('should track pending ACKs gauge', async () => {
      metrics.setDistributedSubPendingAcks(5);
      const output = await metrics.getMetrics();
      expect(output).toContain('topgun_distributed_sub_pending_acks 5');
    });

    test('should track delta updates', async () => {
      metrics.incDistributedSubUpdates('sent', 'ENTER');
      metrics.incDistributedSubUpdates('received', 'UPDATE');
      metrics.incDistributedSubUpdates('received', 'LEAVE');
      const output = await metrics.getMetrics();
      expect(output).toContain('topgun_distributed_sub_updates_total{direction="sent",change_type="ENTER"} 1');
      expect(output).toContain('topgun_distributed_sub_updates_total{direction="received",change_type="UPDATE"} 1');
      expect(output).toContain('topgun_distributed_sub_updates_total{direction="received",change_type="LEAVE"} 1');
    });

    test('should track ACK responses', async () => {
      metrics.incDistributedSubAck('success');
      metrics.incDistributedSubAck('success');
      metrics.incDistributedSubAck('timeout');
      const output = await metrics.getMetrics();
      expect(output).toContain('topgun_distributed_sub_ack_total{status="success"} 2');
      expect(output).toContain('topgun_distributed_sub_ack_total{status="timeout"} 1');
    });

    test('should track ACK responses with count parameter', async () => {
      metrics.incDistributedSubAck('success', 5);
      metrics.incDistributedSubAck('timeout', 2);
      const output = await metrics.getMetrics();
      expect(output).toContain('topgun_distributed_sub_ack_total{status="success"} 5');
      expect(output).toContain('topgun_distributed_sub_ack_total{status="timeout"} 2');
    });

    test('should record registration duration histogram', async () => {
      metrics.recordDistributedSubRegistration('SEARCH', 150);
      metrics.recordDistributedSubRegistration('SEARCH', 250);
      const output = await metrics.getMetrics();
      expect(output).toContain('topgun_distributed_sub_registration_duration_ms_bucket');
      expect(output).toContain('topgun_distributed_sub_registration_duration_ms_count{type="SEARCH"} 2');
    });

    test('should record update latency histogram', async () => {
      metrics.recordDistributedSubUpdateLatency('QUERY', 15);
      metrics.recordDistributedSubUpdateLatency('QUERY', 5);
      const output = await metrics.getMetrics();
      expect(output).toContain('topgun_distributed_sub_update_latency_ms_bucket');
      expect(output).toContain('topgun_distributed_sub_update_latency_ms_count{type="QUERY"} 2');
    });

    test('should record initial results count histogram', async () => {
      metrics.recordDistributedSubInitialResultsCount('SEARCH', 25);
      metrics.recordDistributedSubInitialResultsCount('SEARCH', 5);
      const output = await metrics.getMetrics();
      expect(output).toContain('topgun_distributed_sub_initial_results_count_bucket');
      expect(output).toContain('topgun_distributed_sub_initial_results_count_count{type="SEARCH"} 2');
    });

    test('should track node disconnect events', async () => {
      metrics.incDistributedSubNodeDisconnect();
      metrics.incDistributedSubNodeDisconnect();
      const output = await metrics.getMetrics();
      expect(output).toContain('topgun_distributed_sub_node_disconnect_total 2');
    });
  });
});

