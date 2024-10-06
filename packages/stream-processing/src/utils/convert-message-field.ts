import { isNumber } from '@topgunbuild/utils';
import { MessageFieldRow, MessageFieldValue } from '@topgunbuild/types';

export const convertMessageField = (storeValue: MessageFieldRow): MessageFieldValue =>
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
