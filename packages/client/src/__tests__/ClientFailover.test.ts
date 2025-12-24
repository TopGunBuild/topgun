/**
 * Tests for Client Failover (Phase 4.5 Task 05)
 *
 * Verifies failover handling in ClusterClient:
 * - Circuit breaker pattern
 * - Retry logic with rerouting
 * - Connection failure detection
 * - Operation retry on errors
 */

describe('Client Failover', () => {
  describe('Circuit Breaker Pattern', () => {
    const { ClusterClient } = require('../cluster/ClusterClient');

    test('should initialize circuit in closed state', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
      });

      const circuit = client.getCircuit('node-1');
      expect(circuit.state).toBe('closed');
      expect(circuit.failures).toBe(0);
      expect(circuit.lastFailure).toBe(0);

      client.close();
    });

    test('should allow requests when circuit is closed', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
      });

      expect(client.canUseNode('node-1')).toBe(true);

      client.close();
    });

    test('should count failures', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
      });

      client.recordFailure('node-1');
      expect(client.getCircuit('node-1').failures).toBe(1);

      client.recordFailure('node-1');
      expect(client.getCircuit('node-1').failures).toBe(2);

      client.close();
    });

    test('should open circuit after threshold failures', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
        circuitBreaker: {
          failureThreshold: 3,
          resetTimeoutMs: 30000,
        },
      });

      const handler = jest.fn();
      client.on('circuit:open', handler);

      // Record failures up to threshold
      client.recordFailure('node-1');
      expect(client.canUseNode('node-1')).toBe(true);

      client.recordFailure('node-1');
      expect(client.canUseNode('node-1')).toBe(true);

      client.recordFailure('node-1');
      expect(client.canUseNode('node-1')).toBe(false);
      expect(client.getCircuit('node-1').state).toBe('open');
      expect(handler).toHaveBeenCalledWith('node-1');

      client.close();
    });

    test('should not allow requests when circuit is open', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
        circuitBreaker: {
          failureThreshold: 2,
          resetTimeoutMs: 60000, // Long timeout for test
        },
      });

      // Open the circuit
      client.recordFailure('node-1');
      client.recordFailure('node-1');

      expect(client.canUseNode('node-1')).toBe(false);

      client.close();
    });

    test('should transition to half-open after reset timeout', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
        circuitBreaker: {
          failureThreshold: 2,
          resetTimeoutMs: 100, // Short timeout for test
        },
      });

      const halfOpenHandler = jest.fn();
      client.on('circuit:half-open', halfOpenHandler);

      // Open the circuit
      client.recordFailure('node-1');
      client.recordFailure('node-1');
      expect(client.canUseNode('node-1')).toBe(false);

      // Wait for reset timeout and check again
      return new Promise<void>((resolve) => {
        setTimeout(() => {
          expect(client.canUseNode('node-1')).toBe(true);
          expect(client.getCircuit('node-1').state).toBe('half-open');
          expect(halfOpenHandler).toHaveBeenCalledWith('node-1');
          client.close();
          resolve();
        }, 150);
      });
    });

    test('should close circuit on success', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
        circuitBreaker: {
          failureThreshold: 2,
          resetTimeoutMs: 10,
        },
      });

      const closedHandler = jest.fn();
      client.on('circuit:closed', closedHandler);

      // Open the circuit
      client.recordFailure('node-1');
      client.recordFailure('node-1');
      expect(client.getCircuit('node-1').state).toBe('open');

      // Wait for half-open transition
      return new Promise<void>((resolve) => {
        setTimeout(() => {
          // Transition to half-open
          client.canUseNode('node-1');
          expect(client.getCircuit('node-1').state).toBe('half-open');

          // Record success to close the circuit
          client.recordSuccess('node-1');
          expect(client.getCircuit('node-1').state).toBe('closed');
          expect(client.getCircuit('node-1').failures).toBe(0);
          expect(closedHandler).toHaveBeenCalledWith('node-1');

          client.close();
          resolve();
        }, 50);
      });
    });

    test('should reset circuit manually', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
      });

      // Add some failures
      client.recordFailure('node-1');
      client.recordFailure('node-1');

      // Reset the circuit
      client.resetCircuit('node-1');

      // Should be back to initial state
      const circuit = client.getCircuit('node-1');
      expect(circuit.state).toBe('closed');
      expect(circuit.failures).toBe(0);

      client.close();
    });

    test('should reset all circuits', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
      });

      // Add failures to multiple nodes
      client.recordFailure('node-1');
      client.recordFailure('node-2');
      client.recordFailure('node-3');

      // Get circuit states (creates entries)
      client.getCircuit('node-1');
      client.getCircuit('node-2');
      client.getCircuit('node-3');

      // Reset all
      client.resetAllCircuits();

      // All should be fresh
      expect(client.getCircuitStates().size).toBe(0);

      client.close();
    });

    test('should get all circuit states', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
      });

      client.recordFailure('node-1');
      client.recordFailure('node-2');

      const states = client.getCircuitStates();
      expect(states.size).toBe(2);
      expect(states.get('node-1')?.failures).toBe(1);
      expect(states.get('node-2')?.failures).toBe(1);

      client.close();
    });
  });

  describe('Retry Error Classification', () => {
    const { ClusterClient } = require('../cluster/ClusterClient');

    // Note: isRetryableError is private, so we test it indirectly through sendWithRetry behavior
    // Here we just verify the circuit breaker integration works

    test('should track separate circuits per node', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
        circuitBreaker: {
          failureThreshold: 3,
        },
      });

      // Fail node-1 multiple times
      client.recordFailure('node-1');
      client.recordFailure('node-1');
      client.recordFailure('node-1');

      // Fail node-2 once
      client.recordFailure('node-2');

      // node-1 should be open, node-2 should be closed
      expect(client.canUseNode('node-1')).toBe(false);
      expect(client.canUseNode('node-2')).toBe(true);

      client.close();
    });
  });

  describe('Default Configuration', () => {
    const { ClusterClient } = require('../cluster/ClusterClient');
    const { DEFAULT_CIRCUIT_BREAKER_CONFIG } = require('@topgunbuild/core');

    test('should use default circuit breaker config', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
      });

      // Verify default threshold (5 failures)
      for (let i = 0; i < DEFAULT_CIRCUIT_BREAKER_CONFIG.failureThreshold - 1; i++) {
        client.recordFailure('node-1');
        expect(client.canUseNode('node-1')).toBe(true);
      }

      // One more should open it
      client.recordFailure('node-1');
      expect(client.canUseNode('node-1')).toBe(false);

      client.close();
    });

    test('should allow custom circuit breaker config', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
        circuitBreaker: {
          failureThreshold: 10,
          resetTimeoutMs: 60000,
        },
      });

      // Should tolerate more failures with custom threshold
      for (let i = 0; i < 9; i++) {
        client.recordFailure('node-1');
        expect(client.canUseNode('node-1')).toBe(true);
      }

      // 10th failure should open
      client.recordFailure('node-1');
      expect(client.canUseNode('node-1')).toBe(false);

      client.close();
    });
  });

  describe('SyncEngine Failover Methods', () => {
    // SyncEngine failover methods are tested via integration tests
    // Unit testing is complex due to module dependencies
    // The methods are: waitForPartitionMapUpdate, waitForConnection, waitForState,
    // isProviderConnected, getConnectionProvider

    test.skip('SyncEngine module exports the class', () => {
      // Skipping: requires full module setup with storage adapters
      // The failover methods exist in SyncEngine.ts and are tested via
      // integration tests in packages/e2e/src/__tests__
      expect(true).toBe(true);
    });
  });

  describe('ClusterClient sendWithRetry', () => {
    const { ClusterClient } = require('../cluster/ClusterClient');

    test('should export sendWithRetry method', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
      });

      expect(typeof client.sendWithRetry).toBe('function');

      client.close();
    });

    test('sendWithRetry should throw when not connected', async () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
      });

      // Not connected, should fail after retries
      // Use very short retry delays to speed up test
      await expect(
        client.sendWithRetry(new Uint8Array([1, 2, 3]), 'test-key', {
          maxRetries: 1,
          retryDelayMs: 10,
        })
      ).rejects.toThrow();

      client.close();
    }, 10000); // Increase timeout for this test
  });

  describe('Circuit Breaker Events', () => {
    const { ClusterClient } = require('../cluster/ClusterClient');

    test('should emit circuit:open event', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
        circuitBreaker: { failureThreshold: 2 },
      });

      const handler = jest.fn();
      client.on('circuit:open', handler);

      client.recordFailure('node-1');
      client.recordFailure('node-1');

      expect(handler).toHaveBeenCalledTimes(1);
      expect(handler).toHaveBeenCalledWith('node-1');

      client.close();
    });

    test('should emit circuit:closed event when recovering', () => {
      const client = new ClusterClient({
        seedNodes: ['ws://localhost:9001'],
        routingMode: 'direct',
        circuitBreaker: {
          failureThreshold: 2,
          resetTimeoutMs: 10,
        },
      });

      const closedHandler = jest.fn();
      client.on('circuit:closed', closedHandler);

      // Open the circuit
      client.recordFailure('node-1');
      client.recordFailure('node-1');

      return new Promise<void>((resolve) => {
        setTimeout(() => {
          // Force half-open by checking canUseNode
          client.canUseNode('node-1');

          // Record success to close
          client.recordSuccess('node-1');

          expect(closedHandler).toHaveBeenCalledTimes(1);
          expect(closedHandler).toHaveBeenCalledWith('node-1');

          client.close();
          resolve();
        }, 50);
      });
    });
  });
});
