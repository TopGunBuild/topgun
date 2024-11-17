import { field, option } from '@dao-xyz/borsh';
import { AbstractAction } from './actions';
import { EncryptedPayload, TransportPayload } from '../types';
import { EncodeHelper } from '../utils/encode-helper';

/**
 * Payload
 */
export class TransportPayloadImpl extends EncodeHelper implements TransportPayload<AbstractAction> {

    @field({ type: option('f64') })
    timestamp: number;

    @field({ type: AbstractAction })
    body: AbstractAction;

    constructor(data: TransportPayload<AbstractAction>) {
        super();
        this.timestamp = data.timestamp;
        this.body = data.body;
    }
}

/**
 * Encrypted payload
 */
export class EncryptedPayloadImpl extends EncodeHelper implements EncryptedPayload {

    @field({ type: Uint8Array })
    encryptedBody: Uint8Array;

    @field({ type: 'string' })
    senderPublicKey: string;

    @field({ type: 'string' })
    recipientPublicKey: string;

    constructor(data: EncryptedPayload) {
        super();
        this.encryptedBody = data.encryptedBody;
        this.senderPublicKey = data.senderPublicKey;
        this.recipientPublicKey = data.recipientPublicKey;
    }
}
