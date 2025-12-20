import { BoundedEventQueue, QueueMetrics } from '../BoundedEventQueue';

describe('BoundedEventQueue', () => {
    let queue: BoundedEventQueue<number>;

    beforeEach(() => {
        queue = new BoundedEventQueue<number>({
            maxSize: 5,
            name: 'test-queue'
        });
    });

    describe('basic operations', () => {
        it('should enqueue and dequeue items correctly', () => {
            expect(queue.enqueue(1)).toBe(true);
            expect(queue.enqueue(2)).toBe(true);
            expect(queue.enqueue(3)).toBe(true);

            expect(queue.dequeue()).toBe(1);
            expect(queue.dequeue()).toBe(2);
            expect(queue.dequeue()).toBe(3);
        });

        it('should return undefined when dequeuing from empty queue', () => {
            expect(queue.dequeue()).toBeUndefined();
        });

        it('should peek at front item without removing', () => {
            queue.enqueue(1);
            queue.enqueue(2);

            expect(queue.peek()).toBe(1);
            expect(queue.peek()).toBe(1); // Still 1, not removed
            expect(queue.size).toBe(2);
        });

        it('should return undefined when peeking at empty queue', () => {
            expect(queue.peek()).toBeUndefined();
        });
    });

    describe('capacity and rejection', () => {
        it('should accept items up to capacity', () => {
            for (let i = 0; i < 5; i++) {
                expect(queue.enqueue(i)).toBe(true);
            }
            expect(queue.size).toBe(5);
            expect(queue.isFull).toBe(true);
        });

        it('should reject items when full', () => {
            // Fill the queue
            for (let i = 0; i < 5; i++) {
                queue.enqueue(i);
            }

            // This should be rejected
            expect(queue.enqueue(6)).toBe(false);
            expect(queue.size).toBe(5);
        });

        it('should call onReject callback when item is rejected', () => {
            const onReject = jest.fn();
            const queueWithCallback = new BoundedEventQueue<number>({
                maxSize: 2,
                name: 'test-queue',
                onReject
            });

            queueWithCallback.enqueue(1);
            queueWithCallback.enqueue(2);
            queueWithCallback.enqueue(3); // This should be rejected

            expect(onReject).toHaveBeenCalledTimes(1);
            expect(onReject).toHaveBeenCalledWith(3);
        });

        it('should accept items after dequeue frees space', () => {
            // Fill the queue
            for (let i = 0; i < 5; i++) {
                queue.enqueue(i);
            }
            expect(queue.isFull).toBe(true);

            // Dequeue one item
            queue.dequeue();
            expect(queue.isFull).toBe(false);

            // Should be able to enqueue again
            expect(queue.enqueue(10)).toBe(true);
            expect(queue.size).toBe(5);
        });
    });

    describe('metrics', () => {
        it('should track metrics correctly', () => {
            queue.enqueue(1);
            queue.enqueue(2);
            queue.enqueue(3);
            queue.dequeue();

            const metrics = queue.getMetrics();
            expect(metrics.enqueued).toBe(3);
            expect(metrics.dequeued).toBe(1);
            expect(metrics.currentSize).toBe(2);
            expect(metrics.rejected).toBe(0);
        });

        it('should track rejected items in metrics', () => {
            // Fill and try to overfill
            for (let i = 0; i < 5; i++) {
                queue.enqueue(i);
            }
            queue.enqueue(100); // rejected
            queue.enqueue(101); // rejected

            const metrics = queue.getMetrics();
            expect(metrics.rejected).toBe(2);
            expect(metrics.enqueued).toBe(5);
        });

        it('should return a copy of metrics (not reference)', () => {
            queue.enqueue(1);
            const metrics1 = queue.getMetrics();
            queue.enqueue(2);
            const metrics2 = queue.getMetrics();

            expect(metrics1.enqueued).toBe(1);
            expect(metrics2.enqueued).toBe(2);
        });
    });

    describe('events', () => {
        it('should emit "full" event when reaching capacity', () => {
            const fullHandler = jest.fn();
            queue.on('full', fullHandler);

            for (let i = 0; i < 5; i++) {
                queue.enqueue(i);
            }

            expect(fullHandler).toHaveBeenCalledTimes(1);
            expect(fullHandler).toHaveBeenCalledWith({
                name: 'test-queue',
                size: 5
            });
        });

        it('should emit "empty" event when drained', () => {
            const emptyHandler = jest.fn();
            queue.on('empty', emptyHandler);

            queue.enqueue(1);
            queue.enqueue(2);
            queue.dequeue();
            expect(emptyHandler).not.toHaveBeenCalled();

            queue.dequeue();
            expect(emptyHandler).toHaveBeenCalledTimes(1);
            expect(emptyHandler).toHaveBeenCalledWith({
                name: 'test-queue'
            });
        });

        it('should emit "highWater" event when reaching high water mark', () => {
            const highWaterHandler = jest.fn();
            const hwQueue = new BoundedEventQueue<number>({
                maxSize: 10,
                name: 'hw-queue',
                highWaterMark: 0.8 // 80% = 8 items
            });
            hwQueue.on('highWater', highWaterHandler);

            // Add 7 items (70%) - no event
            for (let i = 0; i < 7; i++) {
                hwQueue.enqueue(i);
            }
            expect(highWaterHandler).not.toHaveBeenCalled();

            // Add 8th item (80%) - should trigger
            hwQueue.enqueue(7);
            expect(highWaterHandler).toHaveBeenCalledTimes(1);
            expect(highWaterHandler).toHaveBeenCalledWith(
                expect.objectContaining({
                    name: 'hw-queue',
                    usage: 0.8
                })
            );

            // Adding more shouldn't trigger again
            hwQueue.enqueue(8);
            hwQueue.enqueue(9);
            expect(highWaterHandler).toHaveBeenCalledTimes(1);
        });

        it('should reset highWater flag after draining below threshold', () => {
            const highWaterHandler = jest.fn();
            const hwQueue = new BoundedEventQueue<number>({
                maxSize: 10,
                name: 'hw-queue',
                highWaterMark: 0.8
            });
            hwQueue.on('highWater', highWaterHandler);

            // Fill to high water mark
            for (let i = 0; i < 8; i++) {
                hwQueue.enqueue(i);
            }
            expect(highWaterHandler).toHaveBeenCalledTimes(1);

            // Drain below threshold
            for (let i = 0; i < 3; i++) {
                hwQueue.dequeue();
            }

            // Fill again - should trigger again
            for (let i = 0; i < 3; i++) {
                hwQueue.enqueue(i + 100);
            }
            expect(highWaterHandler).toHaveBeenCalledTimes(2);
        });

        it('should not emit "empty" event when dequeuing from already empty queue', () => {
            const emptyHandler = jest.fn();
            queue.on('empty', emptyHandler);

            queue.dequeue(); // Empty queue
            expect(emptyHandler).not.toHaveBeenCalled();
        });
    });

    describe('clear', () => {
        it('should clear the queue', () => {
            queue.enqueue(1);
            queue.enqueue(2);
            queue.enqueue(3);

            queue.clear();

            expect(queue.size).toBe(0);
            expect(queue.isEmpty).toBe(true);
            expect(queue.getMetrics().currentSize).toBe(0);
        });

        it('should emit "empty" event when clearing non-empty queue', () => {
            const emptyHandler = jest.fn();
            queue.on('empty', emptyHandler);

            queue.enqueue(1);
            queue.clear();

            expect(emptyHandler).toHaveBeenCalledTimes(1);
        });

        it('should not emit "empty" event when clearing already empty queue', () => {
            const emptyHandler = jest.fn();
            queue.on('empty', emptyHandler);

            queue.clear();

            expect(emptyHandler).not.toHaveBeenCalled();
        });
    });

    describe('edge cases', () => {
        it('should handle queue with maxSize of 1', () => {
            const tinyQueue = new BoundedEventQueue<number>({
                maxSize: 1,
                name: 'tiny-queue'
            });

            expect(tinyQueue.enqueue(1)).toBe(true);
            expect(tinyQueue.isFull).toBe(true);
            expect(tinyQueue.enqueue(2)).toBe(false);

            expect(tinyQueue.dequeue()).toBe(1);
            expect(tinyQueue.isEmpty).toBe(true);
            expect(tinyQueue.enqueue(3)).toBe(true);
        });

        it('should handle rapid enqueue/dequeue cycles', () => {
            for (let cycle = 0; cycle < 100; cycle++) {
                for (let i = 0; i < 5; i++) {
                    queue.enqueue(i);
                }
                for (let i = 0; i < 5; i++) {
                    queue.dequeue();
                }
            }

            const metrics = queue.getMetrics();
            expect(metrics.enqueued).toBe(500);
            expect(metrics.dequeued).toBe(500);
            expect(metrics.currentSize).toBe(0);
        });
    });
});
