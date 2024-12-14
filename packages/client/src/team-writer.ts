import { Store } from "./store";
import { LoggerService } from "@topgunbuild/logger";
import { 
    Member,
    AddMemberAction,
    RemoveMemberAction,
    Role,
    AddRoleAction,
    RemoveRoleAction,
    Team,
    KeysetWithSecrets,
    UpdateTeamAction,
    PermissionsMap,
    AssignRoleToMemberAction,
    RemoveRoleFromMemberAction,
    RotateKeysAction,
    SelectAction,
    Lockbox,
    KeyScope
} from "@topgunbuild/models";
import { StoreError } from "./errors";
import { convertToPublicKeyset, createKeyset, createLockbox, getLockboxLatestGeneration, isCompleteKeyset, rotateLockbox, scopesMatch } from "@topgunbuild/model-utils";
import { randomId, uniqBy } from "@topgunbuild/common";
import { TeamReader } from "./team-reader";
import { whereString } from "./query-conditions";

export class TeamWriter {
    constructor(
        private readonly team: Team,
        private readonly store: Store,
        private readonly logger: LoggerService,
        private readonly teamKeys: KeysetWithSecrets,
        private readonly reader: TeamReader
    ) {}

    /**
     * Update team details
     */
    public async updateTeam(params: { name?: string, description?: string }): Promise<void> {
        await this.store.upsert('team', {
            ...this.team,
            ...params
        });
        const body = new UpdateTeamAction({
            teamId: this.team.$id,
            ...params
        });
        await this.store.dispatchAction(body);
    }

    /**
     * Add a member
     */
    public async addMember(member: Member, roles?: string[]): Promise<void> {
        try {
            if (!member.$id) {
                throw new StoreError('Member must have an $id property', 'INVALID_INPUT');
            }

            await this.store.upsert('member', member);
            const body = new AddMemberAction({ member, roles });
            await this.store.dispatchAction(body);
        } catch (error) {
            this.logger.error('Failed to add member:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to add member', 'ADD_MEMBER_ERROR');
        }
    }

    /**
     * Add a role
     */
    public async addRole(roleName: string, permissions?: PermissionsMap): Promise<void> {
        try {
            const role: Role = {
                $id: randomId(),
                roleName,
                permissions
            };

            const roleKeys = createKeyset({ type: KeyType.ROLE, name: role.roleName });
            const lockboxRoleKeysForAdmins = createLockbox(roleKeys, this.teamKeys);

            await this.store.upsert('role', role);
            await this.store.dispatchAction(new AddRoleAction({
                role,
                lockboxes: [lockboxRoleKeysForAdmins]
            }));
        } catch (error) {
            this.logger.error('Failed to add role:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to add role', 'ADD_ROLE_ERROR');
        }
    }

    /**
     * Remove a member
     */
    public async removeMember(userId: string): Promise<void> {
        try {
            if (!userId) {
                throw new StoreError('User ID is required', 'INVALID_INPUT');
            }

            await this.store.delete('member', [userId]);
            await this.store.dispatchAction(new RemoveMemberAction({ userId }));
        } catch (error) {
            this.logger.error('Failed to remove member:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to remove member', 'REMOVE_MEMBER_ERROR');
        }
    }

    /**
     * Remove a role
     */
    public async removeRole(roleName: string): Promise<void> {
        try {
            if (!roleName) {
                throw new StoreError('Role name is required', 'INVALID_INPUT');
            }

            await this.store.delete('role', [roleName]);
            await this.store.dispatchAction(new RemoveRoleAction({ roleName }));
        } catch (error) {
            this.logger.error('Failed to remove role:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to remove role', 'REMOVE_ROLE_ERROR');
        }
    }

    /**
     * Assign role to member
     */
    public async assignRoleToMember(userId: string, roleName: string): Promise<void> {
        try {
            if (!userId || !roleName) {
                throw new StoreError('User ID and role name are required', 'INVALID_INPUT');
            }

            const member = await this.store.queryOne<Member>('member', userId);
            if (member) {
                member.roles = Array.isArray(member.roles) ? member.roles : [];
                if (!member.roles.includes(roleName)) {
                    member.roles.push(roleName);
                    await this.store.upsert('member', member);
                }
            }

            await this.store.dispatchAction(new AssignRoleToMemberAction({ userId, roleName }));
        } catch (error) {
            this.logger.error('Failed to add member role:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to add member role', 'ADD_MEMBER_ROLE_ERROR');
        }
    }

    /**
     * Remove role from member
     */
    public async removeRoleFromMember(userId: string, roleName: string): Promise<void> {
        try {
            if (!userId || !roleName) {
                throw new StoreError('User ID and role name are required', 'INVALID_INPUT');
            }

            const member = await this.store.queryOne<Member>('member', userId);
            if (member) {
                member.roles = Array.isArray(member.roles) ? member.roles : [];
                const roleIndex = member.roles.indexOf(roleName);
                if (roleIndex !== -1) {
                    member.roles.splice(roleIndex, 1);
                    await this.store.upsert('member', member);
                }
            }

            await this.store.dispatchAction(new RemoveRoleFromMemberAction({ userId, roleName }));
        } catch (error) {
            this.logger.error('Failed to remove member role:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to remove member role', 'REMOVE_MEMBER_ROLE_ERROR');
        }
    }

    /**
     * Change team keys
     */
    public async changeKeys(newKeys: KeysetWithSecrets): Promise<void> {
        try {
            const { type } = newKeys;

            if (type === KeyType.DEVICE) {
                throw new StoreError('Device keys cannot be changed', 'INVALID_INPUT');
            }

            const oldKeys = await this.reader.getTeamKeys();
            newKeys.generation = oldKeys.generation + 1;

            // Create new lockboxes for all members
            const lockboxes = await this.createMemberLockboxes(newKeys);
            
            await this.store.dispatchAction(new RotateKeysAction({
                scope: { type, name: this.team.$id },
                lockboxes
            }));
        } catch (error) {
            this.logger.error('Failed to change keys:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to change keys', 'CHANGE_KEYS_ERROR');
        }
    }

    /**
     * Create lockboxes for member keys
     */
    private async createMemberLockboxes(newKeys: KeysetWithSecrets): Promise<Lockbox[]> {
        try {
            const members = await this.store.query<Member>(new SelectAction({ entity: 'member' }));
            
            return Promise.all(members.rows.map(async member => {
                if (!member.keys) {
                    throw new StoreError(`Member ${member.$id} has no keys`, 'INVALID_STATE');
                }
                
                return createLockbox({
                    contents: newKeys,
                    recipientKeys: member.keys
                });
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