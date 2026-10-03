use crate::{domain_tools, knowledge_tools, semantic_tools};
use nexus_domain::NexusDomain;
use nexus_semantic::SemanticEngine;
use nexus_tools::ToolRegistry;
use std::path::Path;

/// Everything the MCP transport needs: the domain services and the registry
/// of additional concrete tools. Adding a new tool crate means adding one
/// line in `build_tool_registry` — nothing else in this file changes.
pub struct McpApp {
    pub domain: NexusDomain,
    pub semantic: SemanticEngine,
    pub tools: ToolRegistry,
}

impl McpApp {
    pub fn from_repo_root(repo_root: impl AsRef<Path>) -> Self {
        let repo_root = repo_root.as_ref();
        Self {
            domain: NexusDomain::from_repo_root(repo_root),
            semantic: SemanticEngine::new(repo_root),
            tools: build_tool_registry(repo_root),
        }
    }
}

fn build_tool_registry(repo_root: &Path) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(nexus_tool_graph_insights::GraphInsightsTool));
    nexus_tool_game_studio::tools()
        .into_iter()
        .for_each(|tool| registry.register(tool));
    nexus_tool_indexer::tools()
        .into_iter()
        .for_each(|tool| registry.register(tool));
    // Sub-servers go last and never shadow a built-in or an in-process tool.
    let mut reserved = domain_tools::names();
    reserved.extend(semantic_tools::names());
    reserved.extend(knowledge_tools::names());
    for tool in nexus_tool_subserver::tools_from_config(repo_root) {
        if reserved.contains(&tool.name()) || registry.get(tool.name()).is_some() {
            eprintln!(
                "agent-nexus: sub-server tool \"{}\" collides with an existing tool; set a prefix",
                tool.name()
            );
            continue;
        }
        registry.register(tool);
    }
    registry
}
