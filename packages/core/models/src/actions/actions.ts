import { field, option, variant, vec, serialize } from "@dao-xyz/borsh";
import { Member, Device, Invitation, KeysetPublic, Role, Server, Lockbox } from "../models";
import { Query } from "./query";
import { Sort } from "./sort";
import { randomId, toArray } from "@topgunbuild/common";
import {
    DataChanges,
    DataFrameChangeOperation,
    DevicePublicInfo,
    InvitationInfo,
    KeysetPublicInfo,
    LockboxInfo,
    MemberInfo,
    RoleInfo,
    SelectOptions,
    SelectResult,
    ServerPublicInfo
} from "../types";

/**
 * Base interface for all actions
 */
export interface ActionInfo {
    lockboxes?: LockboxInfo[];
}

/**
 * Base class for all actions
 */
export class AbstractAction implements ActionInfo {
    @field({ type: 'string' })
    id: string;

    @field({ type: option(vec(Lockbox)) })
    lockboxes?: LockboxInfo[];

    constructor(data: {
        id?: string,
        lockboxes?: LockboxInfo[],
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

    @field({ type: Member })
    rootMember: Member;

    @field({ type: Device })
    rootDevice: Device;

    constructor(data: {
        teamId: string,
        lockboxes?: LockboxInfo[],
        name: string,
        description?: string,
        rootMember: MemberInfo,
        rootDevice: DevicePublicInfo
    }) {
        super(data);
        this.teamId = data.teamId;
        this.name = data.name;
        this.description = data.description;
        this.rootMember = new Member(data.rootMember);
        this.rootMember.teamId = data.teamId;
        this.rootDevice = new Device(data.rootDevice);
        this.rootDevice.teamId = data.teamId;
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

    constructor(data: { lockboxes?: LockboxInfo[], teamId: string, name?: string, description?: string }) {
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
    @field({ type: Member })
    member: Member;

    @field({ type: vec('string') })
    roles?: string[];

    constructor(data: {
        lockboxes?: LockboxInfo[],
        roles?: string[],
        member: MemberInfo,
    }) {
        super(data);
        this.member = new Member(data.member);
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

    constructor(data: { lockboxes?: LockboxInfo[], userId: string }) {
        super(data);
        this.userId = data.userId;
    }
}

/**
 * Add role request
 */
@variant(4)
export class AddRoleAction extends AbstractAction {
    @field({ type: Role })
    role: Role;

    constructor(data: { lockboxes?: LockboxInfo[], role: RoleInfo }) {
        super(data);
        this.role = new Role(data.role);
    }
}

/**
 * Remove role request
 */
@variant(5)
export class RemoveRoleAction extends AbstractAction {
    @field({ type: 'string' })
    roleName: string;

    constructor(data: { lockboxes?: LockboxInfo[], roleName: string }) {
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

    constructor(data: { lockboxes?: LockboxInfo[], userId: string, roleName: string }) {
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

    constructor(data: { lockboxes?: LockboxInfo[], userId: string, roleName: string }) {
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
    @field({ type: Device })
    device: Device;

    constructor(data: { lockboxes?: LockboxInfo[], device: DevicePublicInfo }) {
        super(data);
        this.device = new Device(data.device);
    }
}

/**
 * Remove device request
 */
@variant(9)
export class RemoveDeviceAction extends AbstractAction {
    @field({ type: 'string' })
    deviceId: string;

    constructor(data: { lockboxes?: LockboxInfo[], deviceId: string }) {
        super(data);
        this.deviceId = data.deviceId;
    }
}

/**
 * Invite member request
 */
@variant(10)
export class InviteMemberAction extends AbstractAction {
    @field({ type: Invitation })
    invitation: Invitation;

    constructor(data: { lockboxes?: LockboxInfo[], invitation: InvitationInfo }) {
        super(data);
        this.invitation = new Invitation(data.invitation);
    }
}

/**
 * Invite device request
 */
@variant(11)
export class InviteDeviceAction extends AbstractAction {
    @field({ type: Invitation })
    invitation: Invitation;

    constructor(data: { lockboxes?: LockboxInfo[], invitation: InvitationInfo }) {
        super(data);
        this.invitation = new Invitation(data.invitation);
    }
}

/**
 * Revoke invitation request
 */
@variant(12)
export class RevokeInvitationAction extends AbstractAction {
    @field({ type: 'string' })
    invitationId: string; // Invitation ID

    constructor(data: { lockboxes?: LockboxInfo[], invitationId: string }) {
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

    @field({ type: KeysetPublic })
    memberKeys: KeysetPublic;

    constructor(data: {
        lockboxes?: LockboxInfo[],
        invitationId: string,
        userName: string,
        memberKeys: KeysetPublicInfo,
    }) {
        super(data);
        this.invitationId = data.invitationId;
        this.userName = data.userName;
        this.memberKeys = new KeysetPublic(data.memberKeys);
    }
}

/**
 * Admit device request
 */
@variant(14)
export class AdmitDeviceAction extends AbstractAction {
    @field({ type: 'string' })
    invitationId: string; // Invitation ID

    @field({ type: Device })
    device: Device;

    constructor(data: { lockboxes?: LockboxInfo[], invitationId: string, device: DevicePublicInfo }) {
        super(data);
        this.invitationId = data.invitationId;
        this.device = new Device(data.device);
    }
}

/**
 * Change member keys request
 */
@variant(15)
export class ChangeMemberKeysAction extends AbstractAction {
    @field({ type: KeysetPublic })
    keys: KeysetPublic;

    constructor(data: { lockboxes?: LockboxInfo[], keys: KeysetPublicInfo }) {
        super(data);
        this.keys = new KeysetPublic(data.keys);
    }
}

/**
 * Rotate keys request
 */
@variant(16)
export class RotateKeysAction extends AbstractAction {
    @field({ type: 'string' })
    userId: string;

    constructor(data: { lockboxes?: LockboxInfo[], userId: string }) {
        super(data);
        this.userId = data.userId;
    }
}

/**
 * Add server request
 */
@variant(17)
export class AddServerAction extends AbstractAction {
    @field({ type: Server })
    server: Server;

    constructor(data: { lockboxes?: LockboxInfo[], server: ServerPublicInfo }) {
        super(data);
        this.server = new Server(data.server);
    }
}

/**
 * Remove server request
 */
@variant(18)
export class RemoveServerAction extends AbstractAction {
    @field({ type: 'string' })
    host: string;

    constructor(data: { lockboxes?: LockboxInfo[], host: string }) {
        super(data);
        this.host = data.host;
    }
}

/**
 * Change server keys request
 */
@variant(19)
export class ChangeServerKeysAction extends AbstractAction {
    @field({ type: KeysetPublic })
    keys: KeysetPublic;

    constructor(data: { lockboxes?: LockboxInfo[], keys: KeysetPublic }) {
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
    limit: number;

    @field({ type: 'u32' })
    offset: number;

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
        this.limit = data.limit || 10;
        this.offset = data.offset || 0;
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

    constructor(data: { lockboxes?: LockboxInfo[], name: string, description?: string }) {
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

    constructor(data: { lockboxes?: LockboxInfo[], channelId: string, name?: string, description?: string }) {
        super(data);
        this.channelId = data.channelId;
        this.name = data.name;
        this.description = data.description;
    }
}
