import { TGGraphData, TGNode } from '../types/common';

export interface TGStorage
{
    put(key: string, value: TGNode): Promise<void>;

    get(key: string): Promise<TGNode|null>;

    list(options: StorageListOptions): Promise<TGGraphData>
}

export interface StorageListOptions
{
    // Stage 1: filtering
    /** Returned keys must start with this string if defined */
    prefix?: string;
    /** Returned keys must be lexicographically >= this string if defined */
    start?: string;
    /** Returned keys must be lexicographically < this string if defined */
    end?: string;

    // Stage 2: sorting
    /** Return keys in reverse order, MUST be applied before the limit/cursor */
    reverse?: boolean;

    // Stage 3: paginating
    /** Maximum number of keys to return if defined */
    limit?: number;
}