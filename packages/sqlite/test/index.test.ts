import { SQLLiteStore } from '../src';
import { createDatabase } from '../src/node';

it('should', async () =>
{
    const store = new SQLLiteStore({
        createDatabase: createDatabase
    });

    expect(store).not.toBeNull();
});
