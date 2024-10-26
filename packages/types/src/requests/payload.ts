import { field } from '@dao-xyz/borsh';
import { RequestHeader } from './request-header';
import { AbstractRequest } from './requests';

/**
 * Payload
 */
export class Payload {
    @field({ type: RequestHeader })
    header: RequestHeader;

    @field({ type: AbstractRequest })
    body: AbstractRequest;

    constructor(data: {
        header: RequestHeader,
        body: AbstractRequest,
    }) {
        this.header = data.header;
        this.body = data.body;
    }
}