mod app;
mod domain_tools;
mod knowledge_tools;
mod resources;
mod rpc;
#[cfg(test)]
mod semantic_mcp_tests;
mod semantic_tools;
mod stdio;

use app::McpApp;
use serde_json::{json, Value};
use std::path::PathBuf;

const USAGE: &str = "usage:
  agent-nexus                                   serve MCP over stdio
  agent-nexus ingest <projectId> [dir]          ingest a directory (default: the project) and its Git history
  agent-nexus export-graph <projectId> [outDir] write semantic-graph.json + semantic-project.html";

fn main() {
    let repo_root = std::env::var("AGENT_NEXUS_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().expect("failed to read current directory"));
    let args: Vec<String> = std::env::args().skip(1).collect();

    if let Some(command) = args.first() {
        std::process::exit(run_command(repo_root, command, &args[1..]));
    }

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

/// One-shot CLI commands; they call the same tools an MCP client would.
fn run_command(repo_root: PathBuf, command: &str, args: &[String]) -> i32 {
    let (tool, arguments): (&str, Value) = match (command, args) {
        ("ingest", [project, rest @ ..]) => (
            "ingest_repository",
            json!({ "projectId": project, "path": rest.first() }),
        ),
        ("export-graph", [project, rest @ ..]) => (
            "export_semantic_graph",
            json!({ "projectId": project, "outDir": rest.first() }),
        ),
        _ => {
            eprintln!("{USAGE}");
            return 2;
        }
    };
    let app = McpApp::from_repo_root(repo_root);
    let mut arguments = arguments;
    if let Value::Object(o) = &mut arguments {
        o.retain(|_, v| !v.is_null());
    }
    match semantic_tools::call(&app.semantic, &app.domain, tool, arguments) {
        Some(Ok(value)) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&value).unwrap_or_default()
            );
            0
        }
        Some(Err(message)) => {
            eprintln!("agent-nexus: {message}");
            1
        }
        None => 2,
    }
}
