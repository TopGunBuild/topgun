/**
 * Tests for Attribute Factory (Phase 9.02)
 *
 * Tests type-safe attribute generation functionality:
 * - generateAttributes() with schema definition
 * - Nested path support
 * - Multi-value detection
 * - attr() and multiAttr() helpers
 * - createSchema() fluent builder
 */

import {
  generateAttributes,
  attr,
  multiAttr,
  createSchema,
} from '../../query/AttributeFactory';
import type { AttributeSchema } from '../../query/AttributeFactory';

interface TestUser {
  id: string;
  name: string;
  age: number;
  isActive: boolean;
  tags: string[];
  scores: number[];
  address?: {
    street: string;
    city: string;
    country: string;
    zip: number;
  };
  metadata?: {
    preferences: {
      theme: string;
      notifications: boolean;
    };
  };
}

const testUser: TestUser = {
  id: 'user-1',
  name: 'Alice',
  age: 30,
  isActive: true,
  tags: ['developer', 'typescript', 'nodejs'],
  scores: [95, 88, 92],
  address: {
    street: '123 Main St',
    city: 'San Francisco',
    country: 'USA',
    zip: 94105,
  },
  metadata: {
    preferences: {
      theme: 'dark',
      notifications: true,
    },
  },
};

describe('generateAttributes', () => {
  it('should generate simple string attributes', () => {
    const attrs = generateAttributes<TestUser>()({
      id: 'string',
      name: 'string',
    });

    expect(attrs.id.name).toBe('id');
    expect(attrs.id.type).toBe('simple');
    expect(attrs.id.getValue(testUser)).toBe('user-1');

    expect(attrs.name.name).toBe('name');
    expect(attrs.name.getValue(testUser)).toBe('Alice');
  });

  it('should generate number attributes', () => {
    const attrs = generateAttributes<TestUser>()({
      age: 'number',
    });

    expect(attrs.age.name).toBe('age');
    expect(attrs.age.type).toBe('simple');
    expect(attrs.age.getValue(testUser)).toBe(30);
  });

  it('should generate boolean attributes', () => {
    const attrs = generateAttributes<TestUser>()({
      isActive: 'boolean',
    });

    expect(attrs.isActive.name).toBe('isActive');
    expect(attrs.isActive.getValue(testUser)).toBe(true);
  });

  it('should generate multi-value string[] attributes', () => {
    const attrs = generateAttributes<TestUser>()({
      tags: 'string[]',
    });

    expect(attrs.tags.name).toBe('tags');
    expect(attrs.tags.type).toBe('multi');
    expect(attrs.tags.getValues(testUser)).toEqual(['developer', 'typescript', 'nodejs']);
  });

  it('should generate multi-value number[] attributes', () => {
    const attrs = generateAttributes<TestUser>()({
      scores: 'number[]',
    });

    expect(attrs.scores.name).toBe('scores');
    expect(attrs.scores.type).toBe('multi');
    expect(attrs.scores.getValues(testUser)).toEqual([95, 88, 92]);
  });

  it('should support nested paths with dot notation', () => {
    const attrs = generateAttributes<TestUser>()({
      'address.city': 'string',
      'address.country': 'string',
      'address.zip': 'number',
    });

    expect(attrs['address.city'].name).toBe('address.city');
    expect(attrs['address.city'].getValue(testUser)).toBe('San Francisco');

    expect(attrs['address.country'].getValue(testUser)).toBe('USA');
    expect(attrs['address.zip'].getValue(testUser)).toBe(94105);
  });

  it('should support deeply nested paths', () => {
    const attrs = generateAttributes<TestUser>()({
      'metadata.preferences.theme': 'string',
    });

    expect(attrs['metadata.preferences.theme'].getValue(testUser)).toBe('dark');
  });

  it('should handle missing nested paths gracefully', () => {
    const userWithoutAddress: TestUser = {
      id: 'user-2',
      name: 'Bob',
      age: 25,
      isActive: false,
      tags: [],
      scores: [],
    };

    const attrs = generateAttributes<TestUser>()({
      'address.city': 'string',
    });

    expect(attrs['address.city'].getValue(userWithoutAddress)).toBeUndefined();
  });

  it('should handle null values in path', () => {
    const userWithNull = {
      id: 'user-3',
      address: null as unknown as TestUser['address'],
    } as TestUser;

    const attrs = generateAttributes<TestUser>()({
      'address.city': 'string',
    });

    expect(attrs['address.city'].getValue(userWithNull)).toBeUndefined();
  });

  it('should apply name prefix when provided', () => {
    const attrs = generateAttributes<TestUser>()(
      { id: 'string', name: 'string' },
      { namePrefix: 'user' }
    );

    expect(attrs.id.name).toBe('user.id');
    expect(attrs.name.name).toBe('user.name');
    // Values should still work
    expect(attrs.id.getValue(testUser)).toBe('user-1');
  });

  it('should return empty array for multi-value on non-array', () => {
    const userWithBadTags = {
      ...testUser,
      tags: 'not-an-array' as unknown as string[],
    };

    const attrs = generateAttributes<TestUser>()({
      tags: 'string[]',
    });

    expect(attrs.tags.getValues(userWithBadTags)).toEqual([]);
  });
});

describe('attr helper', () => {
  it('should create simple attribute for top-level path', () => {
    const nameAttr = attr<TestUser, string>('name');

    expect(nameAttr.name).toBe('name');
    expect(nameAttr.type).toBe('simple');
    expect(nameAttr.getValue(testUser)).toBe('Alice');
  });

  it('should create simple attribute for nested path', () => {
    const cityAttr = attr<TestUser, string>('address.city');

    expect(cityAttr.name).toBe('address.city');
    expect(cityAttr.getValue(testUser)).toBe('San Francisco');
  });

  it('should handle missing values', () => {
    const userWithoutAddress: Partial<TestUser> = { id: 'test' };
    const cityAttr = attr<Partial<TestUser>, string>('address.city');

    expect(cityAttr.getValue(userWithoutAddress)).toBeUndefined();
  });
});

