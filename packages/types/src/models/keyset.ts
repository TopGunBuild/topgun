import { field } from '@dao-xyz/borsh';
import { randomId } from '@topgunbuild/utils';

// /**
//  * KeyScope represents the scope of a keyset. For example:
//  * - a user: `{type: USER, name: 'alice'}`
//  * - a device: `{type: DEVICE, name: 'laptop'}`
//  * - a role: `{type: ROLE, name: 'MANAGER'}`
//  * - a single-use keyset: `{type: EPHEMERAL, name: EPHEMERAL}`
//  */
// export interface IKeyScope
// {
//     /** The apps are not limited to KeyType, as they will have their own types. */
//     type: string;
//     name: string;
// }
//
// export interface IKeyMetadata extends IKeyScope
// {
//     generation: number;
// }
//
// /**
//  * A Keyset includes the public encryption and signature keys from
//  * a KeysetWithSecrets.
//  */
// export interface IKeyset extends KeyMetadata
// {
//     /** Encryption publicKey */
//     encryption: string;
//     /** Signature publicKey */
//     signature: string;
// }
//
// export interface KeysetWithSecrets extends KeyMetadata
// {
//
// }

export interface IKeyset
{
    id: string;
    teamId: string;
    type: string;
    name: string;
    encryption: string;
    signature: string;
    generation: number;
}

export class Keyset implements IKeyset
{
    @field({ type: 'string' })
    id: string;

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

    constructor(data: {
        id?: string,
        teamId: string,
        type: string,
        name: string,
        encryption: string,
        signature: string,
        generation: number
    })
    {
        this.id         = data.id || randomId(32);
        this.teamId     = data.teamId;
        this.type       = data.type;
        this.name       = data.name;
        this.encryption = data.encryption;
        this.signature  = data.signature;
        this.generation = data.generation || 1;
    }
}
