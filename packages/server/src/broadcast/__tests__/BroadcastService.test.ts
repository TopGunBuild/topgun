import { BroadcastService, BroadcastEvent } from '../BroadcastService';

describe('BroadcastService', () => {
    let broadcastService: BroadcastService;
    let mockCallback: jest.Mock;
    let deliveredEvents: { events: BroadcastEvent[]; excludeClientId?: string }[];

    beforeEach(() => {
        jest.useFakeTimers();
        deliveredEvents = [];
        mockCallback = jest.fn((events, excludeClientId) => {
            deliveredEvents.push({ events, excludeClientId });
        });
    });

    afterEach(() => {
        if (broadcastService?.isActive()) {
            broadcastService.stop();
        }
        jest.useRealTimers();
    });

    describe('Basic functionality', () => {
        it('should buffer events and flush on interval', () => {
            broadcastService = new BroadcastService(mockCallback, {
                flushIntervalMs: 50,
                adaptiveFlush: false
            });
            broadcastService.start();

            // Buffer some events
            broadcastService.buffer([
                { mapName: 'users', eventType: 'UPDATED', key: 'user1' }
            ]);
            broadcastService.buffer([
                { mapName: 'users', eventType: 'UPDATED', key: 'user2' }
            ]);

            // Events should not be delivered yet
            expect(mockCallback).not.toHaveBeenCalled();

            // Advance timer to trigger flush
            jest.advanceTimersByTime(50);

            // Events should now be delivered
            expect(mockCallback).toHaveBeenCalledTimes(1);
            expect(deliveredEvents[0].events).toHaveLength(2);
        });

        it('should group events by excludeClientId', () => {
            broadcastService = new BroadcastService(mockCallback, {
                flushIntervalMs: 50,
                adaptiveFlush: false
            });
            broadcastService.start();

            // Buffer events from different clients
            broadcastService.buffer([
                { mapName: 'users', eventType: 'UPDATED', key: 'user1' }
            ], 'client-a');

            broadcastService.buffer([
                { mapName: 'users', eventType: 'UPDATED', key: 'user2' }
            ], 'client-b');

            broadcastService.buffer([
                { mapName: 'users', eventType: 'UPDATED', key: 'user3' }
            ], 'client-a');

            jest.advanceTimersByTime(50);

            // Should have 2 broadcasts (one for each excludeClientId)
            expect(mockCallback).toHaveBeenCalledTimes(2);

            // Find broadcasts by excludeClientId
            const clientABroadcast = deliveredEvents.find(d => d.excludeClientId === 'client-a');
            const clientBBroadcast = deliveredEvents.find(d => d.excludeClientId === 'client-b');

            expect(clientABroadcast!.events).toHaveLength(2);
            expect(clientBBroadcast!.events).toHaveLength(1);
        });

        it('should flush immediately when buffer exceeds maxBufferSize', () => {
            broadcastService = new BroadcastService(mockCallback, {
                flushIntervalMs: 50,
                maxBufferSize: 5,
                adaptiveFlush: false
            });
            broadcastService.start();

            // Buffer events up to max
            for (let i = 0; i < 5; i++) {
                broadcastService.buffer([
                    { mapName: 'users', eventType: 'UPDATED', key: `user${i}` }
                ]);
            }

            // Should flush immediately without waiting for timer
            expect(mockCallback).toHaveBeenCalledTimes(1);
            expect(deliveredEvents[0].events).toHaveLength(5);
        });

        it('should flush remaining events on stop', () => {
            broadcastService = new BroadcastService(mockCallback, {
                flushIntervalMs: 50,
                adaptiveFlush: false
            });
            broadcastService.start();

            broadcastService.buffer([
                { mapName: 'users', eventType: 'UPDATED', key: 'user1' }
            ]);

            // Stop without waiting for timer
            broadcastService.stop();

            // Events should be flushed
            expect(mockCallback).toHaveBeenCalledTimes(1);
        });
    });

    describe('Adaptive flush', () => {
        it('should flush immediately for low-traffic with adaptive enabled', () => {
            broadcastService = new BroadcastService(mockCallback, {
                flushIntervalMs: 50,
                minBatchSize: 1,
                adaptiveFlush: true
            });
            broadcastService.start();

            // Wait half the interval (25ms)
            jest.advanceTimersByTime(25);

            // Buffer a single event
            broadcastService.buffer([
                { mapName: 'users', eventType: 'UPDATED', key: 'user1' }
            ]);

            // With adaptive flush, should flush immediately since we have few events
            // and waited half the interval
            expect(mockCallback).toHaveBeenCalledTimes(1);
        });

        it('should not adaptive flush if not enough time has passed', () => {
            broadcastService = new BroadcastService(mockCallback, {
                flushIntervalMs: 100,
                minBatchSize: 1,
                adaptiveFlush: true
            });
            broadcastService.start();

            // Don't wait - buffer immediately
            broadcastService.buffer([
                { mapName: 'users', eventType: 'UPDATED', key: 'user1' }
            ]);

            // Should not flush yet (not enough time passed)
            expect(mockCallback).not.toHaveBeenCalled();

            // Now wait for full interval
            jest.advanceTimersByTime(100);
            expect(mockCallback).toHaveBeenCalledTimes(1);
        });
    });

    describe('Statistics', () => {
        it('should track event statistics', () => {
            broadcastService = new BroadcastService(mockCallback, {
                flushIntervalMs: 50,
                adaptiveFlush: false
            });
            broadcastService.start();

            // Buffer events
            broadcastService.buffer([
                { mapName: 'users', eventType: 'UPDATED', key: 'user1' },
                { mapName: 'users', eventType: 'UPDATED', key: 'user2' }
            ]);
            broadcastService.buffer([
                { mapName: 'users', eventType: 'UPDATED', key: 'user3' }
            ]);

            let stats = broadcastService.getStats();
            expect(stats.bufferSize).toBe(3);
            expect(stats.totalEventsBuffered).toBe(3);
            expect(stats.totalFlushes).toBe(0);

            // Flush
            jest.advanceTimersByTime(50);

            stats = broadcastService.getStats();
            expect(stats.bufferSize).toBe(0);
            expect(stats.totalEventsBuffered).toBe(3);
            expect(stats.totalFlushes).toBe(1);
            expect(stats.totalEventsDelivered).toBe(3);
            expect(stats.avgEventsPerFlush).toBe(3);
        });
    });

    describe('Edge cases', () => {
        it('should ignore empty event arrays', () => {
            broadcastService = new BroadcastService(mockCallback, {
                flushIntervalMs: 50,
                adaptiveFlush: false
            });
            broadcastService.start();

            broadcastService.buffer([]);
            jest.advanceTimersByTime(50);

            expect(mockCallback).not.toHaveBeenCalled();
        });

        it('should handle callback errors gracefully', () => {
            const errorCallback = jest.fn(() => {
                throw new Error('Broadcast failed');
            });
            broadcastService = new BroadcastService(errorCallback, {
                flushIntervalMs: 50,
                adaptiveFlush: false
            });
            broadcastService.start();

            broadcastService.buffer([
                { mapName: 'users', eventType: 'UPDATED', key: 'user1' }
            ]);

            // Should not throw
            expect(() => {
                jest.advanceTimersByTime(50);
            }).not.toThrow();

            // Callback was called but failed
            expect(errorCallback).toHaveBeenCalledTimes(1);
        });

        it('should not start twice', () => {
            broadcastService = new BroadcastService(mockCallback, {
                flushIntervalMs: 50
            });
            broadcastService.start();
            broadcastService.start(); // Second call should be no-op

            expect(broadcastService.isActive()).toBe(true);
        });

        it('should handle stop when not running', () => {
            broadcastService = new BroadcastService(mockCallback, {
                flushIntervalMs: 50
            });

            // Stop without start - should not throw
            expect(() => broadcastService.stop()).not.toThrow();
        });
    });

    describe('High-throughput simulation', () => {
        it('should batch 200 incoming batches into ~4 flushes over 200ms', () => {
            broadcastService = new BroadcastService(mockCallback, {
                flushIntervalMs: 50,
                maxBufferSize: 1000,
                adaptiveFlush: false
            });
            broadcastService.start();

            // Simulate 200 batches arriving over 200ms (like 1000 ops/sec scenario)
            for (let i = 0; i < 200; i++) {
                broadcastService.buffer([
                    { mapName: 'data', eventType: 'UPDATED', key: `key${i}` }
                ]);
                // Advance 1ms between batches
                jest.advanceTimersByTime(1);
            }

            // After 200ms we should have approximately 4 flushes (every 50ms)
            const stats = broadcastService.getStats();
            expect(stats.totalFlushes).toBeGreaterThanOrEqual(3);
            expect(stats.totalFlushes).toBeLessThanOrEqual(5);
            expect(stats.totalEventsDelivered).toBe(200);

            // Average events per flush should be ~50 (200 events / 4 flushes)
            expect(stats.avgEventsPerFlush).toBeGreaterThanOrEqual(40);
        });
    });
});
