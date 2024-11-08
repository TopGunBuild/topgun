/**
 * Normalizes the invitation key by removing non-alphanumeric characters.
 * This allows the key to be displayed in blocks (e.g., '4kgd 5mwq 5z4f mfwq')
 * or made URL-safe (e.g., '4kgd+5mwq+5z4f+mfwq').
 */
export const normalizeInvitationKey = (key: string): string => 
    key.replace(/[^a-z\d]/gi, '')