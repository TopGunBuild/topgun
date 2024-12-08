import { field, option, variant, vec, serialize } from "@dao-xyz/borsh";
import { MemberImpl, DeviceImpl, InvitationImpl, KeysetImpl, RoleImpl, ServerImpl, LockboxImpl } from "../models";
import { Query } from "./query";
import { Sort } from "./sort";
import { randomId, toArray } from "@topgunbuild/common";
import { DataChanges, DataFrameChangeOperation, Lockbox, SelectOptions, SelectResult } from "../types";

/**
 * Base interface for all actions
 */
export interface IAction {
    lockboxes?: Lockbox[];
}

/**
 * Base class for all actions
 */
export class AbstractAction implements IAction {
    @field({ type: 'string' })
    id: string;

    @field({ type: option(vec(LockboxImpl)) })
    lockboxes?: Lockbox[];

    constructor(data: {
        id?: string,
        lockboxes?: Lockbox[],
    }) {
        this.id = data.id || randomId();
        this.lockboxes = Array.isArray(data.lockboxes) ? data.lockboxes : [];
    }

    encode(): Uint8Array {
        return serialize(this);
    }
}

/**
 * Create team request
 */
@variant(0)
export class CreateTeamAction extends AbstractAction {
    @field({ type: 'string' })
    teamId: string;
    
    @field({ type: 'string' })
    name: string;

    @field({ type: option('string') })
    description?: string;

    @field({ type: MemberImpl })
    rootMember: MemberImpl;

    @field({ type: DeviceImpl })
    rootDevice: DeviceImpl;

    constructor(data: {
        teamId: string,
        lockboxes?: Lockbox[],
        name: string,
        description?: string,
        rootMember: MemberImpl,
        rootDevice: DeviceImpl
    }) {
        super(data);
        this.teamId = data.teamId;
        this.name = data.name;
        this.description = data.description;
        this.rootMember = data.rootMember;
        this.rootDevice = data.rootDevice;
    }
}

/**
 * Set team action
 */
@variant(1)
export class UpdateTeamAction extends AbstractAction {
    @field({ type: 'string' })
    teamId: string;

    @field({ type: option('string') })
    name?: string;

    @field({ type: option('string') })
    description?: string;

    constructor(data: { lockboxes?: Lockbox[], teamId: string, name?: string, description?: string }) {
        super(data);
        this.teamId = data.teamId;
        this.name = data.name;
        this.description = data.description;
    }
}

/**
 * Add member request
 */
@variant(2)
export class AddMemberAction extends AbstractAction {
    @field({ type: MemberImpl })
    member: MemberImpl;

    @field({ type: vec('string') })
    roles?: string[];

    constructor(data: {
        lockboxes?: Lockbox[],
        roles?: string[],
        member: MemberImpl,
    }) {
        super(data);
        this.member = data.member;
        this.roles = data.roles;
    }
}

/**
 * Remove member request
 */
@variant(3)
export class RemoveMemberAction extends AbstractAction {
    @field({ type: 'string' })
    userId: string;

    constructor(data: { lockboxes?: Lockbox[], userId: string }) {
        super(data);
        this.userId = data.userId;
    }
}

/**
 * Add role request
 */
@variant(4)
export class AddRoleAction extends AbstractAction {
    @field({ type: RoleImpl })
    role: RoleImpl;

    constructor(data: { lockboxes?: Lockbox[], role: RoleImpl }) {
        super(data);
        this.role = data.role;
    }
}

/**
 * Remove role request
 */
@variant(5)
export class RemoveRoleAction extends AbstractAction {
    @field({ type: 'string' })
    roleName: string;

    constructor(data: { lockboxes?: Lockbox[], roleName: string }) {
        super(data);
        this.roleName = data.roleName;
    }
}

/**
 * Assign a role to a member
 */
@variant(6)
export class AssignRoleToMemberAction extends AbstractAction {
    @field({ type: 'string' })
    userId: string;

    @field({ type: 'string' })
    roleName: string;

    constructor(data: { lockboxes?: Lockbox[], userId: string, roleName: string }) {
        super(data);
        this.userId = data.userId;
        this.roleName = data.roleName;
    }
}

/**
 * Remove a role from a member
 */
@variant(7)
export class RemoveRoleFromMemberAction extends AbstractAction {
    @field({ type: 'string' })
    userId: string;

    @field({ type: 'string' })
    roleName: string;

    constructor(data: { lockboxes?: Lockbox[], userId: string, roleName: string }) {
        super(data);
        this.userId = data.userId;
        this.roleName = data.roleName;
    }
}

/**
 * Add device request
 */
