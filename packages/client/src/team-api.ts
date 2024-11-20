import { Store } from "./store";
import { 
    LocalUserContext,
    MemberImpl,
    Member,
    AddMemberAction,
    PermissionsMap,
    RemoveMemberAction,
    Role,
    AddRoleAction,
    RoleImpl,
    RemoveRoleAction,
    RemoveMemberRoleAction,
    Team,
    KeysetImpl,
    KeysetWithSecrets
} from "@topgunbuild/models";
import { LoggerService } from "@topgunbuild/logger";
import { EventEmitter } from "@topgunbuild/eventemitter";
import { StoreError } from "./errors";
import { ChannelAPI } from "./channel-api";

export class TeamAPI extends EventEmitter {
    private readonly context: LocalUserContext;
    private readonly logger: LoggerService;
    #teamKeys: KeysetWithSecrets;

    constructor(
        private readonly team: Team,
        private readonly store: Store,
        teamKeys: KeysetWithSecrets,
    ) {
        super();
        this.#teamKeys = teamKeys;
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

    public channel(channelId: string): ChannelAPI {
        return new ChannelAPI(channelId, this.store, this.logger);
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

            await this.store.upsert('member', member);

            const body = new AddMemberAction({
                member: new MemberImpl({
                    ...member,
                    keys: new KeysetImpl({
                        teamId: this.team.$id,
                        publicKey: this.userId
                    })
                }),
                roles
            });
            await this.store.dispatchAction(body);
        } catch (error) {
            this.logger.error('Failed to add member:', error);
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

            await this.store.delete('member', [userId]);

            const body = new RemoveMemberAction({
                userId
            });
            await this.store.dispatchAction(body);
        } catch (error) {
            this.logger.error('Failed to remove member:', error);
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

            await this.store.upsert('role', role);

            const body = new AddRoleAction({
                role: new RoleImpl({
                    ...role,
                    permissions: Object.entries(role.permissions || {})
                        .filter(([_, value]) => value)
                        .map(([key]) => key)
                })
            });
            await this.store.dispatchAction(body);
        } catch (error) {
            this.logger.error('Failed to add role:', error);
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

            await this.store.delete('role', [roleName]);

            const body = new RemoveRoleAction({ roleName });
            await this.store.dispatchAction(body);
        } catch (error) {
            this.logger.error('Failed to remove role:', error);
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