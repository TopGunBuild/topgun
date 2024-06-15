import { ClientOptions } from './types';
import { ClientQueryBuilder } from './client-query-builder';
import { ClientProviders } from './client-providers';

export class Client
{
    readonly #providers: ClientProviders;

    constructor(options: ClientOptions)
    {
        this.#providers = new ClientProviders(options);
    }

    section(section: string): ClientQueryBuilder
    {
        return new ClientQueryBuilder(section, this.#providers);
    }

    user(publicKey: string)
    {
        return {
            from: (section: string) =>
            {
                return new ClientQueryBuilder(section, this.#providers);
            }
        }
    }

    auth()
    {

    }
}
