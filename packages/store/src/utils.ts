import {
    RecordValue,
    ValueBool,
    ValueEmpty,
    ValueF64,
    ValueString,
    ValueU16,
    ValueU32,
    ValueU8,
    ValueUint8Array,
} from './record/record-values';

function float(value: number): RecordValue
{
    return new ValueF64(value);
}

function uint(value: number): RecordValue
{
    if (value > 0xffff)
    {
        return new ValueU32(value);
    }
    else if (value > 0xff)
    {
        return new ValueU16(value);
    }
    else
    {
        return new ValueU8(value);
    }
}

function unsupportedError(v: unknown)
{
    return new TypeError(`unsupported type ${typeof v}`);
}

export function typeOf(value: unknown): RecordValue
{
    switch (typeof value)
    {
        case 'undefined':
            return new ValueEmpty();

        case 'boolean':
            return new ValueBool(value as boolean);

        case 'number':
            // return !isFinite(v) || Math.floor(v) !== v ? float(v) : v < 0 ? int(v) : uint(v);
            return value >>> 0 === value ? uint(value) : float(value);

        case 'string':
            return new ValueString(value as string);

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
                return new ValueF64(value.getTime() as number);
            }

            throw unsupportedError(value);
        }

        default:
            throw unsupportedError(value);
    }
}

export const toArray = <T>(arr: T|T[]|undefined): T[] =>
{
    if (Array.isArray(arr))
    {
        return arr;
    }
    else if (arr)
    {
        return [arr];
    }
    else
    {
        return [];
    }
};
