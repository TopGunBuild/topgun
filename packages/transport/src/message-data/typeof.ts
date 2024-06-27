import {
    AbstractValue,
    ValueBool, ValueDate,
    ValueEmpty,
    ValueNumber,
    ValueString,
    ValueUint8Array,
} from './query-data-value';

function unsupportedError(v: unknown)
{
    return new TypeError(`unsupported type ${typeof v}`);
}

export function typeOf(value: unknown): AbstractValue
{
    switch (typeof value)
    {
        case 'undefined':
            return new ValueEmpty();

        case 'boolean':
            return new ValueBool(value);

        case 'number':
            return new ValueNumber(value);

        case 'string':
            return new ValueString(value);

        case 'object':
        {
            if (value === null)
            {
                return new ValueEmpty();
            }
            else if (value instanceof Uint8Array)
            {
                return new ValueUint8Array(value as Uint8Array);
            }
            else if (value instanceof Date)
            {
                return new ValueDate(value.getTime() as number);
            }

            throw unsupportedError(value);
        }

        default:
            throw unsupportedError(value);
    }
}


