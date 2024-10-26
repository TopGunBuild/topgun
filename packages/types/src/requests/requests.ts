import { field, option, variant, vec } from "@dao-xyz/borsh";
import { serialize } from "@dao-xyz/borsh";
import { Member, Device, Invitation, Keyset, Role, Server, Lockbox } from "../models";
import { AbstractValue } from "./values";
import { typeOf } from "./typeof";
import { Query } from "./query";
import { Sort } from "./sort";
import { toArray } from "@topgunbuild/utils";

/**
 * Base interface for all requests
 */
export interface IRequest {
    lockboxes?: Lockbox[];
}

/**
 * Base class for all requests
 */
export class AbstractRequest implements IRequest {

    @field({ type: option(vec(Lockbox)) })
    lockboxes?: Lockbox[];

    constructor(data: {
        lockboxes?: Lockbox[],
    }) {
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
export class CreateTeamRequest extends AbstractRequest {
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
    }) {
        super(data);
        this.name = data.name;
        this.rootMember = data.rootMember;
        this.rootDevice = data.rootDevice;
    }
}

/**
 * Add member request
 */
@variant(1)
export class AddMemberRequest extends AbstractRequest {
    @field({ type: Member })
    member: Member;

    @field({ type: vec('string') })
    roles?: string[];

    constructor(data: {
        lockboxes?: Lockbox[],
        roles?: string[],
        member: Member,
    }) {
        super(data);
        this.member = data.member;
        this.roles = data.roles;
    }
}

/**
 * Remove member request
 */
@variant(2)
export class RemoveMemberRequest extends AbstractRequest {
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
@variant(3)
export class AddRoleRequest extends AbstractRequest {
    @field({ type: Role })
    role: Role;

    constructor(data: { lockboxes?: Lockbox[], role: Role }) {
        super(data);
        this.role = data.role;
    }
}

/**
 * Remove role request
 */
@variant(4)
export class RemoveRoleRequest extends AbstractRequest {
    @field({ type: 'string' })
    roleName: string;

    constructor(data: { lockboxes?: Lockbox[], roleName: string }) {
        super(data);
        this.roleName = data.roleName;
    }
}

/**
 * Add member role request
 */
@variant(5)
export class AddMemberRoleRequest extends AbstractRequest {
    @field({ type: 'string' })
    userId: string;

    @field({ type: Role })
    role: Role;

    constructor(data: { lockboxes?: Lockbox[], userId: string, role: Role }) {
        super(data);
        this.userId = data.userId;
        this.role = data.role;
    }
}

/**
 * Remove member role request
 */
@variant(6)
export class RemoveMemberRoleRequest extends AbstractRequest {
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
@variant(7)
export class AddDeviceRequest extends AbstractRequest {
    @field({ type: Device })
    device: Device;

    constructor(data: { lockboxes?: Lockbox[], device: Device }) {
        super(data);
        this.device = data.device;
    }
}

/**
 * Remove device request
 */
@variant(8)
export class RemoveDeviceRequest extends AbstractRequest {
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
@variant(9)
export class InviteMemberRequest extends AbstractRequest {
    @field({ type: Invitation })
    invitation: Invitation;

    constructor(data: { lockboxes?: Lockbox[], invitation: Invitation }) {
        super(data);
        this.invitation = data.invitation;
    }
}

/**
 * Invite device request
 */
@variant(10)
export class InviteDeviceRequest extends AbstractRequest {
    @field({ type: Invitation })
    invitation: Invitation;

    constructor(data: { lockboxes?: Lockbox[], invitation: Invitation }) {
        super(data);
        this.invitation = data.invitation;
    }
}

/**
 * Revoke invitation request
 */
@variant(11)
export class RevokeInvitationRequest extends AbstractRequest {
    @field({ type: 'string' })
    id: string; // Invitation ID

    constructor(data: { lockboxes?: Lockbox[], id: string }) {
        super(data);
        this.id = data.id;
    }
}

/**
 * Admit member request
 */
@variant(12)
export class AdmitMemberRequest extends AbstractRequest {
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
    }) {
        super(data);
        this.id = data.id;
        this.userName = data.userName;
        this.memberKeys = data.memberKeys;
    }
}

/**
 * Admit device request
 */
@variant(13)
export class AdmitDeviceRequest extends AbstractRequest {
    @field({ type: 'string' })
    id: string; // Invitation ID

    @field({ type: Device })
    device: Device;

    constructor(data: { lockboxes?: Lockbox[], id: string, device: Device }) {
        super(data);
        this.id = data.id;
        this.device = data.device;
    }
}

/**
 * Change member keys request
 */
@variant(14)
export class ChangeMemberKeysRequest extends AbstractRequest {
    @field({ type: Keyset })
    keys: Keyset;

