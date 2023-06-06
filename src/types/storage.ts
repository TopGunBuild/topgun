import { TGNode } from './common';

export interface TGStorage
{
    put(key: string, value: TGNode): Promise<void>;
    get(key: string): Promise<TGNode>;
}