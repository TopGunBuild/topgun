import { createDatabase, SQLLiteStore } from '..';

it('should', async () =>
{
    const store = new SQLLiteStore({
        createDatabase: createDatabase
    });

    expect(store).not.toBeNull();
});
