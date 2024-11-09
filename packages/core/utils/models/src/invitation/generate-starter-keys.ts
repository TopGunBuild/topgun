import { normalizeInvitationKey } from "./normalize"
import { createKeyset } from "../keyset/create-keyset"
import { EPHEMERAL_SCOPE } from "@topgunbuild/models"

/**
 * Generates the initial keyset for the invitee.
 * These are temporary keys used only during the invitation process.
 * Once admitted, the invitee will generate their own permanent keys.
 */
export const generateInviteeStarterKeys = (seed: string) => {
    const normalizedSeed = normalizeInvitationKey(seed)
    return createKeyset(EPHEMERAL_SCOPE, normalizedSeed)
}