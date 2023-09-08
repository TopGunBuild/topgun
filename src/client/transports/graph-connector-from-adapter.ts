import { TGGet, TGPut, TGGraphAdapter } from '../../types';
import { TGGraphWireConnector } from './graph-wire-connector';
import { uuidv4 } from '../../utils/uuidv4';
import { NOOP } from '../../utils/noop';

export class TGGraphConnectorFromAdapter extends TGGraphWireConnector
{
    protected readonly adapter: TGGraphAdapter;

    /**
     * Constructor
     */
    constructor(adapter: TGGraphAdapter, name = 'GraphConnectorFromAdapter')
    {
        super(name);
        this.adapter = adapter;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    get({ cb, options, msgId = '' }: TGGet): () => void
    {
        this.adapter
            .get(options)
            .then(graphData => ({
                '#'  : uuidv4(),
                '@'  : msgId,
                'put': graphData
            }))
            .catch((error) =>
            {
                console.warn(error.stack || error);

                return {
                    '#'  : uuidv4(),
                    '@'  : msgId,
                    'err': 'Error fetching node',
                };
            })
            .then((msg) =>
            {
                this.ingest([msg]);
                if (cb)
                {
                    cb(msg);
                }
            });

        return NOOP;
    }

    put({ graph, msgId = '', cb }: TGPut): () => void
    {
        this.adapter
            .put(graph)
            .then(() =>
            {
                return {
                    '#'  : uuidv4(),
                    '@'  : msgId,
                    'err': null,
                    'ok' : true,
                };
            })
            .catch((error) =>
            {
                console.warn(error.stack || error);

                return {
                    '#'  : uuidv4(),
                    '@'  : msgId,
                    'err': 'Error saving put',
                    'ok' : false,
                };
            })
            .then((msg) =>
            {
                this.ingest([msg]);
                if (cb)
                {
                    cb(msg);
                }
            });

        return NOOP;
    }
}
