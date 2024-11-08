import { bigintTime } from '../time';

describe('Time test', () =>
{
    test('bigintTime', () =>
    {
        const time1 = bigintTime();
        const time2 = bigintTime();
        expect(time2 > time1).toBeTruthy();
    })
});
