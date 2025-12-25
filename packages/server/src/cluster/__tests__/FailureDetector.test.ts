import { FailureDetector, DEFAULT_FAILURE_DETECTOR_CONFIG } from '../FailureDetector';

// Increase Jest timeout for timing-based tests
jest.setTimeout(10000);

describe('FailureDetector', () => {
  let detector: FailureDetector;

  beforeEach(() => {
    detector = new FailureDetector({
      heartbeatIntervalMs: 50, // Fast checks
      suspicionTimeoutMs: 200,
      confirmationTimeoutMs: 400,
      phiThreshold: 2, // Low threshold for faster detection
      minSamples: 2,
      initialHeartbeatIntervalMs: 50,
    });
  });

  afterEach(() => {
    detector.stop();
  });

  describe('monitoring', () => {
    it('should start monitoring a node', () => {
      detector.startMonitoring('node-1');
      expect(detector.getMonitoredNodes()).toContain('node-1');
    });

    it('should stop monitoring a node', () => {
      detector.startMonitoring('node-1');
      detector.stopMonitoring('node-1');
      expect(detector.getMonitoredNodes()).not.toContain('node-1');
    });

    it('should auto-start monitoring on first heartbeat', () => {
      detector.recordHeartbeat('node-1');
      expect(detector.getMonitoredNodes()).toContain('node-1');
    });
  });

  describe('heartbeat recording', () => {
    it('should record heartbeats', () => {
      detector.startMonitoring('node-1');
      detector.recordHeartbeat('node-1');
      expect(detector.getPhi('node-1')).toBe(0);
    });

    it('should calculate phi based on heartbeat intervals', async () => {
      detector.startMonitoring('node-1');

      // Send regular heartbeats to build up history
      for (let i = 0; i < 5; i++) {
        detector.recordHeartbeat('node-1');
        await new Promise((r) => setTimeout(r, 50));
      }

      // Wait longer than usual
      await new Promise((r) => setTimeout(r, 200));

      // Phi should increase due to missed heartbeat
      const phi = detector.getPhi('node-1');
      expect(phi).toBeGreaterThan(0);
    });
  });

  describe('suspicion', () => {
    it('should emit nodeSuspected when phi exceeds threshold', async () => {
      const suspectedHandler = jest.fn();
      detector.on('nodeSuspected', suspectedHandler);
      detector.start();
      detector.startMonitoring('node-1');

      // Wait for suspicion timeout
      await new Promise((r) => setTimeout(r, 600));

      expect(suspectedHandler).toHaveBeenCalled();
      expect(detector.isSuspected('node-1')).toBe(true);
    });

    it('should clear suspicion on heartbeat', async () => {
      const recoveredHandler = jest.fn();
      detector.on('nodeRecovered', recoveredHandler);
      detector.start();
      detector.startMonitoring('node-1');

      // Wait for suspicion
      await new Promise((r) => setTimeout(r, 600));
      expect(detector.isSuspected('node-1')).toBe(true);

      // Send heartbeat to recover
      detector.recordHeartbeat('node-1');
      expect(detector.isSuspected('node-1')).toBe(false);
      expect(recoveredHandler).toHaveBeenCalled();
    });
  });

  describe('failure confirmation', () => {
    it('should confirm failure after confirmation timeout', async () => {
      const confirmedHandler = jest.fn();
      detector.on('nodeConfirmedFailed', confirmedHandler);
      detector.start();
      detector.startMonitoring('node-1');

      // Wait for full confirmation (suspicion + confirmation timeout)
      await new Promise((r) => setTimeout(r, 1600));

      expect(confirmedHandler).toHaveBeenCalled();
      expect(detector.isConfirmedFailed('node-1')).toBe(true);
    });

    it('should not confirm if heartbeat received before timeout', async () => {
      const confirmedHandler = jest.fn();
      detector.on('nodeConfirmedFailed', confirmedHandler);
      detector.start();
      detector.startMonitoring('node-1');

      // Wait for suspicion (phi exceeds threshold after ~100ms with 50ms heartbeat interval)
      await new Promise((r) => setTimeout(r, 250));
      expect(detector.isSuspected('node-1')).toBe(true);

      // Send heartbeat before confirmation timeout (400ms) - this clears suspicion
      detector.recordHeartbeat('node-1');
      expect(detector.isSuspected('node-1')).toBe(false);

      // Keep sending heartbeats to prevent re-suspicion
      const heartbeatInterval = setInterval(() => {
        detector.recordHeartbeat('node-1');
      }, 40);

      // Wait for what would be confirmation timeout
      await new Promise((r) => setTimeout(r, 500));

      clearInterval(heartbeatInterval);

      expect(confirmedHandler).not.toHaveBeenCalled();
      expect(detector.isConfirmedFailed('node-1')).toBe(false);
    });
  });

  describe('metrics', () => {
    it('should return correct metrics', () => {
      detector.startMonitoring('node-1');
      detector.startMonitoring('node-2');

      const metrics = detector.getMetrics();
      expect(metrics.monitoredNodes).toBe(2);
      expect(metrics.suspectedNodes).toBe(0);
      expect(metrics.confirmedFailedNodes).toBe(0);
    });
  });

  describe('phi calculation', () => {
    it('should return 0 for nodes with recent heartbeats', () => {
      detector.startMonitoring('node-1');
      detector.recordHeartbeat('node-1');
      expect(detector.calculatePhi('node-1')).toBe(0);
    });

    it('should return 0 for unknown nodes', () => {
      expect(detector.calculatePhi('unknown-node')).toBe(0);
    });

    it('should use fallback calculation with few samples', () => {
      detector.startMonitoring('node-1');
      // Only one sample, should use fallback
      const phi = detector.getPhi('node-1');
      expect(typeof phi).toBe('number');
    });
  });

  describe('default config', () => {
    it('should have sensible defaults', () => {
      expect(DEFAULT_FAILURE_DETECTOR_CONFIG.heartbeatIntervalMs).toBe(1000);
      expect(DEFAULT_FAILURE_DETECTOR_CONFIG.suspicionTimeoutMs).toBe(5000);
      expect(DEFAULT_FAILURE_DETECTOR_CONFIG.confirmationTimeoutMs).toBe(10000);
      expect(DEFAULT_FAILURE_DETECTOR_CONFIG.phiThreshold).toBe(8);
    });
  });
});
