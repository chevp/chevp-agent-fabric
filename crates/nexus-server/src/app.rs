use nexus_domain::NexusDomain;
use nexus_tools::ToolRegistry;
use std::path::Path;

/// Everything the MCP transport needs: the domain services and the registry
/// of additional concrete tools. Adding a new tool crate means adding one
/// line in `build_tool_registry` — nothing else in this file changes.
pub struct McpApp {
    pub domain: NexusDomain,
    pub tools: ToolRegistry,
}

impl McpApp {
    pub fn from_repo_root(repo_root: impl AsRef<Path>) -> Self {
        Self {
            domain: NexusDomain::from_repo_root(repo_root),
            tools: build_tool_registry(),
        }
    }
}

fn build_tool_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(nexus_tool_graph_insights::GraphInsightsTool));
    nexus_tool_game_studio::tools()
        .into_iter()
        .for_each(|tool| registry.register(tool));
    registry
}
