import { DevicePrivateInfo, DevicePublicInfo } from "@topgunbuild/models"
import { convertToPublicKeyset } from "../keyset/convert-keyset"

/**
 * Converts a device with secrets to a public device object
 */
export const convertToPublicDevice = (device: DevicePrivateInfo, teamId: string): DevicePublicInfo => {
  if (!device?.$id || !device?.userId || !device?.deviceName || !device?.keys) {
    throw new Error('Invalid device object: missing required properties')
  }

  return {
    ...device,
    keys: convertToPublicKeyset(device.keys),
    teamId: teamId,
  }
}
  