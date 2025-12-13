import { ORMapRecord } from './ORMap';
import { Timestamp } from './HLC';
import { hashString } from './utils/hash';

/**
 * Convert Timestamp to deterministic string for hashing.
 * Format: millis:counter:nodeId
 */
export function timestampToString(ts: Timestamp): string {
  return `${ts.millis}:${ts.counter}:${ts.nodeId}`;
}

/**
 * Stringify a value in a deterministic way for hashing.
 */
function stringifyValue(value: unknown): string {
  if (value === null || value === undefined) {
    return String(value);
  }
  if (typeof value === 'object') {
    // Sort object keys for deterministic JSON
    return JSON.stringify(value, Object.keys(value as Record<string, unknown>).sort());
  }
  return String(value);
}

/**
 * Hash an ORMap entry (key + all its records).
 * Must be deterministic regardless of insertion order.
 *
 * @param key The key of the entry
 * @param records Map of tag -> record for this key
 * @returns Hash as a number (FNV-1a hash)
 */
export function hashORMapEntry<V>(
  key: string,
  records: Map<string, ORMapRecord<V>>
): number {
  // Sort records by tag for deterministic ordering
  const sortedTags = Array.from(records.keys()).sort();

  // Build deterministic string representation
  const parts: string[] = [`key:${key}`];

  for (const tag of sortedTags) {
    const record = records.get(tag)!;
    // Include tag, value (JSON-stringified), timestamp, and ttl if present
    const valuePart = stringifyValue(record.value);

    let recordStr = `${tag}:${valuePart}:${timestampToString(record.timestamp)}`;
    if (record.ttlMs !== undefined) {
      recordStr += `:ttl=${record.ttlMs}`;
    }
    parts.push(recordStr);
  }

  return hashString(parts.join('|'));
}

/**
 * Hash a single ORMapRecord for comparison.
 * Used when comparing individual records during merge.
 */
export function hashORMapRecord<V>(record: ORMapRecord<V>): number {
  const valuePart = stringifyValue(record.value);

  let str = `${record.tag}:${valuePart}:${timestampToString(record.timestamp)}`;
  if (record.ttlMs !== undefined) {
    str += `:ttl=${record.ttlMs}`;
  }

  return hashString(str);
}

/**
 * Compare two timestamps.
 * Returns:
 *   < 0 if a < b
 *   > 0 if a > b
 *   = 0 if a == b
 */
export function compareTimestamps(a: Timestamp, b: Timestamp): number {
  if (a.millis !== b.millis) {
    return a.millis - b.millis;
  }
  if (a.counter !== b.counter) {
    return a.counter - b.counter;
  }
  return a.nodeId.localeCompare(b.nodeId);
}
