import { RateLimitInterceptor } from '../interceptor/RateLimitInterceptor';
import { ServerOp, OpContext } from '../interceptor/IInterceptor';

describe('RateLimitInterceptor', () => {
    let interceptor: RateLimitInterceptor;

    // Helper to create a mock operation
    const createOp = (id: string = 'op-1'): ServerOp => ({
        mapName: 'test-map',
        key: 'test-key',
        opType: 'PUT',
        record: { value: 'test', timestamp: { millis: Date.now(), counter: 0, nodeId: 'node-1' } },
        id
    });

    // Helper to create a mock context
    const createContext = (clientId: string = 'client-1'): OpContext => ({
        clientId,
        isAuthenticated: true,
        fromCluster: false
    });

    beforeEach(() => {
        jest.useFakeTimers();
        interceptor = new RateLimitInterceptor({ windowMs: 1000, maxOps: 5 });
    });

    afterEach(() => {
        jest.useRealTimers();
    });

    describe('Basic behavior', () => {
        test('should allow requests within limit', async () => {
            const op = createOp();
            const context = createContext();

            // Make 5 requests (at limit)
            for (let i = 0; i < 5; i++) {
                const result = await interceptor.onBeforeOp(op, context);
                expect(result).toEqual(op);
            }
        });

        test('should block requests exceeding limit', async () => {
            const op = createOp();
            const context = createContext();

            // Make 5 requests (at limit)
            for (let i = 0; i < 5; i++) {
                await interceptor.onBeforeOp(op, context);
            }

            // 6th request should be blocked
            await expect(interceptor.onBeforeOp(op, context)).rejects.toThrow('Rate limit exceeded');
        });

        test('should reset counter after time window', async () => {
            const op = createOp();
            const context = createContext();

            // Exhaust the limit
            for (let i = 0; i < 5; i++) {
                await interceptor.onBeforeOp(op, context);
            }

            // 6th should fail
            await expect(interceptor.onBeforeOp(op, context)).rejects.toThrow('Rate limit exceeded');

            // Advance time past the window
            jest.advanceTimersByTime(1001);

            // Should be allowed again after reset
            const result = await interceptor.onBeforeOp(op, context);
            expect(result).toEqual(op);
        });
    });

    describe('Configuration', () => {
        test('should respect custom maxOps configuration', async () => {
            interceptor = new RateLimitInterceptor({ windowMs: 1000, maxOps: 3 });
            const op = createOp();
            const context = createContext();

            // 3 requests should pass
            for (let i = 0; i < 3; i++) {
                await interceptor.onBeforeOp(op, context);
            }

            // 4th should fail
            await expect(interceptor.onBeforeOp(op, context)).rejects.toThrow('Rate limit exceeded');
        });

        test('should respect custom windowMs configuration', async () => {
            interceptor = new RateLimitInterceptor({ windowMs: 5000, maxOps: 2 });
            const op = createOp();
            const context = createContext();

            // Exhaust limit
            await interceptor.onBeforeOp(op, context);
            await interceptor.onBeforeOp(op, context);
            await expect(interceptor.onBeforeOp(op, context)).rejects.toThrow('Rate limit exceeded');

            // Advance time but not past window
            jest.advanceTimersByTime(3000);

            // Should still be blocked
            await expect(interceptor.onBeforeOp(op, context)).rejects.toThrow('Rate limit exceeded');

            // Advance past window
            jest.advanceTimersByTime(2001);

            // Should be allowed now
            const result = await interceptor.onBeforeOp(op, context);
            expect(result).toEqual(op);
        });

        test('should use default configuration when not specified', async () => {
            interceptor = new RateLimitInterceptor();
            const op = createOp();
            const context = createContext();

            // Default is 50 ops per 1000ms
            for (let i = 0; i < 50; i++) {
                await interceptor.onBeforeOp(op, context);
            }

            await expect(interceptor.onBeforeOp(op, context)).rejects.toThrow('Rate limit exceeded');
        });
    });

    describe('Per-client rate limiting', () => {
        test('should isolate limits between different clients', async () => {
            const op = createOp();
            const context1 = createContext('client-1');
            const context2 = createContext('client-2');

            // Exhaust client-1's limit
            for (let i = 0; i < 5; i++) {
                await interceptor.onBeforeOp(op, context1);
            }
            await expect(interceptor.onBeforeOp(op, context1)).rejects.toThrow('Rate limit exceeded');

            // Client-2 should still have full quota
            for (let i = 0; i < 5; i++) {
                const result = await interceptor.onBeforeOp(op, context2);
                expect(result).toEqual(op);
            }
        });

        test('should track each client independently', async () => {
            const op = createOp();
            const clients = ['client-a', 'client-b', 'client-c'];

            // Each client makes 3 requests
            for (const clientId of clients) {
                const context = createContext(clientId);
                for (let i = 0; i < 3; i++) {
                    await interceptor.onBeforeOp(op, context);
                }
            }

            // Each client should still have 2 requests remaining
            for (const clientId of clients) {
                const context = createContext(clientId);
                await interceptor.onBeforeOp(op, context);
                await interceptor.onBeforeOp(op, context);
                await expect(interceptor.onBeforeOp(op, context)).rejects.toThrow('Rate limit exceeded');
            }
        });

        test('should clean up client data on disconnect', async () => {
            const op = createOp();
            const context = createContext('client-disconnect');

            // Make some requests
            await interceptor.onBeforeOp(op, context);
            await interceptor.onBeforeOp(op, context);

            // Disconnect
            await interceptor.onDisconnect(context);

            // After disconnect and reconnect, counter should be reset
            const result = await interceptor.onBeforeOp(op, context);
            expect(result).toEqual(op);

            // Verify fresh quota (should allow 5 requests, not 3)
            for (let i = 0; i < 4; i++) {
                await interceptor.onBeforeOp(op, context);
            }
            await expect(interceptor.onBeforeOp(op, context)).rejects.toThrow('Rate limit exceeded');
        });
    });

    describe('Edge cases', () => {
        test('first request always passes', async () => {
            const op = createOp();
            const context = createContext();

            const result = await interceptor.onBeforeOp(op, context);
            expect(result).toEqual(op);
        });

        test('should handle exact limit boundary correctly', async () => {
            interceptor = new RateLimitInterceptor({ windowMs: 1000, maxOps: 1 });
            const op = createOp();
            const context = createContext();

            // First request should pass
            const result = await interceptor.onBeforeOp(op, context);
            expect(result).toEqual(op);

            // Second request should fail
            await expect(interceptor.onBeforeOp(op, context)).rejects.toThrow('Rate limit exceeded');
        });

        test('should allow request immediately after window reset', async () => {
            const op = createOp();
            const context = createContext();

            // Exhaust limit
            for (let i = 0; i < 5; i++) {
                await interceptor.onBeforeOp(op, context);
            }
            await expect(interceptor.onBeforeOp(op, context)).rejects.toThrow('Rate limit exceeded');

            // Advance exactly to reset time
            jest.advanceTimersByTime(1001);

            // First request after reset should pass
            const result = await interceptor.onBeforeOp(op, context);
            expect(result).toEqual(op);
        });

        test('should work with very short time window', async () => {
            interceptor = new RateLimitInterceptor({ windowMs: 10, maxOps: 2 });
            const op = createOp();
            const context = createContext();

            // Exhaust limit
            await interceptor.onBeforeOp(op, context);
            await interceptor.onBeforeOp(op, context);
            await expect(interceptor.onBeforeOp(op, context)).rejects.toThrow('Rate limit exceeded');

            // Short wait
            jest.advanceTimersByTime(11);

            // Should work again
            const result = await interceptor.onBeforeOp(op, context);
            expect(result).toEqual(op);
        });

        test('should work with very large limit', async () => {
            interceptor = new RateLimitInterceptor({ windowMs: 1000, maxOps: 10000 });
            const op = createOp();
            const context = createContext();

            // Make many requests
            for (let i = 0; i < 10000; i++) {
                await interceptor.onBeforeOp(op, context);
            }

            // Next should fail
            await expect(interceptor.onBeforeOp(op, context)).rejects.toThrow('Rate limit exceeded');
        });

        test('should handle multiple time windows correctly', async () => {
            interceptor = new RateLimitInterceptor({ windowMs: 100, maxOps: 2 });
            const op = createOp();
            const context = createContext();

            // Window 1: use quota
            await interceptor.onBeforeOp(op, context);
            await interceptor.onBeforeOp(op, context);
            await expect(interceptor.onBeforeOp(op, context)).rejects.toThrow('Rate limit exceeded');

            // Window 2
            jest.advanceTimersByTime(101);
            await interceptor.onBeforeOp(op, context);
            await interceptor.onBeforeOp(op, context);
            await expect(interceptor.onBeforeOp(op, context)).rejects.toThrow('Rate limit exceeded');

            // Window 3
            jest.advanceTimersByTime(101);
            await interceptor.onBeforeOp(op, context);
            await interceptor.onBeforeOp(op, context);
            await expect(interceptor.onBeforeOp(op, context)).rejects.toThrow('Rate limit exceeded');
        });
    });

    describe('Error response', () => {
        test('should throw error with descriptive message on limit exceeded', async () => {
            interceptor = new RateLimitInterceptor({ windowMs: 1000, maxOps: 1 });
            const op = createOp();
            const context = createContext();

            await interceptor.onBeforeOp(op, context);

            try {
                await interceptor.onBeforeOp(op, context);
                fail('Expected error to be thrown');
            } catch (error: any) {
                expect(error).toBeInstanceOf(Error);
                expect(error.message).toBe('Rate limit exceeded');
            }
        });
    });

    describe('Interceptor interface', () => {
        test('should have correct name', () => {
            expect(interceptor.name).toBe('RateLimitInterceptor');
        });

        test('should return original op when within limit', async () => {
            const op = createOp('unique-op-id');
            const context = createContext();

            const result = await interceptor.onBeforeOp(op, context);

            expect(result).toBe(op);
            expect(result?.id).toBe('unique-op-id');
        });
    });
});
