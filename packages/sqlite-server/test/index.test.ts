import { createDatabase } from '../src';

it('check database creation', async () =>
{
    const db = await createDatabase();

    expect(typeof db.exec === 'function').toBeTruthy();
});
