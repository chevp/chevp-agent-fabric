import { describe, it, expect } from 'vitest';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { NexusServices } from '../src/app/nexus-services.js';
import { PermissionDeniedError, NotFoundError } from '../src/shared/errors.js';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, '..');

describe('Context Resolver (example project)', () => {
  it('resolves the minimal relevant context for "Implement the checkout button"', async () => {
    const services = NexusServices.fromRepoRoot(repoRoot);

    const context = await services.resolveContext({
      projectId: 'acme-app',
      task: 'Implement the checkout button',
      client: { id: 'copilot', type: 'coding-agent' },
    });

    expect(context.project.id).toBe('acme-app');

    // Skills: the project's checkout-ux skill and, via its dependsOn, the
    // global accessibility skill.
    expect(context.skills.map((s) => s.id).sort()).toEqual(['accessibility', 'checkout-ux']);

    // Behavior: the checkout-button state machine.
    expect(context.behaviors.map((b) => b.id)).toEqual(['checkout-button']);

    // Graph: checkout-button and its immediate neighborhood.
    expect(context.entities.map((e) => e.id).sort()).toEqual([
      'checkout-button',
      'checkout-summary',
      'payment-flow',
    ]);

    // Policies: whatever acme-app grants "copilot".
    expect(context.policies.some((p) => p.permissions.includes('read:project'))).toBe(true);

    // Never the entire project: unrelated skills must not leak in.
    expect(context.skills.map((s) => s.id)).not.toContain('code-style');
  });

  it('does not resolve context for a client without read:project permission', async () => {
    const services = NexusServices.fromRepoRoot(repoRoot);

    await expect(
      services.resolveContext({
        projectId: 'acme-app',
        task: 'Implement the checkout button',
        client: { id: 'no-such-client', type: 'coding-agent' },
      }),
    ).rejects.toBeInstanceOf(PermissionDeniedError);
  });

  it('throws NotFoundError for an unknown project', async () => {
    const services = NexusServices.fromRepoRoot(repoRoot);

    await expect(
      services.resolveContext({
        projectId: 'does-not-exist',
        task: 'anything',
        client: { id: 'copilot', type: 'coding-agent' },
      }),
    ).rejects.toBeInstanceOf(NotFoundError);
  });
});
