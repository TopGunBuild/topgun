import { isNumber } from '@topgunbuild/typed';
import { TGGraphData, TGNode, TGOptionsGet } from '../types';
import { TGStorage } from '../storage';
import { lexicographicCompare, filterMatch } from '../storage/utils';

export class MemoryStorage implements TGStorage
{
    constructor(
        protected map = new Map<string, TGNode>(),
    )
    {
    }

    list(options: TGOptionsGet): Promise<TGGraphData>
    {
        const direction = options && options['-'] ? -1 : 1;
        let keys        = Array.from(this.map.keys())
            .filter(key => filterMatch(key, options))
            .sort((a, b) => direction * lexicographicCompare(a, b));

        if (isNumber(options && options['%']) && keys.length > options['%'])
        {
            keys = keys.slice(0, options['%']);
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
