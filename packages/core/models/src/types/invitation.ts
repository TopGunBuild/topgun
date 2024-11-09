import { Identifiable, type UnixTimestamp } from './utils';

/**
 * Represents a public invitation record in the signature chain
 * Created when:
 * 1. An admin (Alice) invites a new user (Bob)
 * 2. A user's device invites another device (e.g., Bob's laptop inviting Bob's phone)
 */
export interface Invitation extends Identifiable {
    /** Unique identifier for the invitation, encoded in Base58 */
    $id: string

    /** 
     * Public signing key derived from the secret invitation key
     * Used to verify the invitation proof during acceptance
     */
    publicKey: string

    /** 
     * Timestamp when the invitation expires
     * Value of 0 indicates no expiration
     */
    expiration: UnixTimestamp

    /** 
     * Maximum number of times this invitation can be used
     * Value of 0 indicates unlimited uses
     */
    maxUses: number

    /** 
     * Optional user ID for device invitations
     * Only present when inviting additional devices for an existing user
     */
    userId?: string
}

/**
 * Represents the current state of an invitation in the Team state
 * Extends the base Invitation type with usage tracking and revocation status
 */
export interface InvitationState extends Invitation {
    /** Counter tracking how many times this invitation has been used */
    uses: number

    /** 
     * Indicates if the invitation was revoked
     * Once true, the invitation can no longer be used even if not expired
     */
    revoked: boolean
}

/**
 * Represents the proof document presented by an invitee during first connection
 * Used to verify the legitimacy of the invitation claim
 */
export interface ProofOfInvitation extends Identifiable {
    /** 
     * Matches the id of the original invitation
     * Used to look up the corresponding invitation record
     */
    $id: string

    /** 
     * Cryptographic signature proving invitation ownership
     * Signs the combination of userId and invitation id using the derived private key
     */
    signature: string
}
