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

    createTeam(teamName: string, context: LocalContext, seed?: string): TeamService
    {
        const teamKeys = createKeyset(TEAM_SCOPE, seed)

        return new TeamService({ teamName, context, teamKeys }, this.store);
    }

    loadTeam(
        teamName: string,
        context: LocalContext,
        teamKeys: KeysetWithSecrets | Keyring
    ): TeamService
    {
        const teamKeyring = createKeyring(teamKeys)
        return new TeamService({ teamName, context, teamKeyring }, this.store);
    }
}
