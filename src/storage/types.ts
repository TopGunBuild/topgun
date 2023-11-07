import { TGGraphData, TGNode, TGOptionsGet, } from '../types/common';

export interface TGStorage
{
    put(key: string, value: TGNode): Promise<void>;

    get(key: string): Promise<TGNode|null>;

    list(options: TGOptionsGet): Promise<TGGraphData>;
}
