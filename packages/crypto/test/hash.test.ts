import { hashBytes, hashPassword } from '../src';
import { payload } from './data.utils';

describe('crypto', () =>
{
    test('hash has the right length', () =>
    {
        const h = hashBytes('TEST_HASH', payload);
        expect(h.length).toBe(32);
    });

    test('different inputs produce different results when using hash functions', () =>
    {
        const hash1 = hashBytes('SOME_HASH', payload);
        const hash2 = hashBytes('SOME_ELSE_HASH', payload);
        expect(hash2).not.toEqual(hash1);
    });

    test('returns a 32-byte key', () =>
    {
        const password = 'pass123';
        const key      = hashPassword(password);

        expect(key).toHaveLength(32);
    });
});

