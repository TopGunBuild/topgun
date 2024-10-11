import { SqlTable } from './sql-table';
import { ColumnType, UpdateDeleteAction } from './sql-column';
import { SqlColumnGenerator } from './sql-column-generator';

/**
 * Team table definition
 * @type {SqlTable}
 */
export const teamTable = SqlTable.create('team')
    .setColumns(() => [
        {
            name       : 'team_id',
            type       : ColumnType.TEXT,
            primary    : true,
            uniqueIndex: true,
        },
        {
            name: 'name',
            type: ColumnType.TEXT,
        },
        SqlColumnGenerator.createdAt(),
        SqlColumnGenerator.updatedAt(),
    ]);

/**
 * Action table definition
 * @type {SqlTable}
 */
export const actionTable = SqlTable.create('action')
    .setColumns(() => [
        {
            name       : 'action_id',
            type       : ColumnType.TEXT,
            primary    : true,
            uniqueIndex: true,
        },
        {
            name : 'type',
            type : ColumnType.TEXT,
            index: true,
        },
        {
            name: 'state',
            type: ColumnType.BIGINT,
        },
        {
            name: 'hash',
            type: ColumnType.TEXT,
        },
        {
            name: 'prev',
            type: ColumnType.TEXT,
        },
        {
            name: 'body',
            type: ColumnType.JSON,
        },
        {
            name: 'is_invalid',
            type: ColumnType.INTEGER,
        },
        SqlColumnGenerator.createdAt(),
        SqlColumnGenerator.userReference(),
        SqlColumnGenerator.teamReference(),
    ]);

/**
 * Keyset table definition
 * @type {SqlTable}
 */
export const keysetTable = SqlTable.create('keyset')
    .setColumns(() => [
        {
            name       : 'keyset_id',
            type       : ColumnType.TEXT,
            primary    : true,
            uniqueIndex: true,
        },
        {
            name: 'type',
            type: ColumnType.TEXT,
        },
        {
            name: 'name',
            type: ColumnType.TEXT,
        },
        {
            name: 'encryption',
            type: ColumnType.TEXT,
        },
        {
            name: 'signature',
            type: ColumnType.TEXT,
        },
        {
            name: 'generation',
            type: ColumnType.INTEGER,
        },
        SqlColumnGenerator.createdAt(),
        SqlColumnGenerator.teamReference(),
    ]);

/**
 * Lockbox table definition
 * @type {SqlTable}
 */
export const lockboxTable = SqlTable.create('lockbox')
    .setColumns(() => [
        {
            name       : 'lockbox_id',
            type       : ColumnType.TEXT,
            primary    : true,
            uniqueIndex: true,
        },
        {
            name   : 'action',
            type   : ColumnType.TEXT,
            target : actionTable,
            actions: {
                onDelete: UpdateDeleteAction.CASCADE,
            },
        },
        {
            name: 'encryption_public_key',
            type: ColumnType.TEXT,
        },
        {
            name   : 'recipient',
            type   : ColumnType.TEXT,
            target : keysetTable,
            actions: {
                onDelete: UpdateDeleteAction.CASCADE,
            },
        },
        {
            name   : 'contents',
            type   : ColumnType.TEXT,
            target : keysetTable,
            actions: {
                onDelete: UpdateDeleteAction.CASCADE,
            },
        },
        SqlColumnGenerator.createdAt(),
        SqlColumnGenerator.teamReference(),
    ]);

/**
 * Member table definition
 * @type {SqlTable}
 */
export const memberTable = SqlTable.create('member')
    .setColumns(() => [
        {
            name       : 'member_id',
            type       : ColumnType.TEXT,
            primary    : true,
            uniqueIndex: true,
        },
        {
            name: 'name',
            type: ColumnType.TEXT,
        },
        SqlColumnGenerator.createdAt(),
        SqlColumnGenerator.updatedAt(),
        SqlColumnGenerator.keysetReference(),
        SqlColumnGenerator.teamReference(),
    ]);

/**
 * Role table definition
 * @type {SqlTable}
 */
export const roleTable = SqlTable.create('role')
    .setColumns(() => [
        {
            name       : 'role_id',
            type       : ColumnType.TEXT,
            primary    : true,
            uniqueIndex: true,
        },
        {
            name: 'name',
            type: ColumnType.TEXT,
        },
        SqlColumnGenerator.createdAt(),
        SqlColumnGenerator.updatedAt(),
        SqlColumnGenerator.teamReference(),
    ]);

/**
 * Server table definition
 * @type {SqlTable}
 */
export const serverTable = SqlTable.create('server')
    .setColumns(() => [
        {
            name       : 'server_id',
            type       : ColumnType.TEXT,
            primary    : true,
            uniqueIndex: true,
        },
        {
            name: 'host',
            type: ColumnType.TEXT,
        },
        SqlColumnGenerator.keysetReference(),
        SqlColumnGenerator.createdAt(),
        SqlColumnGenerator.updatedAt(),
    ]);

