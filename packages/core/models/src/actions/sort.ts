import { field, vec } from '@dao-xyz/borsh';
import { SortDirection, SortOptions } from '../types';

export class Sort implements SortOptions
{
    @field({ type: vec('string') })
    key: string;

    @field({ type: 'u8' })
    direction: SortDirection;

    constructor(properties: SortOptions)
    {
        this.key       = properties.key;
        this.direction = properties.direction || SortDirection.ASC;
    }
}
