import { field, option, serialize, variant, vec } from '@dao-xyz/borsh';
import { randomId } from '@topgunbuild/utils';
import { AbstractValue} from './values';
import { typeOf } from './typeof';
import { Lockbox } from '../models/lockbox';
import { Member } from '../models/member';
import { Device } from '../models/device';
import { Role } from '../models/role';
import { Server } from '../models/server';
import { Invitation } from '../models/invitation';
import { Keyset } from '../models/keyset';

export interface IActionHeader {
    actionId: string;
    userId: string;
    teamId: string;
    state: bigint;
    context?: Uint8Array;
}

export interface IAction {
    header: ActionHeader;
    lockboxes: Lockbox[];
}

export class ActionHeader implements IActionHeader
{
    @field({ type: 'string' })
    actionId: string;

    @field({ type: 'string' })
    userId: string;

    @field({ type: 'string' })
    teamId: string;

    @field({ type: 'u64' })
    state: bigint;

    @field({ type: option(Uint8Array) })
    context?: Uint8Array;

    constructor(data: {
        id: string,
        userId: string,
        teamId: string,
        state: bigint,
        context: Uint8Array,
    })
    {
        this.actionId = data.id || randomId();
        this.userId   = data.userId;
        this.teamId   = data.teamId;
        this.state    = data.state;
        this.context  = data.context;
    }
}

export class Action implements IAction
{
    @field({ type: ActionHeader })
    header: ActionHeader;

    @field({ type: vec(Lockbox) })
    lockboxes: Lockbox[];

    constructor(data: {
        lockboxes?: Lockbox[],
    })
    {
        this.lockboxes = Array.isArray(data.lockboxes) ? data.lockboxes : [];
    }

    encode(): Uint8Array
    {
        return serialize(this);
    }
}

@variant(0)
export class RootAction extends Action
{
    @field({ type: 'string' })
    name: string;

    @field({ type: Member })
    rootMember: Member;

    @field({ type: Device })
    rootDevice: Device;

    constructor(data: {
        lockboxes?: Lockbox[],
        name: string,
        rootMember: Member,
        rootDevice: Device
    })
    {
        super(data);
        this.name       = data.name;
        this.rootMember = data.rootMember;
        this.rootDevice = data.rootDevice;
    }
}

@variant(1)
export class AddMemberAction extends Action
{
    @field({ type: Member })
    member: Member;

    @field({ type: vec('string') })
    roles?: string[];

    constructor(data: {
        lockboxes?: Lockbox[],
        roles?: string[],
        member: Member,
    })
    {
        super(data);
        this.member = data.member;
        this.roles  = data.roles;
    }
}

@variant(2)
export class RemoveMemberAction extends Action
{
    @field({ type: 'string' })
    userId: string;

    constructor(data: { lockboxes?: Lockbox[], userId: string })
    {
        super(data);
        this.userId = data.userId;
    }
}

@variant(3)
export class AddRoleAction extends Action
{
    @field({ type: Role })
    role: Role;

    constructor(data: { lockboxes?: Lockbox[], role: Role })
    {
        super(data);
        this.role = data.role;
    }
}

@variant(4)
export class RemoveRoleAction extends Action
{
    @field({ type: 'string' })
    roleName: string;

    constructor(data: { lockboxes?: Lockbox[], roleName: string })
    {
        super(data);
        this.roleName = data.roleName;
    }
}

@variant(5)
export class AddMemberRoleAction extends Action
{
    @field({ type: 'string' })
    userId: string;

    @field({ type: Role })
    role: Role;

    constructor(data: { lockboxes?: Lockbox[], userId: string, role: Role })
    {
        super(data);
        this.userId = data.userId;
        this.role   = data.role;
    }
}

@variant(6)
export class RemoveMemberRoleAction extends Action
{
    @field({ type: 'string' })
    userId: string;

    @field({ type: 'string' })
    roleName: string;

    constructor(data: { lockboxes?: Lockbox[], userId: string, roleName: string })
    {
        super(data);
        this.userId   = data.userId;
        this.roleName = data.roleName;
    }
}

@variant(7)
export class AddDeviceAction extends Action
{
    @field({ type: Device })
    device: Device;

    constructor(data: { lockboxes?: Lockbox[], device: Device })
    {
        super(data);
        this.device = data.device;
    }
}

@variant(8)
export class RemoveDeviceAction extends Action
{
    @field({ type: 'string' })
    deviceId: string;

    constructor(data: { lockboxes?: Lockbox[], deviceId: string })
    {
        super(data);
        this.deviceId = data.deviceId;
    }
}

@variant(9)
export class InviteMemberAction extends Action
{
    @field({ type: Invitation })
    invitation: Invitation;

    constructor(data: { lockboxes?: Lockbox[], invitation: Invitation })
    {
        super(data);
        this.invitation = data.invitation;
    }
}

@variant(10)
export class InviteDeviceAction extends Action
{
    @field({ type: Invitation })
    invitation: Invitation;

    constructor(data: { lockboxes?: Lockbox[], invitation: Invitation })
    {
        super(data);
        this.invitation = data.invitation;
    }
}

