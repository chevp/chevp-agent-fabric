import { describe, it, expect } from 'vitest';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js';
import { NexusServices } from '../src/app/nexus-services.js';
import { createNexusMcpServer } from '../src/server/mcp-server.js';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, '..');

const EXPECTED_TOOLS = [
  'list_projects',
  'get_project',
  'list_skills',
  'get_skill',
  'resolve_skills',
  'list_behaviors',
  'get_behavior',
  'get_entity',
  'search_graph',
  'get_related_entities',
  'get_project_policy',
  'check_permission',
  'resolve_context',
];

async function connectedClient() {
  const services = NexusServices.fromRepoRoot(repoRoot);
  const server = createNexusMcpServer(services);
  const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
  const client = new Client({ name: 'test-client', version: '0.0.0' });
  await Promise.all([client.connect(clientTransport), server.connect(serverTransport)]);
  return { client, server };
}

describe('MCP server', () => {
  it('exposes exactly the documented read tools', async () => {
    const { client } = await connectedClient();
    const { tools } = await client.listTools();
    expect(tools.map((t) => t.name).sort()).toEqual([...EXPECTED_TOOLS].sort());
  });

  it('every tool declares an input schema', async () => {
    const { client } = await connectedClient();
    const { tools } = await client.listTools();
    for (const tool of tools) {
      expect(tool.inputSchema).toBeDefined();
      expect(tool.inputSchema.type).toBe('object');
    }
  });

  it('resolve_context round-trips through the MCP protocol', async () => {
    const { client } = await connectedClient();
    const result = await client.callTool({
      name: 'resolve_context',
      arguments: {
        projectId: 'acme-app',
        task: 'Implement the checkout button',
        client: { id: 'copilot', type: 'coding-agent' },
      },
    });

    expect(result.isError).toBeFalsy();
    const content = result.content as { type: string; text: string }[];
    const payload = JSON.parse(content[0]!.text);
    expect(payload.project.id).toBe('acme-app');
    expect(payload.skills.map((s: { id: string }) => s.id).sort()).toEqual([
      'accessibility',
      'checkout-ux',
    ]);
  });

  it('surfaces a domain error as an MCP tool error, not a crash', async () => {
    const { client } = await connectedClient();
    const result = await client.callTool({
      name: 'get_project',
      arguments: { projectId: 'does-not-exist' },
    });

    expect(result.isError).toBe(true);
  });
});
