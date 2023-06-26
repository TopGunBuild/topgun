import { TGGraphAdapter, TGGraphAdapterOptions } from '../types';
import { createGraphAdapter } from '../storage/adapter';
import { MemoryStorage } from './memory-storage';

export function createMemoryAdapter(adapterOptions?: TGGraphAdapterOptions): TGGraphAdapter
{
    return createGraphAdapter(new MemoryStorage(), adapterOptions);
}

export { MemoryStorage } from './memory-storage';