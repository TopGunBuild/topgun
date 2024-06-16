import { ClientOptions } from './types';
import { SectionQueryBuilder } from './query-builders/section-query-builder';
import { ClientService } from './client-service';

export class Client
{
    readonly #service: ClientService;

    constructor(options: ClientOptions)
    {
        this.#service = new ClientService(options);
    }

    section(sectionName: string): SectionQueryBuilder
    {
        return new SectionQueryBuilder(sectionName, this.#service);
    }

    user(publicKey: string)
    {
        return {
            section: (sectionId: string) =>
            {
                return new SectionQueryBuilder(sectionId, this.#service);
            }
        }
    }

    auth()
    {

    }
}