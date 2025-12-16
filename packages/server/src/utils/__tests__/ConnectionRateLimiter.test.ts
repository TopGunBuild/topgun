import { ConnectionRateLimiter, RateLimiterConfig } from '../ConnectionRateLimiter';

describe('ConnectionRateLimiter', () => {
    let rateLimiter: ConnectionRateLimiter;

    beforeEach(() => {
        rateLimiter = new ConnectionRateLimiter({
            maxConnectionsPerSecond: 10,
            maxPendingConnections: 5,
            cooldownMs: 1000,
        });
    });

    afterEach(() => {
        rateLimiter.reset();
    });

    describe('shouldAccept', () => {
        it('should accept connections under rate limit', () => {
            // First 10 connections should be accepted
            for (let i = 0; i < 10; i++) {
                expect(rateLimiter.shouldAccept()).toBe(true);
                rateLimiter.onConnectionAttempt();
                rateLimiter.onConnectionEstablished(); // Complete handshake immediately
            }
        });

        it('should reject connections over rate limit', () => {
            // Accept 10 connections (the limit)
            for (let i = 0; i < 10; i++) {
                expect(rateLimiter.shouldAccept()).toBe(true);
                rateLimiter.onConnectionAttempt();
                rateLimiter.onConnectionEstablished();
            }

            // 11th connection should be rejected
            expect(rateLimiter.shouldAccept()).toBe(false);
        });

        it('should reject when pending connections limit is exceeded', () => {
            // Create 5 pending connections (the limit)
            for (let i = 0; i < 5; i++) {
                expect(rateLimiter.shouldAccept()).toBe(true);
                rateLimiter.onConnectionAttempt();
                // Don't call onConnectionEstablished - leave them pending
            }

            // 6th connection should be rejected due to pending limit
            expect(rateLimiter.shouldAccept()).toBe(false);
        });

        it('should accept after pending connections complete', () => {
            // Create 5 pending connections
            for (let i = 0; i < 5; i++) {
                rateLimiter.onConnectionAttempt();
            }

            // Should be rejected
            expect(rateLimiter.shouldAccept()).toBe(false);

            // Complete one pending connection
            rateLimiter.onConnectionEstablished();

            // Now should accept
            expect(rateLimiter.shouldAccept()).toBe(true);
        });
    });

    describe('window reset', () => {
        it('should reset counter after cooldown period', async () => {
            // Use a short cooldown for testing
            const shortCooldownLimiter = new ConnectionRateLimiter({
                maxConnectionsPerSecond: 5,
                maxPendingConnections: 100,
                cooldownMs: 50, // 50ms cooldown
            });

            // Exhaust the rate limit
            for (let i = 0; i < 5; i++) {
                shortCooldownLimiter.onConnectionAttempt();
                shortCooldownLimiter.onConnectionEstablished();
            }

            // Should be rejected
            expect(shortCooldownLimiter.shouldAccept()).toBe(false);

            // Wait for cooldown
            await new Promise(resolve => setTimeout(resolve, 60));

            // Should be accepted again
            expect(shortCooldownLimiter.shouldAccept()).toBe(true);
        });
    });

    describe('pending connection tracking', () => {
        it('should track pending connections correctly', () => {
            rateLimiter.onConnectionAttempt();
            rateLimiter.onConnectionAttempt();
            rateLimiter.onConnectionAttempt();

            const stats = rateLimiter.getStats();
            expect(stats.pendingConnections).toBe(3);
        });

        it('should decrease pending count on established', () => {
            rateLimiter.onConnectionAttempt();
            rateLimiter.onConnectionAttempt();
            rateLimiter.onConnectionEstablished();

            const stats = rateLimiter.getStats();
            expect(stats.pendingConnections).toBe(1);
            expect(stats.totalConnections).toBe(1);
        });

        it('should decrease pending count on failed', () => {
            rateLimiter.onConnectionAttempt();
            rateLimiter.onConnectionAttempt();
            rateLimiter.onPendingConnectionFailed();

            const stats = rateLimiter.getStats();
            expect(stats.pendingConnections).toBe(1);
        });

        it('should not go below zero pending connections', () => {
            rateLimiter.onPendingConnectionFailed();
            rateLimiter.onPendingConnectionFailed();

            const stats = rateLimiter.getStats();
            expect(stats.pendingConnections).toBe(0);
        });
    });

    describe('getStats', () => {
        it('should return correct stats', () => {
            rateLimiter.onConnectionAttempt();
            rateLimiter.onConnectionEstablished();
            rateLimiter.onConnectionAttempt();
            rateLimiter.onConnectionRejected();

            const stats = rateLimiter.getStats();
            expect(stats.totalConnections).toBe(1);
            expect(stats.pendingConnections).toBe(1);
            expect(stats.totalRejected).toBe(1);
        });
    });

    describe('reset', () => {
        it('should reset all counters', () => {
            rateLimiter.onConnectionAttempt();
            rateLimiter.onConnectionEstablished();
            rateLimiter.onConnectionRejected();

            rateLimiter.reset();

            const stats = rateLimiter.getStats();
            expect(stats.totalConnections).toBe(0);
            expect(stats.pendingConnections).toBe(0);
            expect(stats.totalRejected).toBe(0);
        });
    });

    describe('updateConfig', () => {
        it('should allow runtime config updates', () => {
            // Initial config allows 10 per second
            for (let i = 0; i < 10; i++) {
                rateLimiter.onConnectionAttempt();
                rateLimiter.onConnectionEstablished();
            }
            expect(rateLimiter.shouldAccept()).toBe(false);

            // Update to allow 20 per second
            rateLimiter.updateConfig({ maxConnectionsPerSecond: 20 });

            // Now should accept
            expect(rateLimiter.shouldAccept()).toBe(true);
        });
    });

    describe('default config', () => {
        it('should use default config when not provided', () => {
            const defaultLimiter = new ConnectionRateLimiter();

            // Default is 100 connections per second
            for (let i = 0; i < 100; i++) {
                expect(defaultLimiter.shouldAccept()).toBe(true);
                defaultLimiter.onConnectionAttempt();
                defaultLimiter.onConnectionEstablished();
            }

            // 101st should be rejected
            expect(defaultLimiter.shouldAccept()).toBe(false);
        });
    });

    describe('concurrent access simulation', () => {
        it('should handle rapid connection attempts', () => {
            const results: boolean[] = [];

            // Simulate 20 rapid connection attempts
            for (let i = 0; i < 20; i++) {
                const accepted = rateLimiter.shouldAccept();
                results.push(accepted);
                if (accepted) {
                    rateLimiter.onConnectionAttempt();
                    // Some complete immediately, some stay pending
                    if (i % 2 === 0) {
                        rateLimiter.onConnectionEstablished();
                    }
                } else {
                    rateLimiter.onConnectionRejected();
                }
            }

            // First 5 should be accepted (pending limit)
            // Then pending fills up and rest rejected
            const acceptedCount = results.filter(r => r).length;
            expect(acceptedCount).toBeLessThanOrEqual(10); // Rate limit
        });
    });
});
