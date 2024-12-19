import { Store } from "./store";
import { ClientConfig, CreateTeamParams } from "./types";
import { TeamAPI } from "./team-api";
import { ConsoleLogger, LoggerService } from "@topgunbuild/logger";
import { cloneValue, randomId } from "@topgunbuild/common";
import { CreateTeamCommand } from "./commands/create-team.command";
import { ADMIN_SCOPE, CreateTeamAction, Team, TEAM_SCOPE, TeamInfo } from "@topgunbuild/models";
import {
    convertToPublicDevice,
    convertToPublicKeyset,
    convertToPublicMember,
    convertToPublicUser,
    createDevice,
    createKeyset,
    createLockbox,
    createUser
} from "@topgunbuild/model-utils";
import { parseInvitationCode } from "@topgunbuild/model-utils";

export class Client {
    readonly #store: Store;
    readonly #logger: LoggerService;
    readonly #createTeamCommand: CreateTeamCommand;

    constructor(config: ClientConfig) {
        this.#logger = new ConsoleLogger('Client');
        this.#store = new Store(cloneValue(config), this.#logger);
        this.#createTeamCommand = new CreateTeamCommand(this.#store, this.#logger);
    }

    async loadTeam(teamId: string): Promise<TeamAPI> {
        const team = await this.#store.queryOne('team', teamId);
        return new TeamAPI(team, this.#store, this.#logger, this.#store.context);
    }

    async joinTeamAsDevice(params: { deviceName: string, invitationCode: string }): Promise<TeamAPI> {
        const { deviceName, invitationCode } = params;
        const { teamId, invitationSeed } = parseInvitationCode(invitationCode);
        const userId = null;

        const device = createDevice({ userId, deviceName });
    }

    async joinTeamAsMember(params: { userName: string, invitationCode: string, deviceName: string }): Promise<TeamAPI> {
        const { userName, invitationCode, deviceName } = params;

        const user = createUser(userName);
        const device = createDevice({ userId: user.$id, deviceName });
        const { teamId, invitationSeed } = parseInvitationCode(invitationCode);

        // const invitation = await this.#store.queryOne('invitation', invitationCode);
        // return new TeamAPI(team, this.#store, this.#logger, this.#store.context);
    }

    /**
     * Creates a new team with the current user as the founding member
     * @param params - Parameters for team creation including name, description, and seed
     * @throws Error if:
     * - Team name is empty
     * - Store context is not initialized
     * - Called from server context
     * - User context is missing
     * @returns Promise<TeamAPI> - API instance for the newly created team
     */
    async createTeam(params: CreateTeamParams): Promise<TeamAPI> {
        // Validate team name
        if (!params.name || params.name.trim().length === 0) {
            throw new Error('Team name is required and cannot be empty');
        }

        // Verify store context and permissions
        if (!this.#store.context) {
            throw new Error('Store context is not initialized');
        }

        if (this.#store.isServer) {
            throw new Error('Servers cannot create teams');
        }

        if (!('user' in this.#store.context)) {
            throw new Error('User context required to create team');
        }

        // Generate encryption keys for team and admin access
        const teamKeys = createKeyset(TEAM_SCOPE, params.seed);
        const adminKeys = createKeyset(ADMIN_SCOPE, params.seed);

        // Create encrypted lockboxes for key distribution
        const lockboxTeamKeysForMember = createLockbox({
            contents: teamKeys,
            recipientKeys: this.#store.context.user.keys
        });
        const lockboxAdminKeysForMember = createLockbox({
            contents: adminKeys,
            recipientKeys: this.#store.context.user.keys
        });
        const lockboxUserKeysForDevice = createLockbox({
            contents: this.#store.context.user.keys,
            recipientKeys: this.#store.context.device.keys
        });

        // Store all lockboxes
        this.#store.upsert('lockbox', [lockboxTeamKeysForMember, lockboxAdminKeysForMember, lockboxUserKeysForDevice]);

        // Create and store team information
        const team: TeamInfo = {
            $id: randomId(),
            name: params.name,
            description: params.description,
        };
        this.#store.upsert('team', team);

        // Create and store root member (founding user)
        const rootMember = convertToPublicMember(this.#store.context.user, team.$id);
        this.#store.upsert('member', rootMember);

        // Create and store root device
        const rootDevice = convertToPublicDevice(this.#store.context.device, team.$id);
        this.#store.upsert('device', rootDevice);

        // Dispatch team creation action
        const action = new CreateTeamAction({
            teamId: team.$id,
            name: team.name,
            description: team.description,
            rootMember: rootMember,
            rootDevice: rootDevice,
            lockboxes: [
                lockboxTeamKeysForMember,
                lockboxAdminKeysForMember,
                lockboxUserKeysForDevice
            ],
        });
        await this.#store.dispatchAction(action);

        return new TeamAPI(team, this.#store, teamKeys, this.#logger, this.#store.context, params.seed);
    }
}
