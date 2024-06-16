import {
    Query, SelectOptions,
    Sort,
} from '@topgunbuild/transport';
import { DataNode } from '@topgunbuild/store';
import { toArray } from '@topgunbuild/utils';
import { v4 as uuidv4 } from 'uuid';
import { ClientService } from '../client-service';
import { NodeQueryBuilder } from './node-query-builder';

export class SectionQueryBuilder
{
    readonly #sectionName: string;
    readonly #service: ClientService;

    constructor(sectionName: string, service: ClientService)
    {
        this.#sectionName = sectionName;
        this.#service     = service;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    async select(
        options?: SelectOptions&{
            fields?: string[];
            query?: Query[];
            sort?: Sort[];
        },
    )
    {
        // Set fetch based on either the size or the maximum value (default to maximum u32 (4294967295))
        // queryMessage.fetch = queryMessage.fetch ?? 0xffffffff;
    }

    async insert(values: DataNode): Promise<void>
    async insert(values: DataNode[]): Promise<void>
    async insert(values: DataNode[]|DataNode): Promise<void>
    {
        await Promise.all(
            toArray(values).map(value =>
                this.#service.putNode(
                    this.#sectionName, uuidv4(), value,
                ),
            ),
        );
    }

    node(nodeName: string): NodeQueryBuilder
    {
        return new NodeQueryBuilder(this.#sectionName, nodeName, this.#service);
    }
}
