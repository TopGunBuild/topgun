import { DataStream } from '@topgunbuild/data-streams';
import { DataNode, DataValue, StoreValue } from '@topgunbuild/store';
import { ClientService } from '../client-service';

export class QueryHandler<T extends DataNode[]|DataNode|DataValue>
{
    dataStream: DataStream<T>;
    local: boolean;
    remote: boolean;
    sync: boolean;
    once: boolean;

    get id(): string
    {
        return this.dataStream.name;
    }

    constructor(
        service: ClientService,
        local: boolean,
        remote: boolean,
        sync: boolean,
    )
    {
        this.dataStream = service.createDataStream<T>();
        this.local      = local;
        this.remote     = remote;
        this.sync       = sync;
        service.initQueryHandler<T>(this);
    }

    async maybePutValues(values: StoreValue[]): Promise<void>
    {
    }

    async destroy(): Promise<void>
    {
        this.dataStream.destroy();
    }
}
