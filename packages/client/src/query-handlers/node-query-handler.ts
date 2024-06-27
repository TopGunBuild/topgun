import { DataNode, StoreValue } from '@topgunbuild/store';
import { QueryHandler } from './query-handler';
import { SelectQuery, SelectNodeOptions } from '@topgunbuild/transport';
import { ClientService } from '../client-service';

export class NodeQueryHandler extends QueryHandler<DataNode>
{
    query: SelectQuery;

    constructor(props: {
        service: ClientService,
        options: SelectNodeOptions,
        query: SelectQuery,
    })
    {
        super(props.service, props.options.local, props.options.remote, props.options.sync);
        this.query = props.query;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    async maybePutValues(values: StoreValue[]): Promise<void>
    {

    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    async #fetchFirst(): Promise<void>
    {
        // Get local data
        if (this.local)
        {

        }
    }
}
