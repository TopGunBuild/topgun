import { serialize } from '@dao-xyz/borsh';
import { Ed25519PublicKey, PublicKey } from '@topgunbuild/crypto';
import { randomBytes } from '@topgunbuild/utils';
import { StoreResults } from './result';
import { IdKey } from './id';
import { Store } from './store';
import {
    CloseIteratorMessage,
    CollectNextMessage, PutMessage,
    SearchMessage,
    ValueBool,
    ValueDate,
    ValueEmpty,
    ValueNumber,
    ValueString,
    ValueUint8Array,
} from '@topgunbuild/transport';
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

    put(record: PutMessage): Promise<void>|void
    {
        const data: StoreValue = {
            node_name     : record.node_name,
            field_name    : record.field_name,
            state         : record.state,
            value_is_empty: 0,
            size          : serialize(record).length,
        };

        if (record.value instanceof ValueEmpty)
        {
            data.value_is_empty = 1;
        }
        else if (record.value instanceof ValueBool)
        {
            data.value_bool = record.value.value;
        }
        else if (record.value instanceof ValueString)
        {
            data.value_string = record.value.value;
        }
        else if (record.value instanceof ValueNumber)
        {
            data.value_string = record.value.value;
        }
        else if (record.value instanceof ValueDate)
        {
            data.value_string = record.value.value;
        }
        else if (record.value instanceof ValueUint8Array)
        {
            data.value_byte = record.value.value;
        }

        return this.index.put(data);
    }

    stop(): Promise<void>|void
    {
        return this.index.stop?.();
    }

    search(
        query: SearchMessage,
        from: PublicKey = new Ed25519PublicKey(randomBytes(32)),
    ): Promise<StoreResults>
    {
        return this.index.query(query, from);
    }

    iterate(
        query: SearchMessage,
        from: PublicKey = new Ed25519PublicKey(randomBytes(32)),
    )
    {
        let done        = false;
        let fetchedOnce = false;
        return {
            next : async (count: number) =>
            {
                let res: StoreResults;
                if (!fetchedOnce)
                {
                    fetchedOnce = true;
                    query.fetch = count;
                    res         = await this.index.query(query, from);
                }
                else
                {
                    res = await this.index.next(
                        new CollectNextMessage({ id: query.id, amount: count }),
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
                    new CloseIteratorMessage({ id: query.id }),
                    from,
                );
            },
        };
    }
}
