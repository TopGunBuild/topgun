import { DataNode, StoreResults, StoreValue } from '@topgunbuild/store';
import { SelectQuery, SelectSectionOptions } from '@topgunbuild/transport';
import { cloneValue } from '@topgunbuild/utils';
import { FilterChain, FilterService } from '@topgunbuild/filtering';
import { QueryHandler } from './query-handler';
import { ClientService } from '../client-service';
import { coerceDataValue, toDataNodes } from '../utils';

export class SectionQueryHandler extends QueryHandler<DataNode[], SelectSectionOptions>
{
    filterChain: FilterChain;
    filterService: FilterService;

    constructor(props: {
        service: ClientService,
        query: SelectQuery,
        options: SelectSectionOptions,
        debounce?: number
    })
    {
        super(props);
        this.filterService = new FilterService();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Protected methods
    // -----------------------------------------------------------------------------------------------------

    protected isQualify(value: StoreValue): boolean
    {
        if (value.section !== this.query.section)
        {
            return false;
        }

        const record = {
            [value.field]: coerceDataValue(value)
        };
        return this.filterService.matchRecord(record, this.filterChain);
    }

    protected onOutput(results: StoreResults): void
    {
        const nodes    = toDataNodes(results.results);
        this.lastValue = nodes;
        this.dataStream.publish(cloneValue(nodes));
    }
}
