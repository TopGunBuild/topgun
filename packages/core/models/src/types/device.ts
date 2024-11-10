import { Keyset, KeysetWithSecrets } from "./keyset"
import { Identifiable } from "./utils"

/**
 * Base type containing common device information
 */
export interface DeviceInfo extends Identifiable {

    /** ID of the user who owns this device */
    userId: string

    /** Human-readable name for the device (e.g., "John's Laptop") */
    deviceName: string

    /** Optional additional device metadata (e.g., OS, browser version) */
    deviceInfo?: any

    /** Timestamp when the device was first registered */
    created?: number
}

/**
 * Represents a device with complete cryptographic information
 * Includes private keys and should only be stored locally
 */
export interface DeviceWithSecrets extends DeviceInfo {
    /** Complete set of device cryptographic keys (public + private) */
    keys: KeysetWithSecrets
}

/**
 * Represents a device's public information
 * Safe to transmit and store on server
 */
export interface Device extends DeviceInfo {
    /** Public cryptographic keys for the device */
    keys: Keyset
}

/**
 * Device type used during initial device registration
 * Omits userId as it's not yet associated with a user
 */
export type FirstUseDeviceWithSecrets = Omit<DeviceWithSecrets, 'userId'>

/**
 * Public device information during initial registration
 * Omits userId as it's not yet associated with a user
 */
export type FirstUseDevice = Omit<Device, 'userId'>