import { field, option } from "@dao-xyz/borsh";

export interface IRequestHeader {
    userId: string;
    teamId: string;
    state?: bigint;
}

export class RequestHeader implements IRequestHeader
{
    @field({ type: 'string' })
    userId: string;

    @field({ type: 'string' })
    teamId: string;

    @field({ type: option('u64') })
    state?: bigint;

    constructor(data: {
        userId: string,
        teamId: string,
        state: bigint,
    })
    {
        this.userId    = data.userId;
        this.teamId    = data.teamId;
        this.state     = data.state;
    }
}