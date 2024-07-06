import { DataValue, StoreResults, StoreValue } from '@topgunbuild/store';
import { SelectQuery, SelectOptions } from '@topgunbuild/transport';
import { equal } from '@topgunbuild/utils';
import { QueryHandler } from './query-handler';
import { ClientService } from '../client-service';
import { coerceDataValue } from '../utils';

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

    isQualify(value: StoreValue): boolean
    {
        return value.section === this.query.section
            && value.node === this.query.node
            && value.field === this.query.field;
    }

    onOutput(results: StoreResults): void
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
