import { DevicePublicInfo } from "./device"
import { KeysetPublicInfo } from "./keyset"
import { Identifiable } from "./utils"

/**
 * Represents a member within a team
 * A member is a user with associated roles, devices, and cryptographic keys
 */
export interface MemberInfo extends Identifiable {
    /**
     * The team this member belongs to
     */
    teamId: string;

    /** 
     * Member's display name or email address
     * Must be unique across the system but not used as primary key
     * Used for human-readable identification and external system integration
     */
    userName?: string
  
    /** 
     * Member's public cryptographic keys
     * Used for encryption and signature verification
     */
    keys?: KeysetPublicInfo
  
    /** 
     * Array of role names assigned to this member
     * Controls member's permissions within the team
     */
    roles?: string[]
  
    /** 
     * List of devices registered to this member
     * Each device contains its own public keys and metadata
     */
    devices?: DevicePublicInfo[]
}
