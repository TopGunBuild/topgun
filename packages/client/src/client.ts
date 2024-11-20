import { Store } from "./store";
import { ClientConfig, CreateTeamParams } from "./types";
import { TeamAPI } from "./team-api";
import { randomKey } from "@topgunbuild/crypto";
import { cloneValue, randomId } from "@topgunbuild/common";
import { ConsoleLogger, LoggerService } from "@topgunbuild/logger";
import { 
    ADMIN_SCOPE, 
    CreateTeamAction, 
    DeviceImpl, 
    DeviceWithSecrets, 
    UserWithSecrets,
    KeysetImpl, 
    KeysetWithSecrets,
    MemberImpl, 
    Team, 
    TEAM_SCOPE 
} from "@topgunbuild/models";
import { 
    castServer, 
    convertToPublicMember, 
    convertToPublicDevice, 
    createKeyset, 
    createLockbox 
} from "@topgunbuild/model-utils";

export class Client {
    readonly #store: Store;
    readonly #logger: LoggerService;

    constructor(config: ClientConfig) {
        this.#logger = new ConsoleLogger('Client');
        this.#store = new Store(cloneValue(config), this.#logger);
    }

    async loadTeam(teamId: string): Promise<TeamAPI> {
        const team = await this.#store.getTeam(teamId);
        return new TeamAPI(team, this.#store);
    }

    /**
     * Create a new team with the current user as the founding member
     * 
     * @param params Configuration object for team creation
     * @param params.name The name of the team (required)
     * @param params.description Optional description of the team
     * @param params.seed Optional cryptographic seed. If not provided, a random key will be generated
     * 
     * @throws {Error} If:
     * - Running on server
     * - Team name is invalid
     * - Context is invalid
     * - Team creation fails
     * 
     * @returns Promise<TeamAPI> The team API instance for the newly created team
     */
    async createTeam(params: CreateTeamParams): Promise<TeamAPI> {
        this.#validateTeamParams(params);

        const seed = params.seed ?? randomKey();
        const { user, device } = this.#ensureUserContext();

        // Create the team base object
        const team: Team = {
            $id: randomId(),
            name: params.name,
            description: params.description,
        };

        this.#logger.log(`Creating new team "${team.name}" with ID ${team.$id}`);

        // Create all necessary security assets
        const securityAssets = this.#createTeamSecurityAssets(seed, user.keys);

        // Create device lockbox
        const lockboxUserKeysForDevice = createLockbox({ 
            contents: user.keys, 
            recipientKeys: device.keys 
        });

        // Prepare member and device implementations
        const memberPublicData = convertToPublicMember(user);
        const rootMember = new MemberImpl({
            ...memberPublicData,
            keys: new KeysetImpl({
                ...memberPublicData.keys,
                teamId: team.$id,
                publicKey: '',
            }),
        });

        const devicePublicData = convertToPublicDevice(device);
        const rootDevice = new DeviceImpl({
            ...devicePublicData,
            keys: new KeysetImpl({
                ...devicePublicData.keys,
                publicKey: '',
                teamId: team.$id,
            }),
        });

        // Create the team creation action
        const action = new CreateTeamAction({
            teamId: team.$id,
            name: team.name,
            rootMember,
            rootDevice,
            lockboxes: [
                securityAssets.lockboxTeamKeysForMember,
                securityAssets.lockboxAdminKeysForMember,
                lockboxUserKeysForDevice
            ],
        });

        try {
            await this.#store.dispatchAction(action);
            return new TeamAPI(team, this.#store, securityAssets.teamKeys);
        } catch (error) {
            this.#logger.error('Failed to create team:', error);
            throw new Error(`Failed to create team: ${error['message']}`);
        }
    }

    /**
     * Validates and normalizes team creation parameters
     * @throws {Error} If team name is invalid
     */
    #validateTeamParams(params: CreateTeamParams): void {
        if (!params.name || params.name.trim().length === 0) {
            throw new Error('Team name is required and cannot be empty');
        }
        if (params.description && params.description.trim().length === 0) {
            throw new Error('Team description cannot be empty if provided');
        }
    }

    /**
     * Ensures the context contains necessary user and device information
     * @throws {Error} If neither user nor valid server context is available
     */
    #ensureUserContext(): { user: UserWithSecrets; device: DeviceWithSecrets } {
        if (this.#store.isServer) {
            throw new Error('Servers cannot create teams');
        }

        if ('user' in this.#store.context) {
            return {
                user: this.#store.context.user,
                device: this.#store.context.device
            };
        }

        const { server } = this.#store.context;
        if (!server) {
            throw new Error('Neither user nor server found in context');
        }

        const context = {
            device: castServer.toDevice(server),
            user: castServer.toUser(server)
        };

        this.#store.context = {
            ...this.#store.context,
            ...context
        };

        return context;
    }

    /**
     * Creates encryption keys and lockboxes for a new team
     */
    #createTeamSecurityAssets(seed: string, userKeys: KeysetWithSecrets) {
        // Generate team-level keys
        const teamKeys = createKeyset(TEAM_SCOPE, seed);
        const lockboxTeamKeysForMember = createLockbox({ 
            contents: teamKeys, 
            recipientKeys: userKeys 
        });

        // Generate admin role keys
        const adminKeys = createKeyset(ADMIN_SCOPE, seed);
        const lockboxAdminKeysForMember = createLockbox({ 
            contents: adminKeys, 
            recipientKeys: userKeys 
        });

        return {
            teamKeys,
            adminKeys,
            lockboxTeamKeysForMember,
            lockboxAdminKeysForMember
        };
    }
}
