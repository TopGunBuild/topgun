import { serialize } from '@dao-xyz/borsh';
import { Ed25519PublicKey, PublicKey } from '@topgunbuild/crypto';
import { randomBytes } from '@topgunbuild/utils';
import { StoreResults } from './result';
import { IdKey } from './id';
import { Store } from './store';
import {
    CloseIteratorMessage, SelectNextMessage,
    PutMessage, SelectMessage,
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

    put(message: PutMessage): Promise<void>|void
    {
        const data: StoreValue = {
            section       : message.section,
            node          : message.node,
            field         : message.field,
            state         : message.state,
            value_is_empty: 0,
            size          : serialize(message).length,
        };

        if (message.value instanceof ValueEmpty)
        {
            data.value_is_empty = 1;
        }
        else if (message.value instanceof ValueBool)
        {
            data.value_bool = message.value.value;
        }
        else if (message.value instanceof ValueString)
        {
            data.value_string = message.value.value;
        }
        else if (message.value instanceof ValueNumber)
        {
            data.value_string = message.value.value;
        }
        else if (message.value instanceof ValueDate)
        {
            data.value_string = message.value.value;
        }
        else if (message.value instanceof ValueUint8Array)
        {
            data.value_byte = message.value.value;
        }

        return this.index.put(data);
    }

    stop(): Promise<void>|void
    {
        return this.index.stop();
    }

    select(
        query: SelectMessage,
        from: PublicKey = new Ed25519PublicKey(randomBytes(32)),
    ): Promise<StoreResults>
    {
        return this.index.select(query, from);
    }

    iterate(
        selectMessage: SelectMessage,
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
                    fetchedOnce            = true;
                    selectMessage.pageSize = pageSize;
                    res                    = await this.index.select(selectMessage, from);
                }
                else
                {
                    res = await this.index.next(
                        new SelectNextMessage({ id: selectMessage.id, pageSize }),
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
                    new CloseIteratorMessage({ id: selectMessage.id }),
                    from,
                );
            },
        };
    }
}
