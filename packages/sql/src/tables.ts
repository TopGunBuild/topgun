import { SqlTable } from './sql-table';
import { ColumnType, UpdateDeleteAction } from './sql-column.ts';

/**
 * Team table definition
 * @type {SqlTable}
 */
export const teamTable = SqlTable.create('team')
    .setColumns(() => [
        {
            name   : 'id',
            type   : ColumnType.text,
            primary: true,
        },
        {
            name: 'name',
            type: ColumnType.text,
        },
    ])
    .setConstraints(({ name }) => [
        {
            definition: `create unique index ${name}_uniq_idx on ${name} (id)`,
        },
    ]);

/**
 * Action table definition
 * @type {SqlTable}
 */
export const actionTable = SqlTable.create('action')
    .setColumns(() => [
        {
            name   : 'id',
            type   : ColumnType.text,
            primary: true,
        },
        {
            name: 'type',
            type: ColumnType.text,
        },
        {
            name: 'encrypted_body',
            type: ColumnType.blob
        },
        {
            name: 'is_invalid',
            type: ColumnType.integer,
        },
        {
            name: 'hash',
            type: ColumnType.text,
        },
        {
            name: 'sender_public_key',
            type: ColumnType.text,
        },
        {
            name: 'recipient_public_key',
            type: ColumnType.text,
        },
        {
            name: 'time',
            type: ColumnType.integer,
        },
        {
            name   : 'user_id',
            type   : ColumnType.text,
            target : memberTable,
            actions: {
                onDelete: UpdateDeleteAction.cascade,
            },
        },
        {
            name   : 'team_id',
            type   : ColumnType.text,
            target : teamTable,
            actions: {
                onDelete: UpdateDeleteAction.cascade,
            },
        },
    ])
    .setConstraints(({ name }) => [
        {
            definition: `create unique index ${name}_uniq_idx on ${name} (id)`,
        },
        {
            definition: `create index ${name}_user_id_idx on ${name} (user_id)`,
        },
        {
            definition: `create index ${name}_team_id_idx on ${name} (team_id)`,
        },
    ]);

/**
 * Keyset table definition
 * @type {SqlTable}
 */
export const keysetTable = SqlTable.create('keyset')
    .setColumns(() => [
        {
            name   : 'id',
            type   : ColumnType.text,
            primary: true,
        },
        {
            name: 'type',
            type: ColumnType.text,
        },
        {
            name: 'name',
            type: ColumnType.text,
        },
        {
            name: 'encryption',
            type: ColumnType.text,
        },
        {
            name: 'signature',
            type: ColumnType.text,
        },
        {
            name: 'generation',
            type: ColumnType.integer,
        },
        {
            name   : 'team_id',
            type   : ColumnType.text,
            target : teamTable,
            actions: {
                onDelete: UpdateDeleteAction.cascade,
            },
        },
    ])
    .setConstraints(({ name }) => [
        {
            definition: `create unique index ${name}_uniq_idx on ${name} (id)`,
        },
        {
            definition: `create index ${name}_team_id_idx on ${name} (team_id)`,
        },
    ]);

/**
 * Lockbox table definition
 * @type {SqlTable}
 */
export const lockboxTable = SqlTable.create('lockbox')
    .setColumns(() => [
        {
            name   : 'id',
            type   : ColumnType.text,
            primary: true,
        },
        {
            name   : 'team_id',
            type   : ColumnType.text,
            target : teamTable,
            actions: {
                onDelete: UpdateDeleteAction.cascade,
            },
        },

        /** Every action might include new lockboxes */
        {
            name   : 'action',
            type   : ColumnType.text,
            target : actionTable,
            actions: {
                onDelete: UpdateDeleteAction.cascade,
            },
        },

        /** The public key of the keypair used to encrypt this lockbox  */
        {
            name: 'encryption_public_key',
            type: ColumnType.text,
        },

        /** Manifest for the keyset that can open this lockbox (the lockbox recipient's keys) */
        {
            name   : 'recipient',
            type   : ColumnType.text,
            target : keysetTable,
            actions: {
                onDelete: UpdateDeleteAction.cascade,
            },
        },

        /** Manifest for the keyset that is in this lockbox (the lockbox contents) */
        {
            name   : 'contents',
            type   : ColumnType.text,
            target : keysetTable,
            actions: {
                onDelete: UpdateDeleteAction.cascade,
            },
        },
    ])
    .setConstraints(({ name }) => [
        {
            definition: `create unique index ${name}_uniq_idx on ${name} (id)`,
        },
        {
            definition: `create index ${name}_team_id_idx on ${name} (team_id)`,
        },
    ]);

/**
 * Member table definition
 * @type {SqlTable}
 */
export const memberTable = SqlTable.create('member')
    .setColumns(() => [
        {
            name   : 'id',
            type   : ColumnType.text,
            primary: true,
        },
        {
            name: 'name',
            type: ColumnType.text,
        },
        {
            name  : 'keyset_id',
            type  : ColumnType.text,
            target: keysetTable,
        },
        {
            name   : 'team_id',
            type   : ColumnType.text,
            target : teamTable,
            actions: {
                onDelete: UpdateDeleteAction.cascade,
            },
        },
    ])
    .setConstraints(({ name }) => [
        {
            definition: `create unique index ${name}_uniq_idx on ${name} (id)`,
        },
        {
            definition: `create index ${name}_team_id_idx on ${name} (team_id)`,
        },
    ]);

