import { createKeyset } from "../keyset/create-keyset"
import { KeyType, DevicePrivateInfo } from "@topgunbuild/models"
import { randomKey } from "@topgunbuild/crypto"
import { randomId } from "@topgunbuild/common"

export interface CreateDeviceParams {
  userId: string
  deviceName: string
  deviceInfo?: Record<string, unknown>
  created?: number
  seed?: string
  teamId?: string
}

/**
 * Creates a new device with associated keys and metadata
 * @param params - Device creation parameters
 * @param params.userId - ID of the user who owns this device
 * @param params.deviceName - Human-readable name for the device
 * @param params.deviceInfo - Optional metadata about the device
 * @param params.created - Optional timestamp of device creation
 * @param params.seed - Optional seed for key generation
 * @param params.teamId - Optional team ID to associate with the device
 * @returns DevicePrivateInfo containing device details and keys
 * @throws Error if required parameters are invalid
 */
export const createDevice = ({
  userId,
  deviceName,
  deviceInfo = {},
  created = Date.now(),
  seed = randomKey(),
  teamId,
}: CreateDeviceParams): DevicePrivateInfo => {
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
      deviceInfo: JSON.stringify(deviceInfo),
      teamId,
    }
  } catch (error: unknown) {
    const message = error instanceof Error ? error.message : String(error)
    throw new Error(`Failed to create device: ${message}`)
  }
}