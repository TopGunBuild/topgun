import { TGFederatedAdapterOptions } from './types';
import { TGGraphAdapter, TGGraphData } from '../types';
import { Writer } from './writer';
import { diffCRDT, mergeGraph } from '../crdt';
import { TGPeers } from './peers';

export class BatchWriter
{
    batch: TGGraphData = {};

    /**
     * Constructor
     */
    constructor(
        private readonly peers: TGPeers,
        private readonly persistence: TGGraphAdapter,
        private readonly options: TGFederatedAdapterOptions,
        private readonly writer: Writer
    )
    {
    }

    queueDiff(changes: TGGraphData): TGGraphData|undefined
    {
        const diff = diffCRDT(changes, this.batch);
        this.batch = diff ? mergeGraph(this.batch, diff, 'mutable') : this.batch;
        return diff;
    }

    async writeBatch(): Promise<TGGraphData|null>
    {
        if (!Object.keys(this.batch).length)
        {
            return null
        }
        const toWrite = this.batch;
        this.batch         = {};

        const diff = await this.persistence.put(toWrite);

        if (diff)
        {
            if (this.options.maintainChangelog)
            {
                this.writer.updateChangelog(diff);
            }

            if (this.options.putToPeers)
            {
                this.writer.updatePeers(diff, this.peers);
            }
        }

        return diff;
    }
}
