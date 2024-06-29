import { DataValue, StoreResults, StoreValue } from '@topgunbuild/store';
import { SelectQuery, SelectOptions } from '@topgunbuild/transport';
import { equal } from '@topgunbuild/utils';
import { QueryHandler } from './query-handler';
import { ClientService } from '../client-service';
import { coerceDataValue } from '../utils/to-data-nodes';

export class FieldQueryHandler extends QueryHandler<DataValue, SelectOptions>
{
    constructor(props: {
        service: ClientService,
        options: SelectOptions,
        query: SelectQuery
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
            && value.node === this.query.node
            && value.field === this.query.field;
    }

    protected onOutput(results: StoreResults): void
    {
        let value: DataValue = results.results.length > 0
            ? coerceDataValue(results.results[0])
            : null;

        if (!equal(value, this.lastValue))
        {
            this.lastValue = value;
            this.dataStream.publish(value);
        }
    }
}
