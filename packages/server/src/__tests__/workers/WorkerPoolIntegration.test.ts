/**
 * WorkerPool Integration Tests
 * Phase 1.05: Integration with ServerCoordinator
 */

import { ServerCoordinator } from '../../ServerCoordinator';

// Helper to wait
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

describe('WorkerPool Integration with ServerCoordinator', () => {
  describe('Configuration', () => {
    it('should create server without worker pool by default', async () => {
      const server = new ServerCoordinator({
        port: 0,
        nodeId: 'test-node-1',
      });

      await server.ready();

      expect(server.workerPoolEnabled).toBe(false);
      expect(server.getWorkerPoolStats()).toBeNull();

      await server.shutdown();
    });

    it('should create server with worker pool when enabled', async () => {
      const server = new ServerCoordinator({
        port: 0,
        nodeId: 'test-node-2',
        workerPoolEnabled: true,
        workerPoolConfig: {
          minWorkers: 1,
          maxWorkers: 2,
        },
      });

      await server.ready();
      // Give workers time to initialize
      await wait(200);

      expect(server.workerPoolEnabled).toBe(true);

      const stats = server.getWorkerPoolStats();
      expect(stats).not.toBeNull();
      expect(stats!.activeWorkers + stats!.idleWorkers).toBeGreaterThanOrEqual(1);

      await server.shutdown();
    });

    it('should use custom worker pool config', async () => {
      const server = new ServerCoordinator({
        port: 0,
        nodeId: 'test-node-3',
        workerPoolEnabled: true,
        workerPoolConfig: {
          minWorkers: 2,
          maxWorkers: 4,
          taskTimeout: 10000,
          idleTimeout: 60000,
        },
      });

      await server.ready();
      await wait(200);

      expect(server.workerPoolEnabled).toBe(true);

      const stats = server.getWorkerPoolStats();
      expect(stats).not.toBeNull();

      await server.shutdown();
    });
  });

  describe('Shutdown', () => {
    it('should gracefully shutdown worker pool', async () => {
      const server = new ServerCoordinator({
        port: 0,
        nodeId: 'test-node-4',
        workerPoolEnabled: true,
        workerPoolConfig: {
          minWorkers: 1,
          maxWorkers: 2,
        },
      });

      await server.ready();
      await wait(200);

      // Verify workers are running
      expect(server.workerPoolEnabled).toBe(true);
      const statsBefore = server.getWorkerPoolStats();
      expect(statsBefore).not.toBeNull();

      // Shutdown should complete without errors
      await server.shutdown();

      // After shutdown, getWorkerPoolStats should still return the last stats
      // (since we're just checking the pool was properly terminated)
    });

    it('should shutdown cleanly even with no worker pool', async () => {
      const server = new ServerCoordinator({
        port: 0,
        nodeId: 'test-node-5',
        workerPoolEnabled: false,
      });

      await server.ready();

      // Shutdown should complete without errors
      await server.shutdown();
    });
  });
});
