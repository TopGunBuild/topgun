import { PermissionPolicy, Principal, PermissionType } from '@topgunbuild/core';
import { logger } from '../utils/logger';

export class SecurityManager {
  private policies: PermissionPolicy[] = [];

  constructor(policies: PermissionPolicy[] = []) {
    this.policies = policies;
  }

  public addPolicy(policy: PermissionPolicy) {
    this.policies.push(policy);
  }

  public checkPermission(principal: Principal, mapName: string, action: PermissionType): boolean {
    // 1. Superuser check (optional, but good practice)
    if (principal.roles.includes('ADMIN')) {
      return true;
    }

    // 2. System Map Protection
    if (mapName.startsWith('$sys/')) {
      logger.warn({ userId: principal.userId, mapName }, 'Access Denied: System Map requires ADMIN role');
      return false;
    }

    // 2. Iterate policies to find a match
    for (const policy of this.policies) {
      const hasRole = this.hasRole(principal, policy.role);
      const matchesMap = this.matchesMap(mapName, policy.mapNamePattern, principal);

      if (hasRole && matchesMap) {
        if (policy.actions.includes('ALL') || policy.actions.includes(action)) {
          return true;
        }
      } else {
        // Trace why it failed matching if needed (verbose)
        // logger.trace({ policy, hasRole, matchesMap, mapName, user: principal.userId }, 'Policy mismatch');
      }
    }

    logger.warn({
      userId: principal.userId,
      roles: principal.roles,
      mapName,
      action,
      policyCount: this.policies.length
    }, 'SecurityManager: Access Denied - No matching policy found');

    return false;
  }

  public filterObject(object: any, principal: Principal, mapName: string): any {
    if (!object || typeof object !== 'object') return object;
    if (principal.roles.includes('ADMIN')) return object;

    if (Array.isArray(object)) {
      return object.map(item => this.filterObject(item, principal, mapName));
    }

    let allowedFields: Set<string> | null = null;
    let accessGranted = false;

    for (const policy of this.policies) {
      if (this.hasRole(principal, policy.role) && this.matchesMap(mapName, policy.mapNamePattern, principal)) {
        if (policy.actions.includes('ALL') || policy.actions.includes('READ')) {
          accessGranted = true;

          // If any policy allows everything, return immediately
          if (!policy.allowedFields || policy.allowedFields.length === 0 || policy.allowedFields.includes('*')) {
            return object;
          }

          if (allowedFields === null) allowedFields = new Set();
          policy.allowedFields.forEach(f => allowedFields!.add(f));
        }
      }
    }

    if (!accessGranted) return null;
    if (allowedFields === null) return object; // Should have returned above, but as fallback

    const filtered: any = {};
    for (const key of Object.keys(object)) {
      if (allowedFields.has(key)) {
        filtered[key] = object[key];
      }
    }
    return filtered;
  }

  private hasRole(principal: Principal, role: string): boolean {
    return principal.roles.includes(role);
  }

  private matchesMap(mapName: string, pattern: string, principal?: Principal): boolean {
    // Dynamic substitution for {userId}
    let finalPattern = pattern;
    if (pattern.includes('{userId}') && principal) {
      finalPattern = pattern.replace('{userId}', principal.userId);
    }

    if (finalPattern === '*') return true;
    if (finalPattern === mapName) return true;

    if (finalPattern.endsWith('*')) {
      const prefix = finalPattern.slice(0, -1);
      return mapName.startsWith(prefix);
    }

    return false;
  }
}
