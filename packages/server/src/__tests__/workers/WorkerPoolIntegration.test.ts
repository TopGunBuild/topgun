/**
 * WorkerPool Integration Tests
 * Phase 1.05-08: Integration with ServerCoordinator
 */

import { ServerCoordinator, ServerFactory } from '../../';
import { MerkleWorker, CRDTMergeWorker, SerializationWorker } from '../../workers';

// Helper to wait
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

describe('WorkerPool Integration with ServerCoordinator', () => {
  describe('Configuration', () => {
    it('should create server without worker pool by default', async () => {
      const server = ServerFactory.create({
        port: 0,
        nodeId: 'test-node-1',
      });

      await server.ready();

      expect(server.workerPoolEnabled).toBe(false);
      expect(server.getWorkerPoolStats()).toBeNull();

      await server.shutdown();
    });

    it('should create server with worker pool when enabled', async () => {
      const server = ServerFactory.create({
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
      const server = ServerFactory.create({
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
      const server = ServerFactory.create({
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
      const server = ServerFactory.create({
        port: 0,
        nodeId: 'test-node-5',
        workerPoolEnabled: false,
      });

      await server.ready();

      // Shutdown should complete without errors
      await server.shutdown();
    });
  });

  describe('Worker Accessors', () => {
    it('should return null for workers when pool is disabled', async () => {
      const server = ServerFactory.create({
        port: 0,
        nodeId: 'test-node-6',
        workerPoolEnabled: false,
      });

      await server.ready();

      expect(server.getMerkleWorker()).toBeNull();
      expect(server.getCRDTMergeWorker()).toBeNull();
      expect(server.getSerializationWorker()).toBeNull();

      await server.shutdown();
    });

    it('should return workers when pool is enabled', async () => {
      const server = ServerFactory.create({
        port: 0,
        nodeId: 'test-node-7',
        workerPoolEnabled: true,
        workerPoolConfig: {
          minWorkers: 1,
          maxWorkers: 2,
        },
      });

      await server.ready();
      await wait(200);

      const merkleWorker = server.getMerkleWorker();
      const crdtWorker = server.getCRDTMergeWorker();
      const serializationWorker = server.getSerializationWorker();

      expect(merkleWorker).not.toBeNull();
      expect(merkleWorker).toBeInstanceOf(MerkleWorker);

      expect(crdtWorker).not.toBeNull();
      expect(crdtWorker).toBeInstanceOf(CRDTMergeWorker);

      expect(serializationWorker).not.toBeNull();
      expect(serializationWorker).toBeInstanceOf(SerializationWorker);

      await server.shutdown();
    });

    it('should allow using SerializationWorker for batch operations', async () => {
      const server = ServerFactory.create({
        port: 0,
        nodeId: 'test-node-8',
        workerPoolEnabled: true,
        workerPoolConfig: {
          minWorkers: 1,
          maxWorkers: 2,
        },
      });

      await server.ready();
      await wait(200);

      const serializer = server.getSerializationWorker();
      expect(serializer).not.toBeNull();

      // Test batch serialization (inline since < 10 items)
      const items = [
        { id: 1, name: 'Item 1' },
        { id: 2, name: 'Item 2' },
        { id: 3, name: 'Item 3' },
      ];

      const serialized = await serializer!.serializeBatch(items);
      expect(serialized.length).toBe(3);

      const deserialized = await serializer!.deserializeBatch<typeof items[0]>(serialized);
      expect(deserialized).toEqual(items);

      await server.shutdown();
    });
  });
});
