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
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- index signature allows additional JWT claim fields (e.g. email, metadata) whose shape varies by auth provider
  [key: string]: any;
}
