import { ClientService } from '../client-service';
import { FieldQueryBuilder, SelectBuilder } from '../query-builders';
import { NodeQueryHandler } from '../query-handlers';
import { Message } from '@topgunbuild/types';
import { mergeObjects } from '@topgunbuild/utils';

export class MessageApi
{
    readonly #roomSid: string;
    readonly #messageSid: string;
    readonly #service: ClientService;

    constructor(roomSid: string, messageSid: string, service: ClientService)
    {
        this.#roomSid    = roomSid;
        this.#messageSid = messageSid;
        this.#service    = service;
    }

    select(options?: SelectNodeOptions): SelectBuilder<Message, SelectNodeOptions>
    {
        return new SelectBuilder<Message, SelectNodeOptions>(
            new NodeQueryHandler({
                service: this.#service,
                query  : new SelectQuery({
                    fields : options.fields,
                    section: this.#roomSid,
                    node   : this.#messageSid,
                }),
                options: mergeObjects<SelectNodeOptions>({
                    local : true,
                    remote: true,
                    sync  : false,
                    fields: [],
                }, options),
            })
        );
    }

    put(node: Message): Promise<void>
    {
        return this.#service.putNode(this.#roomSid, this.#messageSid, node);
    }

    delete(): Promise<void>
    {
        return this.#service.delete(this.#roomSid, this.#messageSid);
    }

    field(fieldName: string): FieldQueryBuilder
    {
        return new FieldQueryBuilder(
            this.#roomSid,
            this.#messageSid,
            fieldName,
            this.#service,
        );
    }
}
