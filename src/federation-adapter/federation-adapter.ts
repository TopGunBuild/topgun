import { TGFederatedAdapterOptions } from './types';
import { TGGraphAdapter, TGGraphData, TGOptionsGet } from '../types';
import { Writer } from './writer';
import { TGPeers } from './peers';
import { TGExtendedLoggerType } from '../logger';
import { ConnectToPeer } from './connect-to-peer';

export class TGFederationAdapter implements TGGraphAdapter
{
    internal: TGGraphAdapter;
    peers: TGPeers;
    persistence: TGGraphAdapter;
    options: TGFederatedAdapterOptions;
    logger: TGExtendedLoggerType;

    private readonly writer: Writer;

    /**
     * Constructor
     */
    constructor(
        internal: TGGraphAdapter,
        peers: TGPeers,
        persistence: TGGraphAdapter,
        options: TGFederatedAdapterOptions,
        logger: TGExtendedLoggerType
    )
    {
        const defaultOptions: TGFederatedAdapterOptions = {
            putToPeers: true
        };

        this.internal    = internal;
        this.peers       = peers;
        this.persistence = persistence;
        this.options     = Object.assign(defaultOptions, options || {});
        this.logger      = logger;
        this.writer      = new Writer(this.persistence, this.peers, this.options, this.logger);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    async get(getOpts: TGOptionsGet): Promise<TGGraphData>
    {
        this.logger.log('get', getOpts);
        await this.writer.updateFromPeers(getOpts);
        return this.internal.get(getOpts);
    }

    async put(data: TGGraphData): Promise<TGGraphData|null>
    {
        this.logger.log('put', data);
        const diff = await this.persistence.put(data);

        if (!diff)
        {
            return diff
        }

        if (this.options.putToPeers)
        {
            this.writer.updatePeers(diff, this.peers.getPeers());
        }

        return diff
    }

    connectToPeers(): () => void
    {
        const connectors: ConnectToPeer[] = [];

        if (this.peers.size && this.options.reversePeerSync)
        {
            this.peers.getPeers().forEach(async (peer) =>
            {
                const connector = new ConnectToPeer(peer, this.persistence, this.options, this.writer, this.logger);
                connector.connect();
                connectors.push(connector);
            });
        }

        return () => connectors.forEach(c => c.disconnect());
    }
}