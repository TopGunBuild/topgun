/**
 * Parameterized server for A/B performance testing
 *
 * Environment variables:
 *   PERF_PROFILE - Performance profile to use:
 *     - baseline      (default highThroughput preset)
 *     - low-delay     (writeCoalescingMaxDelayMs: 1)
 *     - large-batch   (writeCoalescingMaxBatch: 1000)
 *     - no-coalescing (writeCoalescingEnabled: false)
 *     - aggressive    (all optimizations combined)
 */
import { ServerFactory, MemoryServerAdapter } from '@topgunbuild/server';

type PerfProfile = 'baseline' | 'low-delay' | 'large-batch' | 'no-coalescing' | 'aggressive';

const PROFILE = (process.env.PERF_PROFILE || 'baseline') as PerfProfile;
const PORT = parseInt(process.env.PORT || '8080');

interface ProfileConfig {
    writeCoalescingEnabled?: boolean;
    writeCoalescingPreset?: 'conservative' | 'balanced' | 'highThroughput' | 'aggressive';
    writeCoalescingMaxDelayMs?: number;
    writeCoalescingMaxBatch?: number;
    writeCoalescingMaxBytes?: number;
    backpressureEnabled?: boolean;
}

const profiles: Record<PerfProfile, ProfileConfig> = {
    // Baseline: current default settings (highThroughput preset)
    'baseline': {
        writeCoalescingEnabled: true,
        writeCoalescingPreset: 'highThroughput',
        // maxBatchSize: 500, maxDelayMs: 10, maxBatchBytes: 256KB
    },

    // Low delay: minimize timer-based latency
    'low-delay': {
        writeCoalescingEnabled: true,
        writeCoalescingPreset: 'highThroughput',
        writeCoalescingMaxDelayMs: 1, // 10ms -> 1ms
    },

    // Large batch: increase batch size to reduce flush frequency
    'large-batch': {
        writeCoalescingEnabled: true,
        writeCoalescingPreset: 'highThroughput',
        writeCoalescingMaxBatch: 1000, // 500 -> 1000
    },

    // No coalescing: direct writes (baseline for measuring coalescing overhead)
    'no-coalescing': {
        writeCoalescingEnabled: false,
    },

    // Aggressive: all optimizations combined
    'aggressive': {
        writeCoalescingEnabled: true,
        writeCoalescingPreset: 'aggressive',
        writeCoalescingMaxDelayMs: 1,
        writeCoalescingMaxBatch: 2000,
        writeCoalescingMaxBytes: 512 * 1024, // 512KB
        backpressureEnabled: true,
    },
};

const config = profiles[PROFILE];

console.log('');
console.log('╔══════════════════════════════════════════════════════════════════╗');
console.log('║              TOPGUN PERFORMANCE A/B TEST SERVER                  ║');
console.log('╠══════════════════════════════════════════════════════════════════╣');
console.log(`║  Profile: ${PROFILE.padEnd(55)}║`);
console.log(`║  Port: ${PORT.toString().padEnd(58)}║`);
console.log('╠══════════════════════════════════════════════════════════════════╣');
console.log('║  Configuration:                                                  ║');

Object.entries(config).forEach(([key, value]) => {
    const line = `    ${key}: ${JSON.stringify(value)}`;
    console.log(`║  ${line.padEnd(63)}║`);
});

console.log('╚══════════════════════════════════════════════════════════════════╝');
console.log('');

const server = ServerFactory.create({
    port: PORT,
    clusterPort: parseInt(process.env.CLUSTER_PORT || '9080'),
    metricsPort: parseInt(process.env.METRICS_PORT || '9091'),
    nodeId: process.env.NODE_ID || `perf-${PROFILE}`,
    storage: new MemoryServerAdapter(),

    // Apply profile configuration
    ...config,

    // Permissive security for testing
    securityPolicies: [
        {
            role: 'USER',
            mapNamePattern: '*',
            actions: ['ALL']
        },
        {
            role: 'ADMIN',
            mapNamePattern: '*',
            actions: ['ALL']
        }
    ]
});

console.log(`Server running on ws://localhost:${PORT} with profile: ${PROFILE}`);

// Graceful Shutdown
const shutdown = async (signal: string) => {
    console.log(`\nReceived ${signal}. Shutting down...`);
    try {
        await server.shutdown();
        console.log('Shutdown complete.');
        process.exit(0);
    } catch (err) {
        console.error('Shutdown error:', err);
        process.exit(1);
    }
};

process.on('SIGTERM', () => shutdown('SIGTERM'));
process.on('SIGINT', () => shutdown('SIGINT'));
