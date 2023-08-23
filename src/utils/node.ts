import { isObject, isString } from '@topgunbuild/typed';
import { TGNode } from '../types';

export function isNode(data: unknown): boolean
{
    return isObject(data) &&
        isString(data && data._ && data._['#']) &&
        isObject(data && data._ && data._['>']);
}

export function getNodeSoul(data: unknown): string
{
    return isNode(data) ? (data as TGNode)._['#'] : null;
}