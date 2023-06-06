import { TGNode } from '../types/common';

export interface TGStorage
{
    put(key: string, value: TGNode): Promise<void>;

    get(key: string): Promise<TGNode|null>;
}

export interface StorageListOptions
{
    /** Returned keys must start with this string if defined */
    prefix?: string;
    /** Maximum number of keys to return if defined */
    limit?: number;
    /** Returned keys must be lexicographically >= this string if defined */
    start?: string;
    /** Returned keys must be lexicographically < this string if defined */
    end?: string;
}