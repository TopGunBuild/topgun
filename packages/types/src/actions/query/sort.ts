import { field, vec } from '@dao-xyz/borsh';

/**
 * Enum to define sorting directions
 */
export enum SortDirection
{
    ASC,
    DESC
}

export interface SortParams
{
    key: string;
    direction?: SortDirection;
}

export class Sort implements SortParams
{
    @field({ type: vec('string') })
    key: string;

    @field({ type: 'u8' })
    direction: SortDirection;

    constructor(properties: SortParams)
    {
        this.key       = properties.key;
        this.direction = properties.direction || SortDirection.ASC;
    }
}
