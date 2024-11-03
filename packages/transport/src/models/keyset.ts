import { field } from '@dao-xyz/borsh';
import { randomId } from '@topgunbuild/utils';
import { Base58, Keyset } from '@topgunbuild/types';

export class KeysetImpl implements Keyset
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
    encryption: Base58;

    @field({ type: 'string' })
    signature: Base58;

    @field({ type: 'u32' })
    generation: number;

    @field({ type: 'string' })
    publicKey: Base58;

    constructor(data: {
        $id?: string,
        teamId: string,
        type: string,
        name: string,
        encryption: Base58,
        signature: Base58,
        generation: number,
        publicKey: Base58
    })
    {
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
