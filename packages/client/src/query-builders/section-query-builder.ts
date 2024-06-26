import { DataNode } from '@topgunbuild/store';
import { mergeObjects, randomId, toArray } from '@topgunbuild/utils';
import { ClientService } from '../client-service';
import { NodeQueryBuilder } from './node-query-builder';
import { SelectMessage, SelectSectionOptions } from '@topgunbuild/transport';
import { SelectBuilder } from './select-builder';
import { SectionQueryHandler } from '../query-handlers/section-query-handler';

export class SectionQueryBuilder
{
    readonly #section: string;
    readonly #service: ClientService;

    constructor(section: string, service: ClientService)
    {
        this.#section = section;
        this.#service = service;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    select(options?: SelectSectionOptions): SelectBuilder<DataNode[]>
    {
        if (options?.limit > this.#service.options.rowLimit)
        {
            throw new Error(`Limit for rows (controlled by 'rowLimit' setting) exceeded, max rows: ${this.#service.options.rowLimit}`);
        }

        const handler = new SectionQueryHandler({
            service: this.#service,
            message: new SelectMessage(options),
            options: mergeObjects<SelectSectionOptions>({
                limit : this.#service.options.rowLimit,
                local : true,
                remote: true,
                sync  : false,
            }, options),
        });
        return new SelectBuilder<DataNode[]>(handler);
    }

    async insert(values: DataNode): Promise<void>
    async insert(values: DataNode[]): Promise<void>
    async insert(values: DataNode[]|DataNode): Promise<void>
    {
        await Promise.all(
            toArray(values).map(value =>
                this.#service.putNode(
                    this.#section, randomId(), value,
                ),
            ),
        );
    }

    node(nodeName: string): NodeQueryBuilder
    {
        return new NodeQueryBuilder(this.#section, nodeName, this.#service);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------


}
