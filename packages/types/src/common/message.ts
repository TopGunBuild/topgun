import { RowCollection } from './collection';

/**
 * This interface can encapsulate the metadata associated with a field,
 * including fieldName, teamId, channelId, messageId
 */
export interface FieldMetadata
{
    teamId: string;
    channelId: string;
    messageId: string;
    fieldName: string;
}

/**
 * This interface can provide details about the field,
 * and the timestamp of the last update
 */
export interface FieldUpdateInfo extends FieldMetadata
{
    state: bigint;
}

/**
 * This interface represents a single row in the database, containing all
 * information about a field, its value, and deletion information.
 */
export interface MessageFieldRow extends FieldUpdateInfo
{
    value_is_empty: number|boolean;
    value_string?: string;
    value_bool?: number|boolean;
    value_number?: number;
    value_byte?: Uint8Array;
    value_date?: number|string;
    deleted?: number;
}

/**
 * This interface represents the data types that can be assigned to
 * a message field.
 */
export type MessageFieldValue = boolean|string|number|Uint8Array|null;

/**
 * This interface represents a single database row containing
 * all message information and field values.
 */
export type MessageRow = Record<string, MessageFieldValue>;

/**
 * This interface represents a collection of MessageRow instances,
 * providing information about the presence of previous and next pages.
 */
export interface MessageRowCollection extends RowCollection<MessageRow>
{
}
