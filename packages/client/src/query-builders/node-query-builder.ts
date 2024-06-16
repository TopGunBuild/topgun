import { DataNode } from '@topgunbuild/store';
import { SelectOptions } from '@topgunbuild/transport';
import { ClientService } from '../client-service';
import { FieldQueryBuilder } from './field-query-builder';

export class NodeQueryBuilder
{
    readonly #section: string;
    readonly #node: string;
    readonly #service: ClientService;

    constructor(section: string, node: string, service: ClientService)
    {
        this.#section = section;
        this.#node    = node;
        this.#service = service;
    }

    async select(options?: SelectOptions&{ fields?: string[] })
    {

    }

    async put(node: DataNode)
    {
        return this.#service.putNode(this.#section, this.#node, node);
    }

    async delete()
    {

    }

    field(fieldName: string): FieldQueryBuilder
    {
        return new FieldQueryBuilder(
            this.#section,
            this.#node,
            fieldName,
            this.#service,
        );
    }
}
