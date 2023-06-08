import { createIndexedDBAdapter } from './indexeddb-adapter';
import { TGGraphConnectorFromAdapter } from '../client/transports/graph-connector-from-adapter';

export class TGIndexedDBConnector extends TGGraphConnectorFromAdapter
{
    constructor(storageKey?: string)
    {
        super(createIndexedDBAdapter(storageKey), 'TGIndexedDBConnector');
    }
}
