import { DeviceWithSecrets, Device } from "../../../../models/dist"
import { convertToPublicKeyset } from "../keyset/convert-keyset"

/**
 * Converts a device with secrets to a public device object
 */
export const convertToPublicDevice = (device: DeviceWithSecrets): Device => {
  if (!device?.$id || !device?.userId || !device?.deviceName || !device?.keys) {
    throw new Error('Invalid device object: missing required properties')
  }

  return {
    ...device,
    keys: convertToPublicKeyset(device.keys),
  }
}
  