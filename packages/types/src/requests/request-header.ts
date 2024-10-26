import { field, option } from "@dao-xyz/borsh";
import { randomId } from "@topgunbuild/utils";

export interface IRequestHeader {
    requestId: string;
    userId: string;
    teamId: string;
    state?: bigint;
}

export class RequestHeader implements IRequestHeader
{
    @field({ type: 'string' })
    requestId: string;

    @field({ type: 'string' })
    userId: string;

    @field({ type: 'string' })
    teamId: string;

    @field({ type: option('u64') })
    state?: bigint;

    constructor(data: {
        id: string,
        userId: string,
        teamId: string,
        state: bigint,
        context: Uint8Array,
    })
    {
        this.requestId = data.id || randomId();
        this.userId    = data.userId;
        this.teamId    = data.teamId;
        this.state     = data.state;
    }
}