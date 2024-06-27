import { DataNode } from '@topgunbuild/store';
import { SelectQuery, SelectNodeOptions } from '@topgunbuild/transport';
import { ClientService } from '../client-service';
import { FieldQueryBuilder } from './field-query-builder';
import { mergeObjects } from '@topgunbuild/utils';
import { NodeQueryHandler } from '../query-handlers/node-query-handler';
import { SelectBuilder } from './select-builder';

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

    select(options?: SelectNodeOptions): SelectBuilder<DataNode>
    {
        const handler = new NodeQueryHandler({
            service: this.#service,
            query  : new SelectQuery({
                fields : options.fields,
                section: this.#section,
                node   : this.#node,
            }),
            options: mergeObjects<SelectNodeOptions>({
                local : true,
                remote: true,
                sync  : false,
                fields: [],
            }, options),
        });
        return new SelectBuilder<DataNode>(handler);
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
