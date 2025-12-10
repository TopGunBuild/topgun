/**
 * Utility functions for processing metrics data from TopGun server
 */

/** Labeled metric value from Prometheus */
export interface LabeledMetric {
    value: number;
    labels?: Record<string, string>;
}

/**
 * Extract numeric value from a metric.
 * Handles both simple numbers and arrays of labeled metrics (Prometheus format).
 *
 * @example
 * // Simple number
 * getMetricValue(42) // => 42
 *
 * // Array of labeled metrics (sums all values)
 * getMetricValue([
 *   { value: 50, labels: { type: 'PUT' } },
 *   { value: 100, labels: { type: 'GET' } }
 * ]) // => 150
 *
 * // Invalid input
 * getMetricValue(undefined) // => 0
 */
export function getMetricValue(metric: unknown): number {
    if (typeof metric === 'number') {
        return metric;
    }
    if (Array.isArray(metric)) {
        return metric.reduce((sum: number, m: LabeledMetric) => sum + (m?.value || 0), 0);
    }
    return 0;
}

/**
 * Format bytes to human-readable format.
 *
 * @example
 * formatBytes(0) // => '0 B'
 * formatBytes(1024) // => '1.0 KB'
 * formatBytes(1536) // => '1.5 KB'
 * formatBytes(1048576) // => '1.0 MB'
 * formatBytes(1073741824) // => '1.0 GB'
 */
export function formatBytes(bytes: number): string {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`;
}

/**
 * Create a lookup map from an array of items with _key field.
 *
 * @example
 * const items = [
 *   { _key: 'node-1', name: 'Node 1' },
 *   { _key: 'node-2', name: 'Node 2' }
 * ];
 * createLookupByKey(items) // => { 'node-1': {...}, 'node-2': {...} }
 */
export function createLookupByKey<T extends { _key: string }>(
    items: T[]
): Record<string, T> {
    const lookup: Record<string, T> = {};
    for (const item of items) {
        if (item._key) {
            lookup[item._key] = item;
        }
    }
    return lookup;
}