@variant(8)
export class AddDeviceAction extends AbstractAction {
    @field({ type: DeviceImpl })
    device: DeviceImpl;

    constructor(data: { lockboxes?: Lockbox[], device: DeviceImpl }) {
        super(data);
        this.device = data.device;
    }
}

/**
 * Remove device request
 */
@variant(9)
export class RemoveDeviceAction extends AbstractAction {
    @field({ type: 'string' })
    deviceId: string;

    constructor(data: { lockboxes?: Lockbox[], deviceId: string }) {
        super(data);
        this.deviceId = data.deviceId;
    }
}

/**
 * Invite member request
 */
@variant(10)
export class InviteMemberAction extends AbstractAction {
    @field({ type: InvitationImpl })
    invitation: InvitationImpl;

    constructor(data: { lockboxes?: Lockbox[], invitation: InvitationImpl }) {
        super(data);
        this.invitation = data.invitation;
    }
}

/**
 * Invite device request
 */
@variant(11)
export class InviteDeviceAction extends AbstractAction {
    @field({ type: InvitationImpl })
    invitation: InvitationImpl;

    constructor(data: { lockboxes?: Lockbox[], invitation: InvitationImpl }) {
        super(data);
        this.invitation = data.invitation;
    }
}

/**
 * Revoke invitation request
 */
@variant(12)
export class RevokeInvitationAction extends AbstractAction {
    @field({ type: 'string' })
    invitationId: string; // Invitation ID

    constructor(data: { lockboxes?: Lockbox[], invitationId: string }) {
        super(data);
        this.invitationId = data.invitationId;
    }
}

/**
 * Admit member request
 */
@variant(13)
export class AdmitMemberAction extends AbstractAction {
    @field({ type: 'string' })
    invitationId: string; // Invitation ID

    @field({ type: 'string' })
    userName: string;

    @field({ type: KeysetImpl })
    memberKeys: KeysetImpl;

    constructor(data: {
        lockboxes?: Lockbox[],
        invitationId: string,
        userName: string,
        memberKeys: KeysetImpl,
    }) {
        super(data);
        this.invitationId = data.invitationId;
        this.userName = data.userName;
        this.memberKeys = data.memberKeys;
    }
}

/**
 * Admit device request
 */
@variant(14)
export class AdmitDeviceAction extends AbstractAction {
    @field({ type: 'string' })
    invitationId: string; // Invitation ID

    @field({ type: DeviceImpl })
    device: DeviceImpl;

    constructor(data: { lockboxes?: Lockbox[], invitationId: string, device: DeviceImpl }) {
        super(data);
        this.invitationId = data.invitationId;
        this.device = data.device;
    }
}

/**
 * Change member keys request
 */
@variant(15)
export class ChangeMemberKeysAction extends AbstractAction {
    @field({ type: KeysetImpl })
    keys: KeysetImpl;

    constructor(data: { lockboxes?: Lockbox[], keys: KeysetImpl }) {
        super(data);
        this.keys = data.keys;
    }
}

/**
 * Rotate keys request
 */
@variant(16)
export class RotateKeysAction extends AbstractAction {
    @field({ type: 'string' })
    userId: string;

    constructor(data: { lockboxes?: Lockbox[], userId: string }) {
        super(data);
        this.userId = data.userId;
    }
}

/**
 * Add server request
 */
@variant(17)
export class AddServerAction extends AbstractAction {
    @field({ type: ServerImpl })
    server: ServerImpl;

    constructor(data: { lockboxes?: Lockbox[], server: ServerImpl }) {
        super(data);
        this.server = data.server;
    }
}

/**
 * Remove server request
 */
@variant(18)
export class RemoveServerAction extends AbstractAction {
    @field({ type: 'string' })
    host: string;

    constructor(data: { lockboxes?: Lockbox[], host: string }) {
        super(data);
        this.host = data.host;
    }
}

/**
 * Change server keys request
 */
@variant(19)
export class ChangeServerKeysAction extends AbstractAction {
    @field({ type: KeysetImpl })
    keys: KeysetImpl;

    constructor(data: { lockboxes?: Lockbox[], keys: KeysetImpl }) {
        super(data);
        this.keys = data.keys;
    }
}

/**
 * Put message request
 */
@variant(20)
export class PutMessageAction extends AbstractAction {
    @field({ type: 'string' })
    channelId: string;

    @field({ type: 'string' })
    messageId: string;

    @field({ type: 'string' })
    value: string;

    @field({ type: 'u64' })
    state: bigint;

