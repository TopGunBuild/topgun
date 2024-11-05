import { 
  Device, 
  DeviceWithSecrets, 
  Member, 
  Server, 
  ServerWithSecrets, 
  User, 
  UserWithSecrets 
} from "@topgunbuild/types"

/**
 * Utility functions to transform Server objects into other entity types
 */
export const castServer = {
  /**
   * Converts a Server to a Member entity
   * @param server - Server to convert
   * @returns Member object with server properties
   */
  toMember: (server: Server): Member => ({
    $id: server.host,
    userName: server.host,
    keys: server.keys,
    roles: [], // Initialize with empty roles array
  }),

  /**
   * Converts a Server to a User entity, preserving secrets if present
   * @param server - Server or ServerWithSecrets to convert
   * @returns User or UserWithSecrets depending on input type
   */
  toUser: <T extends Server | ServerWithSecrets>(server: T) => ({
    $id: server.host,
    userName: server.host,
    keys: server.keys,
  } as T extends Server ? User : UserWithSecrets),

  /**
   * Converts a Server to a Device entity, preserving secrets if present
   * @param server - Server or ServerWithSecrets to convert
   * @returns Device or DeviceWithSecrets depending on input type
   */
  toDevice: <T extends Server | ServerWithSecrets>(server: T) => ({
    userId: server.host,
    deviceName: server.host,
    $id: server.host,
    keys: server.keys,
  } as T extends Server ? Device : DeviceWithSecrets),
}