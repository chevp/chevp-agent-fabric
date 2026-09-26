import { describe, it, expect, afterEach } from 'vitest';
import path from 'node:path';
import { FsPolicyStore, PolicyEngine } from '../src/policies/policy-engine.js';
import { requirePermission } from '../src/policies/permission-checker.js';
import { PermissionDeniedError } from '../src/shared/errors.js';
import { createTmpRepo, writeFile } from './helpers/tmp-repo.js';

describe('PolicyEngine', () => {
  let cleanup: (() => Promise<void>) | undefined;
  afterEach(async () => {
    await cleanup?.();
    cleanup = undefined;
  });

  it('grants permissions declared for a client', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    await writeFile(
      path.join(repo.projectsRoot, 'acme-app', 'policies', 'copilot.yaml'),
      'client: copilot\npermissions:\n  - read:project\n  - write:code-reference\n',
    );

    const engine = new PolicyEngine(new FsPolicyStore(repo.globalRoot, repo.projectsRoot));

    expect(await engine.hasPermission('copilot', 'read:project', 'acme-app')).toBe(true);
    expect(await engine.hasPermission('copilot', 'write:decision', 'acme-app')).toBe(false);
  });

  it('falls back to global policy when no project-specific rule exists for a client', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    await writeFile(
      path.join(repo.globalRoot, 'policies', 'ide-agent.yaml'),
      'client: ide-agent\npermissions:\n  - read:project\n',
    );

    const engine = new PolicyEngine(new FsPolicyStore(repo.globalRoot, repo.projectsRoot));

    expect(await engine.hasPermission('ide-agent', 'read:project', 'acme-app')).toBe(true);
  });

  it('project-scoped rules for a client override its global rules', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    await writeFile(
      path.join(repo.globalRoot, 'policies', 'copilot.yaml'),
      'client: copilot\npermissions:\n  - read:project\n',
    );
    await writeFile(
      path.join(repo.projectsRoot, 'acme-app', 'policies', 'copilot.yaml'),
      'client: copilot\npermissions:\n  - read:project\n  - write:code-reference\n',
    );

    const engine = new PolicyEngine(new FsPolicyStore(repo.globalRoot, repo.projectsRoot));
    const permissions = await engine.getPermissions('copilot', 'acme-app');

    expect([...permissions].sort()).toEqual(['read:project', 'write:code-reference']);
  });

  it('requirePermission throws PermissionDeniedError when unauthorized', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    const engine = new PolicyEngine(new FsPolicyStore(repo.globalRoot, repo.projectsRoot));

    await expect(
      requirePermission(engine, { id: 'unknown-client', type: 'coding-agent' }, 'read:project'),
    ).rejects.toBeInstanceOf(PermissionDeniedError);
  });
});
