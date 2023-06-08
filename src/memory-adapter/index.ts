import { TGGraphAdapter } from '../types';
import { createGraphAdapter } from '../storage/adapter';
import { MemoryStorage } from './memory-storage';

export function createMemoryAdapter(): TGGraphAdapter
{
    return createGraphAdapter(new MemoryStorage());
}

export { MemoryStorage } from './memory-storage';