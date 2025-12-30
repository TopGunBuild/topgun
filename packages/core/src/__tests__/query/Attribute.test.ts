import {
  SimpleAttribute,
  MultiValueAttribute,
  simpleAttribute,
  multiAttribute,
} from '../../query/Attribute';

interface User {
  id: string;
  email: string;
  age?: number;
  tags: string[];
}

describe('SimpleAttribute', () => {
  const emailAttr = simpleAttribute<User, string>('email', (u) => u.email);
  const ageAttr = simpleAttribute<User, number>('age', (u) => u.age);

  it('should extract simple value', () => {
    const user: User = { id: '1', email: 'test@example.com', tags: [] };
    expect(emailAttr.getValue(user)).toBe('test@example.com');
  });

  it('should return undefined for missing property', () => {
    const user: User = { id: '1', email: 'test@example.com', tags: [] };
    expect(ageAttr.getValue(user)).toBeUndefined();
  });

  it('should return single-element array from getValues', () => {
    const user: User = { id: '1', email: 'test@example.com', tags: [] };
    expect(emailAttr.getValues(user)).toEqual(['test@example.com']);
  });

  it('should return empty array from getValues when undefined', () => {
    const user: User = { id: '1', email: 'test@example.com', tags: [] };
    expect(ageAttr.getValues(user)).toEqual([]);
  });

  it('should have type simple', () => {
    expect(emailAttr.type).toBe('simple');
  });

  it('should have correct name', () => {
    expect(emailAttr.name).toBe('email');
    expect(ageAttr.name).toBe('age');
  });

  it('should work with factory function', () => {
    const attr = simpleAttribute<User, string>('email', (u) => u.email);
    expect(attr).toBeInstanceOf(SimpleAttribute);
    expect(attr.name).toBe('email');
  });
});

describe('MultiValueAttribute', () => {
  const tagsAttr = multiAttribute<User, string>('tags', (u) => u.tags);
  const emptyArrayAttr = multiAttribute<User, string>(
    'empty',
    () => []
  );

  it('should extract array of values', () => {
    const user: User = { id: '1', email: 'test@example.com', tags: ['admin', 'dev'] };
    expect(tagsAttr.getValues(user)).toEqual(['admin', 'dev']);
  });

  it('should return first value from getValue', () => {
    const user: User = { id: '1', email: 'test@example.com', tags: ['admin', 'dev'] };
    expect(tagsAttr.getValue(user)).toBe('admin');
  });

  it('should return empty array when no values', () => {
    const user: User = { id: '1', email: 'test@example.com', tags: [] };
    expect(tagsAttr.getValues(user)).toEqual([]);
  });

  it('should return undefined from getValue when no values', () => {
    const user: User = { id: '1', email: 'test@example.com', tags: [] };
    expect(tagsAttr.getValue(user)).toBeUndefined();
  });

  it('should have type multi', () => {
    expect(tagsAttr.type).toBe('multi');
  });

  it('should have correct name', () => {
    expect(tagsAttr.name).toBe('tags');
  });

  it('should work with factory function', () => {
    const attr = multiAttribute<User, string>('tags', (u) => u.tags);
    expect(attr).toBeInstanceOf(MultiValueAttribute);
    expect(attr.name).toBe('tags');
  });

  it('should handle single value in array', () => {
    const user: User = { id: '1', email: 'test@example.com', tags: ['single'] };
    expect(tagsAttr.getValues(user)).toEqual(['single']);
    expect(tagsAttr.getValue(user)).toBe('single');
  });
});

describe('Attribute edge cases', () => {
  it('should handle null values in simple attribute', () => {
    const nullableAttr = simpleAttribute<{ val: string | null }, string | null>(
      'val',
      (r) => r.val
    );
    expect(nullableAttr.getValue({ val: null })).toBeNull();
    expect(nullableAttr.getValues({ val: null })).toEqual([null]);
  });

  it('should handle numeric values', () => {
    const numAttr = simpleAttribute<{ num: number }, number>('num', (r) => r.num);
    expect(numAttr.getValue({ num: 42 })).toBe(42);
    expect(numAttr.getValue({ num: 0 })).toBe(0);
  });

  it('should handle boolean values', () => {
    const boolAttr = simpleAttribute<{ active: boolean }, boolean>('active', (r) => r.active);
    expect(boolAttr.getValue({ active: true })).toBe(true);
    expect(boolAttr.getValue({ active: false })).toBe(false);
    expect(boolAttr.getValues({ active: false })).toEqual([false]);
  });

  it('should handle nested property extraction', () => {
    interface Nested {
      user: { profile: { name: string } };
    }
    const nestedAttr = simpleAttribute<Nested, string>(
      'userName',
      (r) => r.user.profile.name
    );
    expect(nestedAttr.getValue({ user: { profile: { name: 'John' } } })).toBe('John');
  });
});
