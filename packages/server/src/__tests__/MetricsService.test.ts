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
});

