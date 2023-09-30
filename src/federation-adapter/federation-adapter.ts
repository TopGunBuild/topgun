import { TGFederatedAdapterOptions } from './types';
import { TGGraphAdapter, TGGraphData, TGOptionsGet, TGOriginators } from '../types';
import { PeersWriter } from './peers-writer';
import { TGPeers } from './peers';
import { TGExtendedLoggerType } from '../logger';
import { PeerChangeHandler } from './peer-change-handler';

export class TGFederationAdapter implements TGGraphAdapter
{
    serverName: string;
    internal: TGGraphAdapter;
    peers: TGPeers;
    persistence: TGGraphAdapter;
    options: TGFederatedAdapterOptions;
    logger: TGExtendedLoggerType;

    private readonly writer: PeersWriter;

    /**
     * Constructor
     */
    constructor(
        serverName: string,
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

        this.serverName  = serverName;
        this.internal    = internal;
        this.peers       = peers;
        this.persistence = persistence;
        this.options     = Object.assign(defaultOptions, options || {});
        this.logger      = logger;
        this.writer      = new PeersWriter(this.serverName, this.persistence, this.peers, this.options, this.logger);
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

    async put(data: TGGraphData, originators?: TGOriginators): Promise<TGGraphData|null>
    {
        this.logger.log('put', data);
        const diff = await this.persistence.put(data, originators);

        if (!diff)
        {
            return diff
        }

        if (this.options.putToPeers)
        {
            this.writer.updatePeers(diff, this.peers.getPeers(), originators);
        }

        return diff
    }

    connectToPeers(): () => void
    {
        const handlers: PeerChangeHandler[] = [];

        if (this.peers.size && this.options.reversePeerSync)
        {
            this.peers.getPeers().forEach(async (peer) =>
            {
                const changeHandler = new PeerChangeHandler(this.serverName, peer, this.writer, this.logger);
                changeHandler.connect();
                handlers.push(changeHandler);
            });
        }

        return () => handlers.forEach(c => c.disconnect());
    }
}