import { SQLLiteTable } from './types';

/**
 * xxx table definition
 * @type {SQLLiteTable}
 */
export const xxxTable = SQLLiteTable.create('')
    .setColumns(() => [])
    .setConstraints(({ name }) => []);

/**
 * Team table definition
 * @type {SQLLiteTable}
 */
export const teamTable = SQLLiteTable.create('tg_team')
    .setColumns(() => [
        { name: 'id', type: 'TEXT', primary: true },
        { name: 'name', type: 'TEXT' },
    ])
    .setConstraints(({ name }) => [
        {
            definition: `create unique index ${name}_uniq_idx on ${name} (id)`,
        },
    ]);

/**
 * Action table definition
 * @type {SQLLiteTable}
 */
export const actionTable = SQLLiteTable.create('tg_action')
    .setColumns(() => [
        { name: 'is_invalid', type: 'INTEGER' },
        { name: 'hash', type: 'TEXT' },

        // { name: 'sender_public_key', type: 'TEXT' },
        // { name: 'recipient_public_key', type: 'TEXT' },

        // {
        //     name: 'body',
        //     table: SQLLiteTable.create('body')
        //         .setColumns(() => [
        //             { name: 'time', type: 'INTEGER' },
        //             { name: 'user_id', type: 'TEXT'},
        //             { name: 'type', type: 'TEXT' },
        //         ])
        // },
        // { name: 'encrypted_body', type: 'BLOB' },

        { name: 'type', type: 'TEXT' },
        { name: 'time', type: 'INTEGER' },
        { name: 'user_id', type: 'TEXT', target: memberTable },
        { name: 'team_id', type: 'TEXT', target: teamTable },
    ])
    .setConstraints(({ name }) => []);

/**
 * KeySet table definition
 * @type {SQLLiteTable}
 */
export const keysetTable = SQLLiteTable.create('tg_keyset')
    .setColumns(() => [
        { name: 'id', type: 'TEXT', primary: true },
        { name: 'team_id', type: 'TEXT', target: teamTable },
        { name: 'type', type: 'TEXT' },
        { name: 'name', type: 'TEXT' },
        { name: 'encryption', type: 'TEXT' },
        { name: 'signature', type: 'TEXT' },
        { name: 'generation', type: 'INTEGER' },
    ])
    .setConstraints(({ name }) => [
        {
            definition: `create unique index ${name}_uniq_idx on ${name} (id)`,
        },
    ]);

/**
 * Lockbox table definition
 * @type {SQLLiteTable}
 */
export const lockboxTable = SQLLiteTable.create('tg_lockbox')
    .setColumns(() => [

        /** Every action might include new lockboxes */
        {
            name: 'action',
            type: 'TEXT',
            target: actionTable,
            targetColumn: 'id',
            actions: {
                onDelete: 'cascade'
            }
        },

        /** The public key of the keypair used to encrypt this lockbox  */
        { name: 'encryption_public_key', type: 'TEXT' },

        /** Manifest for the keyset that can open this lockbox (the lockbox recipient's keys) */
        {
            name: 'recipient',
            type: 'TEXT',
            target: keysetTable,
            targetColumn: 'id',
            actions: {
                onDelete: 'cascade'
            }
        },

        /** Manifest for the keyset that is in this lockbox (the lockbox contents) */
        {
            name: 'contents',
            type: 'TEXT',
            target: keysetTable,
            targetColumn: 'id',
            actions: {
                onDelete: 'cascade'
            }
        },
    ])
    .setConstraints(({ name }) => [

    ]);

/**
 * Member table definition
 * @type {SQLLiteTable}
 */
export const memberTable = SQLLiteTable.create('tg_member')
    .setColumns(() => [
        { name: 'id', type: 'TEXT', primary: true },
        { name: 'team_id', type: 'TEXT' },
        { name: 'name', type: 'TEXT' },
        { name: 'keyset_id', type: 'TEXT' },
    ])
    .setConstraints(({ name }) => [
        {
            definition: `create unique index ${name}_uniq_idx on ${name} (id)`,
        },
    ]);

/**
 * Role table definition
 * @type {SQLLiteTable}
 */
export const roleTable = SQLLiteTable.create('tg_role')
    .setColumns(() => [
        { name: 'id', type: 'TEXT', primary: true },
        { name: 'team_id', type: 'TEXT', target: teamTable },
        { name: 'name', type: 'TEXT' },
    ])
    .setConstraints(({ name }) => [
        {
            definition: `create unique index ${name}_uniq_idx on ${name} (id)`,
        },
    ]);

/**
 * Server table definition
 * @type {SQLLiteTable}
 */
export const serverTable = SQLLiteTable.create('tg_server')
    .setColumns(() => [
        { name: 'id', type: 'TEXT', primary: true },
        { name: 'host', type: 'TEXT' },
        { name: 'keyset_id', type: 'TEXT', target: keysetTable },
    ])
    .setConstraints(({ name }) => []);

/**
 * Message table definition
 * @type {SQLLiteTable}
 */
export const messageTable = SQLLiteTable.create('tg_message')
    .setColumns(() => [
        { name: 'team_id', type: 'TEXT', primary: true, target: teamTable },
        { name: 'section', type: 'TEXT', primary: true },
        { name: 'node', type: 'TEXT', primary: true },
        { name: 'field', type: 'TEXT', primary: true },
        { name: 'state', type: 'INTEGER' },
        { name: 'value_is_empty', type: 'INTEGER' },
        { name: 'value_string', type: 'TEXT' },
        { name: 'value_bool', type: 'INTEGER' },
        { name: 'value_number', type: 'REAL' },
        { name: 'value_byte', type: 'BLOB' },
        { name: 'value_date', type: 'TEXT' },
        { name: 'deleted', type: 'INTEGER' },
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
    ]);
