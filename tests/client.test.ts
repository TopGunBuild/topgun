import { test, expectTypeOf } from 'vitest';
import { TGClient } from '../src/client';

test('Client', () =>
{
    expectTypeOf(TGClient).toBeObject();
});