import { field, option } from '@dao-xyz/borsh';
import { KeysetImpl } from './keyset';
import { Device } from '../types';
import { randomId } from '@topgunbuild/common';
import { EncodeHelper } from '../utils/encode-helper';

export class DeviceImpl extends EncodeHelper implements Device
{
    @field({ type: 'string' })
    $id: string;

    @field({ type: 'string' })
    teamId: string;

    @field({ type: 'string' })
    userId: string;

    @field({ type: KeysetImpl })
    keys: KeysetImpl;

    @field({ type: option('f64') })
    created?: number;

    @field({ type: option('string') })
    deviceInfo?: string;

    @field({ type: 'string' })
    deviceName: string;

    constructor(data: {
        $id?: string,
        userId: string,
        keys: KeysetImpl,
        created?: number,
        deviceInfo?: string,
        deviceId?: string,
        deviceName?: string
    })
    {
        super();
        this.$id        = data.$id || randomId(32);
        this.userId     = data.userId;
        this.keys       = data.keys;
        this.created    = data.created;
        this.deviceInfo = data.deviceInfo;
        this.deviceName = data.deviceName || 'Unknown Device';
    }
}
