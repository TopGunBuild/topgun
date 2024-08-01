import { ClientService } from '../client-service';
import { SelectBuilder } from '../query-builders';
import { SectionQueryHandler } from '../query-handlers';
import { mergeObjects, randomId, toArray } from '@topgunbuild/utils';
import { DataNode } from '@topgunbuild/store';
import { SelectQuery, SelectSectionOptions } from '@topgunbuild/transport';

export class Room
{
    readonly #roomSid: string;
    readonly #service: ClientService;

    constructor(roomSid: string, service: ClientService)
    {
        this.#roomSid = roomSid;
        this.#service = service;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Messages
    // -----------------------------------------------------------------------------------------------------

    messages(options?: SelectSectionOptions): SelectBuilder<DataNode[], SelectSectionOptions>
    {
        if (options?.limit > this.#service.options.rowLimit)
        {
            throw new Error(`Limit for rows (controlled by 'rowLimit' setting) exceeded, max rows: ${this.#service.options.rowLimit}`);
        }

        return new SelectBuilder<DataNode[], SelectSectionOptions>(
            new SectionQueryHandler({
                service: this.#service,
                query  : new SelectQuery(options),
                options: mergeObjects<SelectSectionOptions>({
                    limit : this.#service.options.rowLimit,
                    local : true,
                    remote: true,
                    sync  : false,
                }, options),
            }),
        );
    }

    async addMessages(values: DataNode): Promise<void>
    async addMessages(values: DataNode[]): Promise<void>
    async addMessages(values: DataNode[]|DataNode): Promise<void>
    {
        // TODO: Add values validation
        await Promise.all(
            toArray(values).map(value =>
                this.#service.putNode(
                    this.#roomSid, randomId(), value,
                ),
            ),
        );
    }

    message(sid: string)
    {
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Crypto
    // -----------------------------------------------------------------------------------------------------

    encrypt(payload: DataNode, roleName?: string)
    {
    }

    decrypt(payload: any)
    {
    }

    sign(contents: any)
    {
    }

    verify(message)
    {
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Keys
    // -----------------------------------------------------------------------------------------------------

    keys(scope: any)
    {
    }

    roleKeys(roleName: string, generation?: number)
    {
    }

    teamKeys(generation?: number)
    {
    }

    teamKeyring()
    {
    }

    adminKeys(generation?: number)
    {
    }

    changeKeys()
    {
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Members
    // -----------------------------------------------------------------------------------------------------

    member()
    {
    }

    members()
    {
    }

    removeMember()
    {
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Roles
    // -----------------------------------------------------------------------------------------------------

    roles()
    {
    }

    memberHasRole()
    {
    }

    memberIsAdmin()
    {
    }

    hasRole()
    {
    }

    membersInRole(roleName: string)
    {
    }

    admins()
    {
    }

    addRole(role)
    {
    }

    removeRole(roleName: string)
    {
    }

    addMemberRole(userId: string, roleName: string)
    {
    }

    removeMemberRole(userId: string, roleName: string)
    {
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Devices
    // -----------------------------------------------------------------------------------------------------

    hasDevice(deviceId: string)
    {
    }

    device(deviceId: string)
    {
    }

    removeDevice(deviceId: string)
    {
    }

    deviceWasRemoved(deviceId: string)
    {
    }

    memberByDeviceId(deviceId: string)
    {
    }

    verifyIdentityProof()
    {
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Invitations
    // -----------------------------------------------------------------------------------------------------

    inviteMember()
    {
    }

    inviteDevice()
    {
    }

    hasInvitation()
    {
    }

    getInvitation()
    {
    }

    validateInvitation()
    {
    }

    admitMember()
    {
    }

    join()
    {
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Servers
    // -----------------------------------------------------------------------------------------------------
}
