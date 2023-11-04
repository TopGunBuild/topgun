import { isNumber } from '@topgunbuild/typed';
import { TGGraphData, TGNode } from '../types';
import { TGQueryListOptions, TGStorage } from '../storage';
import { lexicographicCompare, listFilterMatch } from '../storage/utils';

export class MemoryStorage implements TGStorage
{
    constructor(
        protected map = new Map<string, TGNode>(),
    )
    {
    }

    list(options: TGQueryListOptions): Promise<TGGraphData>
    {
        const direction = options?.reverse ? -1 : 1;
        let keys        = Array.from(this.map.keys())
            .filter(key => listFilterMatch(options, key))
            .sort((a, b) => direction * lexicographicCompare(a, b));

        if (isNumber(options?.limit) && keys.length > options?.limit)
        {
            keys = keys.slice(0, options.limit);
        }

        const result = keys.reduce((accum: TGGraphData, key: string) => ({ ...accum, [key]: this.map.get(key) }), {});

        return Promise.resolve(result);
    }

    put(key: string, value: TGNode): Promise<void>
    {
        return Promise.resolve(this.putSync(key, value));
    }

    get(key: string): Promise<TGNode>
    {
        return Promise.resolve(this.getSync(key));
    }

    putSync(key: string, value: TGNode): void
    {
        this.map.set(key, value);
    }

    getSync(key: string): TGNode|null
    {
        return this.map.get(key) || null;
    }
}
