/**
 * UAT Integration Tests for Phase 01: Security Hardening
 *
 * These tests verify the security features implemented in phase 01:
 * - Rate-limited logging (UAT 1-2)
 * - JWT secret validation in production (UAT 3-5)
 * - HLC strict mode clock drift detection (UAT 6-7)
 */

import { HLC, Timestamp } from '../../../packages/core/src';
import { RateLimitedLogger, BaseLogger } from '../../../packages/server/src/utils/RateLimitedLogger';
import { validateJwtSecret, DEFAULT_JWT_SECRET } from '../../../packages/server/src/utils/validateConfig';

// Mock logger to capture log output with full tracking
const createMockLogger = (): BaseLogger & {
  errorCalls: { obj: object; msg: string }[];
  warnCalls: { obj: object; msg: string }[];
  clear: () => void;
} => {
  const errorCalls: { obj: object; msg: string }[] = [];
  const warnCalls: { obj: object; msg: string }[] = [];
  return {
    errorCalls,
    warnCalls,
    error: (obj: object, msg: string) => {
      errorCalls.push({ obj, msg });
    },
    warn: (obj: object, msg: string) => {
      warnCalls.push({ obj, msg });
    },
    clear: () => {
      errorCalls.length = 0;
      warnCalls.length = 0;
    },
  };
};

