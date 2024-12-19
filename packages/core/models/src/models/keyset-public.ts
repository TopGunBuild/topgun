import { field } from '@dao-xyz/borsh';
import { KeysetPublicInfo } from '../types';
import { EncodeHelper } from '../utils/encode-helper';

export class KeysetPublic extends EncodeHelper implements KeysetPublicInfo
{
    @field({ type: 'string' })
    type: string;

    @field({ type: 'string' })
    name: string;

    @field({ type: 'string' })
    encryption: string;

    @field({ type: 'string' })
    signature: string;

    @field({ type: 'u32' })
    generation: number;

    constructor(data: {
        type: string,
        name: string,
        encryption: string,
        signature: string,
        generation: number,
    })
    {
        super();
        this.type       = data.type;
        this.name       = data.name;
        this.encryption = data.encryption;
        this.signature  = data.signature;
        this.generation = data.generation || 1;
    }
}
