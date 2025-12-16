import { CoalescingWriter } from '../CoalescingWriter';
import { WebSocket } from 'ws';
import { deserialize } from '@topgunbuild/core';

// Mock WebSocket
class MockWebSocket {
    readyState: number = WebSocket.OPEN;
    sentMessages: Uint8Array[] = [];

    send(data: Uint8Array) {
        this.sentMessages.push(data);
    }

    close() {
        this.readyState = WebSocket.CLOSED;
    }
}

describe('CoalescingWriter', () => {
    let mockWs: MockWebSocket;
    let writer: CoalescingWriter;

    beforeEach(() => {
        mockWs = new MockWebSocket();
        jest.useFakeTimers({ legacyFakeTimers: true });
    });

    afterEach(() => {
        writer?.close();
        jest.useRealTimers();
    });

    describe('basic operations', () => {
        it('should send single message without batching', () => {
            writer = new CoalescingWriter(mockWs as any, {
                maxBatchSize: 100,
                maxDelayMs: 5,
                maxBatchBytes: 65536
            });

            writer.write({ type: 'TEST', data: 'hello' });

            // Advance timer to trigger flush
            jest.advanceTimersByTime(10);
            jest.runAllImmediates();

            expect(mockWs.sentMessages.length).toBe(1);

            // Single message should be sent directly without BATCH wrapper
            const msg = deserialize(mockWs.sentMessages[0]);
            expect(msg).toEqual({ type: 'TEST', data: 'hello' });
        });

        it('should batch multiple messages', () => {
            writer = new CoalescingWriter(mockWs as any, {
                maxBatchSize: 100,
                maxDelayMs: 5,
                maxBatchBytes: 65536
            });

            writer.write({ type: 'MSG1' });
            writer.write({ type: 'MSG2' });
            writer.write({ type: 'MSG3' });

            // Advance timer to trigger flush
            jest.advanceTimersByTime(10);
            jest.runAllImmediates();

            expect(mockWs.sentMessages.length).toBe(1);

            // Multiple messages should be wrapped in BATCH
            const batch = deserialize(mockWs.sentMessages[0]) as any;
            expect(batch.type).toBe('BATCH');
            expect(batch.count).toBe(3);
        });
    });

    describe('flush triggers', () => {
        it('should flush when maxBatchSize reached', () => {
            writer = new CoalescingWriter(mockWs as any, {
                maxBatchSize: 3,
                maxDelayMs: 1000, // Long delay to ensure size triggers flush
                maxBatchBytes: 65536
            });

            writer.write({ type: 'MSG1' });
            writer.write({ type: 'MSG2' });

            // Not flushed yet
            expect(mockWs.sentMessages.length).toBe(0);

            writer.write({ type: 'MSG3' }); // This should trigger flush

            expect(mockWs.sentMessages.length).toBe(1);
        });

        it('should flush when maxDelayMs elapsed', () => {
            writer = new CoalescingWriter(mockWs as any, {
                maxBatchSize: 100,
                maxDelayMs: 5,
                maxBatchBytes: 65536
            });

            writer.write({ type: 'MSG1' });

            // Not flushed yet
            expect(mockWs.sentMessages.length).toBe(0);

            // Advance time past maxDelayMs
            jest.advanceTimersByTime(10);
            jest.runAllImmediates();

            expect(mockWs.sentMessages.length).toBe(1);
        });

        it('should flush when maxBatchBytes exceeded', () => {
            writer = new CoalescingWriter(mockWs as any, {
                maxBatchSize: 100,
                maxDelayMs: 1000,
                maxBatchBytes: 50 // Very small to trigger byte limit
            });

            // Write a message that exceeds byte limit
            writer.write({ type: 'MSG1', data: 'a'.repeat(100) });

            expect(mockWs.sentMessages.length).toBe(1);
        });
    });

    describe('urgent messages', () => {
        it('should send urgent messages immediately without batching', () => {
            writer = new CoalescingWriter(mockWs as any, {
                maxBatchSize: 100,
                maxDelayMs: 1000,
                maxBatchBytes: 65536
            });

            writer.write({ type: 'NORMAL' });
            writer.write({ type: 'URGENT' }, true); // urgent

            // Urgent message should be sent immediately
            expect(mockWs.sentMessages.length).toBe(1);

            const msg = deserialize(mockWs.sentMessages[0]);
            expect(msg).toEqual({ type: 'URGENT' });

            // Normal message still pending
            jest.advanceTimersByTime(1100);
            jest.runAllImmediates();

            expect(mockWs.sentMessages.length).toBe(2);
        });

        it('should not batch AUTH_ACK and PONG messages when marked urgent', () => {
            writer = new CoalescingWriter(mockWs as any, {
                maxBatchSize: 100,
                maxDelayMs: 5,
                maxBatchBytes: 65536
            });

            writer.write({ type: 'AUTH_ACK' }, true);

            expect(mockWs.sentMessages.length).toBe(1);
            const msg = deserialize(mockWs.sentMessages[0]);
            expect(msg).toEqual({ type: 'AUTH_ACK' });
        });
    });

    describe('socket errors', () => {
        it('should handle socket errors gracefully', () => {
            writer = new CoalescingWriter(mockWs as any, {
                maxBatchSize: 100,
                maxDelayMs: 5,
                maxBatchBytes: 65536
            });

            // Close socket
            mockWs.readyState = WebSocket.CLOSED;

            // Should not throw
            expect(() => writer.write({ type: 'TEST' })).not.toThrow();

            jest.advanceTimersByTime(10);
            jest.runAllImmediates();

            // No messages sent since socket is closed
            expect(mockWs.sentMessages.length).toBe(0);
        });

        it('should discard messages when socket not ready', () => {
            writer = new CoalescingWriter(mockWs as any, {
                maxBatchSize: 100,
                maxDelayMs: 5,
                maxBatchBytes: 65536
            });

            writer.write({ type: 'MSG1' });
            mockWs.readyState = WebSocket.CLOSING;

            jest.advanceTimersByTime(10);
            jest.runAllImmediates();

            expect(mockWs.sentMessages.length).toBe(0);
        });
    });

    describe('metrics', () => {
        it('should track metrics correctly', () => {
            writer = new CoalescingWriter(mockWs as any, {
                maxBatchSize: 100,
                maxDelayMs: 5,
                maxBatchBytes: 65536
            });

            writer.write({ type: 'MSG1' });
            writer.write({ type: 'MSG2' });
            writer.write({ type: 'MSG3' });

            jest.advanceTimersByTime(10);
            jest.runAllImmediates();

            const metrics = writer.getMetrics();
            expect(metrics.messagesSent).toBe(3);
            expect(metrics.batchesSent).toBe(1);
            expect(metrics.avgMessagesPerBatch).toBe(3);
            expect(metrics.pendingMessages).toBe(0);
            expect(metrics.pendingBytes).toBe(0);
        });

        it('should track pending messages before flush', () => {
            writer = new CoalescingWriter(mockWs as any, {
                maxBatchSize: 100,
                maxDelayMs: 1000,
                maxBatchBytes: 65536
            });

            writer.write({ type: 'MSG1' });
            writer.write({ type: 'MSG2' });

            const metrics = writer.getMetrics();
            expect(metrics.pendingMessages).toBe(2);
            expect(metrics.pendingBytes).toBeGreaterThan(0);
        });
    });

    describe('close', () => {
        it('should flush pending messages on close', () => {
            writer = new CoalescingWriter(mockWs as any, {
                maxBatchSize: 100,
                maxDelayMs: 1000,
                maxBatchBytes: 65536
            });

            writer.write({ type: 'MSG1' });
            writer.write({ type: 'MSG2' });

            expect(mockWs.sentMessages.length).toBe(0);

            writer.close();

            expect(mockWs.sentMessages.length).toBe(1);
        });

        it('should not accept messages after close', () => {
            writer = new CoalescingWriter(mockWs as any, {
                maxBatchSize: 100,
                maxDelayMs: 5,
                maxBatchBytes: 65536
            });

            writer.close();
            writer.write({ type: 'MSG1' });

            jest.advanceTimersByTime(10);
            jest.runAllImmediates();

            expect(mockWs.sentMessages.length).toBe(0);
        });
    });

    describe('writeRaw', () => {
        it('should queue pre-serialized data', () => {
            writer = new CoalescingWriter(mockWs as any, {
                maxBatchSize: 100,
                maxDelayMs: 5,
                maxBatchBytes: 65536
            });

            const rawData = new Uint8Array([1, 2, 3, 4, 5]);
            writer.writeRaw(rawData);

            jest.advanceTimersByTime(10);
            jest.runAllImmediates();

            expect(mockWs.sentMessages.length).toBe(1);
        });
    });
});
