import { Identifiable } from "./utils"

/**
 * Represents a mapping of permission names to their granted status
 * Example: { 'read': true, 'write': false, 'admin': true }
 */
export type PermissionsMap = Record<string, boolean>

/**
 * Represents a role in the system with its associated permissions
 */
export interface RoleInfo extends Identifiable {
    /** 
     * Unique identifier for the role
     * Examples: 'admin', 'user', 'moderator'
     */
    roleName: string

    /** 
     * Optional map of permissions assigned to this role
     * If undefined, role has no explicit permissions
     */
    permissions?: string[]
}
