import { Message } from '../../types'
import { Event } from './event'
import { Queue } from './queue'
import { MiddlewareSystem } from './middleware-system'

type ProcessDupesOption = 'process_dupes'|'dont_process_dupes'

export class ProcessQueue<T = Message, U = any, V = any> extends Queue<T>
{
    public isProcessing: boolean;
    public readonly middleware: MiddlewareSystem<T, U, V>;
    public readonly completed: Event<T>;
    public readonly emptied: Event<boolean>;
    public readonly processDupes: ProcessDupesOption;

    protected alreadyProcessed: T[];

    constructor(
        name                             = 'ProcessQueue',
        processDupes: ProcessDupesOption = 'process_dupes'
    )
    {
        super(name);
        this.alreadyProcessed = [];
        this.isProcessing     = false;
        this.processDupes     = processDupes;
        this.completed        = new Event<T>(`${name}.processed`);
        this.emptied          = new Event<boolean>(`${name}.emptied`);
        this.middleware       = new MiddlewareSystem<T, U, V>(`${name}.middleware`);
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

    public enqueueMany(items: readonly T[]): ProcessQueue<T, U, V>
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
