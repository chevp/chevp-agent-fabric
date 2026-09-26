mod app;
mod domain_tools;
mod rpc;
mod stdio;

use app::McpApp;
use std::path::PathBuf;

fn main() {
    let repo_root = std::env::var("AGENT_NEXUS_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().expect("failed to read current directory"));

    eprintln!(
        "agent-nexus: serving MCP over stdio (repo root: {})",
        repo_root.display()
    );

    let app = McpApp::from_repo_root(repo_root);
    if let Err(err) = stdio::serve(&app) {
        eprintln!("agent-nexus: fatal I/O error: {err}");
        std::process::exit(1);
    }
}
