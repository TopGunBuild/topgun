import { check, parse, shuffleAttackCutoff } from './settings'
import { pubFromSoul } from './soul'
import { GraphData, Node } from '../types';
import { isObject } from '../utils/is-object';
import { isDefined } from '../utils/is-defined';

export function unpack(passedValue: any, key: string, node: Node): any
{
    let value = passedValue;

    if (!value)
    {
        return
    }

    if (isObject(value) && ':' in value)
    {
        const val = value[':'];
        if (isDefined(val))
        {
            return val;
        }
    }

    if (isObject(value) && 'm' in value)
    {
        const val = value.m;
        if (isDefined(val))
        {
            value = parse(val);
        }
    }

    if (!key || !node)
    {
        return;
    }
    if (value === node[key])
    {
        return value;
    }
    if (!check(node[key]))
    {
        return value;
    }
    const soul = node && node._ && node._['#'];
    const state = node && node._ && node._['>'] && node._['>'][key];
    if (
        value &&
        4 === value.length &&
        soul === value[0] &&
        key === value[1] &&
        Math.floor(state) === Math.floor(value[3])
    )
    {
        console.log('value', value);
        return value[2]
    }
    if (state < shuffleAttackCutoff)
    {
        return value;
    }
}

export function unpackNode(
    node: Node,
    mut: 'immutable'|'mutable' = 'immutable'
): Node
{
    if (!node)
    {
        return node
    }

    const result: Node =
              mut === 'mutable'
                  ? node
                  : {
                      _: node._
                  };

    for (const key in node)
    {
        if (key === '_')
        {
            continue
        }

        result[key] = unpack(parse(node[key]), key, node);
    }

    return result;
}

export function unpackGraph(
    graph: GraphData,
    mut: 'immutable'|'mutable' = 'immutable'
): GraphData
{
    const unpackedGraph: GraphData = mut === 'mutable' ? graph : {};

    for (const soul in graph)
    {
        if (!soul)
        {
            continue
        }

        const node = graph[soul];
        const pub  = pubFromSoul(soul);

        unpackedGraph[soul] = node && pub ? unpackNode(node, mut) : node;
    }

    return unpackedGraph;
}