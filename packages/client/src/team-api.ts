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
    KeyMetadata,
    SelectAction,
    Lockbox,
    RotateKeysAction,
    ADMIN
} from "@topgunbuild/models";
import { LoggerService } from "@topgunbuild/logger";
import { EventEmitter } from "@topgunbuild/eventemitter";
import { StoreError } from "./errors";
import { ChannelAPI } from "./channel-api";
import { randomId, uniqBy } from "@topgunbuild/common";
import { convertToPublicKeyset, createKeyset, createLockbox, decryptLockbox, getLockboxLatestGeneration, isCompleteKeyset, rotateLockbox, scopesMatch } from "@topgunbuild/model-utils";
import { randomKey } from "@topgunbuild/crypto";
import { whereNumber, whereString } from "./query-conditions";

export class TeamAPI extends EventEmitter {
    private readonly context: LocalUserContext;
    private readonly logger: LoggerService;
    // #teamKeys: KeysetWithSecrets;
    #seed: string;

    constructor(
        private readonly team: Team,
        private readonly store: Store,
        teamKeys: KeysetWithSecrets,
        seed: string
    ) {
        super();
        // this.#teamKeys = teamKeys;
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

    /**
     * Checks if a member exists in the store
     * @param userId The ID of the member to check
     * @returns True if the member exists, false otherwise
     */
    public async hasMember(userId: string): Promise<boolean> {
        return !!(await this.store.queryOne('member', userId));
    }

    /**
     * Retrieves a member from the store
     * @param userId The ID of the member to retrieve
     * @returns The member object if found, null otherwise
     */
    public async getMember(userId: string): Promise<Member | null> {
        return await this.store.queryOne('member', userId);
    }

    /**
     * Checks if a member has a specific role
     * @param userId The ID of the member to check
     * @param roleName The name of the role to check for
     * @returns True if the member has the role, false otherwise
     */
    public async memberHasRole(userId: string, roleName: string): Promise<boolean> {
        const member = await this.getMember(userId);
        return member?.roles?.includes(roleName) ?? false;
    }

    /**
     * Checks if a member is an admin
     * @param userId The ID of the member to check
     * @returns True if the member is an admin, false otherwise
     */
    public async memberIsAdmin(userId: string): Promise<boolean> {
        return this.memberHasRole(userId, ADMIN);
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
            const member = await this.store.queryOne<Member>('member', userId);
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
            const member = await this.store.queryOne<Member>('member', userId);
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

    public async changeKeys(newKeys: KeysetWithSecrets) {
        const { device, user } = this.context
        const { type } = newKeys;

        if (type === KeyType.DEVICE) {
            throw new StoreError('Device keys cannot be changed', 'INVALID_INPUT');
        }
        const isForUser = type === KeyType.USER;
        const isForServer = type === KeyType.SERVER;

        const oldKeys: KeysetWithSecrets = user.keys;
        newKeys.generation = oldKeys.generation + 1;
        // TODO: Rotate keys for all members
    }

    /**
     * Retrieves the admin role keyset
     * This is a convenience method that returns the keys for the 'ADMIN' role
     * @param generation Optional generation number for the keys
     * @returns The keyset containing the admin secret keys
     * @throws {StoreError} If admin keys cannot be retrieved or decrypted
     */
    public async getAdminKeys(generation?: number): Promise<KeysetWithSecrets> {
        try {
            return this.getRoleKeys('ADMIN', generation);
        } catch (error) {
            console.error('Failed to get admin keys:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to get admin keys', 'GET_ADMIN_KEYS_ERROR');
        }
    }

    /**
     * Retrieves the team keys for the current team or a specific generation
     * @param generation Optional generation number for the keys
     * @returns The keyset containing the team secret keys
     * @throws {StoreError} If:
     *         - Team keys cannot be retrieved
     *         - Keys cannot be decrypted
     */
    public async getTeamKeys(generation?: number): Promise<KeysetWithSecrets> {
        try {
            return this.getKeys({
                type: KeyType.TEAM,
                name: this.team.$id,
                ...(generation !== undefined && { generation })
            });
        } catch (error) {
            console.error('Failed to get team keys:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to get team keys', 'GET_TEAM_KEYS_ERROR');
        }
    }
    
    /**
     * Retrieves secret keys for a specific role
     * @param roleName The name of the role to get keys for
     * @param generation Optional generation number for the keys
     * @returns The keyset containing the secret keys for the role
     * @throws {StoreError} If:
     *         - Role name is invalid/missing
     *         - Keys cannot be retrieved
     *         - Keys cannot be decrypted
     */
    public async getRoleKeys(roleName: string, generation?: number): Promise<KeysetWithSecrets> {
        try {
            if (!roleName) {
                throw new StoreError('Role name is required', 'INVALID_INPUT');
            }

            return this.getKeys({
                type: KeyType.ROLE,
                name: roleName,
                ...(generation !== undefined && { generation })
            });
        } catch (error) {
            console.error('Failed to get role keys:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to get role keys', 'GET_ROLE_KEYS_ERROR');
        }
    }

    /**
     * Retrieves the secret keys with the highest generation value for the specified scope.
     * @param scope The scope of the keys to retrieve:
     *             - KeyScope for basic scope types
     *             - KeyMetadata for detailed scope with name and generation
     * @returns The keyset containing the secret keys with the highest generation value
     * @throws {StoreError} If:
     *         - Input validation fails
     *         - Keys cannot be retrieved
     *         - Keys cannot be decrypted
     */
    public async getKeys(scope: KeyScope | KeyMetadata): Promise<KeysetWithSecrets> {
        try {
            // Validate scope type and extract metadata
            if (!scope || typeof scope !== 'object') {
                throw new StoreError('Invalid scope parameter', 'INVALID_INPUT');
            }

            const { type, name, generation } = 'type' in scope 
                ? scope as KeyMetadata 
                : { type: scope as KeyScope, name: undefined, generation: undefined };

            // Validate required fields
            if (!type || !(String(type) in KeyType)) {
                throw new StoreError('Invalid key type', 'INVALID_INPUT');
            }

            if (type === KeyType.ROLE && !name) {
                throw new StoreError('Name is required for ROLE scope', 'INVALID_INPUT');
            }

            // Ensure device context is available
            if (!this.context?.device?.keys?.encryption?.publicKey) {
                throw new StoreError('Device encryption keys not available', 'INVALID_STATE');
            }

            // Get lockboxes from the store based on scope
            const lockboxQuery = new SelectAction({
                entity: 'lockbox',
                query: [
                    whereString('type', '=', String(type)),
                    whereString('recipientPublicKey', '=', this.context.device.keys.encryption.publicKey),
                    ...(name ? [whereString('name', '=', name)] : []),
                    ...(generation !== undefined ? [whereNumber('generation', '=', generation)] : [])
                ]
            });
            
            const lockboxResult = await this.store.query<Lockbox>(lockboxQuery);
            
            if (!lockboxResult.rows || lockboxResult.rows.length === 0) {
                throw new StoreError(`No keys found for scope: ${JSON.stringify(scope)}`, 'KEYS_NOT_FOUND');
            }

            const highestGenerationLockbox = getLockboxLatestGeneration(lockboxResult.rows);
            if (!highestGenerationLockbox) {
                throw new StoreError(`No valid lockbox found for scope: ${JSON.stringify(scope)}`, 'KEYS_NOT_FOUND');
            }

            return decryptLockbox(highestGenerationLockbox, this.context.device.keys);
        } catch (error) {
            this.logger.error('Failed to get keys:', error);
            throw error instanceof StoreError 
                ? error 
                : new StoreError('Failed to get keys', 'GET_KEYS_ERROR');
        }
    }

    /**
     * Creates lockboxes for a member's keys
     * @param member The member to create lockboxes for
     * @returns The lockboxes created for the member
     * @throws {StoreError} If the keyset is invalid or the rotation fails
     */
    private async createMemberLockboxes(member: Member): Promise<Lockbox[]> {
        try {
            // Input validation
            if (!member) {
                throw new StoreError('Member is required', 'INVALID_INPUT');
            }

            // Ensure roles is an array
            const roles = Array.isArray(member.roles) ? member.roles : [];
            
            // Validate device keys
            if (!member?.keys) {
                throw new StoreError('Member keys not available', 'INVALID_STATE');
            }

            // Get all required keys
            const roleKeys = await Promise.all(
                roles.map(role => 
                    this.getRoleKeys(role).catch(error => {
                        this.logger.error(`Failed to get keys for role ${role}:`, error);
                        throw new StoreError(`Failed to get keys for role ${role}`, 'GET_KEYS_ERROR');
                    })
                )
            );
            const teamKeys = await this.getTeamKeys();

            // Create lockboxes
            return [...roleKeys, teamKeys].map(keyset => createLockbox({
                contents: keyset,
                recipientKeys: member.keys
            }));
        } catch (error) {
            this.logger.error('Failed to create member lockboxes:', error);
            throw error instanceof StoreError 
                ? error 
                : new StoreError('Failed to create member lockboxes', 'CREATE_LOCKBOX_ERROR');
        }
    }

    /**
     * Rotates the keys for the given scope or keyset
     * @param keys The scope or keyset to rotate
     * @returns The new lockboxes created by the rotation
     * @throws {StoreError} If the keyset is invalid or the rotation fails
     */
    private async rotateKeys(keys: KeyScope | KeysetWithSecrets): Promise<Lockbox[]> {
        try {
            // Create or use provided keyset
            const newKeyset = isCompleteKeyset(keys) ? keys : createKeyset(keys, this.#seed);
            const { type, name } = newKeyset;

            if (!type || !name) {
                throw new StoreError('Invalid keyset: type and name are required', 'INVALID_INPUT');
            }

            // Find all lockboxes for this type/name
            const lockboxes = await this.store.query<Lockbox>(new SelectAction({
                entity: 'lockbox',
                query: [
                    whereString('type', '=', String(type)),
                    whereString('name', '=', name)
                ]
            }));

            if (!lockboxes?.rows?.length) {
                throw new StoreError(`No lockboxes found for type=${type}, name=${name}`, 'NOT_FOUND');
            }

            // Get unique scopes from existing lockboxes
            const visibleScopes: KeyScope[] = uniqBy(
                lockboxes.rows.map(lockbox => ({
                    type: lockbox.recipientType,
                    name: lockbox.recipientName,
                })),
                scope => `${scope.type}:${scope.name}`
            );

            // Create new keysets for all affected scopes
            const newKeysets = [newKeyset, ...visibleScopes.map(scope => createKeyset(scope, this.#seed))];

            // Create new lockboxes for each keyset
            const newLockboxes = await Promise.all(
                newKeysets.map(async keyset => {
                    const keysetLockboxes = await this.store.query<Lockbox>(new SelectAction({
                        entity: 'lockbox',
                        query: [
                            whereString('contentsType', '=', String(keyset.type)),
                            whereString('contentsName', '=', keyset.name)
                        ]
                    }));

                    const oldLockbox = getLockboxLatestGeneration(keysetLockboxes.rows);
                    if (!oldLockbox) {
                        this.logger.warn(`No existing lockbox found for type=${keyset.type}, name=${keyset.name}`);
                        return null;
                    }

                    const updatedKeyset = newKeysets.find(k => scopesMatch(k, {
                        type: oldLockbox.recipientType,
                        name: oldLockbox.recipientName
                    }));

                    return rotateLockbox({
                        oldLockbox,
                        newContents: keyset,
                        updatedRecipientKeys: updatedKeyset ? convertToPublicKeyset(updatedKeyset) : undefined
                    });
                })
            );

            // Filter out null values and return valid lockboxes
            return newLockboxes.filter((lockbox): lockbox is Lockbox => lockbox !== null);
        } catch (error) {
            this.logger.error('Failed to rotate keys:', error);
            throw error instanceof StoreError 
                ? error 
                : new StoreError('Failed to rotate keys', 'ROTATE_KEYS_ERROR');
        }
    }
}