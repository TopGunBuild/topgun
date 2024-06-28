import { DataNode, DataValue, StoreValue } from '@topgunbuild/store';
import { isNumber } from '@topgunbuild/utils';

export const toDataNodes = (values: StoreValue[]): DataNode[] =>
{
    const map = new Map<string, DataNode>();

    for (const value of values)
    {
        const nodeId = `${value.section}/${value.node}`;

        if (!map.has(nodeId))
        {
            map.set(nodeId, {
                _id: nodeId,
            });
        }

        map.get(nodeId)[value.field] = coerceDataValue(value);
    }

    return Array.from(map.values());
};

export const coerceDataValue = (storeValue: StoreValue): DataValue =>
{
    if (storeValue.value_is_empty)
    {
        return null;
    }
    else if (isNumber(storeValue.value_bool))
    {
        return storeValue.value_bool === 1;
    }
    else if (isNumber(storeValue.value_number))
    {
        return storeValue.value_number;
    }
    else if (storeValue.value_date)
    {
        return storeValue.value_date;
    }
    else if (storeValue.value_byte)
    {
        return storeValue.value_byte;
    }

    return storeValue.value_string;
};
