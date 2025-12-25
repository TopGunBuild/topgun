import { WebSocket } from 'ws';
import { CoalescingWriter, CoalescingWriterOptions } from '../../utils/CoalescingWriter';
import {
    coalescingPresets,
    getCoalescingPreset,
    CoalescingPreset,
} from '../../utils/coalescingPresets';

// Mock WebSocket
class MockWebSocket {
    readyState = WebSocket.OPEN;
    sentData: Uint8Array[] = [];

    send(data: Uint8Array) {
        this.sentData.push(data);
    }
}

describe('CoalescingWriter', () => {
    let mockSocket: MockWebSocket;

    beforeEach(() => {
        mockSocket = new MockWebSocket();
        jest.useFakeTimers({ legacyFakeTimers: true });
    });

    afterEach(() => {
        jest.useRealTimers();
    });

    describe('Presets', () => {
        it('should have all expected presets', () => {
            expect(coalescingPresets.conservative).toBeDefined();
            expect(coalescingPresets.balanced).toBeDefined();
            expect(coalescingPresets.highThroughput).toBeDefined();
            expect(coalescingPresets.aggressive).toBeDefined();
        });

        it('should use default options when no options provided', () => {
            const writer = new CoalescingWriter(mockSocket as unknown as WebSocket);
            const options = writer.getOptions();

            // Default is now highThroughput values via ServerCoordinator, but CoalescingWriter
            // itself uses conservative defaults (100, 5, 65536)
            expect(options.maxBatchSize).toBe(100);
            expect(options.maxDelayMs).toBe(5);
            expect(options.maxBatchBytes).toBe(65536);
        });

        it('should accept custom options', () => {
            const customOptions: CoalescingWriterOptions = {
                maxBatchSize: 200,
                maxDelayMs: 15,
                maxBatchBytes: 131072,
            };

            const writer = new CoalescingWriter(
                mockSocket as unknown as WebSocket,
                customOptions
            );
            const options = writer.getOptions();

            expect(options.maxBatchSize).toBe(200);
            expect(options.maxDelayMs).toBe(15);
            expect(options.maxBatchBytes).toBe(131072);
        });

        it('should merge partial options with defaults', () => {
            const writer = new CoalescingWriter(mockSocket as unknown as WebSocket, {
                maxBatchSize: 250,
            });
            const options = writer.getOptions();

            expect(options.maxBatchSize).toBe(250);
            expect(options.maxDelayMs).toBe(5); // default
            expect(options.maxBatchBytes).toBe(65536); // default
        });

        it('should get preset by name', () => {
            const highThroughput = getCoalescingPreset('highThroughput');
            expect(highThroughput.maxBatchSize).toBe(500);
            expect(highThroughput.maxDelayMs).toBe(2);
            expect(highThroughput.maxBatchBytes).toBe(262144);
        });

        it('should return independent copies from getCoalescingPreset', () => {
            const preset1 = getCoalescingPreset('balanced');
            const preset2 = getCoalescingPreset('balanced');

            preset1.maxBatchSize = 999;
            expect(preset2.maxBatchSize).toBe(300); // original value
        });
    });

    describe('Preset Values', () => {
        it('conservative preset should have expected values', () => {
            expect(coalescingPresets.conservative).toEqual({
                maxBatchSize: 100,
                maxDelayMs: 2,
                maxBatchBytes: 65536,
            });
        });

        it('balanced preset should have expected values', () => {
            expect(coalescingPresets.balanced).toEqual({
                maxBatchSize: 300,
                maxDelayMs: 2,
                maxBatchBytes: 131072,
            });
        });

        it('highThroughput preset should have expected values', () => {
            expect(coalescingPresets.highThroughput).toEqual({
                maxBatchSize: 500,
                maxDelayMs: 2,
                maxBatchBytes: 262144,
            });
        });

        it('aggressive preset should have expected values', () => {
            expect(coalescingPresets.aggressive).toEqual({
                maxBatchSize: 1000,
                maxDelayMs: 5,
                maxBatchBytes: 524288,
            });
        });
    });

    describe('Metrics', () => {
        it('should track immediate vs timed flushes', () => {
            const writer = new CoalescingWriter(mockSocket as unknown as WebSocket, {
                maxBatchSize: 2,
                maxDelayMs: 100,
                maxBatchBytes: 1000000,
            });

            // Send 2 messages - should trigger immediate flush (batch full)
            writer.write({ type: 'TEST', data: '1' });
            writer.write({ type: 'TEST', data: '2' });

            let metrics = writer.getMetrics();
            expect(metrics.immediateFlushes).toBe(1);
            expect(metrics.timedFlushes).toBe(0);

            // Send 1 message and wait for timer
            writer.write({ type: 'TEST', data: '3' });

            // Advance timers to trigger timed flush
            jest.advanceTimersByTime(100);
            jest.runAllImmediates();

            metrics = writer.getMetrics();
            expect(metrics.immediateFlushes).toBe(1);
            expect(metrics.timedFlushes).toBe(1);

            writer.close();
        });

        it('should calculate batch utilization', () => {
            const writer = new CoalescingWriter(mockSocket as unknown as WebSocket, {
                maxBatchSize: 10,
                maxDelayMs: 5,
                maxBatchBytes: 1000000,
            });

            // Send 5 messages (50% of maxBatchSize)
            for (let i = 0; i < 5; i++) {
                writer.write({ type: 'TEST', data: i.toString() });
            }

            // Force flush via timer
            jest.advanceTimersByTime(5);
            jest.runAllImmediates();

            const metrics = writer.getMetrics();
            expect(metrics.batchUtilization).toBe(0.5); // 5/10 = 50%

            writer.close();
        });

        it('should report accurate avgBytesPerBatch', () => {
            const writer = new CoalescingWriter(mockSocket as unknown as WebSocket, {
                maxBatchSize: 100,
                maxDelayMs: 5,
                maxBatchBytes: 1000000,
            });

            writer.write({ type: 'TEST', data: 'hello' });
            jest.advanceTimersByTime(5);
            jest.runAllImmediates();

            const metrics = writer.getMetrics();
            expect(metrics.avgBytesPerBatch).toBeGreaterThan(0);
            expect(metrics.avgBytesPerBatch).toBe(metrics.bytesSent / metrics.batchesSent);

            writer.close();
        });

        it('should track immediateFlushRatio', () => {
            const writer = new CoalescingWriter(mockSocket as unknown as WebSocket, {
                maxBatchSize: 2,
                maxDelayMs: 100,
                maxBatchBytes: 1000000,
            });

            // 1 immediate flush (batch full)
            writer.write({ type: 'TEST', data: '1' });
            writer.write({ type: 'TEST', data: '2' });

            // 1 timed flush
            writer.write({ type: 'TEST', data: '3' });
            jest.advanceTimersByTime(100);
            jest.runAllImmediates();

            const metrics = writer.getMetrics();
            expect(metrics.immediateFlushRatio).toBe(0.5); // 1 immediate / 2 total

            writer.close();
        });
    });

    describe('Batch Size Behavior', () => {
        it('should batch more messages with highThroughput preset', () => {
            const conservativeWriter = new CoalescingWriter(
                mockSocket as unknown as WebSocket,
                coalescingPresets.conservative
            );
            const highThroughputSocket = new MockWebSocket();
            const highThroughputWriter = new CoalescingWriter(
                highThroughputSocket as unknown as WebSocket,
                coalescingPresets.highThroughput
            );

            // Send 150 messages
            for (let i = 0; i < 150; i++) {
                conservativeWriter.write({ type: 'TEST', i });
                highThroughputWriter.write({ type: 'TEST', i });
            }

            // Conservative should have flushed once (at 100)
            // HighThroughput should not have flushed yet (maxBatchSize=500)
            expect(mockSocket.sentData.length).toBe(1);
            expect(highThroughputSocket.sentData.length).toBe(0);

            conservativeWriter.close();
            highThroughputWriter.close();
        });

        it('should flush earlier with conservative preset', () => {
            const writer = new CoalescingWriter(
                mockSocket as unknown as WebSocket,
                coalescingPresets.conservative
            );

            // Send 100 messages - exactly maxBatchSize
            for (let i = 0; i < 100; i++) {
                writer.write({ type: 'TEST', i });
            }

            // Should have flushed immediately
            expect(mockSocket.sentData.length).toBe(1);

            writer.close();
        });

        it('should respect maxBatchBytes limit', () => {
            const writer = new CoalescingWriter(mockSocket as unknown as WebSocket, {
                maxBatchSize: 1000, // High limit
                maxDelayMs: 100, // High delay
                maxBatchBytes: 500, // Low bytes limit
            });

            // Send messages until bytes limit triggers flush
            // Each message is roughly 30-50 bytes serialized
            for (let i = 0; i < 20; i++) {
                writer.write({ type: 'TEST', data: 'x'.repeat(30) });
            }

            // Should have flushed due to bytes limit
            expect(mockSocket.sentData.length).toBeGreaterThan(0);

            const metrics = writer.getMetrics();
            expect(metrics.immediateFlushes).toBeGreaterThan(0);

            writer.close();
        });
    });

    describe('Urgent Messages', () => {
        it('should send urgent messages immediately without batching', () => {
            const writer = new CoalescingWriter(mockSocket as unknown as WebSocket, {
                maxBatchSize: 100,
                maxDelayMs: 100,
                maxBatchBytes: 1000000,
            });

            // Queue some normal messages
            writer.write({ type: 'NORMAL', data: '1' });
            writer.write({ type: 'NORMAL', data: '2' });

            // Send urgent message
            writer.write({ type: 'URGENT', data: 'important' }, true);

            // Urgent should be sent immediately
            expect(mockSocket.sentData.length).toBe(1);

            // Normal messages should still be pending
            const metrics = writer.getMetrics();
            expect(metrics.pendingMessages).toBe(2);

            writer.close();
        });
    });
});
