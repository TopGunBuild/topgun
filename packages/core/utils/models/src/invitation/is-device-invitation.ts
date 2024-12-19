import { DeviceInvitation, TeamInvitation } from "@topgunbuild/models"

/**
 * Checks if an invitation is a device invitation
 * @param invitation - The invitation to check
 * @returns True if the invitation is a device invitation, false otherwise
 */
export const isDeviceInvitation = (invitation: TeamInvitation): invitation is DeviceInvitation => {
    return 'userId' in invitation;
}
