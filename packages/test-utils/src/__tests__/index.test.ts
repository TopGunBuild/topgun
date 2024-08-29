import { randomPort } from '..';

it('Port should be number', async () =>
{
    const port = await randomPort();

    expect(typeof port === 'number').toBeTruthy();
});
