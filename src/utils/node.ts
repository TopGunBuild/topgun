import { isObject, isString } from '@topgunbuild/typed';
import { TGNode } from '../types';

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