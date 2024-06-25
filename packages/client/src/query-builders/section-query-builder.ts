import { DataNode } from '@topgunbuild/store';
import { randomId, toArray } from '@topgunbuild/utils';
import { ClientService } from '../client-service';
import { NodeQueryBuilder } from './node-query-builder';
import { SelectSectionOptions } from '@topgunbuild/transport';

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

    select(options?: SelectSectionOptions)
    {
        const defaultOptions: SelectSectionOptions = {
            limit : this.#service.options.rowLimit,
            local : true,
            remote: true,
            sync  : false,
        };
        options                                    = Object.assign(defaultOptions, options || {});

        if (options?.limit > this.#service.options.rowLimit)
        {
            throw new Error(`Limit for rows (controlled by 'rowLimit' setting) exceeded, max rows: ${this.#service.options.rowLimit}`);
        }

        return this.#service.select(options);
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
}
