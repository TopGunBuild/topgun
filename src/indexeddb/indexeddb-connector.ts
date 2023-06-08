import { createIndexedDBAdapter } from './indexeddb-adapter';
import { TGGraphConnectorFromAdapter } from '../client/transports/graph-connector-from-adapter';
import { TGGraphAdapterOptions } from '../types';

export class TGIndexedDBConnector extends TGGraphConnectorFromAdapter
{
    constructor(storageKey?: string, adapterOptions?: TGGraphAdapterOptions)
    {
        super(createIndexedDBAdapter(storageKey, adapterOptions), 'TGIndexedDBConnector');
    }
}
