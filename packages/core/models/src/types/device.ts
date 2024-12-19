import { KeysetPublicInfo, KeysetPrivateInfo } from "./keyset"
import { Identifiable } from "./utils"

/**
 * Base interface containing common device information
 */
export interface DeviceInfo extends Identifiable {
    /** ID of the team that owns this device */
    teamId: string;

    /** ID of the user who owns this device */
    userId: string;

    /** Human-readable name for the device (e.g., "John's Laptop") */
    deviceName: string;

    /** Timestamp when the device was first registered (Unix timestamp in ms) */
    created: number;

    /** Device information */
    deviceInfo?: string;
}

/**
 * Represents a device with complete cryptographic information.
 * Includes private keys and should only be stored locally.
 */
export interface DevicePrivateInfo extends DeviceInfo {
    /** Complete set of device cryptographic keys (public + private) */
    keys: KeysetPrivateInfo;
}

/**
 * Represents a device's public information.
 * Safe to transmit and store on server.
 */
export interface DevicePublicInfo extends DeviceInfo {
    /** Public cryptographic keys for the device */
    keys: KeysetPublicInfo;
}

/**
 * Device type used during initial device registration.
 * Omits userId as it's not yet associated with a user.
 */
export type DeviceRegistrationPrivate = Omit<DevicePrivateInfo, 'userId'>;

/**
 * Public device information during initial registration.
 * Omits userId as it's not yet associated with a user.
 */
export type DeviceRegistrationPublic = Omit<DevicePublicInfo, 'userId'>;