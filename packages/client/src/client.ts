import { ClientOptions } from './types';
import { SectionQueryBuilder } from './query-builders';
import { ClientService } from './client-service';

export class Client
{
    readonly #service: ClientService;

    constructor(options: ClientOptions)
    {
        this.#service = new ClientService(options);
    }

    section(section: string): SectionQueryBuilder
    {
        return new SectionQueryBuilder(section, this.#service);
    }

    user(publicKey: string)
    {
        return {
            section: (sectionId: string) =>
            {
                return new SectionQueryBuilder(`~@${publicKey}/${sectionId}`, this.#service);
            },
        };
    }

    auth()
    {

    }
}
