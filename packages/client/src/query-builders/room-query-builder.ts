import { DataNode } from '@topgunbuild/store';
import { mergeObjects, randomId, toArray } from '@topgunbuild/utils';
import { SelectQuery, SelectSectionOptions } from '@topgunbuild/transport';
import { ClientService } from '../client-service';
import { NodeQueryBuilder } from './node-query-builder';
import { SelectBuilder } from './select-builder';
import { SectionQueryHandler } from '../query-handlers';

export class RoomQueryBuilder
{
    readonly #roomSid: string;
    readonly #service: ClientService;

    constructor(roomSid: string, service: ClientService)
    {
        this.#roomSid = roomSid;
        this.#service = service;
    }

    select(options?: SelectSectionOptions): SelectBuilder<DataNode[], SelectSectionOptions>
    {
        if (options?.limit > this.#service.options.rowLimit)
        {
            throw new Error(`Limit for rows (controlled by 'rowLimit' setting) exceeded, max rows: ${this.#service.options.rowLimit}`);
        }

        return new SelectBuilder<DataNode[], SelectSectionOptions>(
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

    async insert(values: DataNode): Promise<void>
    async insert(values: DataNode[]): Promise<void>
    async insert(values: DataNode[]|DataNode): Promise<void>
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
