/**
 * Serialization Micro-Benchmarks
 *
 * Measures performance of MessagePack serialization:
 * - serialize(): Convert JS object to binary
 * - deserialize(): Convert binary to JS object
 *
 * Tests with realistic message sizes used in TopGun protocol.
 */

import { bench, describe } from 'vitest';
import { serialize, deserialize } from '../index';

describe('Serialization', () => {
  // Small message (AUTH)
  const smallMessage = {
    type: 'AUTH',
    token: 'eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJ1c2VySWQiOiJ0ZXN0In0.test',
  };
  const smallBinary = serialize(smallMessage);

  // Medium message (single CLIENT_OP)
  const mediumMessage = {
    type: 'CLIENT_OP',
    payload: {
      id: 'op-12345',
      mapName: 'users',
      opType: 'PUT',
      key: 'user-abc-123',
      record: {
        value: { name: 'John Doe', email: 'john@example.com', age: 30 },
        timestamp: { millis: Date.now(), counter: 42, nodeId: 'node-xyz' },
      },
    },
  };
  const mediumBinary = serialize(mediumMessage);

  // Large message (OP_BATCH with 10 operations)
  const largeMessage = {
    type: 'OP_BATCH',
    payload: {
      ops: Array.from({ length: 10 }, (_, i) => ({
        id: `op-${i}`,
        mapName: 'test-map',
        opType: 'PUT',
        key: `key-${i}`,
        record: {
          value: { data: `value-${i}`, index: i, nested: { x: i, y: i * 2 } },
          timestamp: { millis: Date.now(), counter: i, nodeId: 'node-1' },
        },
      })),
    },
  };
  const largeBinary = serialize(largeMessage);

  // Extra large message (OP_BATCH with 50 operations)
  const xlMessage = {
    type: 'OP_BATCH',
    payload: {
      ops: Array.from({ length: 50 }, (_, i) => ({
        id: `op-${i}`,
        mapName: `map-${i % 10}`,
        opType: 'PUT',
        key: `key-${i}`,
        record: {
          value: {
            data: `value-${i}`,
            description: `This is item number ${i} with some extra text`,
            metadata: { created: Date.now(), version: i, tags: ['a', 'b', 'c'] },
          },
          timestamp: { millis: Date.now(), counter: i, nodeId: 'node-bench' },
        },
      })),
    },
  };
  const xlBinary = serialize(xlMessage);

  describe('serialize()', () => {
    bench('small message (AUTH)', () => {
      serialize(smallMessage);
    });

    bench('medium message (CLIENT_OP)', () => {
      serialize(mediumMessage);
    });

    bench('large message (10 ops batch)', () => {
      serialize(largeMessage);
    });

    bench('XL message (50 ops batch)', () => {
      serialize(xlMessage);
    });
  });

  describe('deserialize()', () => {
    bench('small message (AUTH)', () => {
      deserialize(smallBinary);
    });

    bench('medium message (CLIENT_OP)', () => {
      deserialize(mediumBinary);
    });

    bench('large message (10 ops batch)', () => {
      deserialize(largeBinary);
    });

    bench('XL message (50 ops batch)', () => {
      deserialize(xlBinary);
    });
  });

  describe('roundtrip', () => {
    bench('medium message roundtrip', () => {
      deserialize(serialize(mediumMessage));
    });

    bench('large message roundtrip', () => {
      deserialize(serialize(largeMessage));
    });
  });
});
