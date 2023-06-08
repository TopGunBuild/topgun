import { isNumber } from 'topgun-typed';
import { MAX_KEY_SIZE, MAX_VALUE_SIZE } from './constants';
import { TGGraphAdapterOptions, TGNode } from '../types';
import { TextEncoder } from '../sea/shims';
import { StorageListOptions } from './types';

const encoder = new TextEncoder();

export function assertPutEntry(soul: string, node: TGNode, options: TGGraphAdapterOptions): void
{
    const maxKeySize = isNumber(options?.maxKeySize) ? options.maxKeySize : MAX_KEY_SIZE;
    const maxValueSize = isNumber(options?.maxValueSize) ? options.maxValueSize : MAX_VALUE_SIZE;
    assertKeySize(soul, maxKeySize);
    assertValueSize(node, maxValueSize, soul)
}

export function assertKeySize(key: string, maxKeySize: number): void
{
    if (new Blob([key]).size <= maxKeySize)
    {
        return;
    }
    throw new RangeError( `Key "${key}" is larger than the limit of ${maxKeySize} bytes.`);
}

export function assertValueSize(value: TGNode, maxValueSize: number, key?: string): void
{
    if (roughSizeOfObject(value) <= maxValueSize)
    {
        return;
    }
    if (key !== undefined)
    {
        throw new RangeError(
            `Value for key "${key}" is above the limit of ${maxValueSize} bytes.`
        );
    }
    throw new RangeError(`Values cannot be larger than ${maxValueSize} bytes.`);
}

function roughSizeOfObject(object: object): number
{
    const objectList = [];
    const stack      = [object];
    let bytes        = 0;

    while (stack.length)
    {
        const value: any = stack.pop();

        if (typeof value === 'boolean')
        {
            bytes += 4;
        }
        else if (typeof value === 'string')
        {
            bytes += (value as string).length * 2;
        }
        else if (typeof value === 'number')
        {
            bytes += 8;
        }
        else if
        (
            typeof value === 'object'
            && objectList.indexOf(value) === -1
        )
        {
            objectList.push(value);

            for (const i in value)
            {
                stack.push(value[i]);
            }
        }
    }
    return bytes;
}

export function arrayCompare<T extends any[]|NodeJS.TypedArray>(
    a: T,
    b: T
): number
{
    const minLength = Math.min(a.length, b.length);
    for (let i = 0; i < minLength; i++)
    {
        const aElement = a[i];
        const bElement = b[i];
        if (aElement < bElement) return -1;
        if (aElement > bElement) return 1;
    }
    return a.length - b.length;
}

// Compares x and y lexicographically using a UTF-8 collation
export function lexicographicCompare(x: string, y: string): number
{
    const xEncoded = encoder.encode(x);
    const yEncoded = encoder.encode(y);
    return arrayCompare(xEncoded, yEncoded);
}

export function listFilterMatch(
    options: StorageListOptions|undefined,
    name: string
): boolean
{
    return !(
        (options?.prefix !== undefined && !name.startsWith(options.prefix)) ||
        (options?.start !== undefined && lexicographicCompare(name, options.start) < 0) ||
        (options?.end !== undefined && lexicographicCompare(name, options.end) >= 0)
    );
}