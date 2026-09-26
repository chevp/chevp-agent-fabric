import { PermissionDeniedError } from '../shared/errors.js';
import type { ClientInfo } from '../shared/types.js';
import type { PolicyEngine } from './policy-engine.js';

/** Throws PermissionDeniedError unless `client` holds `permission` for `projectId`. */
export async function requirePermission(
  policyEngine: PolicyEngine,
  client: ClientInfo,
  permission: string,
  projectId?: string,
): Promise<void> {
  const allowed = await policyEngine.hasPermission(client.id, permission, projectId);
  if (!allowed) {
    throw new PermissionDeniedError(client.id, permission);
  }
}
