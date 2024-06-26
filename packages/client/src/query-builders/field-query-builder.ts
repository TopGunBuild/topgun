import { DataValue } from '@topgunbuild/store';
import { SelectFieldMessage, SelectFieldOptions } from '@topgunbuild/transport';
import { ClientService } from '../client-service';
import { mergeObjects } from '@topgunbuild/utils';
import { FieldQueryHandler } from '../query-handlers/field-query-handler';
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

    select(options?: SelectFieldOptions): SelectBuilder<DataValue>
    {
        const handler = new FieldQueryHandler({
            service: this.#service,
            message: new SelectFieldMessage({
                section: this.#section,
                node   : this.#node,
                field  : this.#field,
            }),
            options: mergeObjects<SelectFieldOptions>({
                local : true,
                remote: true,
                sync  : false,
            }, options),
        });
        return new SelectBuilder<DataValue>(handler);
    }

    async put(value: DataValue)
    {
        return this.#service.putValue(
            this.#section,
            this.#node,
            this.#field,
            value,
        );
    }

    async delete()
    {

    }
}
