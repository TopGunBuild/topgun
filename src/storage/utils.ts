import { isNumber, isString, isEmptyObject, isBoolean } from '@topgunbuild/typed';
import textEncoder from '@topgunbuild/textencoder';
import { MAX_KEY_SIZE, MAX_VALUE_SIZE } from './constants';
import { TGGraphAdapterOptions, TGGraphData, TGNode, TGOptionsGet, TGQueryListOptions } from '../types';
import { getNodeSoul } from '../utils/node';

export function arrayNodesToObject(nodes: TGNode[]): TGGraphData
{
    return nodes.reduce((accum: TGGraphData, node: TGNode) => ({ ...accum, [node._['#']]: node }), {});
}

export function filterNodesByQueryOptions(nodes: TGNode[], options: TGQueryListOptions): TGNode[]
{
    const direction   = options?.reverse ? -1 : 1;
    let filteredNodes = nodes
        .filter(node => listFilterMatch(options, getNodeSoul(node)))
        .sort((a, b) => direction * lexicographicCompare(getNodeSoul(a), getNodeSoul(b)));

    if (isNumber(options?.limit) && filteredNodes.length > options?.limit)
    {
        filteredNodes = filteredNodes.slice(0, options.limit);
    }

    return filteredNodes;
}

export function getStorageListOptions(optionsGet: TGOptionsGet): TGQueryListOptions|null
{
    if (isEmptyObject(optionsGet))
    {
        return null;
    }

    const listOptions: TGQueryListOptions = {};
    const getPath                         = (path: string) => isString(optionsGet.equals)
        ? [optionsGet.equals, path].join('/')
        : path;

    if (isString(optionsGet.start))
    {
        listOptions.start = getPath(optionsGet.start);
    }
    if (isString(optionsGet.end))
    {
        listOptions.end = getPath(optionsGet.end);
    }
    if (isBoolean(optionsGet.reverse))
    {
        listOptions.reverse = optionsGet.reverse;
    }
    if (isNumber(optionsGet.limit))
    {
        listOptions.limit = optionsGet.limit;
    }

    if (isString(optionsGet.prefix))
    {
        listOptions.prefix = getPath(optionsGet.prefix);
    }
    else if (isString(optionsGet.equals) && !isEmptyObject(listOptions))
    {
        listOptions.prefix = `${optionsGet.equals}/`;
    }

    return isEmptyObject(listOptions) ? null : listOptions;
}

export function assertPutEntry(soul: string, node: TGNode, options: TGGraphAdapterOptions): void
{
    const maxKeySize   = isNumber(options?.maxKeySize) ? options.maxKeySize : MAX_KEY_SIZE;
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
    throw new RangeError(`Key "${key}" is larger than the limit of ${maxKeySize} bytes.`);
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
    const xEncoded = textEncoder.encode(x);
    const yEncoded = textEncoder.encode(y);
    return arrayCompare(xEncoded, yEncoded);
}

export function filterMatch(template: string, options: TGOptionsGet)
{
    if (template === options?.equals)
    {
        return true;
    }

    return listFilterMatch(options, template);
}

export function listFilterMatch(
    options: TGQueryListOptions|undefined,
    name: string
): boolean
{
    if (!isString(name))
    {
        return false;
    }

    return !(
        (options?.prefix !== undefined && !name.startsWith(options.prefix)) ||
        (options?.start !== undefined && lexicographicCompare(name, options.start) < 0) ||
        (options?.end !== undefined && lexicographicCompare(name, options.end) >= 0)
    );
}