    constructor(data: {
        channelId: string,
        messageId: string,
        value: string,
        state: bigint,
    }) {
        super({});
        this.channelId = data.channelId;
        this.messageId = data.messageId;
        this.value = data.value;
        this.state = data.state;
    }
}

/**
 * Delete message request
 */
@variant(21)
export class DeleteMessageAction extends AbstractAction {
    @field({ type: 'string' })
    channelId: string;

    @field({ type: 'string' })
    messageId: string;

    constructor(data: {
        channelId: string,
        messageId: string,
        fieldName: string,
    }) {
        super({});
        this.channelId = data.channelId;
        this.messageId = data.messageId;
    }
}


/**
 * DataFrame change operation request
 */
export class DataFrameChangeOperationAction implements DataFrameChangeOperation<string> {
    @field({ type: 'string' })
    element: string;

    @field({ type: 'string' })
    type: 'added' | 'deleted' | 'updated';

    @field({ type: 'u64' })
    timestamp: number;

    constructor(data: { element: string, type: 'added' | 'deleted' | 'updated', timestamp: number }) {
        this.element = data.element;
        this.type = data.type;
        this.timestamp = data.timestamp;
    }
}

/**
 * Data changes request
 */
@variant(22)
export class DataChangesAction implements DataChanges<string> {
    @field({ type: option(vec(DataFrameChangeOperationAction)) })
    changes?: DataFrameChangeOperationAction[];

    @field({ type: option(vec('string')) })
    collection?: string[];

    @field({ type: 'u64' })
    total: number;

    @field({ type: 'string' })
    queryHash: string;

    constructor(data: { changes?: DataFrameChangeOperationAction[], collection?: string[], total: number, queryHash: string }) {
        this.changes = data.changes;
        this.collection = data.collection;
        this.total = data.total;
        this.queryHash = data.queryHash;
    }
}

/**
 * Select request
 */
@variant(23)
export class SelectAction extends AbstractAction implements SelectOptions {
    @field({ type: 'string' })
    entity: string;

    @field({ type: option('string') })
    channelId?: string;

    @field({ type: option('string') })
    messageId?: string;

    @field({ type: vec(Query) })
    query: Query[];

    @field({ type: vec(Sort) })
    sort: Sort[];

    @field({ type: vec('string') })
    fields: string[];

    @field({ type: 'u16' })
    pageSize: number;

    @field({ type: 'u32' })
    pageOffset: number;

    constructor(data: {
        entity: string,
        channelId?: string,
        messageId?: string
    } & SelectOptions) {
        super({});
        this.entity = data.entity;
        this.channelId = data.channelId;
        this.messageId = data.messageId;
        this.query = toArray(data.query);
        this.sort = toArray(data.sort);
        this.fields = toArray(data.fields);
        this.pageSize = data.pageSize || 10;
        this.pageOffset = data.pageOffset || 0;
    }
}

/**
 * Select result
 */
@variant(24)
export class SelectResultAction extends AbstractAction implements SelectResult<string> {
    @field({ type: vec('string') })
    rows: string[];

    @field({ type: 'u64' })
    total: number;

    @field({ type: option('bool') })
    hasNextPage?: boolean;

    @field({ type: option('bool') })
    hasPreviousPage?: boolean;

    @field({ type: option('string') })
    queryHash?: string;

    constructor(data: SelectResult<string>) {
        super({});
        this.rows = data.rows;
        this.total = data.total;
        this.hasNextPage = data.hasNextPage;
        this.hasPreviousPage = data.hasPreviousPage;
        this.queryHash = data.queryHash;
    }
}

/**
 * Cancel select request
 */
@variant(25)
export class CancelSelectAction extends AbstractAction {
    @field({ type: 'string' })
    queryHash: string;

    constructor(data: { queryHash: string }) {
        super({});
        this.queryHash = data.queryHash;
    }
}

/**
 * Create channel request
 */
@variant(26)
export class CreateChannelAction extends AbstractAction {
    @field({ type: 'string' })
    name: string;

    @field({ type: option('string') })
    description?: string;

    constructor(data: { lockboxes?: Lockbox[], name: string, description?: string }) {
        super(data);
        this.name = data.name;
        this.description = data.description;
    }
}

/**
 * Update channel request
 */
@variant(27)
export class UpdateChannelAction extends AbstractAction {
    @field({ type: 'string' })
    channelId: string;

    @field({ type: option('string') })
    name?: string;

    @field({ type: option('string') })
    description?: string;

    constructor(data: { lockboxes?: Lockbox[], channelId: string, name?: string, description?: string }) {
        super(data);
        this.channelId = data.channelId;
        this.name = data.name;
        this.description = data.description;
    }
}
