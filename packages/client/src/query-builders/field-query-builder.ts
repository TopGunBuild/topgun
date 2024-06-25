import { DataValue } from '@topgunbuild/store';
import { SelectFieldOptions } from '@topgunbuild/transport';
import { ClientService } from '../client-service';

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

    async select(options?: SelectFieldOptions)
    {

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
