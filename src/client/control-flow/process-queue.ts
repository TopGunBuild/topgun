import { TGMessage } from '../../types'
import { TGEvent } from './event'
import { TGQueue } from './queue'
import { TGMiddlewareSystem } from './middleware-system'

type TGProcessDupesOption = 'process_dupes'|'dont_process_dupes'

export class TGProcessQueue<T = TGMessage, U = any, V = any> extends TGQueue<T>
{
    public isProcessing: boolean;
    public readonly middleware: TGMiddlewareSystem<T, U, V>;
    public readonly completed: TGEvent<T>;
    public readonly emptied: TGEvent<boolean>;
    public readonly processDupes: TGProcessDupesOption;

    protected alreadyProcessed: T[];

    constructor(
        name                               = 'ProcessQueue',
        processDupes: TGProcessDupesOption = 'process_dupes'
    )
    {
        super(name);
        this.alreadyProcessed = [];
        this.isProcessing     = false;
        this.processDupes     = processDupes;
        this.completed        = new TGEvent<T>(`${name}.processed`);
        this.emptied          = new TGEvent<boolean>(`${name}.emptied`);
        this.middleware       = new TGMiddlewareSystem<T, U, V>(`${name}.middleware`);
    }

    public has(item: T): boolean
    {
        return super.has(item) || this.alreadyProcessed.indexOf(item) !== -1
    }

    public async processNext(b?: U, c?: V): Promise<void>
    {
        let item            = this.dequeue();
        const processedItem = item;

        if (!item)
        {
            return
        }

        item = (await this.middleware.process(item, b, c)) as T|undefined;

        if (processedItem && this.processDupes === 'dont_process_dupes')
        {
            this.alreadyProcessed.push(processedItem)
        }

        if (item)
        {
            this.completed.trigger(item);
        }
    }

    public enqueueMany(items: readonly T[]): TGProcessQueue<T, U, V>
    {
        super.enqueueMany(items);
        return this
    }

    public async process(): Promise<void>
    {
        if (this.isProcessing)
        {
            return
        }

        if (!this.count())
        {
            return
        }

        this.isProcessing = true;
        while (this.count())
        {
            try
            {
                await this.processNext()
            }
            catch (e)
            {
                console.error('Process Queue error', e);
            }
        }

        this.emptied.trigger(true);
        this.isProcessing = false;
    }
}
