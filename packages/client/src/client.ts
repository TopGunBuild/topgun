import { Keyring, KeysetWithSecrets, LocalContext, TEAM_SCOPE } from "@topgunbuild/models";
import { createKeyset, createKeyring } from "@topgunbuild/model-utils";
import { Store } from "./store";
import { ClientConfig } from "./types";
import { TeamService } from "./team-service";


export class Client
{
    private readonly store: Store;
    private readonly config: ClientConfig;

    constructor(config: ClientConfig)
    {
        this.config = config;
        this.store = new Store(config);
    }

    // readonly #service: ClientService;

    // constructor(options: ClientConfig)
    // {
    //     this.#service = new ClientService(options);
    // }

    // room(roomSid: string): RoomQueryBuilder
    // {
    //     return new RoomQueryBuilder(roomSid, this.#service);
    // }

    // user(publicKey: string)
    // {
    //     return {
    //         section: (sectionId: string) =>
    //         {
    //             return new RoomQueryBuilder(`~@${publicKey}/${sectionId}`, this.#service);
    //         },
    //     };
    // }

    // auth()
    // {

    // }
}
