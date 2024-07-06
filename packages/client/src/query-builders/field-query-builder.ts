import { DataValue } from '@topgunbuild/store';
import { SelectOptions, SelectQuery } from '@topgunbuild/transport';
import { mergeObjects } from '@topgunbuild/utils';
import { ClientService } from '../client-service';
import { FieldQueryHandler } from '../query-handlers';
import { SelectBuilder } from './select-builder';

export class FieldQueryBuilder
{
    readonly #section: string;
    readonly #node: string;
    readonly #field: string;
    readonly #service: ClientService;

    constructor(
        sectionName: string,
        nodeName: string,
        fieldName: string,
        service: ClientService,
    )
    {
        this.#section = sectionName;
        this.#node    = nodeName;
        this.#field   = fieldName;
        this.#service = service;
    }

    select(options?: SelectOptions): SelectBuilder<DataValue, SelectOptions>
    {
        return new SelectBuilder<DataValue, SelectOptions>(
            new FieldQueryHandler({
                service: this.#service,
                query  : new SelectQuery({
                    section: this.#section,
                    node   : this.#node,
                    field  : this.#field,
                }),
                options: mergeObjects<SelectOptions>({
                    local : true,
                    remote: true,
                    sync  : false,
                }, options),
            })
        );
    }

    put(value: DataValue): Promise<void>
    {
        return this.#service.putValue(
            this.#section,
            this.#node,
            this.#field,
            value,
        );
    }

    delete(): Promise<void>
    {
        return this.#service.delete(
            this.#section,
            this.#node,
            this.#field,
        );
    }
}
