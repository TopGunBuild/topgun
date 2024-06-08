import { createDatabase, SQLLiteStore } from '../src';

it('should', async () =>
{
    const store = new SQLLiteStore({
        createDatabase: createDatabase
    });

    expect(store).not.toBeNull();
});
