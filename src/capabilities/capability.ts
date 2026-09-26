/**
 * A `Capability` describes an external system or tool that agents can reach
 * through (or alongside) the Control Plane — e.g. a Figma MCP server, a
 * GitHub MCP server, a Jira MCP server. Agent Nexus does not implement these
 * integrations; it only advertises that they exist and lets a client
 * discover which ones apply to a project.
 */
export interface Capability {
  id: string;
  name: string;
  description: string;
}
