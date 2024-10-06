/**
 * KeyScope represents the scope of a keyset. For example:
 * - a user: `{type: USER, name: 'alice'}`
 * - a device: `{type: DEVICE, name: 'laptop'}`
 * - a role: `{type: ROLE, name: 'MANAGER'}`
 * - a single-use keyset: `{type: EPHEMERAL, name: EPHEMERAL}`
 */
export type KeyScope = {
    /** The apps are not limited to KeyType, as they will have their own types. */
    type: string;
    name: string;
};
