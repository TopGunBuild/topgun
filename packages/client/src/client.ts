import { ClientOptions } from './types';
import { RoomQueryBuilder } from './query-builders';
import { ClientService } from './client-service';

export class Client
{
    readonly #service: ClientService;

    constructor(options: ClientOptions)
    {
        this.#service = new ClientService(options);
    }

    room(roomSid: string): RoomQueryBuilder
    {
        return new RoomQueryBuilder(roomSid, this.#service);
    }

    user(publicKey: string)
    {
        return {
            section: (sectionId: string) =>
            {
                return new RoomQueryBuilder(`~@${publicKey}/${sectionId}`, this.#service);
            },
        };
    }

    auth()
    {

    }
}
