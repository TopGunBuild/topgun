import { field } from '@dao-xyz/borsh';
import { IRequestHeader, RequestHeader } from './request-header';
import { AbstractRequest } from './requests';

export interface IPayload {
    header: IRequestHeader;
    body: AbstractRequest;
}

/**
 * Payload
 */
export class Payload implements IPayload {
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