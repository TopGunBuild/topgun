import { DataValue, StoreValue } from '@topgunbuild/store';
import { SelectFieldMessage, SelectFieldOptions } from '@topgunbuild/transport';
import { QueryHandler } from './query-handler';
import { ClientService } from '../client-service';

export class FieldQueryHandler extends QueryHandler<DataValue>
{
    selectMessage: SelectFieldMessage;

    constructor(props: {
        service: ClientService,
        options: SelectFieldOptions,
        message: SelectFieldMessage
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
