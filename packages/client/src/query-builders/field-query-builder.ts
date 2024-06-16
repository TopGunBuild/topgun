import { ClientService } from '../client-service';
import { DataValue } from '@topgunbuild/store';
import { SelectOptions } from '@topgunbuild/transport';

export class FieldQueryBuilder
{
    readonly #sectionName: string;
    readonly #nodeName: string;
    readonly #fieldName: string;
    readonly #service: ClientService;

    constructor(
        sectionName: string,
        nodeName: string,
        fieldName: string,
        service: ClientService,
    )
    {
        this.#sectionName = sectionName;
        this.#nodeName    = nodeName;
        this.#fieldName   = fieldName;
        this.#service     = service;
    }

    async select(options?: SelectOptions)
    {

    }

    async put(value: DataValue)
    {
        return this.#service.putValue(
            this.#sectionName,
            this.#nodeName,
            this.#fieldName,
            value
        );
    }

    async delete()
    {

    }
}
