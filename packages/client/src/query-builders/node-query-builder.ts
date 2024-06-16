import { ClientService } from '../client-service';
import { DataNode } from '@topgunbuild/store';
import { SelectOptions } from '@topgunbuild/transport';
import { FieldQueryBuilder } from './field-query-builder';

export class NodeQueryBuilder
{
    readonly #sectionName: string;
    readonly #nodeName: string;
    readonly #service: ClientService;

    constructor(sectionName: string, nodeName: string, service: ClientService)
    {
        this.#sectionName = sectionName;
        this.#nodeName    = nodeName;
        this.#service     = service;
    }

    async select(options?: SelectOptions&{ fields?: string[] })
    {

    }

    async put(node: DataNode)
    {
        return this.#service.putNode(this.#sectionName, this.#nodeName, node);
    }

    async delete()
    {

    }

    field(fieldName: string): FieldQueryBuilder
    {
        return new FieldQueryBuilder(
            this.#sectionName,
            this.#nodeName,
            fieldName,
            this.#service,
        );
    }
}
