import {
    TaskletScheduler,
    Tasklet,
    ProgressState,
    FilterTasklet,
    MapTasklet,
    ForEachTasklet,
    ReduceTasklet,
} from '../index';

describe('TaskletScheduler', () => {
    let scheduler: TaskletScheduler;

    beforeEach(() => {
        scheduler = new TaskletScheduler({
            defaultTimeBudgetMs: 5,
            maxConcurrent: 10,
        });
    });

    afterEach(() => {
        scheduler.shutdown();
    });

    describe('Basic Operations', () => {
        it('should schedule and complete a simple tasklet', async () => {
            const tasklet: Tasklet<number> = {
                name: 'simple',
                call: () => 'DONE',
                getResult: () => 42,
            };

            const result = await scheduler.schedule(tasklet);
            expect(result).toBe(42);
        });

        it('should handle multi-iteration tasklets', async () => {
            let iterations = 0;
            const maxIterations = 5;

            const tasklet: Tasklet<number> = {
                name: 'multi-iteration',
                call: () => {
                    iterations++;
                    return iterations >= maxIterations ? 'DONE' : 'MADE_PROGRESS';
                },
                getResult: () => iterations,
            };

            const result = await scheduler.schedule(tasklet);
            expect(result).toBe(maxIterations);
        });

        it('should run multiple tasklets concurrently', async () => {
            const results: number[] = [];
            const createTasklet = (id: number): Tasklet<number> => ({
                name: `concurrent-${id}`,
                call: () => 'DONE',
                getResult: () => {
                    results.push(id);
                    return id;
                },
            });

            const promises = [
                scheduler.schedule(createTasklet(1)),
                scheduler.schedule(createTasklet(2)),
                scheduler.schedule(createTasklet(3)),
            ];

            const taskletResults = await Promise.all(promises);
            expect(taskletResults).toHaveLength(3);
            expect(results).toContain(1);
            expect(results).toContain(2);
            expect(results).toContain(3);
        });
    });

    describe('runSync', () => {
        it('should run tasklet synchronously', () => {
            let iterations = 0;
            const tasklet: Tasklet<number> = {
                name: 'sync',
                call: () => {
                    iterations++;
                    return iterations >= 3 ? 'DONE' : 'MADE_PROGRESS';
                },
                getResult: () => iterations,
            };

            const result = scheduler.runSync(tasklet);
            expect(result).toBe(3);
        });

        it('should throw if tasklet makes no progress', () => {
            const tasklet: Tasklet<void> = {
                name: 'stuck',
                call: () => 'NO_PROGRESS',
                getResult: () => undefined,
            };

            expect(() => scheduler.runSync(tasklet)).toThrow('made no progress');
        });
    });

    describe('Cancellation', () => {
        it('should cancel tasklet by name pattern', async () => {
            let cancelled = false;
            const tasklet: Tasklet<void> = {
                name: 'cancellable',
                call: () => 'MADE_PROGRESS',
                getResult: () => undefined,
                onCancel: () => { cancelled = true; },
            };

            const promise = scheduler.schedule(tasklet);

            // Wait for tasklet to start
            await new Promise(resolve => setImmediate(resolve));

            const count = scheduler.cancel('cancellable');
            expect(count).toBe(1);
            expect(cancelled).toBe(true);

            await expect(promise).rejects.toThrow('was cancelled');
        });

        it('should cancel all tasklets', async () => {
            const tasklet1: Tasklet<void> = {
                name: 'task1',
                call: () => 'MADE_PROGRESS',
                getResult: () => undefined,
            };
            const tasklet2: Tasklet<void> = {
                name: 'task2',
                call: () => 'MADE_PROGRESS',
                getResult: () => undefined,
            };

            const promise1 = scheduler.schedule(tasklet1);
            const promise2 = scheduler.schedule(tasklet2);

            await new Promise(resolve => setImmediate(resolve));

            const count = scheduler.cancelAll();
            expect(count).toBe(2);

            await expect(promise1).rejects.toThrow('was cancelled');
            await expect(promise2).rejects.toThrow('was cancelled');
        });
    });

    describe('Error Handling', () => {
        it('should reject if tasklet throws', async () => {
            const tasklet: Tasklet<void> = {
                name: 'throws',
                call: () => { throw new Error('Tasklet error'); },
                getResult: () => undefined,
            };

            await expect(scheduler.schedule(tasklet)).rejects.toThrow('Tasklet error');
        });

        it('should reject if max concurrent reached', async () => {
            const limitedScheduler = new TaskletScheduler({ maxConcurrent: 1 });

            const longTasklet: Tasklet<void> = {
                name: 'long',
                call: () => 'MADE_PROGRESS',
                getResult: () => undefined,
            };

            // Schedule first tasklet
            const promise1 = limitedScheduler.schedule(longTasklet);
            await new Promise(resolve => setImmediate(resolve));

            // Try to schedule second
            await expect(limitedScheduler.schedule(longTasklet))
                .rejects.toThrow('Max concurrent tasklets');

            limitedScheduler.cancelAll();
            await promise1.catch(() => {}); // Ignore cancellation
            limitedScheduler.shutdown();
        });

        it('should reject new tasklets after shutdown', async () => {
            scheduler.shutdown();

            const tasklet: Tasklet<void> = {
                name: 'post-shutdown',
                call: () => 'DONE',
                getResult: () => undefined,
            };

            await expect(scheduler.schedule(tasklet))
                .rejects.toThrow('shutting down');
        });
    });

    describe('Statistics', () => {
        it('should track tasklet statistics', async () => {
            const tasklet: Tasklet<number> = {
                name: 'tracked',
                call: () => 'DONE',
                getResult: () => 1,
            };

            await scheduler.schedule(tasklet);

            const stats = scheduler.getStats();
            expect(stats.totalScheduled).toBe(1);
            expect(stats.completedTasklets).toBe(1);
            expect(stats.activeTasklets).toBe(0);
        });

        it('should track multi-iteration statistics', async () => {
            let iterations = 0;
            const tasklet: Tasklet<number> = {
                name: 'multi',
                call: () => {
                    iterations++;
                    return iterations >= 3 ? 'DONE' : 'MADE_PROGRESS';
                },
                getResult: () => iterations,
            };

            await scheduler.schedule(tasklet);

            const stats = scheduler.getStats();
            expect(stats.totalIterations).toBeGreaterThanOrEqual(3);
            expect(stats.singleIterationCompletions).toBe(0);
        });

        it('should track single iteration completions', async () => {
            const tasklet: Tasklet<number> = {
                name: 'single',
                call: () => 'DONE',
                getResult: () => 1,
            };

            await scheduler.schedule(tasklet);

            const stats = scheduler.getStats();
            expect(stats.singleIterationCompletions).toBe(1);
        });

        it('should reset statistics', async () => {
            const tasklet: Tasklet<number> = {
                name: 'reset-test',
                call: () => 'DONE',
                getResult: () => 1,
            };

            await scheduler.schedule(tasklet);
            scheduler.resetStats();

            const stats = scheduler.getStats();
            expect(stats.totalScheduled).toBe(0);
            expect(stats.completedTasklets).toBe(0);
        });
    });

    describe('State', () => {
        it('should report running state', async () => {
            expect(scheduler.running).toBe(false);

            const tasklet: Tasklet<void> = {
                name: 'running-check',
                call: () => 'MADE_PROGRESS',
                getResult: () => undefined,
            };

            const promise = scheduler.schedule(tasklet);
            await new Promise(resolve => setImmediate(resolve));

            expect(scheduler.running).toBe(true);
            expect(scheduler.activeCount).toBe(1);

            scheduler.cancelAll();
            await promise.catch(() => {});

            // Wait for scheduler to stop
            await new Promise(resolve => setTimeout(resolve, 10));
            expect(scheduler.running).toBe(false);
        });
    });
});

