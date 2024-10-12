import { SqlTable } from './sql-table';
import { ColumnType, UpdateDeleteAction } from './sql-column';
import { SqlColumnGenerator } from './sql-column-generator';

/**
 * Team table definition
 * Defines a table for storing team information.
 * @type {SqlTable}
 */
export const teamTable: SqlTable = SqlTable.create('team')
    .setColumns(() => [
        {
            name       : 'team_id',  // Unique identifier for the team
            type       : ColumnType.TEXT,
            primary    : true,
            uniqueIndex: true,
        },
        {
            name: 'name', // Name of the team
            type: ColumnType.TEXT,
        },
        SqlColumnGenerator.createdAt(),
        SqlColumnGenerator.updatedAt(),
    ]);

/**
 * Action table definition
 * Defines a table for storing actions.
 * @type {SqlTable}
 */
export const actionTable: SqlTable = SqlTable.create('action')
    .setColumns(() => [
        {
            name       : 'action_id', // Unique identifier for the action
            type       : ColumnType.TEXT,
            primary    : true,
            uniqueIndex: true,
        },
        {
            name : 'type', // Type of action
            type : ColumnType.TEXT,
            index: true,
        },
        {
            name: 'state', // The "bigint" value is used based on a last-write-wins method.
            type: ColumnType.BIGINT,
        },
        {
            name: 'hash', // Hash value associated with the action
            type: ColumnType.TEXT,
        },
        {
            name: 'prev', // Hash value of the previous action
            type: ColumnType.TEXT,
        },
        {
            name: 'body', // JSON body containing action details
            type: ColumnType.JSON,
        },
        {
            name: 'is_invalid', // Indicates if the action is invalid
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
export const keysetTable: SqlTable = SqlTable.create('keyset')
    .setColumns(() => [
        {
            name       : 'keyset_id', // Unique identifier for the keyset
            type       : ColumnType.TEXT,
            primary    : true,
            uniqueIndex: true,
        },
        {
            name: 'type', // Type of keyset
            type: ColumnType.TEXT,
        },
        {
            name: 'name', // Name of the keyset
            type: ColumnType.TEXT,
        },
        {
            name: 'encryption', // Encryption method used
            type: ColumnType.TEXT,
        },
        {
            name: 'signature', // Signature method used
            type: ColumnType.TEXT,
        },
        {
            name: 'generation', // Generation number for the keyset
            type: ColumnType.INTEGER,
        },
        SqlColumnGenerator.createdAt(),
        SqlColumnGenerator.teamReference(),
    ]);

/**
 * Lockbox table definition
 * @type {SqlTable}
 */
export const lockboxTable: SqlTable = SqlTable.create('lockbox')
    .setColumns(() => [
        {
            name       : 'lockbox_id', // Unique identifier for the lockbox
            type       : ColumnType.TEXT,
            primary    : true,
            uniqueIndex: true,
        },
        {
            name   : 'action', // Foreign key reference to the action table
            type   : ColumnType.TEXT,
            target : actionTable,
            actions: {
                onDelete: UpdateDeleteAction.CASCADE,
            },
        },
        {
            name: 'encryption_public_key', // Public key for encryption
            type: ColumnType.TEXT,
        },
        {
            name   : 'recipient', // Recipient of the lockbox
            type   : ColumnType.TEXT,
            target : keysetTable,
            actions: {
                onDelete: UpdateDeleteAction.CASCADE,
            },
        },
        {
            name   : 'contents', // Contents of the lockbox
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
export const memberTable: SqlTable = SqlTable.create('member')
    .setColumns(() => [
        {
            name       : 'member_id', // Unique identifier for the member
            type       : ColumnType.TEXT,
            primary    : true,
            uniqueIndex: true,
        },
        {
            name: 'name', // Name of the member
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
export const roleTable: SqlTable = SqlTable.create('role')
    .setColumns(() => [
        {
            name       : 'role_id', // Unique identifier for the role
            type       : ColumnType.TEXT,
            primary    : true,
            uniqueIndex: true,
        },
        {
            name: 'name', // Name of the role
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
export const serverTable: SqlTable = SqlTable.create('server')
    .setColumns(() => [
        {
            name       : 'server_id', // Unique identifier for the server
            type       : ColumnType.TEXT,
            primary    : true,
            uniqueIndex: true,
        },
        {
            name: 'host', // Host address of the server
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
export const deviceTable: SqlTable = SqlTable.create('device')
    .setColumns(() => [
        {
            name       : 'device_id', // Unique identifier for the device
            type       : ColumnType.TEXT,
            primary    : true,
            uniqueIndex: true,
        },
        {
            name: 'device_info', // Information about the device
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
export const invitationTable: SqlTable = SqlTable.create('invitation')
    .setColumns(() => [
        {
            name       : 'invitation_id', // Unique identifier for the invitation
            type       : ColumnType.TEXT,
            primary    : true,
            uniqueIndex: true,
        },
        {
            name: 'public_key', // Public key for the invitation
            type: ColumnType.TEXT,
        },
        {
            name: 'expiration', // Expiration date of the invitation
            type: ColumnType.TIMESTAMP,
        },
        {
            name: 'max_uses', // Maximum number of uses for the invitation
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
export const messageTable: SqlTable = SqlTable.create('message')
    .setColumns(() => [
        {
            name   : 'channel_id', // Unique identifier for the channel where the message belongs
            type   : ColumnType.TEXT,
            primary: true,
        },
        {
            name   : 'message_id', // Unique identifier for the message within the channel
            type   : ColumnType.TEXT,
            primary: true,
        },
        {
            name: 'deleted', // Indicates whether the message has been deleted
            type: ColumnType.BOOLEAN,
        },
        {
            name: 'values', // Values associated with the message
            type: ColumnType.HSTORE,
        },
        {
            name: 'state', // The modification time for each field in the "values" column is stored
            type: ColumnType.HSTORE,
        },
        SqlColumnGenerator.updatedAt(),
        SqlColumnGenerator.createdAt(),
        SqlColumnGenerator.teamReference(),
        SqlColumnGenerator.userReference(),
    ])
    .setIndexes(({ name }) => [
        {
            name   : `${name}_uniq_idx`, // Unique index for the message
            columns: ['team_id', 'channel_id', 'message_id'],
            unique : true,
        },
        {
            name   : `${name}_values_idx`, // Index for the values of the message
            columns: ['values'],
            using  : 'GIN',
        },
        {
            name   : `${name}_state_idx`, // Index for the state of the message
            columns: ['state'],
            using  : 'GIN',
        },
    ]);
