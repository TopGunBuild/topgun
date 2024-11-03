import { DeviceWithSecrets } from "./device"
import { ServerWithSecrets } from "./server"
import { UserWithSecrets } from "./user"

/**
 * Union type representing either a user context or server context
 * Used to handle different authentication scenarios in the system
 */
export type LocalContext = LocalUserContext | LocalServerContext

/**
 * Represents the local context for a user session
 * Contains sensitive information and should only be stored securely
 */
export type LocalUserContext = {
    /** Complete user information including private keys */
    user: UserWithSecrets

    /** Device information including private keys */
    device: DeviceWithSecrets

    /** Optional client information */
    client?: Client
}

/**
 * Represents the local context for a server instance
 * Contains sensitive server information and should be stored securely
 */
export type LocalServerContext = {
    /** Server information including private keys */
    server: ServerWithSecrets

    /** Optional client information */
    client?: Client
}

/**
 * Represents client application information
 * Used for tracking and versioning purposes
 */
export type Client = {
    /** Name of the client application */
    name: string

    /** Version string of the client application */
    version: string
}