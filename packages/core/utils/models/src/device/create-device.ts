import { createKeyset } from "../keyset/create-keyset"
import { UnixTimestamp, KeyType, DeviceWithSecrets } from "../../../../models/dist"
import { randomKey } from "@topgunbuild/crypto"
import { randomId } from "../../../common/dist"

export interface CreateDeviceParams {
  userId: string
  deviceName: string
  deviceInfo?: Record<string, unknown>
  created?: UnixTimestamp
  seed?: string
}

/**
 * Creates a new device with associated keys and metadata
 * @param params - Device creation parameters
 * @param params.userId - ID of the user who owns this device
 * @param params.deviceName - Human-readable name for the device
 * @param params.deviceInfo - Optional metadata about the device
 * @param params.created - Optional timestamp of device creation
 * @param params.seed - Optional seed for key generation
 * @returns DeviceWithSecrets containing device details and keys
 * @throws Error if required parameters are invalid
 */
export const createDevice = ({
  userId,
  deviceName,
  deviceInfo = {},
  created = Date.now() as UnixTimestamp,
  seed = randomKey(),
}: CreateDeviceParams): DeviceWithSecrets => {
  // Input validation
  if (!userId?.trim()) {
    throw new Error('User ID is required')
  }
  if (!deviceName?.trim()) {
    throw new Error('Device name is required')
  }

  try {
    const deviceId = randomId()
    const keys = createKeyset(
      { 
        type: KeyType.DEVICE, 
        name: deviceId 
      }, 
      seed
    )

    return {
      $id: deviceId,
      userId: userId.trim(),
      deviceName: deviceName.trim(),
      keys,
      created,
      deviceInfo
    }
  } catch (error: unknown) {
    const message = error instanceof Error ? error.message : String(error)
    throw new Error(`Failed to create device: ${message}`)
  }
}