describe('multiAttr helper', () => {
  it('should create multi-value attribute for array field', () => {
    const tagsAttr = multiAttr<TestUser, string>('tags');

    expect(tagsAttr.name).toBe('tags');
    expect(tagsAttr.type).toBe('multi');
    expect(tagsAttr.getValues(testUser)).toEqual(['developer', 'typescript', 'nodejs']);
  });

  it('should return empty array for non-array', () => {
    const userWithBadTags = { tags: 'not-array' } as unknown as TestUser;
    const tagsAttr = multiAttr<TestUser, string>('tags');

    expect(tagsAttr.getValues(userWithBadTags)).toEqual([]);
  });

  it('should return empty array for undefined', () => {
    const emptyUser = {} as TestUser;
    const tagsAttr = multiAttr<TestUser, string>('tags');

    expect(tagsAttr.getValues(emptyUser)).toEqual([]);
  });
});

describe('createSchema fluent builder', () => {
  it('should build schema with string fields', () => {
    const schema = createSchema<TestUser>()
      .string('id')
      .string('name')
      .build();

    expect(schema).toEqual({
      id: 'string',
      name: 'string',
    });
  });

  it('should build schema with number fields', () => {
    const schema = createSchema<TestUser>()
      .number('age')
      .build();

    expect(schema).toEqual({
      age: 'number',
    });
  });

  it('should build schema with boolean fields', () => {
    const schema = createSchema<TestUser>()
      .boolean('isActive')
      .build();

    expect(schema).toEqual({
      isActive: 'boolean',
    });
  });

  it('should build schema with array fields', () => {
    const schema = createSchema<TestUser>()
      .stringArray('tags')
      .numberArray('scores')
      .build();

    expect(schema).toEqual({
      tags: 'string[]',
      scores: 'number[]',
    });
  });

  it('should build complete schema with nested paths', () => {
    const schema = createSchema<TestUser>()
      .string('id')
      .string('name')
      .number('age')
      .boolean('isActive')
      .stringArray('tags')
      .string('address.city')
      .string('address.country')
      .build();

    expect(schema).toEqual({
      id: 'string',
      name: 'string',
      age: 'number',
      isActive: 'boolean',
      tags: 'string[]',
      'address.city': 'string',
      'address.country': 'string',
    });
  });

  it('should generate attributes directly', () => {
    const attrs = createSchema<TestUser>()
      .string('id')
      .string('name')
      .number('age')
      .generate();

    expect(attrs.id.getValue(testUser)).toBe('user-1');
    expect(attrs.name.getValue(testUser)).toBe('Alice');
    expect(attrs.age.getValue(testUser)).toBe(30);
  });

  it('should support generate with options', () => {
    const attrs = createSchema<TestUser>()
      .string('id')
      .generate({ namePrefix: 'test' });

    expect(attrs.id.name).toBe('test.id');
    expect(attrs.id.getValue(testUser)).toBe('user-1');
  });
});

describe('Integration with IndexedLWWMap', () => {
  it('should work with hash index', () => {
    const { HLC } = require('../../HLC');
    const { IndexedLWWMap } = require('../../IndexedLWWMap');

    const hlc = new HLC('test-node');
    const map = new IndexedLWWMap(hlc);

    const attrs = generateAttributes<TestUser>()({
      id: 'string',
      name: 'string',
      'address.city': 'string',
    });

    map.addHashIndex(attrs.id);
    map.addHashIndex(attrs['address.city']);

    map.set('user-1', testUser);

    // Query by id
    const resultById = map.query({ type: 'eq', attribute: 'id', value: 'user-1' });
    expect(resultById.toArray()).toContain('user-1');

    // Query by nested path
    const resultByCity = map.query({ type: 'eq', attribute: 'address.city', value: 'San Francisco' });
    expect(resultByCity.toArray()).toContain('user-1');
  });

  it('should work with navigable index', () => {
    const { HLC } = require('../../HLC');
    const { IndexedLWWMap } = require('../../IndexedLWWMap');

    const hlc = new HLC('test-node');
    const map = new IndexedLWWMap(hlc);

    const ageAttr = attr<TestUser, number>('age');
    map.addNavigableIndex(ageAttr);

    map.set('user-1', testUser);
    map.set('user-2', { ...testUser, id: 'user-2', age: 25 });
    map.set('user-3', { ...testUser, id: 'user-3', age: 35 });

    // Range query
    const result = map.query({ type: 'gte', attribute: 'age', value: 30 });
    const keys = result.toArray();

    expect(keys).toContain('user-1'); // age 30
    expect(keys).toContain('user-3'); // age 35
    expect(keys).not.toContain('user-2'); // age 25
  });

  it('should work with inverted index for multi-value', () => {
    const { HLC } = require('../../HLC');
    const { IndexedLWWMap } = require('../../IndexedLWWMap');

    const hlc = new HLC('test-node');
    const map = new IndexedLWWMap(hlc);

    const tagsAttr = multiAttr<TestUser, string>('tags');
    map.addHashIndex(tagsAttr);

    map.set('user-1', testUser);
    map.set('user-2', { ...testUser, id: 'user-2', tags: ['python', 'django'] });

    // Query by tag
    const result = map.query({ type: 'eq', attribute: 'tags', value: 'typescript' });
    expect(result.toArray()).toContain('user-1');
    expect(result.toArray()).not.toContain('user-2');
  });
});
