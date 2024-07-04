import { DataNode, StoreResults, StoreValue } from '@topgunbuild/store';
import { SelectQuery, SelectSectionOptions } from '@topgunbuild/transport';
import { cloneValue, equal } from '@topgunbuild/utils';
import { FilterExpressionTree, FilterService } from '@topgunbuild/filtering';
import { QueryHandler } from './query-handler';
import { ClientService } from '../client-service';
import { coerceDataValue, toDataNodes } from '../utils';
import { convertQueryToFilterExpressionTree } from '../utils/convert-query-to-filter';

export class SectionQueryHandler extends QueryHandler<DataNode[], SelectSectionOptions>
{
    filterExpressionTree: FilterExpressionTree;
    filterService: FilterService;

    constructor(props: {
        service: ClientService,
        query: SelectQuery,
        options: SelectSectionOptions,
        debounce?: number
    })
    {
        super(props);
        this.filterService        = new FilterService();
        this.filterExpressionTree = convertQueryToFilterExpressionTree(props.query);
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
            [value.field]: coerceDataValue(value),
        };
        return this.filterService.matchRecord(record, this.filterExpressionTree, value.field);
    }

    protected onOutput(results: StoreResults): void
    {
        const nodes = toDataNodes(results.results);

        // SELECT name, age
        // FROM clients
        // EXCEPT SELECT name, age
        // FROM employees;

        // TODO: need performance test with string merging or sqlite EXCEPT
        if (!equal(nodes, this.lastValue))
        {
            this.lastValue = nodes;
            this.dataStream.publish(cloneValue(nodes));
        }
    }
}
