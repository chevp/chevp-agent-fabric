import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import Fastify, { type FastifyInstance } from 'fastify';
import type { NexusServices } from '../app/nexus-services.js';
import { NexusError } from '../shared/errors.js';

/** Connects the MCP server over stdio — the primary transport for IDE/CLI agents. */
export async function serveStdio(server: McpServer): Promise<void> {
  const transport = new StdioServerTransport();
  await server.connect(transport);
}

/**
 * A minimal HTTP surface for debugging and for clients that cannot speak
 * MCP-over-stdio. It is a thin adapter over `NexusServices`, not a second
 * implementation of the domain logic, and it is NOT a substitute for the
 * MCP transport (no tool discovery/negotiation is implemented here).
 */
export function createHttpServer(services: NexusServices): FastifyInstance {
  const app = Fastify({ logger: false });

  app.setErrorHandler((err, _req, reply) => {
    if (err instanceof NexusError) {
      const status = err.code === 'NOT_FOUND' ? 404 : err.code === 'PERMISSION_DENIED' ? 403 : 400;
      reply.status(status).send({ error: err.code, message: err.message });
      return;
    }
    reply.status(500).send({ error: 'INTERNAL_ERROR', message: 'Internal error' });
  });

  app.get('/health', async () => ({ status: 'ok' }));

  app.get('/projects', async () => services.listProjects());
  app.get<{ Params: { id: string } }>('/projects/:id', async (req) =>
    services.getProject(req.params.id),
  );

  app.get<{ Querystring: { projectId?: string } }>('/skills', async (req) =>
    services.listSkills(req.query.projectId),
  );

  app.post<{ Body: Record<string, unknown> }>('/resolve-context', async (req) =>
    services.resolveContext(req.body as unknown as Parameters<typeof services.resolveContext>[0]),
  );

  return app;
}
