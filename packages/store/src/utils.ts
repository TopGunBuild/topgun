import { serialize } from '@dao-xyz/borsh';
import {
    DeleteQuery,
    PutQuery,
    ValueBool,
    ValueDate,
    ValueEmpty,
    ValueNumber,
    ValueString,
    ValueUint8Array,
} from '@topgunbuild/transport';
import { StoreValue } from './store-value';

export const toStoreValue = (query: PutQuery|DeleteQuery): StoreValue =>
{
    const data: StoreValue = {
        section       : query.section,
        node          : query.node,
        field         : query.field,
        state         : query.state,
        value_is_empty: 0,
        size          : serialize(query).length,
        deleted       : 0,
    };

    if (query instanceof DeleteQuery)
    {
        data.deleted = 1;
    }
    else if (query instanceof PutQuery)
    {
        if (query.value instanceof ValueEmpty)
        {
            data.value_is_empty = 1;
        }
        else if (query.value instanceof ValueBool)
        {
            data.value_bool = query.value.value;
        }
        else if (query.value instanceof ValueString)
        {
            data.value_string = query.value.value;
        }
        else if (query.value instanceof ValueNumber)
        {
            data.value_number = query.value.value;
        }
        else if (query.value instanceof ValueDate)
        {
            data.value_date = query.value.value;
        }
        else if (query.value instanceof ValueUint8Array)
        {
            data.value_byte = query.value.value;
        }
    }

    return data;
};
