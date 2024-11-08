import { field, option, serialize } from '@dao-xyz/borsh';
import { AbstractRequest } from './requests';
import { TransportMetadata, TransportPayload, UnixTimestamp } from '../types';

export class TransportMetadataImpl implements TransportMetadata {

    @field({ type: 'string' })
    userId: string;

    @field({ type: 'string' })
    teamId: string;

    @field({ type: option('u64') })
    state?: bigint;

    @field({ type: option('f64') })
    timestamp: UnixTimestamp;

    @field({ type: option('string') })
    messageType?: string;

    @field({ type: option('string') })
    version?: string;

    constructor(data: TransportMetadata) {
        this.userId = data.userId;
        this.teamId = data.teamId;
        this.state = data.state;
        this.timestamp = data.timestamp;
        this.messageType = data.messageType;
        this.version = data.version;
    }
}

/**
 * Payload
 */
export class TransportPayloadImpl implements TransportPayload<AbstractRequest> {

    @field({ type: 'string' })
    $id: string;

    @field({ type: TransportMetadataImpl })
    meta: TransportMetadata;

    @field({ type: AbstractRequest })
    body: AbstractRequest;

    constructor(data: {
        meta: TransportMetadata,
        body: AbstractRequest,
    }) {
        this.meta = data.meta;
        this.body = data.body;
    }

    encode(): Uint8Array {
        return serialize(this);
    }
}
