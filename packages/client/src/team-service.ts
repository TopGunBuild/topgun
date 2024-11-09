import { randomKey } from "@topgunbuild/crypto";
import { Store } from "./store";
import { QueryCb } from "./types";
import {
    ADMIN_SCOPE,
    LocalUserContext,
    MessageRow,
    SelectOptions,
    SelectRequest,
    SelectResult,
    TeamOptions
} from "@topgunbuild/models";
import { castServer, convertToPublicDevice, convertToPublicMember, createKeyset, createLockbox, isNewTeam } from "@topgunbuild/model-utils";
import { ConsoleLogger, LoggerService } from "@topgunbuild/logger";
import { assert } from "@topgunbuild/common";
import { EventEmitter } from "@topgunbuild/eventemitter";

export class TeamService extends EventEmitter {
    private readonly options: TeamOptions;
    private readonly store: Store;
    private readonly seed: string;
    private readonly context: LocalUserContext;
    private readonly logger: LoggerService;

    constructor(options: TeamOptions, store: Store) {
        super();
        this.options = options;
        this.store = store;
        this.seed = options.seed ?? randomKey();
        this.logger = new ConsoleLogger('TeamService');

        if ('user' in options.context) {
            this.context = options.context
        } else {
            // If we're on a server, we'll use the server's hostname for everything
            // and the server's keys as both user keys and device keys
            const { server } = options.context
            this.context = {
                ...options.context,
                device: castServer.toDevice(server),
                user: castServer.toUser(server),
            }
        }
        const { device, user } = this.context;

        if (isNewTeam(options)) {
            // Create a new team with the current user as founding member
            assert(!this.isServer, `Servers can't create teams`);
            this.logger.log(`Creating new team ${options.teamName}`, this.context);

            // Team & role secrets are never stored in plaintext, only encrypted into individual
            // lockboxes. Here we generate new keysets for the team and for the admin role, and store
            // these in new lockboxes for the founding member
            const lockboxTeamKeysForMember = createLockbox({ contents: options.teamKeys, recipientKeys: user.keys })
            const adminKeys = createKeyset(ADMIN_SCOPE, this.seed)
            const lockboxAdminKeysForMember = createLockbox({ contents: adminKeys, recipientKeys: user.keys })

            // We also store the founding user's keys in a lockbox for the user's device
            const lockboxUserKeysForDevice = createLockbox({ contents: user.keys, recipientKeys: this.context.device.keys })

            // We're creating a new graph; this information is to be recorded in the root link
            const rootPayload = {
                name: options.teamName,
                rootMember: convertToPublicMember(user),
                rootDevice: convertToPublicDevice(device),
                lockboxes: [lockboxTeamKeysForMember, lockboxAdminKeysForMember, lockboxUserKeysForDevice],
            }
        } else {
            // this.logger.log(`Loading team ${options.teamName}`, this.context);
            // TODO: Load existing team
        }
    }

    public get userName() {
        return this.context.user.userName;
    }

    public get userId() {
        return this.context.user.$id
    }

    private get isServer() {
        return 'server' in this.context
    }

    subscribeMessages(options: SelectOptions, cb: QueryCb<SelectResult<MessageRow>>): () => void {
        const query = new SelectRequest({ entity: 'message', ...options });
        return this.store.subscribeQuery<MessageRow>(query, cb);
    }
}
