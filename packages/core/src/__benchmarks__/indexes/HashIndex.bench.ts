/**
 * HashIndex Performance Benchmarks
 *
 * Measures O(1) equality lookup performance at different scales.
 */

import { bench, describe } from 'vitest';
import { HashIndex } from '../../query/indexes/HashIndex';
import { simpleAttribute } from '../../query/Attribute';

interface User {
  id: string;
  email: string;
  status: string;
}

describe('HashIndex Performance', () => {
  const emailAttr = simpleAttribute<User, string>('email', (u) => u.email);

  const sizes = [1_000, 10_000, 100_000, 1_000_000];

  for (const size of sizes) {
    describe(`${size.toLocaleString()} records`, () => {
      const index = new HashIndex(emailAttr);
      const users: User[] = [];

      // Setup: Build index
      for (let i = 0; i < size; i++) {
        const user = {
          id: `${i}`,
          email: `user${i}@test.com`,
          status: 'active',
        };
        users.push(user);
        index.add(`${i}`, user);
      }

      bench('add (new record)', () => {
        const id = `new-${Math.random()}`;
        const user = { id, email: `${id}@test.com`, status: 'active' };
        index.add(id, user);
      });

      bench('retrieve equal (existing)', () => {
        const target = Math.floor(size / 2);
        index.retrieve({ type: 'equal', value: `user${target}@test.com` });
      });

      bench('retrieve equal (non-existing)', () => {
        index.retrieve({ type: 'equal', value: 'nonexistent@test.com' });
      });

      bench('retrieve in (10 values)', () => {
        const values = Array.from(
          { length: 10 },
          (_, i) => `user${i * 100}@test.com`
        );
        index.retrieve({ type: 'in', values });
      });

      bench('retrieve in (100 values)', () => {
        const values = Array.from(
          { length: 100 },
          (_, i) => `user${i * 10}@test.com`
        );
        index.retrieve({ type: 'in', values });
      });

      bench('retrieve has (all keys)', () => {
        index.retrieve({ type: 'has' });
      });

      bench('update (same value)', () => {
        const target = Math.floor(size / 2);
        const user = users[target];
        index.update(`${target}`, user, user);
      });

      bench('update (different value)', () => {
        const target = Math.floor(size / 2);
        const oldUser = users[target];
        const newUser = { ...oldUser, email: 'newemail@test.com' };
        index.update(`${target}`, oldUser, newUser);
      });

      bench('remove', () => {
        const target = Math.floor(Math.random() * size);
        const user = users[target];
        index.remove(`${target}`, user);
      });
    });
  }

  // Collision testing (many records with same attribute value)
  describe('Hash collisions (10,000 records with same email)', () => {
    const index = new HashIndex(emailAttr);
    const email = 'shared@test.com';

    // Setup: All users share same email
    for (let i = 0; i < 10_000; i++) {
      const user = { id: `${i}`, email, status: 'active' };
      index.add(`${i}`, user);
    }

    bench('retrieve equal (10K collisions)', () => {
      const result = index.retrieve({ type: 'equal', value: email });
      // Force iteration
      let count = 0;
      for (const _ of result) count++;
    });

    bench('add to collision bucket', () => {
      const id = `new-${Math.random()}`;
      const user = { id, email, status: 'active' };
      index.add(id, user);
    });
  });
});
