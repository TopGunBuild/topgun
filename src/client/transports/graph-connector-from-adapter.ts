import { Get, Put, GraphAdapter } from '../../types'
import { generateMessageId } from '../graph/graph-utils'
import { GraphWireConnector } from './graph-wire-connector'

const NOOP = () => undefined;

export class GraphConnectorFromAdapter extends GraphWireConnector
{
    protected readonly adapter: GraphAdapter;

    /**
     * Constructor
     */
    constructor(adapter: GraphAdapter, name = 'GraphConnectorFromAdapter')
    {
        super(name);
        this.adapter = adapter;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    public get({ soul, cb, opts, msgId = '' }: Get): () => void
    {
        this.adapter
            .get(soul, opts)
            .then(node => ({
                '#': generateMessageId(),
                '@': msgId,
                put: node
                    ? {
                        [soul]: node
                    }
                    : undefined
            }))
            .catch(error =>
            {
                console.warn(error.stack || error);

                return {
                    '#': generateMessageId(),
                    '@': msgId,
                    err: 'Error fetching node'
                }
            })
            .then(msg =>
            {
                this.ingest([msg]);
                if (cb)
                {
                    cb(msg)
                }
            });

        return NOOP
    }

    public put({ graph, msgId = '', cb }: Put): () => void
    {
        this.adapter
            .put(graph)
            .then(() =>
            {
                return {
                    '#': generateMessageId(),
                    '@': msgId,
                    err: null,
                    ok : true
                }
            })
            .catch(error =>
            {
                console.warn(error.stack || error);

                return {
                    '#': generateMessageId(),
                    '@': msgId,
                    err: 'Error saving put',
                    ok : false
                }
            })
            .then(msg =>
            {
                this.ingest([msg]);
                if (cb)
                {
                    cb(msg)
                }
            });

        return NOOP
    }
}
