/**
 * Transport Benchmark - compares ws vs uWebSockets.js performance
 *
 * Usage:
 *   npx ts-node scripts/benchmark-transports.ts
 */

import WebSocket from 'ws';
import { WsTransport, UWebSocketsTransport, type IWebSocketTransport } from '../packages/server/src/transport';

interface BenchmarkResult {
    transport: string;
    connections: number;
    messagesPerSecond: number;
    latencyP50: number;
    latencyP99: number;
    memoryMB: number;
}

async function benchmarkTransport(
    name: string,
    transport: IWebSocketTransport,
    numConnections: number,
    durationMs: number,
): Promise<BenchmarkResult> {
    await transport.start({ port: 0 });
    const port = transport.getPort();

    // Track metrics
    let messageCount = 0;
    const latencies: number[] = [];

    // Echo server - respond to each message
    transport.onConnection((conn) => {
        conn.onMessage((data) => {
            conn.send(data);
        });
    });

    // Create connections
    const clients: WebSocket[] = [];
    for (let i = 0; i < numConnections; i++) {
        const ws = new WebSocket(`ws://localhost:${port}`);
        ws.binaryType = 'arraybuffer';
        clients.push(ws);
        await new Promise<void>((resolve) => ws.on('open', resolve));
    }

    // Setup message handlers with latency tracking
    for (const ws of clients) {
        ws.on('message', (data: ArrayBuffer) => {
            const view = new DataView(data);
            const sentTime = Number(view.getBigUint64(0));
            const latency = Number(process.hrtime.bigint() - BigInt(sentTime)) / 1_000_000; // ms
            latencies.push(latency);
            messageCount++;
        });
    }

    // Get baseline memory
    if (global.gc) global.gc();
    const memBefore = process.memoryUsage().heapUsed;

    // Start sending messages
    const startTime = Date.now();
    const endTime = startTime + durationMs;

    const sendMessage = (ws: WebSocket) => {
        if (Date.now() >= endTime || ws.readyState !== WebSocket.OPEN) return;

        const buffer = new ArrayBuffer(8);
        const view = new DataView(buffer);
        view.setBigUint64(0, process.hrtime.bigint());
        ws.send(buffer);

        setImmediate(() => sendMessage(ws));
    };

    // Start all clients sending
    for (const ws of clients) {
        sendMessage(ws);
    }

    // Wait for duration
    await new Promise((resolve) => setTimeout(resolve, durationMs));

    // Calculate results
    const actualDuration = (Date.now() - startTime) / 1000;
    const messagesPerSecond = Math.round(messageCount / actualDuration);

    // Sort latencies for percentiles
    latencies.sort((a, b) => a - b);
    const latencyP50 = latencies[Math.floor(latencies.length * 0.5)] ?? 0;
    const latencyP99 = latencies[Math.floor(latencies.length * 0.99)] ?? 0;

    // Memory
    if (global.gc) global.gc();
    const memAfter = process.memoryUsage().heapUsed;
    const memoryMB = Math.round((memAfter - memBefore) / 1024 / 1024);

    // Cleanup
    for (const ws of clients) {
        ws.close();
    }
    await transport.stop();

    return {
        transport: name,
        connections: numConnections,
        messagesPerSecond,
        latencyP50: Math.round(latencyP50 * 100) / 100,
        latencyP99: Math.round(latencyP99 * 100) / 100,
        memoryMB,
    };
}

async function main() {
    console.log('=== WebSocket Transport Benchmark ===\n');

    const numConnections = 10;
    const durationMs = 5000;

    console.log(`Connections: ${numConnections}`);
    console.log(`Duration: ${durationMs / 1000}s\n`);

    const results: BenchmarkResult[] = [];

    console.log('Testing WsTransport (ws library)...');
    const wsResult = await benchmarkTransport('ws', new WsTransport(), numConnections, durationMs);
    results.push(wsResult);
    console.log(`  Messages/sec: ${wsResult.messagesPerSecond.toLocaleString()}`);
    console.log(`  Latency p50: ${wsResult.latencyP50}ms, p99: ${wsResult.latencyP99}ms`);

    // Small pause between tests
    await new Promise((resolve) => setTimeout(resolve, 1000));

    console.log('\nTesting UWebSocketsTransport (uWebSockets.js)...');
    const uwsResult = await benchmarkTransport('uwebsockets', new UWebSocketsTransport(), numConnections, durationMs);
    results.push(uwsResult);
    console.log(`  Messages/sec: ${uwsResult.messagesPerSecond.toLocaleString()}`);
    console.log(`  Latency p50: ${uwsResult.latencyP50}ms, p99: ${uwsResult.latencyP99}ms`);

    // Summary
    console.log('\n=== Summary ===');
    console.log('┌───────────────┬─────────────┬───────────┬───────────┐');
    console.log('│ Transport     │ Msgs/sec    │ p50 (ms)  │ p99 (ms)  │');
    console.log('├───────────────┼─────────────┼───────────┼───────────┤');
    for (const r of results) {
        console.log(
            `│ ${r.transport.padEnd(13)} │ ${r.messagesPerSecond.toLocaleString().padStart(11)} │ ${r.latencyP50.toString().padStart(9)} │ ${r.latencyP99.toString().padStart(9)} │`,
        );
    }
    console.log('└───────────────┴─────────────┴───────────┴───────────┘');

    // Improvement
    const improvement = ((uwsResult.messagesPerSecond - wsResult.messagesPerSecond) / wsResult.messagesPerSecond) * 100;
    console.log(`\nuWebSockets.js is ${improvement > 0 ? '+' : ''}${Math.round(improvement)}% throughput vs ws`);
}

main().catch(console.error);
