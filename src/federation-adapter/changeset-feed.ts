import { isNotEmptyObject } from '@topgunbuild/typed';
import { TGGraphAdapter, TGGraphData } from '../types';
import { TGChangeSetEntry } from './types';
import { createLex } from '../client/link/lex';
import { CHANGELOG_SOUL } from './constants';

export class ChangesetFeed
{
    lastKey: string;
    changes: TGChangeSetEntry[];
    graphPromise: Promise<TGGraphData|null>|null;
    peer: TGGraphAdapter;

    static getChangesetFeed(peer: TGGraphAdapter, from: string): () => Promise<TGChangeSetEntry|null>
    {
        const feed = new ChangesetFeed(peer, from);
        return () => feed.getNext();
    }

    /**
     * Constructor
     */
    constructor(peer: TGGraphAdapter, from: string)
    {
        this.peer         = peer;
        this.lastKey      = from;
        this.changes      = [];
        this.graphPromise = null;
    }

    async getNext(): Promise<TGChangeSetEntry|null>
    {
        if (!this.changes.length && !this.graphPromise)
        {
            this.graphPromise = this.peer.get(
                createLex(CHANGELOG_SOUL).start(this.lastKey).getQuery()
            );
            const graph       = await this.graphPromise;
            // console.log(graph, createLex(CHANGELOG_SOUL).start(lastKey).getQuery());
            this.graphPromise = null;

            if (isNotEmptyObject(graph))
            {
                console.log(graph);
                for (const key in graph)
                {
                    if (key && key !== '_')
                    {
                        this.changes.unshift([key, graph[key]]);
                        this.lastKey = key
                    }
                }
            }
        }
        else if (this.graphPromise)
        {
            await this.graphPromise;
            this.graphPromise = null
        }

        const entry = this.changes.pop();
        return entry || null
    }
}