describe('IteratorTasklets', () => {
    let scheduler: TaskletScheduler;

    beforeEach(() => {
        scheduler = new TaskletScheduler();
    });

    afterEach(() => {
        scheduler.shutdown();
    });

    describe('FilterTasklet', () => {
        it('should filter items matching predicate', async () => {
            const items = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
            const tasklet = new FilterTasklet(
                'filter-even',
                items[Symbol.iterator](),
                (n) => n % 2 === 0
            );

            const result = await scheduler.schedule(tasklet);
            expect(result).toEqual([2, 4, 6, 8, 10]);
        });

        it('should handle empty input', async () => {
            const items: number[] = [];
            const tasklet = new FilterTasklet(
                'filter-empty',
                items[Symbol.iterator](),
                () => true
            );

            const result = await scheduler.schedule(tasklet);
            expect(result).toEqual([]);
        });

        it('should handle large input with chunking', async () => {
            const items = Array.from({ length: 10000 }, (_, i) => i);
            const tasklet = new FilterTasklet(
                'filter-large',
                items[Symbol.iterator](),
                (n) => n % 100 === 0,
                { timeBudgetMs: 1, maxItemsPerIteration: 100 }
            );

            const result = await scheduler.schedule(tasklet);
            expect(result.length).toBe(100); // 0, 100, 200, ..., 9900
            expect(tasklet.processed).toBe(10000);
        });
    });

    describe('MapTasklet', () => {
        it('should transform items', async () => {
            const items = [1, 2, 3];
            const tasklet = new MapTasklet(
                'map-double',
                items[Symbol.iterator](),
                (n) => n * 2
            );

            const result = await scheduler.schedule(tasklet);
            expect(result).toEqual([2, 4, 6]);
        });

        it('should transform to different type', async () => {
            const items = [1, 2, 3];
            const tasklet = new MapTasklet(
                'map-stringify',
                items[Symbol.iterator](),
                (n) => `item-${n}`
            );

            const result = await scheduler.schedule(tasklet);
            expect(result).toEqual(['item-1', 'item-2', 'item-3']);
        });
    });

    describe('ForEachTasklet', () => {
        it('should apply action to each item', async () => {
            const items = [1, 2, 3];
            const collected: number[] = [];
            const tasklet = new ForEachTasklet(
                'foreach-collect',
                items[Symbol.iterator](),
                (n) => collected.push(n)
            );

            const count = await scheduler.schedule(tasklet);
            expect(count).toBe(3);
            expect(collected).toEqual([1, 2, 3]);
        });
    });

    describe('ReduceTasklet', () => {
        it('should reduce to single value', async () => {
            const items = [1, 2, 3, 4, 5];
            const tasklet = new ReduceTasklet(
                'reduce-sum',
                items[Symbol.iterator](),
                0,
                (acc, n) => acc + n
            );

            const result = await scheduler.schedule(tasklet);
            expect(result).toBe(15);
        });

        it('should build complex structures', async () => {
            const items = ['a', 'b', 'c'];
            const tasklet = new ReduceTasklet(
                'reduce-object',
                items[Symbol.iterator](),
                {} as Record<string, number>,
                (acc, str, ) => ({ ...acc, [str]: str.charCodeAt(0) })
            );

            const result = await scheduler.schedule(tasklet);
            expect(result).toEqual({ a: 97, b: 98, c: 99 });
        });
    });

    describe('runSync with Iterator Tasklets', () => {
        it('should run filter synchronously', () => {
            const items = [1, 2, 3, 4, 5];
            const tasklet = new FilterTasklet(
                'sync-filter',
                items[Symbol.iterator](),
                (n) => n > 3
            );

            const result = scheduler.runSync(tasklet);
            expect(result).toEqual([4, 5]);
        });
    });
});

describe('TaskletScheduler Performance', () => {
    it('should process many items without blocking event loop', async () => {
        const scheduler = new TaskletScheduler({
            defaultTimeBudgetMs: 2,
        });

        const items = Array.from({ length: 100000 }, (_, i) => i);
        const tasklet = new FilterTasklet(
            'perf-filter',
            items[Symbol.iterator](),
            (n) => n % 1000 === 0,
            { timeBudgetMs: 2, maxItemsPerIteration: 500 }
        );

        // Track event loop responsiveness
        let eventLoopTicks = 0;
        const interval = setInterval(() => eventLoopTicks++, 1);

        const result = await scheduler.schedule(tasklet);

        clearInterval(interval);
        scheduler.shutdown();

        // Should have yielded to event loop multiple times
        expect(eventLoopTicks).toBeGreaterThan(5);
        expect(result.length).toBe(100);
    }, 10000);
});
