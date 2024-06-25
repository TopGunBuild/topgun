import { DataStream } from '@topgunbuild/data-streams/src';
import { QueryCb } from '../types';

export class QueryService<T>
{
    dataStream: DataStream<any>;
    local: boolean;
    remote: boolean;
    sync: boolean;
    once: boolean;
    cb?: QueryCb<T>;

    constructor(
        dataStream: DataStream<any>,
        local: boolean,
        remote: boolean,
        sync: boolean,
        once: boolean,
        cb: QueryCb<T>
    )
    {
        this.dataStream = dataStream;
        this.local      = local;
        this.remote     = remote;
        this.sync       = sync;
        this.once       = once;
        this.cb         = cb;
    }

    async destroy(): Promise<void>
    {
        this.dataStream.destroy();
    }
}