    constructor(data: { lockboxes?: Lockbox[], keys: Keyset }) {
        super(data);
        this.keys = data.keys;
    }
}

/**
 * Rotate keys request
 */
@variant(15)
export class RotateKeysRequest extends AbstractRequest {
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
@variant(16)
export class AddServerRequest extends AbstractRequest {
    @field({ type: Server })
    server: Server;

    constructor(data: { lockboxes?: Lockbox[], server: Server }) {
        super(data);
        this.server = data.server;
    }
}

/**
 * Remove server request
 */
@variant(17)
export class RemoveServerRequest extends AbstractRequest {
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
@variant(18)
export class ChangeServerKeysRequest extends AbstractRequest {
    @field({ type: Keyset })
    keys: Keyset;

    constructor(data: { lockboxes?: Lockbox[], keys: Keyset }) {
        super(data);
        this.keys = data.keys;
    }
}

/**
 * Set team name request
 */
@variant(19)
export class SetTeamNameRequest extends AbstractRequest {
    @field({ type: 'string' })
    teamName: string;

    constructor(data: { lockboxes?: Lockbox[], teamName: string }) {
        super(data);
        this.teamName = data.teamName;
    }
}

/**
 * Put message request
 */
@variant(20)
export class PutMessageRequest extends AbstractRequest {
    @field({ type: 'string' })
    channelId: string;

    @field({ type: 'string' })
    messageId: string;

    @field({ type: 'string' })
    fieldName: string;

    @field({ type: AbstractValue })
    value: AbstractValue;

    constructor(data: {
        channelId: string,
        messageId: string,
        fieldName: string,
        value: unknown,
    }) {
        super({});
        this.channelId = data.channelId;
        this.messageId = data.messageId;
        this.fieldName = data.fieldName;
        this.value = typeOf(data.value);
    }
}

/**
 * Delete message request
 */
@variant(21)
export class DeleteMessageRequest extends AbstractRequest {
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
    }) {
        super({});
        this.channelId = data.channelId;
        this.messageId = data.messageId;
        this.fieldName = data.fieldName;
    }
}

/**
 * Data changes request
 */
@variant(22)
export class DataChangesRequest extends AbstractRequest {
    @field({ type: 'string' })
    operation: 'insert' | 'update' | 'delete';

    @field({ type: 'string' })
    rowData: string;

    @field({ type: option('string') })
    oldData?: string;

    constructor(data: {
        operation: 'insert' | 'update' | 'delete',
        rowData: string,
        oldData?: string,
    }) {
        super({});
        this.operation = data.operation;
        this.rowData = data.rowData;
        this.oldData = data.oldData;
    }
}

/**
 * Select request interface
 */
export interface ISelectRequest {
    channelId: string;
    messageId?: string;
    fieldName?: string;
    query: Query[];
    sort: Sort[];
    fields: string[];
    pageSize: number;
    pageOffset: number;
}

/**
 * Select request
 */
@variant(23)
export class SelectRequest extends AbstractRequest implements ISelectRequest {
    @field({ type: 'string' })
    channelId: string;

    @field({ type: option('string') })
    messageId?: string;

    @field({ type: option('string') })
    fieldName?: string;

    @field({ type: vec(Query) })
    query: Query[];

    @field({ type: vec(Sort) })
    sort: Sort[];

    @field({ type: vec('string') })
    fields: string[];

    @field({ type: 'u16' })
    pageSize: number;

    @field({ type: 'u32'})
    pageOffset: number;

    constructor(data: ISelectRequest) {
        super({});
        this.channelId = data.channelId;
        this.messageId = data.messageId;
        this.fieldName = data.fieldName;
        this.query = toArray(data.query);
        this.sort = toArray(data.sort);
        this.fields = toArray(data.fields);
        this.pageSize = data.pageSize || 10;
        this.pageOffset = data.pageOffset || 0;
    }
}

/**
 * The query result
 */
export type ISelectResult<T> = {
    rows: T[];
    total: number;
    hasNextPage?: boolean;
    hasPreviousPage?: boolean;
    queryHash?: string;
};

/**
 * Select result
 */
@variant(24)
export class SelectResult extends AbstractRequest implements ISelectResult<string> {
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

    constructor(data: ISelectResult<string>) {
        super({});
        this.rows = data.rows;
        this.total = data.total;
        this.hasNextPage = data.hasNextPage;
        this.hasPreviousPage = data.hasPreviousPage;
        this.queryHash = data.queryHash;
    }
}
