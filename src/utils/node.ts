import { isObject, isString } from '@topgunbuild/typed';
import { TGNode, TGRefNode } from '../types';

export function isNode(value: unknown): value is TGNode
{
    return isObject(value) &&
        isString(value && value._ && value._['#']) &&
        isObject(value && value._ && value._['>']);
}

export function getNodeSoul(value: unknown): string
{
    return isNode(value) ? (value as TGNode)._['#'] : null;
}

export function isRefNode(value: unknown): value is TGRefNode
{
    return isObject(value) && isString(value['#']);
}