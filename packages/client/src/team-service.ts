import { randomKey } from "@topgunbuild/crypto";
import { Store } from "./store";
import { ADMIN_SCOPE, LocalUserContext, TeamOptions } from "@topgunbuild/models";
import { castServer, convertToPublicDevice, convertToPublicMember, createKeyset, createLockbox, isNewTeam } from "@topgunbuild/model-utils";
import { ConsoleLogger, LoggerService } from "@topgunbuild/logger";
import { assert } from "@topgunbuild/common";
import { EventEmitter } from "@topgunbuild/eventemitter";
import { ChannelService } from "./channel-service";

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
            this.logger.log(`Creating new team ${options.name}`, this.context);

            // Team & role secrets are never stored in plaintext, only encrypted into individual
            // lockboxes. Here we generate new keysets for the team and for the admin role, and store
            // these in new lockboxes for the founding member
            const lockboxTeamKeysForMember = createLockbox({ contents: options.keys, recipientKeys: user.keys })
            const adminKeys = createKeyset(ADMIN_SCOPE, this.seed)
            const lockboxAdminKeysForMember = createLockbox({ contents: adminKeys, recipientKeys: user.keys })

            // We also store the founding user's keys in a lockbox for the user's device
            const lockboxUserKeysForDevice = createLockbox({ contents: user.keys, recipientKeys: this.context.device.keys })

            // We're creating a new graph; this information is to be recorded in the root link
            const rootPayload = {
                name: options.name,
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

    

    // public getChannel(channelId: string): ChannelService {
    //     return new ChannelService(channelId, this.store, this);
    // }

    // public getChannels(): ChannelService[] {
    //     return [];
    // }

    // public createChannel(name: string, description?: string): ChannelService {
    //     return new ChannelService(name, this.store, this);
    // }

    /**
     * Add messages to the store
     * @param channelId The channel ID
     * @param messages Array of messages to add
     * @throws {StoreError} If messages are invalid
     */
    public async addMessages<T extends StoreItem>(channelId: string, messages: T[]): Promise<void> {
        try {
            if (!Array.isArray(messages) || messages.length === 0) {
                throw new StoreError('Invalid messages array', 'INVALID_INPUT');
            }

            await this.upsert('message', messages);

            for (const message of messages) {
                const body = new PutMessageAction({
                    channelId,
                    messageId: message.$id,
                    value: JSON.stringify(message)
                });
                await this.sendRequest(body);
            }
        } catch (error) {
            console.error('Failed to add messages:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to add messages', 'ADD_MESSAGE_ERROR');
        }
    }

    /**
     * Add a member to the store
     * @param member Member to add
     * @throws {StoreError} If member is invalid
     */
    public async addMember(member: Member, roles?: string[]): Promise<void> {
        try {
            if (!member.$id) {
                throw new StoreError('Member must have an $id property', 'INVALID_INPUT');
            }

            await this.upsert('member', member);

            const body = new AddMemberAction({
                member: new MemberImpl({
                    ...member,
                    keys: new KeysetImpl({
                        teamId: this.teamId,
                        publicKey: this.userId
                    })
                }),
                roles
            });
            await this.sendRequest(body);
        } catch (error) {
            console.error('Failed to add member:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to add member', 'ADD_MEMBER_ERROR');
        }
    }

    /**
     * Remove a member from the store
     * @param userId ID of the member to remove
     * @throws {StoreError} If removal fails
     */
    public async removeMember(userId: string): Promise<void> {
        try {
            if (!userId) {
                throw new StoreError('User ID is required', 'INVALID_INPUT');
            }

            await this.delete('member', [userId]);

            const body = new RemoveMemberAction({
                userId
            });
            await this.sendRequest(body);
        } catch (error) {
            console.error('Failed to remove member:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to remove member', 'REMOVE_MEMBER_ERROR');
        }
    }

    /**
     * Add a role to the store
     * @param role Role to add
     * @throws {StoreError} If role is invalid
     */
    public async addRole(role: Role<PermissionsMap>): Promise<void> {
        try {
            if (!role.$id) {
                throw new StoreError('Role must have an $id property', 'INVALID_INPUT');
            }

            await this.upsert('role', role);

            const body = new AddRoleAction({
                role: new RoleImpl({
                    ...role,
                    permissions: Object.entries(role.permissions || {})
                        .filter(([_, value]) => value)
                        .map(([key]) => key)
                })
            });
            await this.sendRequest(body);
        } catch (error) {
            console.error('Failed to add role:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to add role', 'ADD_ROLE_ERROR');
        }
    }

    /**
     * Remove a role from the store
     * @param roleName Name of the role to remove
     * @throws {StoreError} If removal fails
     */
    public async removeRole(roleName: string): Promise<void> {
        try {
            if (!roleName) {
                throw new StoreError('Role name is required', 'INVALID_INPUT');
            }

            await this.delete('role', [roleName]);

            const body = new RemoveRoleAction({ roleName });
            await this.sendRequest(body);
        } catch (error) {
            console.error('Failed to remove role:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to remove role', 'REMOVE_ROLE_ERROR');
        }
    }

    /**
     * Add a member with a role to the store
     * @param userId ID of the member
     * @param roleName Name of the role to assign to the member
     * @throws {StoreError} If member or role is invalid
     */
    public async addMemberRole(userId: string, roleName: string): Promise<void> {
        try {
            // Validate inputs
            if (!userId || !roleName) {
                throw new StoreError('User ID and role name are required', 'INVALID_INPUT');
            }

            // Get existing member or create new one with default roles array
            const member = await this.storageManager.get<Member>('member', userId) || {
                $id: userId,
                roles: []
            };

            // Ensure roles is an array and add new role if not present
            member.roles = Array.isArray(member.roles) ? member.roles : [];
            if (!member.roles.includes(roleName)) {
                member.roles.push(roleName);
                await this.upsert('member', member);
            }

            // Send request regardless of local changes to ensure server consistency
            await this.sendRequest(new AddMemberRoleAction({ userId, roleName }));
        } catch (error) {
            console.error('Failed to add member role:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to add member role', 'ADD_MEMBER_ROLE_ERROR');
        }
    }

    /**
     * Remove a member role from the store
     * @param userId ID of the member
     * @param roleName Name of the role to remove
     * @throws {StoreError} If member or role is invalid
     */
    public async removeMemberRole(userId: string, roleName: string): Promise<void> {
        try {
            // Validate inputs
            if (!userId || !roleName) {
                throw new StoreError('User ID and role name are required', 'INVALID_INPUT');
            }

            // Get existing member
            const member = await this.storageManager.get<Member>('member', userId);
            if (member) {
                // Ensure roles is an array and remove role if present
                member.roles = Array.isArray(member.roles) ? member.roles : [];
                const roleIndex = member.roles.indexOf(roleName);
                if (roleIndex !== -1) {
                    member.roles.splice(roleIndex, 1);
                    await this.upsert('member', member);
                }
            }

            // Send request regardless of local changes to ensure server consistency
            await this.sendRequest(new RemoveMemberRoleAction({ userId, roleName }));
        } catch (error) {
            console.error('Failed to remove member role:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to remove member role', 'REMOVE_MEMBER_ROLE_ERROR');
        }
    }
}