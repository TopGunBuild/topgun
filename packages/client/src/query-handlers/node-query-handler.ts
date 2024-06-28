import { DataNode, StoreResults, StoreValue } from '@topgunbuild/store';
import { SelectQuery, SelectNodeOptions } from '@topgunbuild/transport';
import { cloneValue, equalObjects } from '@topgunbuild/utils';
import { QueryHandler } from './query-handler';
import { ClientService } from '../client-service';
import { toDataNodes } from '../utils/to-data-nodes';

export class NodeQueryHandler extends QueryHandler<DataNode, SelectNodeOptions>
{
    constructor(props: {
        service: ClientService,
        options: SelectNodeOptions,
        query: SelectQuery,
    })
    {
        super(props);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Protected methods
    // -----------------------------------------------------------------------------------------------------

    protected isQualify(value: StoreValue): boolean
    {
        return value.section === this.query.section
            && value.node === this.query.node;
    }

    protected onOutput(results: StoreResults): void
    {
        const nodes           = toDataNodes(results.results);
        const value: DataNode = nodes.length > 0 ? nodes[0] : null;

        if (!equalObjects(value, this.lastValue))
        {
            this.lastValue = value;
            this.dataStream.publish(cloneValue(value));
        }
    }
}
