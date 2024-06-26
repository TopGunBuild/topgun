import { DataNode, StoreValue } from '@topgunbuild/store';
import { QueryHandler } from './query-handler';
import { SelectNodeMessage, SelectNodeOptions } from '@topgunbuild/transport';
import { ClientService } from '../client-service';

export class NodeQueryHandler extends QueryHandler<DataNode>
{
    selectMessage: SelectNodeMessage;

    constructor(props: {
        service: ClientService,
        options: SelectNodeOptions,
        message: SelectNodeMessage,
    })
    {
        super(props.service, props.options.local, props.options.remote, props.options.sync);
        this.selectMessage = props.message;
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
}
