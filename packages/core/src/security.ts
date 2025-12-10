export type PermissionType = 'READ' | 'PUT' | 'REMOVE' | 'ALL';

export interface PermissionPolicy {
  role: string;
  mapNamePattern: string; // e.g., "users", "public.*", "*"
  actions: PermissionType[];
  allowedFields?: string[];
}

export interface Principal {
    userId: string;
    roles: string[];
    [key: string]: any;
}

