import { Predicates, PredicateNode, evaluatePredicate } from '../predicate';

describe('Predicate', () => {
  describe('Predicates class - Node creation', () => {
    test('equal() should create eq predicate node', () => {
      const node = Predicates.equal('name', 'Alice');
      expect(node).toEqual({ op: 'eq', attribute: 'name', value: 'Alice' });
    });

    test('notEqual() should create neq predicate node', () => {
      const node = Predicates.notEqual('status', 'inactive');
      expect(node).toEqual({ op: 'neq', attribute: 'status', value: 'inactive' });
    });

    test('greaterThan() should create gt predicate node', () => {
      const node = Predicates.greaterThan('age', 18);
      expect(node).toEqual({ op: 'gt', attribute: 'age', value: 18 });
    });

    test('greaterThanOrEqual() should create gte predicate node', () => {
      const node = Predicates.greaterThanOrEqual('score', 100);
      expect(node).toEqual({ op: 'gte', attribute: 'score', value: 100 });
    });

    test('lessThan() should create lt predicate node', () => {
      const node = Predicates.lessThan('price', 50);
      expect(node).toEqual({ op: 'lt', attribute: 'price', value: 50 });
    });

    test('lessThanOrEqual() should create lte predicate node', () => {
      const node = Predicates.lessThanOrEqual('quantity', 10);
      expect(node).toEqual({ op: 'lte', attribute: 'quantity', value: 10 });
    });

    test('like() should create like predicate node', () => {
      const node = Predicates.like('email', '%@gmail.com');
      expect(node).toEqual({ op: 'like', attribute: 'email', value: '%@gmail.com' });
    });

    test('regex() should create regex predicate node', () => {
      const node = Predicates.regex('phone', '^\\+1');
      expect(node).toEqual({ op: 'regex', attribute: 'phone', value: '^\\+1' });
    });

    test('between() should create compound and predicate with gte and lte', () => {
      const node = Predicates.between('age', 18, 65);
      expect(node).toEqual({
        op: 'and',
        children: [
          { op: 'gte', attribute: 'age', value: 18 },
          { op: 'lte', attribute: 'age', value: 65 }
        ]
      });
    });
  });

  describe('Logical operators - Node creation', () => {
    test('and() should combine multiple predicates', () => {
      const node = Predicates.and(
        Predicates.equal('status', 'active'),
        Predicates.greaterThan('age', 18)
      );
      expect(node.op).toBe('and');
      expect(node.children).toHaveLength(2);
    });

    test('or() should combine multiple predicates', () => {
      const node = Predicates.or(
        Predicates.equal('role', 'admin'),
        Predicates.equal('role', 'moderator')
      );
      expect(node.op).toBe('or');
      expect(node.children).toHaveLength(2);
    });

    test('not() should negate a predicate', () => {
      const node = Predicates.not(Predicates.equal('banned', true));
      expect(node.op).toBe('not');
      expect(node.children).toHaveLength(1);
      expect(node.children![0]).toEqual({ op: 'eq', attribute: 'banned', value: true });
    });

    test('and() with no arguments should create empty children array', () => {
      const node = Predicates.and();
      expect(node).toEqual({ op: 'and', children: [] });
    });

    test('or() with no arguments should create empty children array', () => {
      const node = Predicates.or();
      expect(node).toEqual({ op: 'or', children: [] });
    });
  });

  describe('evaluatePredicate - Comparison operators', () => {
    test('eq should match equal values', () => {
      const predicate = Predicates.equal('name', 'Alice');
      expect(evaluatePredicate(predicate, { name: 'Alice' })).toBe(true);
      expect(evaluatePredicate(predicate, { name: 'Bob' })).toBe(false);
    });

    test('eq should handle strict equality (no type coercion)', () => {
      const predicate = Predicates.equal('value', 1);
      expect(evaluatePredicate(predicate, { value: 1 })).toBe(true);
      expect(evaluatePredicate(predicate, { value: '1' })).toBe(false);
    });

    test('neq should match non-equal values', () => {
      const predicate = Predicates.notEqual('status', 'deleted');
      expect(evaluatePredicate(predicate, { status: 'active' })).toBe(true);
      expect(evaluatePredicate(predicate, { status: 'deleted' })).toBe(false);
    });

    test('gt should match greater values', () => {
      const predicate = Predicates.greaterThan('age', 18);
      expect(evaluatePredicate(predicate, { age: 19 })).toBe(true);
      expect(evaluatePredicate(predicate, { age: 18 })).toBe(false);
      expect(evaluatePredicate(predicate, { age: 17 })).toBe(false);
    });

    test('gte should match greater or equal values', () => {
      const predicate = Predicates.greaterThanOrEqual('score', 100);
      expect(evaluatePredicate(predicate, { score: 101 })).toBe(true);
      expect(evaluatePredicate(predicate, { score: 100 })).toBe(true);
      expect(evaluatePredicate(predicate, { score: 99 })).toBe(false);
    });

    test('lt should match lesser values', () => {
      const predicate = Predicates.lessThan('price', 50);
      expect(evaluatePredicate(predicate, { price: 49 })).toBe(true);
      expect(evaluatePredicate(predicate, { price: 50 })).toBe(false);
      expect(evaluatePredicate(predicate, { price: 51 })).toBe(false);
    });

    test('lte should match lesser or equal values', () => {
      const predicate = Predicates.lessThanOrEqual('quantity', 10);
      expect(evaluatePredicate(predicate, { quantity: 9 })).toBe(true);
      expect(evaluatePredicate(predicate, { quantity: 10 })).toBe(true);
      expect(evaluatePredicate(predicate, { quantity: 11 })).toBe(false);
    });

    test('comparison operators should work with strings', () => {
      const predicate = Predicates.greaterThan('name', 'B');
      expect(evaluatePredicate(predicate, { name: 'Charlie' })).toBe(true);
      expect(evaluatePredicate(predicate, { name: 'Alice' })).toBe(false);
    });

    test('comparison operators should work with dates (as numbers)', () => {
      const now = Date.now();
      const predicate = Predicates.greaterThan('createdAt', now - 1000);
      expect(evaluatePredicate(predicate, { createdAt: now })).toBe(true);
      expect(evaluatePredicate(predicate, { createdAt: now - 2000 })).toBe(false);
    });
  });

  describe('evaluatePredicate - String operators', () => {
    describe('like operator (SQL-style)', () => {
      test('% should match any sequence of characters', () => {
        const predicate = Predicates.like('email', '%@gmail.com');
        expect(evaluatePredicate(predicate, { email: 'user@gmail.com' })).toBe(true);
        expect(evaluatePredicate(predicate, { email: 'test.user@gmail.com' })).toBe(true);
        expect(evaluatePredicate(predicate, { email: 'user@yahoo.com' })).toBe(false);
      });

      test('% at start should match suffix', () => {
        const predicate = Predicates.like('filename', '%.txt');
        expect(evaluatePredicate(predicate, { filename: 'document.txt' })).toBe(true);
        expect(evaluatePredicate(predicate, { filename: 'image.png' })).toBe(false);
      });

      test('% at end should match prefix', () => {
        const predicate = Predicates.like('url', 'https://%');
        expect(evaluatePredicate(predicate, { url: 'https://example.com' })).toBe(true);
        expect(evaluatePredicate(predicate, { url: 'http://example.com' })).toBe(false);
      });

      test('_ should match single character', () => {
        const predicate = Predicates.like('code', 'A_C');
        expect(evaluatePredicate(predicate, { code: 'ABC' })).toBe(true);
        expect(evaluatePredicate(predicate, { code: 'ADC' })).toBe(true);
        expect(evaluatePredicate(predicate, { code: 'ABBC' })).toBe(false);
        expect(evaluatePredicate(predicate, { code: 'AC' })).toBe(false);
      });

      test('like should be case-insensitive', () => {
        const predicate = Predicates.like('name', 'john%');
        expect(evaluatePredicate(predicate, { name: 'John Doe' })).toBe(true);
        expect(evaluatePredicate(predicate, { name: 'JOHN SMITH' })).toBe(true);
        expect(evaluatePredicate(predicate, { name: 'johnny' })).toBe(true);
      });

      test('like should escape regex special characters', () => {
        const predicate = Predicates.like('text', 'test.value%');
        expect(evaluatePredicate(predicate, { text: 'test.value123' })).toBe(true);
        expect(evaluatePredicate(predicate, { text: 'testXvalue123' })).toBe(false);
      });

      test('like should return false for non-string values', () => {
        const predicate = Predicates.like('value', '%test%');
        expect(evaluatePredicate(predicate, { value: 123 })).toBe(false);
        expect(evaluatePredicate(predicate, { value: null })).toBe(false);
        expect(evaluatePredicate(predicate, { value: { nested: 'test' } })).toBe(false);
      });
    });

    describe('regex operator', () => {
      test('should match regex pattern', () => {
        const predicate = Predicates.regex('phone', '^\\+1\\d{10}$');
        expect(evaluatePredicate(predicate, { phone: '+11234567890' })).toBe(true);
        expect(evaluatePredicate(predicate, { phone: '+441234567890' })).toBe(false);
        expect(evaluatePredicate(predicate, { phone: '1234567890' })).toBe(false);
      });

      test('should match email pattern', () => {
        const predicate = Predicates.regex('email', '^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}$');
        expect(evaluatePredicate(predicate, { email: 'test@example.com' })).toBe(true);
        expect(evaluatePredicate(predicate, { email: 'invalid-email' })).toBe(false);
      });

      test('regex should be case-sensitive by default', () => {
        const predicate = Predicates.regex('text', '^Hello');
        expect(evaluatePredicate(predicate, { text: 'Hello World' })).toBe(true);
        expect(evaluatePredicate(predicate, { text: 'hello world' })).toBe(false);
      });

      test('regex should return false for non-string values', () => {
        const predicate = Predicates.regex('value', '\\d+');
        expect(evaluatePredicate(predicate, { value: 123 })).toBe(false);
        expect(evaluatePredicate(predicate, { value: null })).toBe(false);
      });
    });
  });

  describe('evaluatePredicate - Logical operators', () => {
    test('and should require all conditions to be true', () => {
      const predicate = Predicates.and(
        Predicates.equal('status', 'active'),
        Predicates.greaterThan('age', 18),
        Predicates.lessThan('age', 65)
      );
      expect(evaluatePredicate(predicate, { status: 'active', age: 30 })).toBe(true);
      expect(evaluatePredicate(predicate, { status: 'inactive', age: 30 })).toBe(false);
      expect(evaluatePredicate(predicate, { status: 'active', age: 17 })).toBe(false);
      expect(evaluatePredicate(predicate, { status: 'active', age: 70 })).toBe(false);
    });

    test('or should require at least one condition to be true', () => {
      const predicate = Predicates.or(
        Predicates.equal('role', 'admin'),
        Predicates.equal('role', 'moderator'),
        Predicates.equal('role', 'owner')
      );
      expect(evaluatePredicate(predicate, { role: 'admin' })).toBe(true);
      expect(evaluatePredicate(predicate, { role: 'moderator' })).toBe(true);
      expect(evaluatePredicate(predicate, { role: 'owner' })).toBe(true);
      expect(evaluatePredicate(predicate, { role: 'user' })).toBe(false);
    });

    test('not should negate the condition', () => {
      const predicate = Predicates.not(Predicates.equal('banned', true));
      expect(evaluatePredicate(predicate, { banned: false })).toBe(true);
      expect(evaluatePredicate(predicate, { banned: true })).toBe(false);
    });

    test('empty and should return true (vacuous truth)', () => {
      const predicate = Predicates.and();
      expect(evaluatePredicate(predicate, { anything: 'value' })).toBe(true);
    });

    test('empty or should return false', () => {
      const predicate = Predicates.or();
      expect(evaluatePredicate(predicate, { anything: 'value' })).toBe(false);
    });

    test('nested logical operators should work', () => {
      // (role = 'admin' OR role = 'moderator') AND status = 'active'
      const predicate = Predicates.and(
        Predicates.or(
          Predicates.equal('role', 'admin'),
          Predicates.equal('role', 'moderator')
        ),
        Predicates.equal('status', 'active')
      );
      expect(evaluatePredicate(predicate, { role: 'admin', status: 'active' })).toBe(true);
      expect(evaluatePredicate(predicate, { role: 'moderator', status: 'active' })).toBe(true);
      expect(evaluatePredicate(predicate, { role: 'admin', status: 'inactive' })).toBe(false);
      expect(evaluatePredicate(predicate, { role: 'user', status: 'active' })).toBe(false);
    });

    test('deeply nested logical operators should work', () => {
      // NOT ((status = 'deleted' OR status = 'banned') AND age < 18)
      const predicate = Predicates.not(
        Predicates.and(
          Predicates.or(
            Predicates.equal('status', 'deleted'),
            Predicates.equal('status', 'banned')
          ),
          Predicates.lessThan('age', 18)
        )
      );
      expect(evaluatePredicate(predicate, { status: 'active', age: 25 })).toBe(true);
      expect(evaluatePredicate(predicate, { status: 'deleted', age: 25 })).toBe(true);
      expect(evaluatePredicate(predicate, { status: 'deleted', age: 16 })).toBe(false);
      expect(evaluatePredicate(predicate, { status: 'banned', age: 17 })).toBe(false);
    });
  });

  describe('evaluatePredicate - between helper', () => {
    test('between should include both boundaries', () => {
      const predicate = Predicates.between('age', 18, 65);
      expect(evaluatePredicate(predicate, { age: 18 })).toBe(true);
      expect(evaluatePredicate(predicate, { age: 65 })).toBe(true);
      expect(evaluatePredicate(predicate, { age: 40 })).toBe(true);
      expect(evaluatePredicate(predicate, { age: 17 })).toBe(false);
      expect(evaluatePredicate(predicate, { age: 66 })).toBe(false);
    });

    test('between should work with strings', () => {
      const predicate = Predicates.between('letter', 'B', 'D');
      expect(evaluatePredicate(predicate, { letter: 'B' })).toBe(true);
      expect(evaluatePredicate(predicate, { letter: 'C' })).toBe(true);
      expect(evaluatePredicate(predicate, { letter: 'D' })).toBe(true);
      expect(evaluatePredicate(predicate, { letter: 'A' })).toBe(false);
      expect(evaluatePredicate(predicate, { letter: 'E' })).toBe(false);
    });
  });

  describe('evaluatePredicate - Edge cases', () => {
    test('should return false for null data', () => {
      const predicate = Predicates.equal('name', 'Alice');
      expect(evaluatePredicate(predicate, null)).toBe(false);
    });

    test('should return false for undefined data', () => {
      const predicate = Predicates.equal('name', 'Alice');
      expect(evaluatePredicate(predicate, undefined)).toBe(false);
    });

    test('should handle missing attribute in data', () => {
      const predicate = Predicates.equal('name', 'Alice');
      expect(evaluatePredicate(predicate, { age: 25 })).toBe(false);
    });

    test('should match undefined value in data with undefined target', () => {
      const predicate = Predicates.equal('optional', undefined);
      expect(evaluatePredicate(predicate, { name: 'Test' })).toBe(true);
    });

    test('should match null value in data', () => {
      const predicate = Predicates.equal('nullField', null);
      expect(evaluatePredicate(predicate, { nullField: null })).toBe(true);
      expect(evaluatePredicate(predicate, { nullField: undefined })).toBe(false);
    });

    test('should handle comparison with null values', () => {
      const predicate = Predicates.greaterThan('value', 10);
      expect(evaluatePredicate(predicate, { value: null })).toBe(false);
      expect(evaluatePredicate(predicate, { value: undefined })).toBe(false);
    });

    test('should return false when predicate has no attribute for leaf node', () => {
      const invalidPredicate: PredicateNode = { op: 'eq', value: 'test' };
      expect(evaluatePredicate(invalidPredicate, { name: 'test' })).toBe(false);
    });

    test('not with undefined children should return true (vacuous NOT)', () => {
      const predicate: PredicateNode = { op: 'not' };
      // NOT of nothing is vacuously true
      expect(evaluatePredicate(predicate, { any: 'data' })).toBe(true);
    });

    test('and/or with undefined children should work', () => {
      const andPredicate: PredicateNode = { op: 'and' };
      const orPredicate: PredicateNode = { op: 'or' };
      expect(evaluatePredicate(andPredicate, { any: 'data' })).toBe(true);
      expect(evaluatePredicate(orPredicate, { any: 'data' })).toBe(false);
    });

    test('should return false for unknown operator', () => {
      const unknownPredicate: PredicateNode = { op: 'unknown' as any, attribute: 'name', value: 'test' };
      expect(evaluatePredicate(unknownPredicate, { name: 'test' })).toBe(false);
    });
  });

  describe('evaluatePredicate - Complex real-world scenarios', () => {
    test('user filtering scenario', () => {
      const predicate = Predicates.and(
        Predicates.equal('status', 'active'),
        Predicates.not(Predicates.equal('role', 'guest')),
        Predicates.greaterThanOrEqual('age', 18),
        Predicates.or(
          Predicates.like('email', '%@company.com'),
          Predicates.equal('verified', true)
        )
      );

      const activeVerifiedUser = { status: 'active', role: 'user', age: 25, email: 'test@gmail.com', verified: true };
      const activeCompanyUser = { status: 'active', role: 'admin', age: 30, email: 'admin@company.com', verified: false };
      const guestUser = { status: 'active', role: 'guest', age: 25, email: 'guest@company.com', verified: true };
      const inactiveUser = { status: 'inactive', role: 'user', age: 25, email: 'user@company.com', verified: true };
      const minorUser = { status: 'active', role: 'user', age: 16, email: 'minor@company.com', verified: true };
      const unverifiedExternal = { status: 'active', role: 'user', age: 25, email: 'user@external.com', verified: false };

      expect(evaluatePredicate(predicate, activeVerifiedUser)).toBe(true);
      expect(evaluatePredicate(predicate, activeCompanyUser)).toBe(true);
      expect(evaluatePredicate(predicate, guestUser)).toBe(false);
      expect(evaluatePredicate(predicate, inactiveUser)).toBe(false);
      expect(evaluatePredicate(predicate, minorUser)).toBe(false);
      expect(evaluatePredicate(predicate, unverifiedExternal)).toBe(false);
    });

    test('product search scenario', () => {
      const predicate = Predicates.and(
        Predicates.between('price', 10, 100),
        Predicates.regex('name', '^(iPhone|Samsung|Pixel)'),
        Predicates.greaterThan('stock', 0)
      );

      expect(evaluatePredicate(predicate, { name: 'iPhone 14', price: 50, stock: 10 })).toBe(true);
      expect(evaluatePredicate(predicate, { name: 'Samsung Galaxy', price: 80, stock: 5 })).toBe(true);
      expect(evaluatePredicate(predicate, { name: 'Nokia Phone', price: 50, stock: 10 })).toBe(false);
      expect(evaluatePredicate(predicate, { name: 'iPhone 15', price: 150, stock: 10 })).toBe(false);
      expect(evaluatePredicate(predicate, { name: 'Pixel 8', price: 50, stock: 0 })).toBe(false);
    });

    test('date range filtering scenario', () => {
      const startOfYear = new Date('2024-01-01').getTime();
      const endOfYear = new Date('2024-12-31').getTime();

      const predicate = Predicates.between('createdAt', startOfYear, endOfYear);

      expect(evaluatePredicate(predicate, { createdAt: new Date('2024-06-15').getTime() })).toBe(true);
      expect(evaluatePredicate(predicate, { createdAt: new Date('2023-12-31').getTime() })).toBe(false);
      expect(evaluatePredicate(predicate, { createdAt: new Date('2025-01-01').getTime() })).toBe(false);
    });
  });
});
