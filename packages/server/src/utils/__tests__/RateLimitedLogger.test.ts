import { RateLimitedLogger } from '../RateLimitedLogger';

// Mock the logger module
jest.mock('../logger', () => ({
    logger: {
        warn: jest.fn(),
        error: jest.fn(),
    },
}));

import { logger } from '../logger';

const mockLogger = logger as jest.Mocked<typeof logger>;

describe('RateLimitedLogger', () => {
    beforeEach(() => {
        jest.clearAllMocks();
        jest.useFakeTimers();
    });

    afterEach(() => {
        jest.useRealTimers();
    });

    describe('basic throttling', () => {
        it('should allow logs up to maxPerWindow limit', () => {
            const rateLimitedLogger = new RateLimitedLogger({ maxPerWindow: 3, windowMs: 10000 });

            // Call error() 3 times with same key
            rateLimitedLogger.error('test-key', { data: 1 }, 'Error message 1');
            rateLimitedLogger.error('test-key', { data: 2 }, 'Error message 2');
            rateLimitedLogger.error('test-key', { data: 3 }, 'Error message 3');

            // All 3 calls should be logged
            expect(mockLogger.error).toHaveBeenCalledTimes(3);
            expect(mockLogger.error).toHaveBeenNthCalledWith(1, { data: 1 }, 'Error message 1');
            expect(mockLogger.error).toHaveBeenNthCalledWith(2, { data: 2 }, 'Error message 2');
            expect(mockLogger.error).toHaveBeenNthCalledWith(3, { data: 3 }, 'Error message 3');
        });

        it('should suppress logs exceeding limit', () => {
            const rateLimitedLogger = new RateLimitedLogger({ maxPerWindow: 2, windowMs: 1000 });

            // Call error() 5 times with same key
            for (let i = 0; i < 5; i++) {
                rateLimitedLogger.error('test-key', { index: i }, `Error ${i}`);
            }

            // Only 2 calls should be made to baseLogger.error
            expect(mockLogger.error).toHaveBeenCalledTimes(2);
            expect(mockLogger.error).toHaveBeenNthCalledWith(1, { index: 0 }, 'Error 0');
            expect(mockLogger.error).toHaveBeenNthCalledWith(2, { index: 1 }, 'Error 1');
        });

        it('should track different keys independently', () => {
            const rateLimitedLogger = new RateLimitedLogger({ maxPerWindow: 2, windowMs: 10000 });

            // Call error() 3 times with key "a"
            rateLimitedLogger.error('key-a', { key: 'a', index: 0 }, 'Error a-0');
            rateLimitedLogger.error('key-a', { key: 'a', index: 1 }, 'Error a-1');
            rateLimitedLogger.error('key-a', { key: 'a', index: 2 }, 'Error a-2'); // Suppressed

            // Call error() 3 times with key "b"
            rateLimitedLogger.error('key-b', { key: 'b', index: 0 }, 'Error b-0');
            rateLimitedLogger.error('key-b', { key: 'b', index: 1 }, 'Error b-1');
            rateLimitedLogger.error('key-b', { key: 'b', index: 2 }, 'Error b-2'); // Suppressed

            // Should have 4 total calls (2 for each key)
            expect(mockLogger.error).toHaveBeenCalledTimes(4);
        });
    });

    describe('window reset', () => {
        it('should reset window after windowMs expires', () => {
            const rateLimitedLogger = new RateLimitedLogger({ windowMs: 100, maxPerWindow: 1 });

            // First call - should be logged
            rateLimitedLogger.error('test-key', { call: 1 }, 'Error 1');
            expect(mockLogger.error).toHaveBeenCalledTimes(1);

            // Second call immediately - should be suppressed
            rateLimitedLogger.error('test-key', { call: 2 }, 'Error 2');
            expect(mockLogger.error).toHaveBeenCalledTimes(1); // Still 1

            // Advance time past window
            jest.advanceTimersByTime(150);

            // Third call after window reset - should be logged (plus suppression summary)
            rateLimitedLogger.error('test-key', { call: 3 }, 'Error 3');

            // Should have 2 error calls total (1 initial, 1 after reset)
            expect(mockLogger.error).toHaveBeenCalledTimes(2);
            expect(mockLogger.error).toHaveBeenLastCalledWith({ call: 3 }, 'Error 3');
        });

        it('should log suppression summary when window resets', () => {
            const rateLimitedLogger = new RateLimitedLogger({ windowMs: 100, maxPerWindow: 1 });

            // First call - logged
            rateLimitedLogger.error('test-key', { call: 1 }, 'Error 1');

            // Next 2 calls - suppressed
            rateLimitedLogger.error('test-key', { call: 2 }, 'Error 2');
            rateLimitedLogger.error('test-key', { call: 3 }, 'Error 3');

            expect(mockLogger.error).toHaveBeenCalledTimes(1);
            expect(mockLogger.warn).not.toHaveBeenCalled();

            // Advance time past window
            jest.advanceTimersByTime(150);

            // Next call triggers window reset and suppression summary
            rateLimitedLogger.error('test-key', { call: 4 }, 'Error 4');

            // Should have warning with suppression count
            expect(mockLogger.warn).toHaveBeenCalledTimes(1);
            expect(mockLogger.warn).toHaveBeenCalledWith(
                expect.objectContaining({ key: 'test-key', suppressedCount: 2 }),
                expect.stringContaining('suppressed 2 messages')
            );

            // Should have 2 error calls
            expect(mockLogger.error).toHaveBeenCalledTimes(2);
        });
    });

    describe('warn method', () => {
        it('should support warn method with same throttling behavior as error', () => {
            const rateLimitedLogger = new RateLimitedLogger({ maxPerWindow: 2, windowMs: 10000 });

            // Call warn() 4 times with same key
            rateLimitedLogger.warn('warn-key', { index: 0 }, 'Warning 0');
            rateLimitedLogger.warn('warn-key', { index: 1 }, 'Warning 1');
            rateLimitedLogger.warn('warn-key', { index: 2 }, 'Warning 2'); // Suppressed
            rateLimitedLogger.warn('warn-key', { index: 3 }, 'Warning 3'); // Suppressed

            // Only 2 calls should be made to baseLogger.warn
            expect(mockLogger.warn).toHaveBeenCalledTimes(2);
            expect(mockLogger.warn).toHaveBeenNthCalledWith(1, { index: 0 }, 'Warning 0');
            expect(mockLogger.warn).toHaveBeenNthCalledWith(2, { index: 1 }, 'Warning 1');
        });
    });

    describe('cleanup', () => {
        it('should remove stale entries on cleanup', () => {
            const rateLimitedLogger = new RateLimitedLogger({ windowMs: 100, maxPerWindow: 1 });

            // Add some entries
            rateLimitedLogger.error('key-1', {}, 'Error 1');
            rateLimitedLogger.error('key-2', {}, 'Error 2');

            expect(rateLimitedLogger.getTrackedKeyCount()).toBe(2);

            // Advance time past cleanup threshold (default: 5 * windowMs = 500ms)
            jest.advanceTimersByTime(600);

            // Run cleanup
            rateLimitedLogger.cleanup();

            expect(rateLimitedLogger.getTrackedKeyCount()).toBe(0);
        });

        it('should emit suppression summary for cleaned up entries that had suppressions', () => {
            const rateLimitedLogger = new RateLimitedLogger({ windowMs: 100, maxPerWindow: 1 });

            // Add entry with suppressions
            rateLimitedLogger.error('key-1', {}, 'Error 1');
            rateLimitedLogger.error('key-1', {}, 'Error 2'); // Suppressed
            rateLimitedLogger.error('key-1', {}, 'Error 3'); // Suppressed

            jest.clearAllMocks();

            // Advance time past cleanup threshold
            jest.advanceTimersByTime(600);

            // Run cleanup
            rateLimitedLogger.cleanup();

            // Should emit suppression summary
            expect(mockLogger.warn).toHaveBeenCalledWith(
                expect.objectContaining({ key: 'key-1', suppressedCount: 2 }),
                expect.stringContaining('cleanup')
            );
        });
    });

    describe('default config', () => {
        it('should use default config when not provided', () => {
            const rateLimitedLogger = new RateLimitedLogger();

            // Default is maxPerWindow: 5
            for (let i = 0; i < 5; i++) {
                rateLimitedLogger.error('test-key', { index: i }, `Error ${i}`);
            }

            // All 5 should be logged
            expect(mockLogger.error).toHaveBeenCalledTimes(5);

            // 6th should be suppressed
            rateLimitedLogger.error('test-key', { index: 5 }, 'Error 5');
            expect(mockLogger.error).toHaveBeenCalledTimes(5);
        });
    });
});
