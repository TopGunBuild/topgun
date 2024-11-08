import { isNode, windowOrGlobal, randomBytes } from '..';

it('windowOrGlobal', () =>
{
    expect(typeof windowOrGlobal.setInterval === 'function').toEqual(true);
});

it('isNode', () =>
{
    expect(isNode()).toBeTruthy();
});

it('randomBytes', () =>
{
    expect(randomBytes(12) instanceof Uint8Array).toBeTruthy();
    expect(randomBytes(12).length).toBe(12);
});