describe('UAT Phase 01: Security Hardening', () => {
  // ============================================================
  // UAT 1: Rate-Limited Logging Suppresses Repeated Errors
  // ============================================================
  describe('UAT 1: Rate-Limited Logging Suppresses Repeated Errors', () => {
    /**
     * Expected behavior:
     * When the same invalid message error occurs more than 5 times within 10 seconds
     * for a single client, subsequent errors are suppressed from logs.
     * Only the first 5 log entries appear.
     * When the window resets, a summary message shows how many were suppressed.
     */

    beforeEach(() => {
      jest.useFakeTimers();
    });

    afterEach(() => {
      jest.useRealTimers();
    });

    test('should log exactly 5 errors when 10 occur within 10 seconds', () => {
      const mockLogger = createMockLogger();
      const rateLimitedLogger = new RateLimitedLogger({
        maxPerWindow: 5,
        windowMs: 10000,
        baseLogger: mockLogger,
      });

      // Simulate 10 invalid message errors from the same client
      const clientId = 'client-123';
      const key = `invalid-message:${clientId}`;

      for (let i = 0; i < 10; i++) {
        rateLimitedLogger.error(key, { clientId, errorIndex: i }, `Invalid message from client ${clientId}`);
      }

      // Verify: Exactly 5 error logs should have been emitted
      expect(mockLogger.errorCalls).toHaveLength(5);

      // Verify the first 5 calls have correct indices
      for (let i = 0; i < 5; i++) {
        expect(mockLogger.errorCalls[i].obj).toEqual({ clientId, errorIndex: i });
      }

      // No summary yet (window hasn't reset)
      expect(mockLogger.warnCalls).toHaveLength(0);
    });

    test('should log suppression summary when window resets', () => {
      const mockLogger = createMockLogger();
      const rateLimitedLogger = new RateLimitedLogger({
        maxPerWindow: 5,
        windowMs: 10000,
        baseLogger: mockLogger,
      });

      const clientId = 'client-456';
      const key = `invalid-message:${clientId}`;

      // Send 10 errors (5 logged, 5 suppressed)
      for (let i = 0; i < 10; i++) {
        rateLimitedLogger.error(key, { clientId, errorIndex: i }, `Invalid message ${i}`);
      }

      expect(mockLogger.errorCalls).toHaveLength(5);
      expect(mockLogger.warnCalls).toHaveLength(0); // No summary yet

      // Advance time past the window (10 seconds)
      jest.advanceTimersByTime(11000);

      // Send one more error - this should trigger window reset and summary
      rateLimitedLogger.error(key, { clientId, errorIndex: 10 }, `Invalid message 10`);

      // Should have logged the suppression summary
      expect(mockLogger.warnCalls).toHaveLength(1);
      expect(mockLogger.warnCalls[0].msg).toContain('suppressed 5 messages');
      expect(mockLogger.warnCalls[0].obj).toMatchObject({
        key,
        suppressedCount: 5,
      });

      // New error should be logged (new window started)
      expect(mockLogger.errorCalls).toHaveLength(6);
    });

    test('default config uses 5 max per 10 second window', () => {
      const mockLogger = createMockLogger();
      const rateLimitedLogger = new RateLimitedLogger({
        baseLogger: mockLogger,
      });

      // Send exactly 6 errors
      for (let i = 0; i < 6; i++) {
        rateLimitedLogger.error('test-key', { i }, `Error ${i}`);
      }

      // Only 5 should be logged (default maxPerWindow)
      expect(mockLogger.errorCalls).toHaveLength(5);
    });
  });

  // ============================================================
  // UAT 2: Rate-Limiting Is Per-Client
  // ============================================================
  describe('UAT 2: Rate-Limiting Is Per-Client', () => {
    /**
     * Expected behavior:
     * If two different clients send invalid messages, each gets their own rate limit bucket.
     * One bad client cannot suppress error logging for other clients.
     */

    test('should track each client independently with separate log limits', () => {
      const mockLogger = createMockLogger();
      const rateLimitedLogger = new RateLimitedLogger({
        maxPerWindow: 3,
        windowMs: 10000,
        baseLogger: mockLogger,
      });

      const client1Key = 'invalid-message:client-1';
      const client2Key = 'invalid-message:client-2';

      // Client 1 sends 5 errors (3 logged, 2 suppressed)
      for (let i = 0; i < 5; i++) {
        rateLimitedLogger.error(client1Key, { clientId: 'client-1', i }, `Error from client-1`);
      }

      // Client 2 sends 5 errors (3 logged, 2 suppressed)
      for (let i = 0; i < 5; i++) {
        rateLimitedLogger.error(client2Key, { clientId: 'client-2', i }, `Error from client-2`);
      }

      // Both clients should be tracked independently
      expect(rateLimitedLogger.getTrackedKeyCount()).toBe(2);

      // Total logged: 3 from client-1 + 3 from client-2 = 6
      expect(mockLogger.errorCalls).toHaveLength(6);

      // Verify we have logs from both clients
      const client1Logs = mockLogger.errorCalls.filter(c => (c.obj as any).clientId === 'client-1');
      const client2Logs = mockLogger.errorCalls.filter(c => (c.obj as any).clientId === 'client-2');
      expect(client1Logs).toHaveLength(3);
      expect(client2Logs).toHaveLength(3);
    });

    test('bad client exhausting limit does not affect other clients', () => {
      const mockLogger = createMockLogger();
      const rateLimitedLogger = new RateLimitedLogger({
        maxPerWindow: 2,
        windowMs: 10000,
        baseLogger: mockLogger,
      });

      // Bad client exhausts their limit (sends 100 errors, only 2 logged)
      const badClientKey = 'invalid-message:bad-client';
      for (let i = 0; i < 100; i++) {
        rateLimitedLogger.error(badClientKey, { clientId: 'bad-client', i }, `Spam error`);
      }

      // Verify bad client only got 2 logs
      expect(mockLogger.errorCalls).toHaveLength(2);

      // Good client should still be able to log normally
      const goodClientKey = 'invalid-message:good-client';
      rateLimitedLogger.error(goodClientKey, { clientId: 'good-client' }, `Legitimate error`);

      // Good client's error should be logged (total now 3: 2 from bad + 1 from good)
      expect(mockLogger.errorCalls).toHaveLength(3);

      // Verify the last log is from the good client
      const lastLog = mockLogger.errorCalls[mockLogger.errorCalls.length - 1];
      expect((lastLog.obj as any).clientId).toBe('good-client');

      // Both clients tracked
      expect(rateLimitedLogger.getTrackedKeyCount()).toBe(2);
    });
  });

  // ============================================================
  // UAT 3: Production Startup Fails Without JWT_SECRET
  // ============================================================
  describe('UAT 3: Production Startup Fails Without JWT_SECRET', () => {
    /**
     * Expected behavior:
     * Running the server with NODE_ENV=production and no JWT_SECRET set
     * causes the server to refuse to start with a clear error message
     * mentioning JWT_SECRET is required.
     */

    const originalEnv = process.env.NODE_ENV;

    afterEach(() => {
      process.env.NODE_ENV = originalEnv;
    });

    test('should throw error in production without JWT_SECRET', () => {
      process.env.NODE_ENV = 'production';

      expect(() => {
        validateJwtSecret(undefined, undefined);
      }).toThrow('JWT_SECRET is required in production mode');
    });

    test('error message should be clear and actionable', () => {
      process.env.NODE_ENV = 'production';

      try {
        validateJwtSecret(undefined, undefined);
        fail('Should have thrown an error');
      } catch (error: any) {
        // Should mention it's a security error
        expect(error.message).toContain('SECURITY ERROR');
        // Should mention JWT_SECRET
        expect(error.message).toContain('JWT_SECRET');
        // Should mention production mode
        expect(error.message).toContain('production');
        // Should provide a hint on how to generate a secret
        expect(error.message).toContain('openssl rand -base64 32');
      }
    });
  });

  // ============================================================
  // UAT 4: Production Startup Fails With Default JWT Secret
  // ============================================================
  describe('UAT 4: Production Startup Fails With Default JWT Secret', () => {
    /**
     * Expected behavior:
     * Running the server with NODE_ENV=production and JWT_SECRET="topgun-secret-dev"
     * (the default) causes the server to refuse to start with a clear error message
     * about not using default secrets in production.
     */

    const originalEnv = process.env.NODE_ENV;

    afterEach(() => {
      process.env.NODE_ENV = originalEnv;
    });

    test('should throw error when using default secret in production', () => {
      process.env.NODE_ENV = 'production';

      expect(() => {
        validateJwtSecret('topgun-secret-dev', undefined);
      }).toThrow('Default JWT_SECRET cannot be used in production mode');
    });

    test('should throw error when default secret comes from env var', () => {
      process.env.NODE_ENV = 'production';

      expect(() => {
        validateJwtSecret(undefined, 'topgun-secret-dev');
      }).toThrow('Default JWT_SECRET cannot be used in production mode');
    });

    test('error message explains why default is insecure', () => {
      process.env.NODE_ENV = 'production';

      try {
        validateJwtSecret(DEFAULT_JWT_SECRET, undefined);
        fail('Should have thrown an error');
      } catch (error: any) {
        // Should explain the secret is publicly known
        expect(error.message).toContain('publicly known');
        // Should mention insecurity
        expect(error.message).toContain('insecure');
        // Should suggest generating a new secret
        expect(error.message).toContain('openssl rand -base64 32');
      }
    });

    test('default secret constant matches expected value', () => {
      expect(DEFAULT_JWT_SECRET).toBe('topgun-secret-dev');
    });
  });

  // ============================================================
  // UAT 5: Development Mode Allows Default Secret
  // ============================================================
  describe('UAT 5: Development Mode Allows Default Secret', () => {
    /**
     * Expected behavior:
     * Running the server without NODE_ENV=production (or with NODE_ENV=development)
     * starts successfully even without explicit JWT_SECRET, using the default for convenience.
     */

    const originalEnv = process.env.NODE_ENV;

    afterEach(() => {
      process.env.NODE_ENV = originalEnv;
    });

    test('should allow default secret in development mode', () => {
      process.env.NODE_ENV = 'development';

      const result = validateJwtSecret(undefined, undefined);
      expect(result).toBe('topgun-secret-dev');
    });

    test('should allow default secret in test mode', () => {
      process.env.NODE_ENV = 'test';

      const result = validateJwtSecret(undefined, undefined);
      expect(result).toBe('topgun-secret-dev');
    });

    test('should allow default secret when NODE_ENV is not set', () => {
      delete process.env.NODE_ENV;

      const result = validateJwtSecret(undefined, undefined);
      expect(result).toBe('topgun-secret-dev');
    });

    test('should allow explicit default secret in development', () => {
      process.env.NODE_ENV = 'development';

      const result = validateJwtSecret('topgun-secret-dev', undefined);
      expect(result).toBe('topgun-secret-dev');
    });

    test('should allow custom secret in development', () => {
      process.env.NODE_ENV = 'development';

      const result = validateJwtSecret('my-custom-secret', undefined);
      expect(result).toBe('my-custom-secret');
    });
  });

  // ============================================================
  // UAT 6: HLC Strict Mode Rejects Large Drift
  // ============================================================
  describe('UAT 6: HLC Strict Mode Rejects Large Drift', () => {
    /**
     * Expected behavior:
     * When creating an HLC with `{ strictMode: true, maxDriftMs: 1000 }`
     * and calling update() with a timestamp more than 1 second in the future,
     * it throws an error with a message showing the drift amount and threshold.
     */

    beforeEach(() => {
      jest.restoreAllMocks();
    });

    test('should throw error when drift exceeds threshold in strict mode', () => {
      const currentTime = 1000000;
      jest.spyOn(Date, 'now').mockImplementation(() => currentTime);

      const strictHlc = new HLC('strict-node', {
        strictMode: true,
        maxDriftMs: 1000, // 1 second threshold
      });

      const futureTimestamp: Timestamp = {
        millis: currentTime + 5000, // 5 seconds in the future
        counter: 0,
        nodeId: 'remote-node',
      };

      expect(() => {
        strictHlc.update(futureTimestamp);
      }).toThrow('Clock drift detected');
    });

    test('error message should show drift amount and threshold', () => {
      const currentTime = 1000000;
      jest.spyOn(Date, 'now').mockImplementation(() => currentTime);

      const strictHlc = new HLC('strict-node', {
        strictMode: true,
        maxDriftMs: 1000,
      });

      const futureTimestamp: Timestamp = {
        millis: currentTime + 5000, // 5 seconds ahead
        counter: 0,
        nodeId: 'remote-node',
      };

      try {
        strictHlc.update(futureTimestamp);
        fail('Should have thrown an error');
      } catch (error: any) {
        // Should show the drift amount
        expect(error.message).toContain('5000ms');
        // Should show the threshold
        expect(error.message).toContain('1000ms');
      }
    });

    test('should accept timestamps within threshold in strict mode', () => {
      const currentTime = 1000000;
      jest.spyOn(Date, 'now').mockImplementation(() => currentTime);

      const strictHlc = new HLC('strict-node', {
        strictMode: true,
        maxDriftMs: 5000, // 5 second threshold
      });

      const withinThreshold: Timestamp = {
        millis: currentTime + 3000, // 3 seconds ahead, within 5s threshold
        counter: 0,
        nodeId: 'remote-node',
      };

      // Should not throw
      expect(() => {
        strictHlc.update(withinThreshold);
      }).not.toThrow();
    });

    test('should use default maxDriftMs of 60000 when not specified', () => {
      const currentTime = 1000000;
      jest.spyOn(Date, 'now').mockImplementation(() => currentTime);

      const strictHlc = new HLC('strict-node', { strictMode: true });

      // 50 seconds ahead - within default 60s threshold
      const withinDefault: Timestamp = {
        millis: currentTime + 50000,
        counter: 0,
        nodeId: 'remote',
      };
      expect(() => strictHlc.update(withinDefault)).not.toThrow();

      // 70 seconds ahead - exceeds default 60s threshold
      const exceedsDefault: Timestamp = {
        millis: currentTime + 70000,
        counter: 0,
        nodeId: 'remote',
      };
      expect(() => strictHlc.update(exceedsDefault)).toThrow('Clock drift detected');
    });
  });

  // ============================================================
  // UAT 7: HLC Default Mode Warns But Accepts Large Drift
  // ============================================================
  describe('UAT 7: HLC Default Mode Warns But Accepts Large Drift', () => {
    /**
     * Expected behavior:
     * When creating an HLC without strict mode (default) and calling update()
     * with a future timestamp, it logs a warning but does NOT throw an error.
     * The HLC continues to function.
     */

    beforeEach(() => {
      jest.restoreAllMocks();
    });

    test('should warn but not throw when drift detected in default mode', () => {
      const consoleWarnSpy = jest.spyOn(console, 'warn').mockImplementation();
      const currentTime = 1000000;
      jest.spyOn(Date, 'now').mockImplementation(() => currentTime);

      const permissiveHlc = new HLC('permissive-node'); // strictMode defaults to false

      const futureTimestamp: Timestamp = {
        millis: currentTime + 100000, // 100 seconds in the future
        counter: 0,
        nodeId: 'remote-node',
      };

      // Should NOT throw
      expect(() => {
        permissiveHlc.update(futureTimestamp);
      }).not.toThrow();

      // Should have logged a warning
      expect(consoleWarnSpy).toHaveBeenCalled();
      expect(consoleWarnSpy).toHaveBeenCalledWith(expect.stringContaining('Clock drift detected'));

      consoleWarnSpy.mockRestore();
    });

    test('HLC continues to function after accepting drifted timestamp', () => {
      jest.spyOn(console, 'warn').mockImplementation(); // Suppress warning
      const currentTime = 1000000;
      jest.spyOn(Date, 'now').mockImplementation(() => currentTime);

      const hlc = new HLC('test-node');

      const futureTimestamp: Timestamp = {
        millis: currentTime + 100000,
        counter: 0,
        nodeId: 'remote-node',
      };

      hlc.update(futureTimestamp);

      // HLC should continue to function and produce valid timestamps
      const ts = hlc.now();
      expect(ts.millis).toBe(currentTime + 100000);
      expect(ts.nodeId).toBe('test-node');
      expect(typeof ts.counter).toBe('number');
    });

    test('default mode is strictMode: false', () => {
      const hlc = new HLC('test-node');
      expect(hlc.getStrictMode).toBe(false);
    });

    test('warning includes drift details', () => {
      const consoleWarnSpy = jest.spyOn(console, 'warn').mockImplementation();
      const currentTime = 1000000;
      jest.spyOn(Date, 'now').mockImplementation(() => currentTime);

      const hlc = new HLC('test-node');

      const futureTimestamp: Timestamp = {
        millis: currentTime + 100000,
        counter: 0,
        nodeId: 'remote-node',
      };

      hlc.update(futureTimestamp);

      // Warning should mention the drift amount
      const warnCall = consoleWarnSpy.mock.calls[0][0];
      expect(warnCall).toContain('Clock drift');

      consoleWarnSpy.mockRestore();
    });
  });
});
