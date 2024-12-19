import { 
  DevicePublicInfo, 
  DevicePrivateInfo,  
  MemberInfo,
  Server, 
  ServerPrivateInfo, 
  UserPublicInfo, 
  UserPrivateInfo 
} from "@topgunbuild/models"

/**
 * Utility functions to transform Server objects into other entity types
 */
export const castServer = {
  /**
   * Converts a Server to a Member entity
   * @param server - Server to convert
   * @returns Member object with server properties
   */
  toMember: (server: Server, teamId: string): MemberInfo => ({
    $id: server.host,
    teamId: teamId,
    userName: server.host,
    keys: server.keys,
    roles: [], // Initialize with empty roles array
  }),

  /**
   * Converts a Server to a User entity, preserving secrets if present
   * @param server - Server or ServerWithSecrets to convert
   * @returns User or UserWithSecrets depending on input type
   */
  toUser: <T extends Server | ServerPrivateInfo>(server: T) => ({
    $id: server.host,
    userName: server.host,
    keys: server.keys,
  } as T extends Server ? UserPublicInfo : UserPrivateInfo),

  /**
   * Converts a Server to a Device entity, preserving secrets if present
   * @param server - Server or ServerWithSecrets to convert
   * @returns Device or DeviceWithSecrets depending on input type
   */
  toDevice: <T extends Server | ServerPrivateInfo>(server: T) => ({
    userId: server.host,
    deviceName: server.host,
    $id: server.host,
    keys: server.keys,
  } as T extends Server ? DevicePublicInfo : DevicePrivateInfo),
}