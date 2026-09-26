import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { NexusServices } from './app/nexus-services.js';
import { createNexusMcpServer } from './server/mcp-server.js';
import { serveStdio, createHttpServer } from './server/transport.js';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, '..');

async function main(): Promise<void> {
  const services = NexusServices.fromRepoRoot(repoRoot);
  const mode = process.env.AGENT_NEXUS_TRANSPORT ?? 'stdio';

  if (mode === 'http') {
    const app = createHttpServer(services);
    const port = Number(process.env.PORT ?? 3333);
    await app.listen({ port, host: '0.0.0.0' });
    console.error(`Agent Nexus HTTP debug server listening on :${port}`);
    return;
  }

  const server = createNexusMcpServer(services);
  await serveStdio(server);
  console.error('Agent Nexus MCP server running on stdio');
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
