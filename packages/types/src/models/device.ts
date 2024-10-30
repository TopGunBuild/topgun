import { field, option } from '@dao-xyz/borsh';
import { Keyset } from './keyset';
import { Identifiable } from '../common';
import { randomId } from '@topgunbuild/utils';

export interface IDeviceInfo extends Identifiable
{
    teamId: string;
    userId: string;
    keys: Keyset;
    deviceInfo?: string;
    created?: string;
}

// export interface IDeviceWithSecrets extends IDeviceInfo
// {
//     keys: KeysetWithSecrets;
// }

export class Device implements IDeviceInfo
{
    @field({ type: 'string' })
    $id: string;

    @field({ type: 'string' })
    teamId: string;

    @field({ type: 'string' })
    userId: string;

    @field({ type: Keyset })
    keys: Keyset;

    @field({ type: option('string') })
    created?: string;

    @field({ type: option('string') })
    deviceInfo?: string;

    constructor(data: {
        $id?: string,
        teamId: string,
        userId: string,
        keys: Keyset,
        created?: string,
        deviceInfo?: string
    })
    {
        this.$id        = data.$id || randomId(32);
        this.teamId     = data.teamId;
        this.userId     = data.userId;
        this.keys       = data.keys;
        this.created    = data.created;
        this.deviceInfo = data.deviceInfo;
    }
}
