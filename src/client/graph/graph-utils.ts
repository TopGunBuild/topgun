import { isDefined, isObject, isNumber } from 'topgun-typed';
import { TGGraphData, TGNode, TGOptionsGet, TGPathData, TGValue } from '../../types';
import { isSupportValue } from '../../utils/is-support';
import { filterNodesByListOptions, storageListOptionsFromGetOptions } from '../../storage/utils';

export function diffSets(
    initial: readonly string[],
    updated: readonly string[],
): readonly [readonly string[], readonly string[]]
{
    return [
        updated.filter(key => initial.indexOf(key) === -1),
        initial.filter(key => updated.indexOf(key) === -1),
    ];
}

export function getNodesFromGraph(
    options: TGOptionsGet,
    graph: TGGraphData
): TGNode[]
{
    const allNodes    = Object.values(graph);
    const listOptions = storageListOptionsFromGetOptions(options);
    let filteredNodes = filterNodesByListOptions(allNodes, listOptions);

    if (isNumber(listOptions?.limit) && filteredNodes.length > listOptions?.limit)
    {
        filteredNodes = filteredNodes.slice(0, listOptions.limit);
    }

    return filteredNodes;
}

export function getPathData(
    keys: string[],
    graph: TGGraphData,
): TGPathData
{
    const lastKey = keys[keys.length - 1];

    if (keys.length === 1)
    {
        return {
            complete: lastKey in graph,
            souls   : keys,
            value   : graph[lastKey],
        };
    }

    const {
        value: parentValue,
        souls,
        complete,
    } = getPathData(keys.slice(0, keys.length - 1), graph);

    if (!isObject(parentValue))
    {
        return {
            complete: complete || isDefined(parentValue),
            souls,
            value   : undefined,
        };
    }

    const value = (parentValue as TGNode)[lastKey];

    if (!value)
    {
        return {
            complete: true,
            souls,
            value,
        };
    }

    const edgeSoul = value['#'];

    if (edgeSoul)
    {
        return {
            complete: edgeSoul in graph,
            souls   : [...souls, edgeSoul],
            value   : graph[edgeSoul],
        };
    }

    return {
        complete: true,
        souls,
        value,
    };
}

export function flattenGraphData(data: TGValue, fullPath: string[]): {
    graphData: TGGraphData,
    soul: string
}
{
    if (isObject(data))
    {
        const soul = fullPath.join('/');
        return {
            graphData: flattenGraphByPath(data, [soul]),
            soul
        };
    }
    else
    {
        const propertyName = fullPath.pop();
        const soul         = fullPath.join('/');
        return {
            graphData: flattenGraphByPath({ [propertyName]: data }, [soul]),
            soul
        };
    }
}

export function checkType(d: any, tmp?: any): string
{
    return (d && (tmp = d.constructor) && tmp.name) || typeof d;
}

export function set(list: Array<string>, value: any): {[key: string]: any}
{
    return list.reverse().reduce((a, c) => ({ [c]: a }), value);
}

export function flattenGraphByPath(
    obj: object,
    pathArr: string[] = [],
    target            = {},
): TGGraphData
{
    if (!isSupportValue(obj))
    {
        throw Error(
            'Invalid data: ' + checkType(obj) + ' at ' + pathArr.join('.'),
        );
    }
    else if (!isObject(obj))
    {
        obj = set(pathArr, obj);
    }

    const path = pathArr.join('/');
    if (pathArr.length > 0 && !isObject(target[path]))
    {
        target[path] = {};
    }

    for (const k in obj)
    {
        if (!obj.hasOwnProperty(k) || k === '_')
        {
            continue;
        }

        const value       = obj[k];
        const pathArrFull = [...pathArr, k];
        const pathFull    = pathArrFull.join('/');

        if (!isSupportValue(value))
        {
            console.log(
                'Invalid data: ' +
                checkType(value) +
                ' at ' +
                pathArrFull.join('.'),
            );
            continue;
        }

        if (isObject(value))
        {
            target[path][k] = { '#': pathFull };
            flattenGraphByPath(value, pathArrFull, target);
        }
        else
        {
            target[path][k] = value;
        }
    }
    return target;
}
