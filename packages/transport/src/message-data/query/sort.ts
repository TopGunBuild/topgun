import { field, vec } from '@dao-xyz/borsh';
import { toArray } from '@topgunbuild/utils';

export enum SortDirection
{
    ASC  = 0,
    DESC = 1
}

export class Sort
{
    @field({ type: vec('string') })
    key: string[];

    @field({ type: 'u8' })
    direction: SortDirection;

    constructor(properties: {
        key: string[]|string;
        direction?: SortDirection;
    })
    {
        this.key       = toArray(properties.key);
        this.direction = properties.direction || SortDirection.ASC;
    }
}
