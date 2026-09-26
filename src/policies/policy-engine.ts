import path from 'node:path';
import { listFilesRecursive, readYamlFile } from '../shared/fs-utils.js';
import { PolicyFileSchema } from '../shared/schemas.js';
import { ValidationError } from '../shared/errors.js';
import type { PolicyRule } from '../shared/types.js';

/** Storage abstraction for policies. Filesystem/Git-backed in V1. */
export interface PolicyStore {
  listGlobalPolicies(): Promise<PolicyRule[]>;
  listProjectPolicies(projectId: string): Promise<PolicyRule[]>;
}

export class FsPolicyStore implements PolicyStore {
  constructor(
    private readonly globalRoot: string,
    private readonly projectsRoot: string,
  ) {}

  async listGlobalPolicies(): Promise<PolicyRule[]> {
    const files = await listFilesRecursive(path.join(this.globalRoot, 'policies'), [
      '.yaml',
      '.yml',
    ]);
    return Promise.all(files.map((f) => loadPolicyFile(f, undefined)));
  }

  async listProjectPolicies(projectId: string): Promise<PolicyRule[]> {
    const dir = path.join(this.projectsRoot, projectId, 'policies');
    const files = await listFilesRecursive(dir, ['.yaml', '.yml']);
    return Promise.all(files.map((f) => loadPolicyFile(f, projectId)));
  }
}

async function loadPolicyFile(
  filePath: string,
  projectId: string | undefined,
): Promise<PolicyRule> {
  const raw = await readYamlFile(filePath);
  const result = PolicyFileSchema.safeParse(raw);
  if (!result.success) {
    throw new ValidationError(`Invalid policy at ${filePath}`, result.error.issues);
  }
  return { ...result.data, projectId, sourcePath: filePath };
}

/**
 * The Policy Engine is independent from MCP: it is a plain domain service
 * that resolves and evaluates permissions for a (client, project) pair.
 * Project-scoped rules for a client extend/override the client's global
 * rules for the same project context.
 */
export class PolicyEngine {
  constructor(private readonly store: PolicyStore) {}

  async getPoliciesForClient(clientId: string, projectId?: string): Promise<PolicyRule[]> {
    const global = (await this.store.listGlobalPolicies()).filter((p) => p.client === clientId);
    if (!projectId) return global;
    const project = (await this.store.listProjectPolicies(projectId)).filter(
      (p) => p.client === clientId,
    );
    return project.length > 0 ? project : global;
  }

  async getPermissions(clientId: string, projectId?: string): Promise<Set<string>> {
    const rules = await this.getPoliciesForClient(clientId, projectId);
    return new Set(rules.flatMap((r) => r.permissions));
  }

  async hasPermission(clientId: string, permission: string, projectId?: string): Promise<boolean> {
    const permissions = await this.getPermissions(clientId, projectId);
    return permissions.has(permission);
  }
}