/**
 * Role table definition
 * @type {SqlTable}
 */
export const roleTable = SqlTable.create('role')
    .setColumns(() => [
        {
            name   : 'id',
            type   : ColumnType.text,
            primary: true,
        },
        {
            name: 'name',
            type: ColumnType.text,
        },
        {
            name   : 'team_id',
            type   : ColumnType.text,
            target : teamTable,
            actions: {
                onDelete: UpdateDeleteAction.cascade,
            },
        },
    ])
    .setConstraints(({ name }) => [
        {
            definition: `create unique index ${name}_uniq_idx on ${name} (id)`,
        },
        {
            definition: `create index ${name}_team_id_idx on ${name} (team_id)`,
        },
    ]);

/**
 * Server table definition
 * @type {SqlTable}
 */
export const serverTable = SqlTable.create('server')
    .setColumns(() => [
        {
            name   : 'id',
            type   : ColumnType.text,
            primary: true,
        },
        {
            name: 'host',
            type: ColumnType.text,
        },
        {
            name  : 'keyset_id',
            type  : ColumnType.text,
            target: keysetTable,
        },
    ])
    .setConstraints(({ name }) => [
        {
            definition: `create unique index ${name}_uniq_idx on ${name} (id)`,
        },
    ]);

/**
 * Device table definition
 * @type {SqlTable}
 */
export const deviceTable = SqlTable.create('device')
    .setColumns(() => [
        {
            name   : 'device_id',
            type   : ColumnType.text,
            primary: true,
        },
        {
            name  : 'keyset_id',
            type  : ColumnType.text,
            target: keysetTable,
        },
        {
            name: 'device_info',
            type: ColumnType.text,
        },
        {
            name: 'created',
            type: ColumnType.datetime,
        },
        {
            name   : 'user_id',
            type   : ColumnType.text,
            target : memberTable,
            actions: {
                onDelete: UpdateDeleteAction.cascade,
            },
        },
        {
            name   : 'team_id',
            type   : ColumnType.text,
            target : teamTable,
            actions: {
                onDelete: UpdateDeleteAction.cascade,
            },
        },
    ])
    .setConstraints(({ name }) => [
        {
            definition: `create unique index ${name}_uniq_idx on ${name} (device_id)`,
        },
        {
            definition: `create index ${name}_user_id_idx on ${name} (user_id)`,
        },
        {
            definition: `create index ${name}_team_id_idx on ${name} (team_id)`,
        },
    ]);

/**
 * Invitation table definition
 * @type {SqlTable}
 */
export const invitationTable = SqlTable.create('invitation')
    .setColumns(() => [
        {
            name   : 'id',
            type   : ColumnType.text,
            primary: true,
        },
        {
            name: 'public_key',
            type: ColumnType.text,
        },
        {
            name  : 'expiration',
            type  : ColumnType.datetime,
        },
        {
            name  : 'max_uses',
            type  : ColumnType.integer,
        },
        {
            name   : 'user_id',
            type   : ColumnType.text,
            target : memberTable,
            actions: {
                onDelete: UpdateDeleteAction.cascade,
            },
        },
        {
            name   : 'team_id',
            type   : ColumnType.text,
            target : teamTable,
            actions: {
                onDelete: UpdateDeleteAction.cascade,
            },
        },
    ])
    .setConstraints(({ name }) => [
        {
            definition: `create unique index ${name}_uniq_idx on ${name} (id)`,
        },
        {
            definition: `create index ${name}_user_id_idx on ${name} (user_id)`,
        },
        {
            definition: `create index ${name}_team_id_idx on ${name} (team_id)`,
        },
    ]);

/**
 * Message table definition
 * @type {SqlTable}
 */
export const messageTable = SqlTable.create('message')
    .setColumns(() => [
        {
            name   : 'section',
            type   : ColumnType.text,
            primary: true,
        },
        {
            name   : 'node',
            type   : ColumnType.text,
            primary: true,
        },
        {
            name   : 'field',
            type   : ColumnType.text,
            primary: true,
        },
        {
            name: 'state',
            type: ColumnType.bigint,
        },
        {
            name: 'value_is_empty',
            type: ColumnType.boolean,
        },
        {
            name: 'value_string',
            type: ColumnType.text,
        },
        {
            name: 'value_bool',
            type: ColumnType.boolean,
        },
        {
            name: 'value_number',
            type: ColumnType.numeric,
        },
        {
            name: 'value_byte',
            type: ColumnType.blob,
        },
        {
            name: 'value_date',
            type: ColumnType.datetime,
        },
        {
            name: 'deleted',
            type: ColumnType.boolean,
        },
        {
            name   : 'team_id',
            type   : ColumnType.text,
            target : teamTable,
            actions: {
                onDelete: UpdateDeleteAction.cascade,
            },
        },
    ])
    .setConstraints(({ name }) => [
        {
            definition: `create unique index ${name}_uniq_idx on ${name} (team_id, section, node, field)`,
        },
        {
            definition: `create index ${name}_section_node_idx on ${name} (section, node)`,
        },
        {
            definition: `create index ${name}_section_idx on ${name} (section)`,
        },
        {
            definition: `create index ${name}_deleted_idx on ${name} (deleted);`,
        },
        {
            definition: `create index ${name}_team_id_idx on ${name} (team_id)`,
        },
    ]);