/**
 * Device table definition
 * @type {SqlTable}
 */
export const deviceTable = SqlTable.create('device')
    .setColumns(() => [
        {
            name       : 'device_id',
            type       : ColumnType.TEXT,
            primary    : true,
            uniqueIndex: true,
        },
        {
            name: 'device_info',
            type: ColumnType.TEXT,
        },
        SqlColumnGenerator.createdAt(),
        SqlColumnGenerator.updatedAt(),
        SqlColumnGenerator.keysetReference(),
        SqlColumnGenerator.teamReference(),
        SqlColumnGenerator.userReference(),
    ]);

/**
 * Invitation table definition
 * @type {SqlTable}
 */
export const invitationTable = SqlTable.create('invitation')
    .setColumns(() => [
        {
            name       : 'invitation_id',
            type       : ColumnType.TEXT,
            primary    : true,
            uniqueIndex: true,
        },
        {
            name: 'public_key',
            type: ColumnType.TEXT,
        },
        {
            name: 'expiration',
            type: ColumnType.DATETIME,
        },
        {
            name: 'max_uses',
            type: ColumnType.INTEGER,
        },
        SqlColumnGenerator.createdAt(),
        SqlColumnGenerator.teamReference(),
        SqlColumnGenerator.userReference(),
    ]);

/**
 * Message table definition
 * @type {SqlTable}
 */
export const messageTable = SqlTable.create('message')
    .setColumns(() => [
        {
            name   : 'channel_id',
            type   : ColumnType.TEXT,
            primary: true,
        },
        {
            name   : 'message_id',
            type   : ColumnType.TEXT,
            primary: true,
        },
        {
            name: 'deleted',
            type: ColumnType.BOOLEAN,
        },
        {
            name: 'values',
            type: ColumnType.HSTORE,
        },
        {
            name: 'state',
            type: ColumnType.HSTORE,
        },
        SqlColumnGenerator.updatedAt(),
        SqlColumnGenerator.createdAt(),
        SqlColumnGenerator.teamReference(),
        SqlColumnGenerator.userReference(),
    ])
    .setIndexes(({ name }) => [
        {
            name   : `${name}_uniq_idx`,
            columns: ['team_id', 'channel_id', 'message_id'],
            unique : true,
        },
    ])
    .setConstraints(({ name }) => [
        {
            definition: `create index ${name}_values_idx ON ${name} USING GIN (values)`,
        },
        {
            definition: `create index ${name}_state_idx ON ${name} USING GIN (state)`,
        },
    ]);
/*export const messageTable = SqlTable.create('message')
    .setColumns(() => [
        {
            name   : 'channel_id',
            type   : ColumnType.TEXT,
            primary: true,
        },
        {
            name   : 'message_id',
            type   : ColumnType.TEXT,
            primary: true,
        },
        {
            name   : 'field_name',
            type   : ColumnType.TEXT,
            primary: true,
        },
        {
            name: 'state',
            type: ColumnType.BIGINT,
        },
        {
            name: 'value_is_empty',
            type: ColumnType.BOOLEAN,
        },
        {
            name: 'value_string',
            type: ColumnType.TEXT,
        },
        {
            name: 'value_bool',
            type: ColumnType.BOOLEAN,
        },
        {
            name: 'value_number',
            type: ColumnType.NUMERIC,
        },
        {
            name: 'value_byte',
            type: ColumnType.BLOB,
        },
        {
            name: 'value_date',
            type: ColumnType.DATETIME,
        },
        {
            name: 'deleted',
            type: ColumnType.BOOLEAN,
        },
        {
            name   : 'user_id',
            type   : ColumnType.TEXT,
            target : memberTable,
            actions: {
                onDelete: UpdateDeleteAction.CASCADE,
            },
        },
        {
            name   : 'team_id',
            type   : ColumnType.TEXT,
            target : teamTable,
            actions: {
                onDelete: UpdateDeleteAction.CASCADE,
            },
        },
        {
            name: 'created_at',
            type: ColumnType.DATETIME,
        },
    ])
    .setIndexes(({ name }) => [
        {
            name   : `${name}_uniq_idx`,
            columns: ['team_id', 'channel_id', 'message_id', 'field_name'],
            unique : true,
        },
        {
            name   : `${name}_section_node_idx`,
            columns: ['team_id', 'channel_id', 'message_id'],
        },
        {
            name   : `${name}_team_id_idx`,
            columns: ['team_id'],
        },
        {
            name   : `${name}_team_id_idx`,
            columns: ['user_id'],
        },
    ]);*/
