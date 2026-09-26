import { describe, it, expect } from 'vitest';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js';
import { NexusServices } from '../src/app/nexus-services.js';
import { createNexusMcpServer } from '../src/server/mcp-server.js';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, '..');

/**
 * End-to-end: Task -> Context Resolver -> Skills -> Behavior -> Graph ->
 * Policy -> MCP response, exactly as described in the architecture doc's
 * "example end-to-end request".
 */
describe('End-to-end: task through the Control Plane over MCP', () => {
  it('creates the checkout flow according to the current UX rules', async () => {
    const services = NexusServices.fromRepoRoot(repoRoot);
    const server = createNexusMcpServer(services);
    const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
    const client = new Client({ name: 'copilot', version: '0.0.0' });
    await Promise.all([client.connect(clientTransport), server.connect(serverTransport)]);

    const client_ = { id: 'copilot', type: 'coding-agent' };

    // 1 & 2: identify project + client are inputs to the single resolve_context call.
    const contextResult = await client.callTool({
      name: 'resolve_context',
      arguments: {
        projectId: 'acme-app',
        task: 'Create the checkout flow according to our current UX rules.',
        client: client_,
      },
    });
    expect(contextResult.isError).toBeFalsy();
    const context = JSON.parse((contextResult.content as { text: string }[])[0]!.text);

    // 3: relevant graph entities were resolved, not the whole graph.
    expect(context.entities.map((e: { id: string }) => e.id)).toEqual(
      expect.arrayContaining(['checkout-button', 'payment-flow', 'checkout-summary']),
    );

    // 4: the relevant behavior specification was resolved.
    const behaviorIds = context.behaviors.map((b: { id: string }) => b.id);
    expect(behaviorIds).toContain('checkout-button');
    const checkoutBehavior = context.behaviors.find(
      (b: { id: string }) => b.id === 'checkout-button',
    );
    expect(checkoutBehavior.transitions).toEqual(
      expect.arrayContaining([{ from: 'idle', event: 'submit', to: 'loading' }]),
    );

    // 5: applicable skills were resolved, including the dependency closure.
    expect(context.skills.map((s: { id: string }) => s.id).sort()).toEqual([
      'accessibility',
      'checkout-ux',
    ]);

    // 6: applicable policies for this client were resolved.
    expect(
      context.policies.some((p: { permissions: string[] }) =>
        p.permissions.includes('read:project'),
      ),
    ).toBe(true);

    // The same client's actual permission grant is independently verifiable
    // through the policy tools (policy engine is independent of MCP/context).
    const permissionResult = await client.callTool({
      name: 'check_permission',
      arguments: { client: client_, permission: 'read:project', projectId: 'acme-app' },
    });
    const permissionPayload = JSON.parse((permissionResult.content as { text: string }[])[0]!.text);
    expect(permissionPayload.allowed).toBe(true);

    const writePermissionResult = await client.callTool({
      name: 'check_permission',
      arguments: { client: client_, permission: 'write:behavior', projectId: 'acme-app' },
    });
    const writePermissionPayload = JSON.parse(
      (writePermissionResult.content as { text: string }[])[0]!.text,
    );
    expect(writePermissionPayload.allowed).toBe(false);

    // 7 & 8: a single, minimal, structured context package came back over MCP.
    expect(Object.keys(context).sort()).toEqual(
      [
        'behaviors',
        'contextDocuments',
        'entities',
        'policies',
        'project',
        'relations',
        'skills',
      ].sort(),
    );
  });
});
