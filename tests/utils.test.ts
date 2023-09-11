import { createSoul } from '../src/utils';

describe('Utils', () =>
{
    it('create soul', async () =>
    {
        const soul1 = createSoul(['1 ', 2, 3]);
        expect(soul1).toBe('1/2/3');

        const soul2 = createSoul('1 ', 2, 3);
        expect(soul2).toBe('1/2/3');
    });
});
