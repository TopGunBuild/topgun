import { createGraphAdapter } from './indexeddb-adapter';
import { TGGraphConnectorFromAdapter } from '../client/transports/graph-connector-from-adapter';

export class TGIndexedDbConnector extends TGGraphConnectorFromAdapter 
{
    constructor(storageKey?: string) 
    {
        super(createGraphAdapter(storageKey), 'TGIndexedDbConnector');
    }
}
