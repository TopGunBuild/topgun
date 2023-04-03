import { createGraphAdapter } from './indexeddb-adapter';
import { GraphConnectorFromAdapter } from '../client/transports/graph-connector-from-adapter';

export class IndexedDbConnector extends GraphConnectorFromAdapter
{
    constructor(storageKey?: string)
    {
        super(createGraphAdapter(storageKey), 'IndexedDbConnector');
    }
}
