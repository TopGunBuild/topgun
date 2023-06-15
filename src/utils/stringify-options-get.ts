import { TGOptionsGet } from '../types';
import { replacerSortKeys } from './replacer-sort-keys';

export function stringifyOptionsGet(options: TGOptionsGet): string
{
    return JSON.stringify(options || {}, replacerSortKeys);
}