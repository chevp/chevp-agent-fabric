import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import { z } from 'zod';
import type { NexusServices } from '../app/nexus-services.js';
import { ClientInfoSchema } from '../shared/schemas.js';
import { NexusError } from '../shared/errors.js';

/**
 * Builds the MCP server for Agent Nexus.
 *
 * IMPORTANT: handlers here do nothing but parse input and delegate to
 * `NexusServices`. All business logic (context resolution, graph traversal,
 * skill resolution, policy checks, ...) lives in the application/domain
 * layers, never inline in a tool handler.
 */
export function createNexusMcpServer(services: NexusServices): McpServer {
  const server = new McpServer({
    name: 'agent-nexus',
    version: '0.1.0',
  });

  // --- Projects ------------------------------------------------------------

  server.tool('list_projects', 'Lists all projects known to the Control Plane.', {}, async () =>
    safeInvoke(() => services.listProjects()),
  );

  server.tool(
    'get_project',
    'Gets a single project by id.',
    { projectId: z.string() },
    async ({ projectId }) => safeInvoke(() => services.getProject(projectId)),
  );

  // --- Skills ----------------------------------------------------------------

  server.tool(
    'list_skills',
    'Lists skills. If projectId is given, project skills override global skills of the same id.',
    { projectId: z.string().optional() },
    async ({ projectId }) => safeInvoke(() => services.listSkills(projectId)),
  );

  server.tool(
    'get_skill',
    'Gets a single skill by id, resolved within an optional project scope.',
    { skillId: z.string(), projectId: z.string().optional() },
    async ({ skillId, projectId }) => safeInvoke(() => services.getSkill(skillId, projectId)),
  );

  server.tool(
    'resolve_skills',
    'Resolves the skills relevant to a task, including transitive dependencies.',
    {
      projectId: z.string().optional(),
      task: z.string().optional(),
      explicitIds: z.array(z.string()).optional(),
    },
    async (input) => safeInvoke(() => services.resolveSkills(input)),
  );

  // --- Behavior ----------------------------------------------------------------

  server.tool(
    'list_behaviors',
    'Lists the behavior specifications declared by a project.',
    { projectId: z.string() },
    async ({ projectId }) => safeInvoke(() => services.listBehaviors(projectId)),
  );

  server.tool(
    'get_behavior',
    'Gets a single behavior specification by id.',
    { projectId: z.string(), behaviorId: z.string() },
    async ({ projectId, behaviorId }) =>
      safeInvoke(() => services.getBehavior(projectId, behaviorId)),
  );

  // --- Semantic Content Graph ----------------------------------------------

  server.tool(
    'get_entity',
    'Gets a single graph entity by id.',
    { projectId: z.string(), entityId: z.string() },
    async ({ projectId, entityId }) => safeInvoke(() => services.getEntity(projectId, entityId)),
  );

  server.tool(
    'search_graph',
    'Searches graph entities by id, name or type (case-insensitive substring match).',
    { projectId: z.string(), query: z.string() },
    async ({ projectId, query }) => safeInvoke(() => services.searchGraph(projectId, query)),
  );

  server.tool(
    'get_related_entities',
    'Gets entities related to a given entity, up to a hop depth (default 1).',
    { projectId: z.string(), entityId: z.string(), depth: z.number().int().positive().optional() },
    async ({ projectId, entityId, depth }) =>
      safeInvoke(() => services.getRelatedEntities(projectId, entityId, depth)),
  );

  // --- Policies ----------------------------------------------------------------

  server.tool(
    'get_project_policy',
    'Gets the policy rules granted to a client for a project.',
    { clientId: z.string(), projectId: z.string() },
    async ({ clientId, projectId }) =>
      safeInvoke(() => services.getProjectPolicy(clientId, projectId)),
  );

  server.tool(
    'check_permission',
    'Checks whether a client holds a given permission for an optional project.',
    { client: ClientInfoSchema, permission: z.string(), projectId: z.string().optional() },
    async ({ client, permission, projectId }) =>
      safeInvoke(async () => ({
        allowed: await services.checkPermission(client, permission, projectId),
      })),
  );

  // --- Context Resolver ----------------------------------------------------

  server.tool(
    'resolve_context',
    'Resolves the minimal relevant context (skills, behavior, graph entities, policies) for a task.',
    {
      projectId: z.string(),
      task: z.string(),
      client: ClientInfoSchema,
      requestedEntities: z.array(z.string()).optional(),
    },
    async (input) => safeInvoke(() => services.resolveContext(input)),
  );

  return server;
}

async function safeInvoke(
  fn: () => Promise<unknown>,
): Promise<{ content: { type: 'text'; text: string }[]; isError?: true }> {
  try {
    const value = await fn();
    return { content: [{ type: 'text', text: JSON.stringify(value, null, 2) }] };
  } catch (err) {
    const message = err instanceof NexusError ? `${err.code}: ${err.message}` : 'Internal error';
    return { content: [{ type: 'text', text: message }], isError: true };
  }
}
