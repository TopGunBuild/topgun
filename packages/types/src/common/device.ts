import { field, option } from '@dao-xyz/borsh';
import { Keyset } from './keyset';

// export interface IDeviceInfo
// {
//     userId: string;
//     deviceId: string;
//     deviceName: string;
//     deviceInfo?: any;
//     created?: bigint;
// }

// export interface IDeviceWithSecrets extends IDeviceInfo
// {
//     keys: KeysetWithSecrets;
// }

export class Device
{
    @field({ type: 'string' })
    teamId: string;

    @field({ type: 'string' })
    userId: string;

    @field({ type: 'string' })
    deviceId: string;

    @field({ type: Keyset })
    keys: Keyset;

    @field({ type: option('string') })
    created?: string;

    @field({ type: option('string') })
    deviceInfo?: string;

    constructor(data: {
        teamId: string,
        userId: string,
        deviceId: string,
        keys: Keyset,
        created?: string,
        deviceInfo?: string
    })
    {
        this.teamId     = data.teamId;
        this.userId     = data.userId;
        this.deviceId   = data.deviceId;
        this.keys       = data.keys;
        this.created    = data.created;
        this.deviceInfo = data.deviceInfo;
    }
}
