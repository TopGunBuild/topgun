import { TeamInvitation } from "@topgunbuild/models";

/**
 * Parses an invitation code into a TeamInvitation
 * @param invitationCode - The invitation code to parse
 * @returns The parsed TeamInvitation
 */
export const parseInvitationCode = (invitationCode: string): TeamInvitation => {
    const teamId = invitationCode.slice(0, 12);
    const invitationSeed = invitationCode.slice(12); // the rest of the code is the invitation seed
    return { teamId, invitationSeed }
}