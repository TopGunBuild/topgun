import { BackpressureRegulator, BackpressureConfig } from '../BackpressureRegulator';

describe('BackpressureRegulator', () => {
    let regulator: BackpressureRegulator;

    beforeEach(() => {
        regulator = new BackpressureRegulator({
            syncFrequency: 10,
            maxPendingOps: 5,
            backoffTimeoutMs: 100,
            enabled: true
        });
    });

    afterEach(() => {
        regulator.reset();
    });

    describe('shouldForceSync', () => {
        it('should force sync every N operations', () => {
            let syncCount = 0;

            // Call shouldForceSync 30 times
            for (let i = 0; i < 30; i++) {
                if (regulator.shouldForceSync()) {
                    syncCount++;
                }
            }

            // Should sync exactly 3 times (at 10, 20, 30)
            expect(syncCount).toBe(3);
        });

        it('should reset counter after sync', () => {
            // Call 10 times to trigger sync
            for (let i = 0; i < 10; i++) {
                regulator.shouldForceSync();
            }

            // Counter should be reset
            const stats = regulator.getStats();
            expect(stats.syncCounter).toBe(0);
        });

        it('should track sync forced total in stats', () => {
            // Trigger 2 syncs
            for (let i = 0; i < 20; i++) {
                regulator.shouldForceSync();
            }

            const stats = regulator.getStats();
            expect(stats.syncForcedTotal).toBe(2);
        });

        it('should be disabled when enabled=false', () => {
            const disabledRegulator = new BackpressureRegulator({
                syncFrequency: 10,
                enabled: false
            });

            // Even after 100 calls, should never force sync
            let syncCount = 0;
            for (let i = 0; i < 100; i++) {
                if (disabledRegulator.shouldForceSync()) {
                    syncCount++;
                }
            }

            expect(syncCount).toBe(0);
        });
    });

    describe('registerPending', () => {
        it('should allow pending up to maxPendingOps', () => {
            // Register 5 pending ops (maxPendingOps = 5)
            for (let i = 0; i < 5; i++) {
                expect(regulator.registerPending()).toBe(true);
            }

            expect(regulator.getPendingOps()).toBe(5);
        });

        it('should return false when at capacity', () => {
            // Fill to capacity
            for (let i = 0; i < 5; i++) {
                regulator.registerPending();
            }

            // Should reject new registrations
            expect(regulator.registerPending()).toBe(false);
            expect(regulator.getPendingOps()).toBe(5);
        });

        it('should always return true when disabled', () => {
            const disabledRegulator = new BackpressureRegulator({
                maxPendingOps: 1,
                enabled: false
            });

            // Should always succeed
            for (let i = 0; i < 100; i++) {
                expect(disabledRegulator.registerPending()).toBe(true);
            }
        });
    });

    describe('completePending', () => {
        it('should decrement pending ops', () => {
            regulator.registerPending();
            regulator.registerPending();
            regulator.registerPending();

            expect(regulator.getPendingOps()).toBe(3);

            regulator.completePending();
            expect(regulator.getPendingOps()).toBe(2);

            regulator.completePending();
            expect(regulator.getPendingOps()).toBe(1);
        });

        it('should not go below zero', () => {
            regulator.completePending();
            regulator.completePending();

            expect(regulator.getPendingOps()).toBe(0);
        });

        it('should allow new registrations after completion frees space', () => {
            // Fill to capacity
            for (let i = 0; i < 5; i++) {
                regulator.registerPending();
            }

            expect(regulator.registerPending()).toBe(false);

            // Complete one
            regulator.completePending();

            // Should be able to register again
            expect(regulator.registerPending()).toBe(true);
        });
    });

    describe('waitForCapacity', () => {
        it('should resolve immediately if under capacity', async () => {
            regulator.registerPending();
            regulator.registerPending();

            // Still under capacity (2/5)
            await expect(regulator.waitForCapacity()).resolves.toBeUndefined();
        });

        it('should block when at capacity', async () => {
            // Fill to capacity
            for (let i = 0; i < 5; i++) {
                regulator.registerPending();
            }

            // Create a promise that will wait
            const waitPromise = regulator.waitForCapacity();

            // Complete one pending op after a short delay
            setTimeout(() => {
                regulator.completePending();
            }, 10);

            // Should resolve once capacity becomes available
            await expect(waitPromise).resolves.toBeUndefined();
        });

        it('should timeout after backoffTimeoutMs', async () => {
            // Fill to capacity
            for (let i = 0; i < 5; i++) {
                regulator.registerPending();
            }

            // Wait should timeout after 100ms
            await expect(regulator.waitForCapacity()).rejects.toThrow('Backpressure timeout');

            // Should track timeout in stats
            const stats = regulator.getStats();
            expect(stats.timeoutsTotal).toBe(1);
        });

        it('should track waits in stats', async () => {
            // Fill to capacity
            for (let i = 0; i < 5; i++) {
                regulator.registerPending();
            }

            // Start waiting (will timeout)
            try {
                await regulator.waitForCapacity();
            } catch {
                // Expected timeout
            }

            const stats = regulator.getStats();
            expect(stats.waitsTotal).toBe(1);
        });

        it('should resolve immediately when disabled', async () => {
            const disabledRegulator = new BackpressureRegulator({
                maxPendingOps: 1,
                enabled: false
            });

            // Even though technically "at capacity", should resolve immediately
            for (let i = 0; i < 10; i++) {
                disabledRegulator.registerPending();
            }

            await expect(disabledRegulator.waitForCapacity()).resolves.toBeUndefined();
        });
    });

    describe('getStats', () => {
        it('should return correct utilization percent', () => {
            regulator.registerPending();
            regulator.registerPending();

            const stats = regulator.getStats();
            expect(stats.utilizationPercent).toBe(40); // 2/5 = 40%
        });

        it('should return 0 utilization when maxPendingOps is 0', () => {
            const zeroCapRegulator = new BackpressureRegulator({
                maxPendingOps: 0
            });

            const stats = zeroCapRegulator.getStats();
            expect(stats.utilizationPercent).toBe(0);
        });

        it('should track all metrics', () => {
            // Register some pending
            regulator.registerPending();
            regulator.registerPending();

            // Complete one
            regulator.completePending();

            // Force some syncs
            for (let i = 0; i < 20; i++) {
                regulator.shouldForceSync();
            }

            const stats = regulator.getStats();
            expect(stats.pendingOps).toBe(1);
            expect(stats.syncForcedTotal).toBe(2);
            expect(stats.utilizationPercent).toBe(20);
        });
    });

    describe('isAtCapacity', () => {
        it('should return true when at max capacity', () => {
            for (let i = 0; i < 5; i++) {
                regulator.registerPending();
            }

            expect(regulator.isAtCapacity()).toBe(true);
        });

        it('should return false when under capacity', () => {
            regulator.registerPending();
            regulator.registerPending();

            expect(regulator.isAtCapacity()).toBe(false);
        });

        it('should return false when disabled', () => {
            const disabledRegulator = new BackpressureRegulator({
                maxPendingOps: 1,
                enabled: false
            });

            disabledRegulator.registerPending();
            disabledRegulator.registerPending();

            expect(disabledRegulator.isAtCapacity()).toBe(false);
        });
    });

    describe('reset', () => {
        it('should reset all counters', () => {
            // Build up some state
            for (let i = 0; i < 20; i++) {
                regulator.shouldForceSync();
            }
            regulator.registerPending();
            regulator.registerPending();

            // Reset
            regulator.reset();

            const stats = regulator.getStats();
            expect(stats.pendingOps).toBe(0);
            expect(stats.syncCounter).toBe(0);
            expect(stats.syncForcedTotal).toBe(0);
            expect(stats.waitsTotal).toBe(0);
            expect(stats.timeoutsTotal).toBe(0);
        });

        it('should reject pending waiters', async () => {
            // Fill to capacity
            for (let i = 0; i < 5; i++) {
                regulator.registerPending();
            }

            // Start waiting
            const waitPromise = regulator.waitForCapacity();

            // Reset should reject the waiter
            regulator.reset();

            await expect(waitPromise).rejects.toThrow('BackpressureRegulator reset');
        });
    });

    describe('default configuration', () => {
        it('should use sensible defaults', () => {
            const defaultRegulator = new BackpressureRegulator();

            // Should be enabled by default
            expect(defaultRegulator.isEnabled()).toBe(true);

            // Default syncFrequency is 100
            let syncCount = 0;
            for (let i = 0; i < 100; i++) {
                if (defaultRegulator.shouldForceSync()) {
                    syncCount++;
                }
            }
            expect(syncCount).toBe(1);
        });
    });

    describe('randomization for high utilization', () => {
        it('should potentially force early sync under high load', () => {
            // This test is probabilistic, so we run multiple trials
            const highLoadRegulator = new BackpressureRegulator({
                syncFrequency: 1000, // Very high to avoid normal sync
                maxPendingOps: 10,
                enabled: true
            });

            // Put at 90% utilization
            for (let i = 0; i < 9; i++) {
                highLoadRegulator.registerPending();
            }

            // Run many trials to see if early sync ever triggers
            let earlySyncCount = 0;
            for (let trial = 0; trial < 1000; trial++) {
                // Reset sync counter without clearing utilization
                // Create fresh regulator at high utilization
                const testRegulator = new BackpressureRegulator({
                    syncFrequency: 1000,
                    maxPendingOps: 10,
                    enabled: true
                });
                for (let i = 0; i < 9; i++) {
                    testRegulator.registerPending();
                }

                if (testRegulator.shouldForceSync()) {
                    earlySyncCount++;
                }
            }

            // Should have some early syncs due to 10% probability at high utilization
            // With 1000 trials and 10% probability, we expect ~100 early syncs
            // Allow for statistical variance
            expect(earlySyncCount).toBeGreaterThan(50);
            expect(earlySyncCount).toBeLessThan(200);
        });
    });

    describe('concurrent waiters', () => {
        it('should wake up waiters one at a time', async () => {
            // Fill to capacity
            for (let i = 0; i < 5; i++) {
                regulator.registerPending();
            }

            const resolveOrder: number[] = [];

            // Create multiple waiters
            const wait1 = regulator.waitForCapacity().then(() => {
                resolveOrder.push(1);
            });
            const wait2 = regulator.waitForCapacity().then(() => {
                resolveOrder.push(2);
            });

            // Complete two ops (one at a time)
            regulator.completePending();
            await new Promise(resolve => setTimeout(resolve, 10));
            regulator.completePending();
            await new Promise(resolve => setTimeout(resolve, 10));

            // Both should have resolved in order
            await Promise.all([wait1, wait2]);
            expect(resolveOrder).toEqual([1, 2]);
        });
    });
});
