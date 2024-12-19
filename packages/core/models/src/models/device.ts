import { field, option } from '@dao-xyz/borsh';
import { KeysetPublic } from './keyset-public';
import { DeviceInfo, KeysetPublicInfo } from '../types';
import { randomId } from '@topgunbuild/common';
import { EncodeHelper } from '../utils/encode-helper';

export class Device extends EncodeHelper implements DeviceInfo
{
    @field({ type: 'string' })
    $id: string;

    @field({ type: 'string' })
    teamId: string;

    @field({ type: 'string' })
    userId: string;

    @field({ type: KeysetPublic })
    keys: KeysetPublic;

    @field({ type: 'string' })
    deviceName: string;

    @field({ type: 'f64' })
    created: number;

    @field({ type: option('string') })
    deviceInfo?: string;

    constructor(data: {
        $id?: string,
        userId: string,
        keys: KeysetPublicInfo,
        created: number,
        deviceName: string,
        teamId: string,
        deviceInfo?: string
    })
    {
        super();
        this.$id        = data.$id || randomId(32);
        this.userId     = data.userId;
        this.keys       = new KeysetPublic(data.keys);
        this.created    = data.created;
        this.deviceInfo = data.deviceInfo;
        this.deviceName = data.deviceName || 'Unknown Device';
        this.teamId     = data.teamId;
    }
}
