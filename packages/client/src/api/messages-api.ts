import { SelectQuery, SelectSectionOptions } from '@topgunbuild/transport';
import { mergeObjects, randomId, toArray } from '@topgunbuild/utils';
import { Message } from '@topgunbuild/types';
import { ClientService } from '../client-service';
import { NodeQueryBuilder, SelectBuilder } from '../query-builders';
import { SectionQueryHandler } from '../query-handlers';

export class MessagesApi
{
    readonly #roomSid: string;
    readonly #service: ClientService;

    constructor(roomSid: string, service: ClientService)
    {
        this.#roomSid = roomSid;
        this.#service = service;
    }

    select(options?: SelectSectionOptions): SelectBuilder<Message[], SelectSectionOptions>
    {
        if (options?.limit > this.#service.options.rowLimit)
        {
            throw new Error(`Limit for rows (controlled by 'rowLimit' setting) exceeded, max rows: ${this.#service.options.rowLimit}`);
        }

        return new SelectBuilder<Message[], SelectSectionOptions>(
            new SectionQueryHandler({
                service: this.#service,
                query  : new SelectQuery(options),
                options: mergeObjects<SelectSectionOptions>({
                    limit : this.#service.options.rowLimit,
                    local : true,
                    remote: true,
                    sync  : false,
                }, options),
            })
        );
    }

    async insert(values: Message): Promise<void>
    async insert(values: Message[]): Promise<void>
    async insert(values: Message[]|Message): Promise<void>
    {
        await Promise.all(
            toArray(values).map(value =>
                this.#service.putNode(
                    this.#roomSid, randomId(), value,
                ),
            ),
        );
    }

    node(nodeName: string): NodeQueryBuilder
    {
        return new NodeQueryBuilder(this.#roomSid, nodeName, this.#service);
    }
}
