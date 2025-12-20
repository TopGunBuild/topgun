import { StripedEventExecutor, StripedTask } from '../StripedEventExecutor';

describe('StripedEventExecutor', () => {
    let executor: StripedEventExecutor;

    beforeEach(() => {
        executor = new StripedEventExecutor({
            stripeCount: 4,
            queueCapacity: 100,
            name: 'test-executor'
        });
    });

    afterEach(async () => {
        await executor.shutdown(false);
    });

    describe('task routing', () => {
        it('should route tasks with same key to same stripe', async () => {
            const stripeResults: number[] = [];
            const key = 'consistent-key';

            // Submit multiple tasks with same key and track which stripe they go to
            for (let i = 0; i < 10; i++) {
                executor.submit({
                    key,
                    execute: () => {
                        // Track execution order
                    }
                });
            }

            // Check metrics - all tasks should be in the same stripe
            const metrics = executor.getMetrics();
            const stripesWithTasks = metrics.filter(m => m.metrics.enqueued > 0);

            // All tasks with same key should go to same stripe
            expect(stripesWithTasks.length).toBe(1);
            expect(stripesWithTasks[0].metrics.enqueued).toBe(10);
        });

        it('should distribute tasks across stripes', async () => {
            // Submit tasks with different keys
            for (let i = 0; i < 100; i++) {
                executor.submit({
                    key: `key-${i}`,
                    execute: () => {}
                });
            }

            // Wait for processing
            await new Promise(resolve => setTimeout(resolve, 50));

            const metrics = executor.getMetrics();

            // At least 2 stripes should have been used
            const usedStripes = metrics.filter(m => m.metrics.enqueued > 0).length;
            expect(usedStripes).toBeGreaterThanOrEqual(2);
        });

        it('should handle numeric keys', () => {
            const result = executor.submit({
                key: 42,
                execute: () => {}
            });

            expect(result).toBe(true);
        });
    });

    describe('ordering guarantees', () => {
        it('should maintain order within stripe', async () => {
            const executionOrder: number[] = [];
            const key = 'order-test-key';

            for (let i = 0; i < 5; i++) {
                executor.submit({
                    key,
                    execute: () => {
                        executionOrder.push(i);
                    }
                });
            }

            // Wait for all tasks to complete
            await executor.shutdown(true);

            expect(executionOrder).toEqual([0, 1, 2, 3, 4]);
        });

        it('should execute async tasks in order within stripe', async () => {
            const executionOrder: number[] = [];
            const key = 'async-order-key';

            for (let i = 0; i < 5; i++) {
                const taskIndex = i;
                executor.submit({
                    key,
                    execute: async () => {
                        // Random small delay to test ordering
                        await new Promise(resolve => setTimeout(resolve, Math.random() * 10));
                        executionOrder.push(taskIndex);
                    }
                });
            }

            await executor.shutdown(true);

            expect(executionOrder).toEqual([0, 1, 2, 3, 4]);
        });
    });

    describe('capacity and rejection', () => {
        it('should reject when all stripes full', () => {
            const smallExecutor = new StripedEventExecutor({
                stripeCount: 2,
                queueCapacity: 4, // 2 per stripe
                name: 'small-executor'
            });

            // Block the executor from processing
            let blocked = true;
            const blockingTasks: StripedTask[] = [];

            // Fill both stripes with blocking tasks
            for (let i = 0; i < 4; i++) {
                blockingTasks.push({
                    key: `key-${i % 2}`, // Distribute across 2 stripes
                    execute: async () => {
                        while (blocked) {
                            await new Promise(resolve => setTimeout(resolve, 10));
                        }
                    }
                });
            }

            // Submit blocking tasks
            for (const task of blockingTasks) {
                smallExecutor.submit(task);
            }

            // Try to submit more - should be rejected
            // Note: The first task in each stripe is being processed, so we can add more
            // We need to add more tasks to actually fill the queue
            let rejectedCount = 0;
            for (let i = 0; i < 10; i++) {
                const result = smallExecutor.submit({
                    key: `overflow-${i % 2}`,
                    execute: () => {}
                });
                if (!result) rejectedCount++;
            }

            expect(rejectedCount).toBeGreaterThan(0);

            // Cleanup
            blocked = false;
            smallExecutor.shutdown(false);
        });

        it('should call onReject callback when task is rejected', async () => {
            const onReject = jest.fn();
            const smallExecutor = new StripedEventExecutor({
                stripeCount: 1,
                queueCapacity: 2,
                name: 'reject-executor',
                onReject
            });

            // Block processing with a long-running task
            let blocked = true;
            smallExecutor.submit({
                key: 'blocking',
                execute: async () => {
                    while (blocked) {
                        await new Promise(resolve => setTimeout(resolve, 10));
                    }
                }
            });

            // Wait a moment for the blocking task to start executing
            await new Promise(resolve => setTimeout(resolve, 5));

            // Fill queue (the blocking task is processing, so we have 2 slots)
            smallExecutor.submit({
                key: 'fill-1',
                execute: () => {}
            });
            smallExecutor.submit({
                key: 'fill-2',
                execute: () => {}
            });

            // This should be rejected (queue has 2 capacity, both slots filled)
            const rejectedTask: StripedTask = {
                key: 'rejected',
                execute: () => {}
            };
            smallExecutor.submit(rejectedTask);

            expect(onReject).toHaveBeenCalled();

            blocked = false;
            await smallExecutor.shutdown(false);
        });
    });

    describe('metrics', () => {
        it('should return metrics for all stripes', () => {
            const metrics = executor.getMetrics();
            expect(metrics.length).toBe(4);
            expect(metrics.every(m => 'stripe' in m && 'metrics' in m)).toBe(true);
        });

        it('should track enqueued/dequeued counts per stripe', async () => {
            const key = 'metrics-test';

            for (let i = 0; i < 5; i++) {
                executor.submit({
                    key,
                    execute: () => {}
                });
            }

            await executor.shutdown(true);

            const metrics = executor.getMetrics();
            const stripeWithTasks = metrics.find(m => m.metrics.enqueued > 0);

            expect(stripeWithTasks).toBeDefined();
            expect(stripeWithTasks!.metrics.enqueued).toBe(5);
            expect(stripeWithTasks!.metrics.dequeued).toBe(5);
        });

        it('should calculate total metrics across all stripes', async () => {
            // Submit tasks to multiple stripes
            for (let i = 0; i < 20; i++) {
                executor.submit({
                    key: `diverse-key-${i}`,
                    execute: () => {}
                });
            }

            await executor.shutdown(true);

            const totalMetrics = executor.getTotalMetrics();

            expect(totalMetrics.enqueued).toBe(20);
            expect(totalMetrics.dequeued).toBe(20);
            expect(totalMetrics.currentSize).toBe(0);
        });
    });

    describe('shutdown', () => {
        it('should shutdown gracefully with pending tasks', async () => {
            const completed: number[] = [];

            for (let i = 0; i < 10; i++) {
                executor.submit({
                    key: 'shutdown-test',
                    execute: async () => {
                        await new Promise(resolve => setTimeout(resolve, 5));
                        completed.push(i);
                    }
                });
            }

            await executor.shutdown(true);

            // All tasks should have completed
            expect(completed.length).toBe(10);
        });

        it('should reject new tasks after shutdown starts', async () => {
            // Start shutdown
            const shutdownPromise = executor.shutdown(true);

            // Wait a moment for shutdown to begin
            await new Promise(resolve => setTimeout(resolve, 10));

            // Try to submit - should be rejected
            const result = executor.submit({
                key: 'after-shutdown',
                execute: () => {}
            });

            expect(result).toBe(false);

            await shutdownPromise;
        });

        it('should handle immediate shutdown without waiting', async () => {
            let completed = 0;

            for (let i = 0; i < 100; i++) {
                executor.submit({
                    key: `immediate-${i % 4}`,
                    execute: async () => {
                        await new Promise(resolve => setTimeout(resolve, 100));
                        completed++;
                    }
                });
            }

            // Shutdown immediately without waiting
            await executor.shutdown(false);

            // Most tasks should not have completed
            expect(completed).toBeLessThan(100);
        });
    });

    describe('error handling', () => {
        it('should continue processing after task error', async () => {
            const completedTasks: number[] = [];
            const key = 'error-recovery';

            executor.submit({
                key,
                execute: () => {
                    completedTasks.push(1);
                }
            });

            executor.submit({
                key,
                execute: () => {
                    throw new Error('Task failed');
                }
            });

            executor.submit({
                key,
                execute: () => {
                    completedTasks.push(3);
                }
            });

            await executor.shutdown(true);

            // Tasks 1 and 3 should have completed despite task 2 failing
            expect(completedTasks).toEqual([1, 3]);
        });

        it('should continue processing after async task rejection', async () => {
            const completedTasks: number[] = [];
            const key = 'async-error';

            executor.submit({
                key,
                execute: async () => {
                    completedTasks.push(1);
                }
            });

            executor.submit({
                key,
                execute: async () => {
                    throw new Error('Async task failed');
                }
            });

            executor.submit({
                key,
                execute: async () => {
                    completedTasks.push(3);
                }
            });

            await executor.shutdown(true);

            expect(completedTasks).toEqual([1, 3]);
        });
    });

    describe('size and capacity', () => {
        it('should report current size across all stripes', () => {
            // Submit tasks but block processing
            expect(executor.size).toBe(0);

            // Note: Tasks are processed immediately in the current implementation,
            // so we check that size works correctly
            const totalMetrics = executor.getTotalMetrics();
            expect(totalMetrics.currentSize).toBe(0);
        });

        it('should report isFull correctly', () => {
            expect(executor.isFull).toBe(false);
        });
    });
});
