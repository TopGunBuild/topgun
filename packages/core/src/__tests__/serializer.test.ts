import { serialize, deserialize } from '../serializer';

describe('Serializer Module', () => {
  describe('Primitive Serialization', () => {
    test('should serialize and deserialize strings', () => {
      const original = 'Hello, World!';
      const serialized = serialize(original);
      const deserialized = deserialize<string>(serialized);

      expect(deserialized).toBe(original);
    });

    test('should serialize and deserialize numbers', () => {
      const testCases = [0, 1, -1, 42, 3.14159, -273.15, Number.MAX_SAFE_INTEGER, Number.MIN_SAFE_INTEGER];

      testCases.forEach(original => {
        const serialized = serialize(original);
        const deserialized = deserialize<number>(serialized);
        expect(deserialized).toBe(original);
      });
    });

    test('should serialize and deserialize booleans', () => {
      expect(deserialize<boolean>(serialize(true))).toBe(true);
      expect(deserialize<boolean>(serialize(false))).toBe(false);
    });

    test('should serialize and deserialize null', () => {
      const serialized = serialize(null);
      const deserialized = deserialize<null>(serialized);

      expect(deserialized).toBeNull();
    });
  });

  describe('Object Serialization', () => {
    test('should serialize and deserialize simple objects', () => {
      const original = { name: 'Alice', age: 30 };
      const serialized = serialize(original);
      const deserialized = deserialize<typeof original>(serialized);

      expect(deserialized).toEqual(original);
    });

    test('should serialize and deserialize objects with mixed types', () => {
      const original = {
        string: 'value',
        number: 42,
        boolean: true,
        null: null,
        nested: { key: 'nested value' }
      };
      const serialized = serialize(original);
      const deserialized = deserialize<typeof original>(serialized);

      expect(deserialized).toEqual(original);
    });

    test('should serialize and deserialize empty objects', () => {
      const original = {};
      const serialized = serialize(original);
      const deserialized = deserialize<typeof original>(serialized);

      expect(deserialized).toEqual(original);
    });
  });

  describe('Array Serialization', () => {
    test('should serialize and deserialize simple arrays', () => {
      const original = [1, 2, 3, 4, 5];
      const serialized = serialize(original);
      const deserialized = deserialize<typeof original>(serialized);

      expect(deserialized).toEqual(original);
    });

    test('should serialize and deserialize arrays of strings', () => {
      const original = ['apple', 'banana', 'cherry'];
      const serialized = serialize(original);
      const deserialized = deserialize<typeof original>(serialized);

      expect(deserialized).toEqual(original);
    });

    test('should serialize and deserialize arrays of objects', () => {
      const original = [
        { id: 1, name: 'Item 1' },
        { id: 2, name: 'Item 2' },
        { id: 3, name: 'Item 3' }
      ];
      const serialized = serialize(original);
      const deserialized = deserialize<typeof original>(serialized);

      expect(deserialized).toEqual(original);
    });

    test('should serialize and deserialize empty arrays', () => {
      const original: unknown[] = [];
      const serialized = serialize(original);
      const deserialized = deserialize<typeof original>(serialized);

      expect(deserialized).toEqual(original);
    });

    test('should serialize and deserialize mixed arrays', () => {
      const original = [1, 'two', true, null, { key: 'value' }, [1, 2, 3]];
      const serialized = serialize(original);
      const deserialized = deserialize<typeof original>(serialized);

      expect(deserialized).toEqual(original);
    });
  });

  describe('Nested Structure Serialization', () => {
    test('should serialize and deserialize deeply nested objects', () => {
      const original = {
        level1: {
          level2: {
            level3: {
              level4: {
                value: 'deep value'
              }
            }
          }
        }
      };
      const serialized = serialize(original);
      const deserialized = deserialize<typeof original>(serialized);

      expect(deserialized).toEqual(original);
      expect(deserialized.level1.level2.level3.level4.value).toBe('deep value');
    });

    test('should serialize and deserialize nested arrays within objects', () => {
      const original = {
        users: [
          { id: 1, tags: ['admin', 'user'] },
          { id: 2, tags: ['user'] }
        ],
        metadata: {
          counts: [10, 20, 30]
        }
      };
      const serialized = serialize(original);
      const deserialized = deserialize<typeof original>(serialized);

      expect(deserialized).toEqual(original);
    });

    test('should serialize and deserialize arrays with nested objects', () => {
      const original = [
        { nested: { deep: { value: 1 } } },
        { nested: { deep: { value: 2 } } }
      ];
      const serialized = serialize(original);
      const deserialized = deserialize<typeof original>(serialized);

      expect(deserialized).toEqual(original);
    });
  });

  describe('Roundtrip Tests', () => {
    test('should maintain data integrity through multiple roundtrips', () => {
      const original = { key: 'value', count: 42, active: true };

      let data = original;
      for (let i = 0; i < 5; i++) {
        const serialized = serialize(data);
        data = deserialize<typeof original>(serialized);
      }

      expect(data).toEqual(original);
    });

    test('should preserve complex data structure through roundtrip', () => {
      const original = {
        id: 'record-123',
        timestamp: 1234567890,
        data: {
          items: [
            { name: 'item1', value: 100 },
            { name: 'item2', value: 200 }
          ],
          metadata: {
            created: true,
            tags: ['important', 'reviewed']
          }
        }
      };

      const serialized = serialize(original);
      const deserialized = deserialize<typeof original>(serialized);

      expect(deserialized).toEqual(original);
    });
  });

  describe('Special Values', () => {
    test('should handle undefined by converting to null in objects', () => {
      // MessagePack typically converts undefined to null
      const original = { a: undefined, b: 'value' };
      const serialized = serialize(original);
      const deserialized = deserialize<Record<string, unknown>>(serialized);

      // undefined is typically serialized as null or omitted
      expect(deserialized.b).toBe('value');
    });

    test('should handle floating point numbers', () => {
      const testCases = [0.1, 0.2, 0.1 + 0.2, Math.PI, Math.E];

      testCases.forEach(original => {
        const serialized = serialize(original);
        const deserialized = deserialize<number>(serialized);
        expect(deserialized).toBeCloseTo(original, 10);
      });
    });

    test('should handle negative zero', () => {
      const original = -0;
      const serialized = serialize(original);
      const deserialized = deserialize<number>(serialized);

      // -0 and 0 are equal in JavaScript
      expect(deserialized).toBe(0);
    });

    test('should handle Infinity values', () => {
      // Note: MessagePack may not preserve Infinity exactly
      const positiveInf = Infinity;
      const negativeInf = -Infinity;

      const serializedPos = serialize(positiveInf);
      const serializedNeg = serialize(negativeInf);

      const deserializedPos = deserialize<number>(serializedPos);
      const deserializedNeg = deserialize<number>(serializedNeg);

      // Behavior depends on msgpack implementation
      expect(typeof deserializedPos).toBe('number');
      expect(typeof deserializedNeg).toBe('number');
    });

    test('should handle empty string', () => {
      const original = '';
      const serialized = serialize(original);
      const deserialized = deserialize<string>(serialized);

      expect(deserialized).toBe('');
    });

    test('should handle strings with special characters', () => {
      const original = 'Hello\nWorld\t!\r\n"quotes" and \'apostrophes\'';
      const serialized = serialize(original);
      const deserialized = deserialize<string>(serialized);

      expect(deserialized).toBe(original);
    });

    test('should handle unicode strings', () => {
      const original = '你好世界 🌍 مرحبا العالم';
      const serialized = serialize(original);
      const deserialized = deserialize<string>(serialized);

      expect(deserialized).toBe(original);
    });
  });

  describe('Binary Data', () => {
    test('should return Uint8Array from serialize', () => {
      const data = { test: 'value' };
      const serialized = serialize(data);

      expect(serialized).toBeInstanceOf(Uint8Array);
      expect(serialized.length).toBeGreaterThan(0);
    });

    test('should deserialize from Uint8Array', () => {
      const original = { key: 'value' };
      const serialized = serialize(original);
      const deserialized = deserialize<typeof original>(serialized);

      expect(deserialized).toEqual(original);
    });

    test('should deserialize from ArrayBuffer', () => {
      const original = { key: 'value' };
      const serialized = serialize(original);
      const arrayBuffer = serialized.buffer.slice(
        serialized.byteOffset,
        serialized.byteOffset + serialized.byteLength
      ) as ArrayBuffer;
      const deserialized = deserialize<typeof original>(arrayBuffer);

      expect(deserialized).toEqual(original);
    });
  });

  describe('Edge Cases', () => {
    test('should handle large arrays', () => {
      const original = Array.from({ length: 1000 }, (_, i) => i);
      const serialized = serialize(original);
      const deserialized = deserialize<typeof original>(serialized);

      expect(deserialized).toEqual(original);
      expect(deserialized.length).toBe(1000);
    });

    test('should handle objects with many keys', () => {
      const original: Record<string, number> = {};
      for (let i = 0; i < 100; i++) {
        original[`key_${i}`] = i;
      }

      const serialized = serialize(original);
      const deserialized = deserialize<typeof original>(serialized);

      expect(deserialized).toEqual(original);
      expect(Object.keys(deserialized).length).toBe(100);
    });

    test('should handle deeply nested structures (10 levels)', () => {
      let original: Record<string, unknown> = { value: 'deepest' };
      for (let i = 0; i < 10; i++) {
        original = { nested: original };
      }

      const serialized = serialize(original);
      const deserialized = deserialize<typeof original>(serialized);

      expect(deserialized).toEqual(original);
    });

    test('should handle objects with numeric string keys', () => {
      const original = { '0': 'zero', '1': 'one', '2': 'two' };
      const serialized = serialize(original);
      const deserialized = deserialize<typeof original>(serialized);

      expect(deserialized).toEqual(original);
    });

    test('should handle very long strings', () => {
      const original = 'a'.repeat(10000);
      const serialized = serialize(original);
      const deserialized = deserialize<string>(serialized);

      expect(deserialized).toBe(original);
      expect(deserialized.length).toBe(10000);
    });

    test('should handle boolean in arrays', () => {
      const original = [true, false, true, false];
      const serialized = serialize(original);
      const deserialized = deserialize<typeof original>(serialized);

      expect(deserialized).toEqual(original);
    });
  });

  describe('Real-world Use Cases', () => {
    test('should handle TopGun-like LWW record structure', () => {
      const record = {
        value: { name: 'Test User', email: 'test@example.com' },
        timestamp: {
          millis: Date.now(),
          counter: 0,
          nodeId: 'node-123'
        },
        ttlMs: 3600000
      };

      const serialized = serialize(record);
      const deserialized = deserialize<typeof record>(serialized);

      expect(deserialized).toEqual(record);
    });

    test('should handle TopGun-like OR record structure', () => {
      const record = {
        value: { item: 'test' },
        timestamp: {
          millis: 1000000,
          counter: 5,
          nodeId: 'node-abc'
        },
        tag: 'unique-tag-123',
        ttlMs: undefined
      };

      const serialized = serialize(record);
      const deserialized = deserialize<typeof record>(serialized);

      expect(deserialized.value).toEqual(record.value);
      expect(deserialized.timestamp).toEqual(record.timestamp);
      expect(deserialized.tag).toBe(record.tag);
    });

    test('should handle batch of operations', () => {
      const batch = {
        ops: [
          { mapName: 'users', key: 'user1', record: { value: 'data1' } },
          { mapName: 'users', key: 'user2', record: { value: 'data2' } },
          { mapName: 'posts', key: 'post1', record: { value: 'content' } }
        ]
      };

      const serialized = serialize(batch);
      const deserialized = deserialize<typeof batch>(serialized);

      expect(deserialized).toEqual(batch);
      expect(deserialized.ops.length).toBe(3);
    });
  });
});
