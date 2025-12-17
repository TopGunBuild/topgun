import type { CoalescingWriterOptions } from './CoalescingWriter';

/**
 * Preset configurations for CoalescingWriter.
 * Based on Hazelcast OutboxImpl (batch size 2048) and real-world benchmarking.
 *
 * Trade-offs:
 * - Larger batch size = higher throughput, higher latency
 * - Longer delay = more messages per batch, higher latency
 * - Larger maxBatchBytes = handles larger payloads, more memory
 */
export const coalescingPresets = {
    /**
     * Conservative defaults - good for low-latency workloads.
     * Minimizes batching delay at the cost of more network calls.
     * Use for: gaming, real-time chat, interactive applications.
     */
    conservative: {
        maxBatchSize: 100,
        maxDelayMs: 5,
        maxBatchBytes: 65536, // 64KB
    },

    /**
     * Balanced - good for most workloads.
     * Reasonable trade-off between throughput and latency.
     * Use for: mixed read/write applications, general purpose.
     */
    balanced: {
        maxBatchSize: 300,
        maxDelayMs: 8,
        maxBatchBytes: 131072, // 128KB
    },

    /**
     * High throughput - optimized for write-heavy workloads.
     * Higher batching for better network utilization.
     * Use for: data ingestion, logging, IoT data streams.
     */
    highThroughput: {
        maxBatchSize: 500,
        maxDelayMs: 10,
        maxBatchBytes: 262144, // 256KB
    },

    /**
     * Aggressive - maximum batching for batch processing.
     * Closest to Hazelcast's OutboxImpl (batch size 2048).
     * Use for: batch imports, bulk operations, offline sync.
     */
    aggressive: {
        maxBatchSize: 1000,
        maxDelayMs: 15,
        maxBatchBytes: 524288, // 512KB
    },
} as const satisfies Record<string, CoalescingWriterOptions>;

/**
 * Available preset names for type safety.
 */
export type CoalescingPreset = keyof typeof coalescingPresets;

/**
 * Get preset configuration by name.
 * @param preset - Preset name
 * @returns CoalescingWriterOptions
 */
export function getCoalescingPreset(preset: CoalescingPreset): CoalescingWriterOptions {
    return { ...coalescingPresets[preset] };
}
