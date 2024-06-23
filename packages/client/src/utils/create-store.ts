import * as sqlite from '@topgunbuild/sqlite';
import { StoreWrapper } from '@topgunbuild/store';

export const createStore = async (directory: string): Promise<StoreWrapper> =>
{
    const store = new sqlite.SQLLiteStore(sqlite, { directory });
    await store.start();
    return new StoreWrapper(store);
};