@variant(11)
export class RevokeInvitationAction extends Action
{
    @field({ type: 'string' })
    id: string; // Invitation ID

    constructor(data: { lockboxes?: Lockbox[], id: string })
    {
        super(data);
        this.id = data.id;
    }
}

@variant(12)
export class AdmitMemberAction extends Action
{
    @field({ type: 'string' })
    id: string; // Invitation ID

    @field({ type: 'string' })
    userName: string;

    @field({ type: Keyset })
    memberKeys: Keyset;

    constructor(data: {
        lockboxes?: Lockbox[],
        id: string,
        userName: string,
        memberKeys: Keyset,
    })
    {
        super(data);
        this.id         = data.id;
        this.userName   = data.userName;
        this.memberKeys = data.memberKeys;
    }
}

@variant(13)
export class AdmitDeviceAction extends Action
{
    @field({ type: 'string' })
    id: string; // Invitation ID

    @field({ type: Device })
    device: Device;

    constructor(data: { lockboxes?: Lockbox[], id: string, device: Device })
    {
        super(data);
        this.id     = data.id;
        this.device = data.device;
    }
}

@variant(14)
export class ChangeMemberKeysAction extends Action
{
    @field({ type: Keyset })
    keys: Keyset;

    constructor(data: { lockboxes?: Lockbox[], keys: Keyset })
    {
        super(data);
        this.keys = data.keys;
    }
}

@variant(15)
export class RotateKeysAction extends Action
{
    @field({ type: 'string' })
    userId: string;

    constructor(data: { lockboxes?: Lockbox[], userId: string })
    {
        super(data);
        this.userId = data.userId;
    }
}

@variant(16)
export class AddServerAction extends Action
{
    @field({ type: Server })
    server: Server;

    constructor(data: { lockboxes?: Lockbox[], server: Server })
    {
        super(data);
        this.server = data.server;
    }
}

@variant(17)
export class RemoveServerAction extends Action
{
    @field({ type: 'string' })
    host: string;

    constructor(data: { lockboxes?: Lockbox[], host: string })
    {
        super(data);
        this.host = data.host;
    }
}

@variant(18)
export class ChangeServerKeysAction extends Action
{
    @field({ type: Keyset })
    keys: Keyset;

    constructor(data: { lockboxes?: Lockbox[], keys: Keyset })
    {
        super(data);
        this.keys = data.keys;
    }
}

@variant(19)
export class SetTeamNameAction extends Action
{
    @field({ type: 'string' })
    teamName: string;

    constructor(data: { lockboxes?: Lockbox[], teamName: string })
    {
        super(data);
        this.teamName = data.teamName;
    }
}

@variant(20)
export class PutMessageAction extends Action
{
    @field({ type: 'string' })
    channelId: string;

    @field({ type: 'string' })
    messageId: string;

    @field({ type: 'string' })
    fieldName: string;

    @field({ type: 'u8' })
    deleted: number;

    @field({ type: AbstractValue })
    value: AbstractValue;

    constructor(data: {
        channelId: string,
        messageId: string,
        fieldName: string,
        value: unknown,
        deleted?: number,
        lockboxes?: Lockbox[],
    })
    {
        super(data);
        this.channelId = data.channelId;
        this.messageId = data.messageId;
        this.fieldName = data.fieldName;
        this.value     = typeOf(data.value);
    }
}

@variant(21)
export class DeleteMessageAction extends Action
{
    @field({ type: 'string' })
    channelId: string;

    @field({ type: 'string' })
    messageId: string;

    @field({ type: option('string') })
    fieldName?: string;

    constructor(data: {
        channelId: string,
        messageId: string,
        fieldName: string,
    })
    {
        super({});
        this.channelId = data.channelId;
        this.messageId = data.messageId;
        this.fieldName = data.fieldName;
    }
}

@variant(22)
export class DataChangesAction extends Action
{
    @field({ type: 'string' })
    operation: 'insert' | 'update' | 'delete';

    @field({ type: AbstractValue })
    rowData: AbstractValue;

    @field({ type: option(AbstractValue) })
    oldData?: AbstractValue;

    constructor(data: {
        operation: 'insert' | 'update' | 'delete',
        rowData: AbstractValue,
        oldData?: AbstractValue,
    })
    {
        super({});
        this.operation = data.operation;
        this.rowData   = data.rowData;
        this.oldData  = data.oldData;
    }
}

export type TeamAction =
    |RootAction
    |AddMemberAction
    |AddDeviceAction
    |AddRoleAction
    |AddMemberRoleAction
    |RemoveMemberAction
    |RemoveDeviceAction
    |RemoveRoleAction
    |RemoveMemberRoleAction
    |InviteMemberAction
    |InviteDeviceAction
    |RevokeInvitationAction
    |AdmitMemberAction
    |AdmitDeviceAction
    |ChangeMemberKeysAction
    |RotateKeysAction
    |AddServerAction
    |RemoveServerAction
    |ChangeServerKeysAction
    |PutMessageAction
    |DeleteMessageAction
    |SetTeamNameAction
    |DataChangesAction;
