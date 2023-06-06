import { TGNode, TGStorage } from '../types';

export class MemoryStorage implements TGStorage
{
    constructor(
        protected map = new Map<string, TGNode>(),
    )
    {
    }

    putSync(key: string, value: TGNode): void
    {
        this.map.set(key, value);
    }

    getSync(key: string): TGNode|null
    {
        return this.map.get(key) || null;
    }

    put(key: string, value: TGNode): Promise<void>
    {
        return Promise.resolve(this.putSync(key, value));
    }

    get(key: string): Promise<TGNode>
    {
        return Promise.resolve(this.getSync(key));
    }
}
