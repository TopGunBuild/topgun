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
    RemoveRoleFromMemberAction,
    Team,
    KeysetImpl,
    KeysetWithSecrets,
    AssignRoleToMemberAction,
    KeyType,
    KeyScope,
    LockboxEntry,
    KeyMetadata
} from "@topgunbuild/models";
import { LoggerService } from "@topgunbuild/logger";
import { EventEmitter } from "@topgunbuild/eventemitter";
import { StoreError } from "./errors";
import { ChannelAPI } from "./channel-api";
import { randomId } from "@topgunbuild/common";
import { createKeyset, createLockbox } from "@topgunbuild/model-utils";
import { randomKey } from "@topgunbuild/crypto";

export class TeamAPI extends EventEmitter {
    private readonly context: LocalUserContext;
    private readonly logger: LoggerService;
    #teamKeys: KeysetWithSecrets;
    #seed: string;

    constructor(
        private readonly team: Team,
        private readonly store: Store,
        teamKeys: KeysetWithSecrets,
        seed: string
    ) {
        super();
        this.#teamKeys = teamKeys;
        this.#seed = seed ?? randomKey();
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

    

    /** Returns the keys for the given role. */
    // public roleKeys = (roleName: string, generation?: number) =>
    //     this.keys({ type: KeyType.ROLE, name: roleName, generation })

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
    public async addRole(roleName: string, permissions?: PermissionsMap): Promise<void> {
        try {
            const role: Role<PermissionsMap> = {
                $id: randomId(),
                roleName,
                permissions
            };

            // We're creating this role so we need to generate new keys
            const roleKeys = createKeyset({ type: KeyType.ROLE, name: role.roleName }, this.#seed);

            // Lockbox the keys for the admins
            const lockboxRoleKeysForAdmins = createLockbox(roleKeys, this.#teamKeys);

            await this.store.upsert('role', role);

            const body = new AddRoleAction({
                role: new RoleImpl({
                    ...role,
                    permissions: Object.entries(role.permissions || {})
                        .filter(([_, value]) => value)
                        .map(([key]) => key),
                }),
                lockboxes: [lockboxRoleKeysForAdmins]
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
    public async assignRoleToMember(userId: string, roleName: string): Promise<void> {
        try {
            // Validate inputs
            if (!userId || !roleName) {
                throw new StoreError('User ID and role name are required', 'INVALID_INPUT');
            }

            // Get existing member or create new one with default roles array
            const member = await this.store.getById<Member>('member', userId);
            if (member) {
                member.roles = Array.isArray(member.roles) ? member.roles : [];
                if (!member.roles.includes(roleName)) {
                    member.roles.push(roleName);
                    await this.store.upsert('member', member);
                }
            }

            // Send request regardless of local changes to ensure server consistency
            await this.store.dispatchAction(new AssignRoleToMemberAction({ userId, roleName }));
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
    public async removeRoleFromMember(userId: string, roleName: string): Promise<void> {
        try {
            // Validate inputs
            if (!userId || !roleName) {
                throw new StoreError('User ID and role name are required', 'INVALID_INPUT');
            }

            // Get existing member
            const member = await this.store.getById<Member>('member', userId);
            if (member) {
                // Ensure roles is an array and remove role if present
                member.roles = Array.isArray(member.roles) ? member.roles : [];
                const roleIndex = member.roles.indexOf(roleName);
                if (roleIndex !== -1) {
                    member.roles.splice(roleIndex, 1);
                    await this.store.upsert('member', member);
                }
            }

            // Send request regardless of local changes to ensure server consistency
            await this.store.dispatchAction(new RemoveRoleFromMemberAction({ userId, roleName }));
        } catch (error) {
            console.error('Failed to remove member role:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to remove member role', 'REMOVE_MEMBER_ROLE_ERROR');
        }
    }

    /**
     * Retrieves secret keys available to the current device for the specified scope.
     * @param scope The scope of the keys to retrieve (TEAM, ROLE, USER, etc.)
     * @param options Additional options for key retrieval
     * @param options.name Name identifier (required for ROLE scope)
     * @param options.generation Optional key generation number
     * @returns The keyset containing the secret keys for the specified scope
     * @throws {StoreError} If keys cannot be retrieved or decrypted
     */
    public async getKeys(scope: KeyScope | KeyMetadata): Promise<KeysetWithSecrets> {
        try {
            const { type, name, generation = 0 } = scope as KeyMetadata;
            const keys = this.context.device.keys;

            this.store.subscribeQuery

            // Validate inputs based on scope
            if (scope.type === KeyType.ROLE && !name) {
                throw new StoreError('Name is required for ROLE scope', 'INVALID_INPUT');
            }

            // Get lockboxes from the store based on scope
            const lockboxQuery = {
                type: scope.type,
                ...(name && { name }),
                ...(generation && { generation })
            };
            
            const lockboxes = await this.store.query<LockboxEntry>('lockbox', lockboxQuery);
            
            if (!lockboxes || lockboxes.length === 0) {
                throw new StoreError(`No keys found for scope: ${scope}`, 'KEYS_NOT_FOUND');
            }

            // For team scope, return the team keys we already have
            if (scope.type === KeyType.TEAM) {
                return this.#teamKeys;
            }

            // Try to decrypt each lockbox until we find one we can open
            for (const lockbox of lockboxes) {
                try {
                    // Attempt to decrypt the lockbox using team keys
                    const decryptedKeys = lockbox.decrypt(this.#teamKeys);
                    if (decryptedKeys) {
                        return decryptedKeys;
                    }
                } catch (error) {
                    // Continue to next lockbox if decryption fails
                    this.logger.debug(`Failed to decrypt lockbox: ${error['message']}`);
                    continue;
                }
            }

            throw new StoreError(`Unable to decrypt keys for scope: ${scope}`, 'DECRYPTION_FAILED');
        } catch (error) {
            this.logger.error('Failed to get keys:', error);
            throw error instanceof StoreError 
                ? error 
                : new StoreError('Failed to get keys', 'GET_KEYS_ERROR');
        }
    }
}