import { field } from '@dao-xyz/borsh';
import { randomId } from '@topgunbuild/common';
import { Keyset } from '../types';
import { EncodeHelper } from '../utils/encode-helper';

export class KeysetImpl extends EncodeHelper implements Keyset
{
    @field({ type: 'string' })
    $id: string;

    @field({ type: 'string' })
    teamId: string;

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

    @field({ type: 'string' })
    publicKey: string;

    constructor(data: {
        $id?: string,
        teamId: string,
        type: string,
        name: string,
        encryption: string,
        signature: string,
        generation: number,
        publicKey: string
    })
    {
        super();
        this.$id        = data.$id || randomId(32);
        this.teamId     = data.teamId;
        this.type       = data.type;
        this.name       = data.name;
        this.encryption = data.encryption;
        this.signature  = data.signature;
        this.generation = data.generation || 1;
        this.publicKey  = data.publicKey;
    }
}
