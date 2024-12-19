import { 
    CreateTeamAction, 
    DeviceImpl, 
    DeviceWithSecrets, 
    UserWithSecrets,
    KeysetImpl, 
    Team, 
    TEAM_SCOPE,
    ADMIN_SCOPE,
    KeysetWithSecrets
} from "@topgunbuild/models";
import { 
    castServer, 
    convertToPublicMember, 
    convertToPublicDevice, 
    createKeyset, 
    createLockbox 
} from "@topgunbuild/model-utils";
import { randomId } from "@topgunbuild/common";
import { Store } from "../store";
import { LoggerService } from "@topgunbuild/logger";
import { TeamAPI } from "../team-api";

export interface CreateTeamParams {
    name: string;
    description?: string;
    seed?: string;
}

export class CreateTeamCommand {
    constructor(
        private readonly store: Store,
        private readonly logger: LoggerService
    ) {}

    async execute(params: CreateTeamParams): Promise<TeamAPI> {
        this.validateParams(params);
        const { user, device } = this.ensureUserContext();

        // Create the team base object
        const team: Team = {
            $id: randomId(),
            name: params.name,
            description: params.description,
        };

        this.logger.log(`Creating new team "${team.name}" with ID ${team.$id}`);

        // Create security assets and action
        const securityAssets = this.createSecurityAssets(params.seed, user.keys);
        const action = this.createTeamAction(team, user, device, securityAssets);

        try {
            await this.store.dispatchAction(action);
            return new TeamAPI(
                team, 
                this.store, 
                securityAssets.teamKeys, 
                this.logger,
                this.store.context
            );
        } catch (error) {
            this.logger.error('Failed to create team:', error);
            throw new Error(`Failed to create team: ${error['message']}`);
        }
    }

    private validateParams(params: CreateTeamParams): void {
        if (!params.name || params.name.trim().length === 0) {
            throw new Error('Team name is required and cannot be empty');
        }
        if (params.description && params.description.trim().length === 0) {
            throw new Error('Team description cannot be empty if provided');
        }
    }

    private ensureUserContext(): { user: UserWithSecrets; device: DeviceWithSecrets } {
        if (this.store.isServer) {
            throw new Error('Servers cannot create teams');
        }

        if ('user' in this.store.context) {
            return {
                user: this.store.context.user,
                device: this.store.context.device
            };
        }

        const { server } = this.store.context;
        if (!server) {
            throw new Error('Neither user nor server found in context');
        }

        const context = {
            device: castServer.toDevice(server),
            user: castServer.toUser(server)
        };

        this.store.context = {
            ...this.store.context,
            ...context
        };

        return context;
    }

    private createSecurityAssets(seed: string | undefined, userKeys: KeysetWithSecrets) {
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

    private createTeamAction(
        team: Team, 
        user: UserWithSecrets, 
        device: DeviceWithSecrets,
        securityAssets: ReturnType<typeof this.createSecurityAssets>
    ): CreateTeamAction {
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

        return new CreateTeamAction({
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
    }
} 