import { Ed25519PublicKey, PublicKey } from '@topgunbuild/crypto';
import { randomBytes } from '@topgunbuild/utils';
import { CloseIteratorQuery, SelectNextQuery, SelectQuery } from '@topgunbuild/transport';
import { StoreResults } from './result';
import { IdKey } from './id';
import { Store } from './store';
import { StoreValue } from './store-value';

export class StoreWrapper
{
    constructor(
        readonly index: Store,
    )
    {
    }

    getSize(): number|Promise<number>
    {
        return this.index.getSize();
    }

    del(key: IdKey): Promise<void>|void
    {
        return this.index.del(key);
    }

    get(key: IdKey): Promise<StoreValue|undefined>
    {
        return this.index.get(key);
    }

    put(data: StoreValue): Promise<void>|void
    {
        return this.index.put(data);
    }

    stop(): Promise<void>|void
    {
        return this.index.stop();
    }

    select(
        query: SelectQuery,
        from: PublicKey = new Ed25519PublicKey(randomBytes(32)),
    ): Promise<StoreResults>
    {
        return this.index.select(query, from);
    }

    iterate(
        selectQuery: SelectQuery,
        from: PublicKey = new Ed25519PublicKey(randomBytes(32)),
    )
    {
        let done        = false;
        let fetchedOnce = false;
        return {
            next : async (pageSize: number) =>
            {
                let res: StoreResults;
                if (!fetchedOnce)
                {
                    fetchedOnce          = true;
                    selectQuery.pageSize = pageSize;
                    res                  = await this.index.select(selectQuery, from);
                }
                else
                {
                    res = await this.index.next(
                        new SelectNextQuery({ id: selectQuery.id, pageSize }),
                        from,
                    );
                }
                done = res.left === 0;
                return res;
            },
            done : () => done,
            close: () =>
            {
                return this.index.close(
                    new CloseIteratorQuery({ id: selectQuery.id }),
                    from,
                );
            },
        };
    }
}
