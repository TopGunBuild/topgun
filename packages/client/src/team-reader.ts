import { Store } from "./store";
import { LoggerService } from "@topgunbuild/logger";
import { 
    Member,
    TeamInfo,
    KeyType,
    SelectAction,
    Lockbox,
    LocalUserContext,
    KeysetPrivateInfo,
    KeyScopeInfo,
    KeyBaseInfo
} from "@topgunbuild/models";
import { decryptLockbox, getLockboxLatestGeneration } from "@topgunbuild/model-utils";
import { StoreError } from "./errors";
import { whereString, whereNumber } from "./query-conditions";

export class TeamReader {
    constructor(
        private readonly team: TeamInfo,
        private readonly store: Store,
        private readonly logger: LoggerService,
        private readonly context: LocalUserContext
    ) {}

    /**
     * The team's ID
     */
    public get id(): string {
        return this.team.$id;
    }

    /**
     * The team's name
     */
    public get name(): string {
        return this.team.name;
    }

    /**
     * Check if a member exists
     */
    public async hasMember(userId: string): Promise<boolean> {
        return !!(await this.store.queryOne('member', userId));
    }

    /**
     * Get a member
     */
    public async getMember(userId: string): Promise<Member | null> {
        return await this.store.queryOne('member', userId);
    }

    /**
     * Check if member has role
     */
    public async memberHasRole(userId: string, roleName: string): Promise<boolean> {
        const member = await this.getMember(userId);
        return member?.roles?.includes(roleName) ?? false;
    }

    /**
     * Check if member is admin
     */
    public async memberIsAdmin(userId: string): Promise<boolean> {
        return this.memberHasRole(userId, 'ADMIN');
    }

    /**
     * Check if role exists
     */
    public async hasRole(roleName: string): Promise<boolean> {
        return !!(await this.store.queryOne('role', roleName));
    }

    /**
     * Get team keys
     */
    public async getTeamKeys(generation?: number): Promise<KeysetPrivateInfo> {
        try {
            return this.getKeys({
                type: KeyType.TEAM,
                name: this.team.$id,
                ...(generation !== undefined && { generation })
            });
        } catch (error) {
            this.logger.error('Failed to get team keys:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to get team keys', 'GET_TEAM_KEYS_ERROR');
        }
    }

    /**
     * Get role keys
     */
    public async getRoleKeys(roleName: string, generation?: number): Promise<KeysetPrivateInfo> {
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
            this.logger.error('Failed to get role keys:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to get role keys', 'GET_ROLE_KEYS_ERROR');
        }
    }

    /**
     * Get admin keys
     */
    public async getAdminKeys(generation?: number): Promise<KeysetPrivateInfo> {
        try {
            return this.getRoleKeys('ADMIN', generation);
        } catch (error) {
            this.logger.error('Failed to get admin keys:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to get admin keys', 'GET_ADMIN_KEYS_ERROR');
        }
    }

    /**
     * Get keys for a specific scope
     */
    private async getKeys(scope: KeyScopeInfo | KeyBaseInfo): Promise<KeysetPrivateInfo> {
        try {
            if (!scope || typeof scope !== 'object') {
                throw new StoreError('Invalid scope parameter', 'INVALID_INPUT');
            }

            const { type, name, generation } = 'type' in scope 
                ? scope as KeyBaseInfo 
                : { type: scope as KeyScopeInfo, name: undefined, generation: undefined };

            if (!type) {
                throw new StoreError('Invalid key type', 'INVALID_INPUT');
            }

            if (type === KeyType.ROLE && !name) {
                throw new StoreError('Name is required for ROLE scope', 'INVALID_INPUT');
            }

            const lockboxQuery = new SelectAction({
                entity: 'lockbox',
                query: [
                    whereString('type', '=', String(type)),
                    ...(name ? [whereString('name', '=', name)] : []),
                    ...(generation !== undefined ? [whereNumber('generation', '=', generation)] : [])
                ]
            });
            
            const lockboxResult = await this.store.query<Lockbox>(lockboxQuery);
            
            if (!lockboxResult.rows?.length) {
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

    // ... other read methods from TeamAPI
} 