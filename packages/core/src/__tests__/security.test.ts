import { PermissionType, PermissionPolicy, Principal } from '../security';

describe('Security Module', () => {
  describe('PermissionType', () => {
    test('should accept valid permission types', () => {
      const validTypes: PermissionType[] = ['READ', 'PUT', 'REMOVE', 'ALL'];

      validTypes.forEach(type => {
        const permission: PermissionType = type;
        expect(permission).toBe(type);
      });
    });

    test('should cover all CRUD-like operations', () => {
      const types: PermissionType[] = ['READ', 'PUT', 'REMOVE', 'ALL'];

      expect(types).toContain('READ');
      expect(types).toContain('PUT');
      expect(types).toContain('REMOVE');
      expect(types).toContain('ALL');
      expect(types.length).toBe(4);
    });
  });

  describe('PermissionPolicy', () => {
    test('should create a valid permission policy with all fields', () => {
      const policy: PermissionPolicy = {
        role: 'admin',
        mapNamePattern: '*',
        actions: ['ALL'],
        allowedFields: ['id', 'name', 'email']
      };

      expect(policy.role).toBe('admin');
      expect(policy.mapNamePattern).toBe('*');
      expect(policy.actions).toContain('ALL');
      expect(policy.allowedFields).toEqual(['id', 'name', 'email']);
    });

    test('should create a valid permission policy without optional allowedFields', () => {
      const policy: PermissionPolicy = {
        role: 'viewer',
        mapNamePattern: 'public.*',
        actions: ['READ']
      };

      expect(policy.role).toBe('viewer');
      expect(policy.mapNamePattern).toBe('public.*');
      expect(policy.actions).toEqual(['READ']);
      expect(policy.allowedFields).toBeUndefined();
    });

    test('should support multiple actions in a single policy', () => {
      const policy: PermissionPolicy = {
        role: 'editor',
        mapNamePattern: 'documents.*',
        actions: ['READ', 'PUT']
      };

      expect(policy.actions).toHaveLength(2);
      expect(policy.actions).toContain('READ');
      expect(policy.actions).toContain('PUT');
    });

    test('should support wildcard patterns for mapName', () => {
      const policies: PermissionPolicy[] = [
        { role: 'admin', mapNamePattern: '*', actions: ['ALL'] },
        { role: 'user', mapNamePattern: 'users.*', actions: ['READ'] },
        { role: 'manager', mapNamePattern: 'public.*', actions: ['READ', 'PUT'] },
        { role: 'specific', mapNamePattern: 'users', actions: ['READ'] },
      ];

      expect(policies[0].mapNamePattern).toBe('*');
      expect(policies[1].mapNamePattern).toBe('users.*');
      expect(policies[2].mapNamePattern).toBe('public.*');
      expect(policies[3].mapNamePattern).toBe('users');
    });

    test('should handle empty allowedFields array', () => {
      const policy: PermissionPolicy = {
        role: 'restricted',
        mapNamePattern: 'sensitive',
        actions: ['READ'],
        allowedFields: []
      };

      expect(policy.allowedFields).toEqual([]);
      expect(policy.allowedFields).toHaveLength(0);
    });
  });

  describe('Principal', () => {
    test('should create a valid principal with required fields', () => {
      const principal: Principal = {
        userId: 'user-123',
        roles: ['admin', 'user']
      };

      expect(principal.userId).toBe('user-123');
      expect(principal.roles).toContain('admin');
      expect(principal.roles).toContain('user');
    });

    test('should support empty roles array', () => {
      const principal: Principal = {
        userId: 'guest-user',
        roles: []
      };

      expect(principal.userId).toBe('guest-user');
      expect(principal.roles).toEqual([]);
    });

    test('should support additional custom properties via index signature', () => {
      const principal: Principal = {
        userId: 'user-456',
        roles: ['user'],
        email: 'test@example.com',
        department: 'engineering',
        permissions: ['read', 'write'],
        metadata: { lastLogin: Date.now() }
      };

      expect(principal.userId).toBe('user-456');
      expect(principal.roles).toContain('user');
      expect(principal.email).toBe('test@example.com');
      expect(principal.department).toBe('engineering');
      expect(principal.permissions).toEqual(['read', 'write']);
      expect(principal.metadata).toHaveProperty('lastLogin');
    });

    test('should handle principal with single role', () => {
      const principal: Principal = {
        userId: 'single-role-user',
        roles: ['viewer']
      };

      expect(principal.roles).toHaveLength(1);
      expect(principal.roles[0]).toBe('viewer');
    });

    test('should handle principal with many roles', () => {
      const roles = ['admin', 'user', 'moderator', 'editor', 'reviewer'];
      const principal: Principal = {
        userId: 'multi-role-user',
        roles
      };

      expect(principal.roles).toHaveLength(5);
      roles.forEach(role => {
        expect(principal.roles).toContain(role);
      });
    });
  });

  describe('Permission Policy Matching Scenarios', () => {
    test('should support realistic admin policy configuration', () => {
      const adminPolicy: PermissionPolicy = {
        role: 'admin',
        mapNamePattern: '*',
        actions: ['ALL']
      };

      expect(adminPolicy.actions).toContain('ALL');
      expect(adminPolicy.mapNamePattern).toBe('*');
    });

    test('should support realistic read-only user policy', () => {
      const readOnlyPolicy: PermissionPolicy = {
        role: 'viewer',
        mapNamePattern: 'public.*',
        actions: ['READ'],
        allowedFields: ['title', 'description', 'createdAt']
      };

      expect(readOnlyPolicy.actions).toEqual(['READ']);
      expect(readOnlyPolicy.allowedFields).toBeDefined();
      expect(readOnlyPolicy.allowedFields!.length).toBe(3);
    });

    test('should support realistic editor policy with field restrictions', () => {
      const editorPolicy: PermissionPolicy = {
        role: 'editor',
        mapNamePattern: 'articles.*',
        actions: ['READ', 'PUT'],
        allowedFields: ['title', 'content', 'tags', 'updatedAt']
      };

      expect(editorPolicy.actions).toContain('READ');
      expect(editorPolicy.actions).toContain('PUT');
      expect(editorPolicy.actions).not.toContain('REMOVE');
      expect(editorPolicy.allowedFields).toContain('content');
    });
  });

  describe('Edge Cases', () => {
    test('should handle principal with UUID-style userId', () => {
      const principal: Principal = {
        userId: '550e8400-e29b-41d4-a716-446655440000',
        roles: ['user']
      };

      expect(principal.userId).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/);
    });

    test('should handle policy with complex mapNamePattern', () => {
      const policy: PermissionPolicy = {
        role: 'service',
        mapNamePattern: 'api.v1.users.*',
        actions: ['READ', 'PUT', 'REMOVE']
      };

      expect(policy.mapNamePattern).toBe('api.v1.users.*');
      expect(policy.actions).toHaveLength(3);
    });

    test('should handle principal with null-like custom properties', () => {
      const principal: Principal = {
        userId: 'test-user',
        roles: ['user'],
        nullableField: null,
        undefinedField: undefined,
        emptyString: ''
      };

      expect(principal.nullableField).toBeNull();
      expect(principal.undefinedField).toBeUndefined();
      expect(principal.emptyString).toBe('');
    });

    test('should handle policy with ALL action representing full access', () => {
      const fullAccessPolicy: PermissionPolicy = {
        role: 'superadmin',
        mapNamePattern: '*',
        actions: ['ALL']
      };

      // ALL should imply READ, PUT, REMOVE
      const allAction = fullAccessPolicy.actions.includes('ALL');
      expect(allAction).toBe(true);
    });
  });
});
