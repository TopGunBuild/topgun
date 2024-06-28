import { DataNode, StoreResults, StoreValue } from '@topgunbuild/store';
import { SelectQuery, SelectSectionOptions } from '@topgunbuild/transport';
import { QueryHandler } from './query-handler';
import { ClientService } from '../client-service';
import { toDataNodes } from '../utils/to-data-nodes';
import { cloneValue } from '@topgunbuild/utils';

export class SectionQueryHandler extends QueryHandler<DataNode[], SelectSectionOptions>
{
    constructor(props: {
        service: ClientService,
        query: SelectQuery,
        options: SelectSectionOptions,
        debounce?: number
    })
    {
        super(props);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Protected methods
    // -----------------------------------------------------------------------------------------------------

    protected isQualify(value: StoreValue): boolean
    {
        return false;
    }

    protected onOutput(results: StoreResults): void
    {
        const nodes    = toDataNodes(results.results);
        this.lastValue = nodes;
        this.dataStream.publish(cloneValue(nodes));
    }
}
