//! asset-factory-mcp: a stand-alone MCP server over stdio (newline-delimited
//! JSON-RPC, same wire format as agent-nexus). It runs on its own, and
//! agent-nexus mounts it as a sub-server via `subservers.json`.
//!
//! Root: `ASSET_FACTORY_ROOT`, else the current directory.

mod materials;
mod plan;
mod review;
mod spec;
mod store;
mod tools;

use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use store::Store;

const PROTOCOL_VERSION: &str = "2024-11-05";

fn main() {
    let root = std::env::var("ASSET_FACTORY_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().expect("failed to read current directory"));
    eprintln!("asset-factory-mcp: serving MCP over stdio (root: {})", root.display());
    let store = Store::new(root);
    let tools = tools::all();

    let stdin = io::stdin();
    let mut out = io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(line.trim()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("asset-factory-mcp: dropping malformed message: {e}");
                continue;
            }
        };
        if let Some(response) = handle(&store, &tools, &message) {
            if writeln!(out, "{response}").and_then(|_| out.flush()).is_err() {
                break;
            }
        }
    }
}

fn handle(store: &Store, tools: &[tools::ToolDef], message: &Value) -> Option<Value> {
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(Value::as_str).unwrap_or_default();
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match method {
        "initialize" => json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "asset-factory-mcp", "version": env!("CARGO_PKG_VERSION") }
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools.iter().map(|t| json!({
            "name": t.name, "description": t.description, "inputSchema": (t.schema)()
        })).collect::<Vec<_>>() }),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match tools.iter().find(|t| t.name == name) {
                None => error_result(format!("unknown tool \"{name}\"")),
                Some(t) => match (t.call)(store, &args) {
                    Ok(v) => json!({ "content": [{ "type": "text",
                        "text": serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string()) }] }),
                    Err(e) => error_result(e),
                },
            }
        }
        _ if id.is_none() => return None,
        _ => return Some(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": "Method not found" } })),
    };
    id.map(|id| json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn error_result(message: String) -> Value {
    json!({ "content": [{ "type": "text", "text": message }], "isError": true })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(store: &Store, name: &str, args: Value) -> Value {
        let r = handle(store, &tools::all(), &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": name, "arguments": args } })).unwrap();
        r["result"].clone()
    }

    fn payload(result: &Value) -> Value {
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    #[test]
    fn lists_tools_and_ignores_notifications() {
        let store = Store::new(".");
        let r = handle(&store, &tools::all(), &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })).unwrap();
        assert_eq!(r["result"]["tools"].as_array().unwrap().len(), tools::all().len());
        assert!(handle(&store, &tools::all(), &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).is_none());
    }

    #[test]
    fn plan_and_review_round_trip_through_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let spec = serde_json::to_value(spec::tests::sample()).unwrap();
        std::fs::create_dir_all(dir.path().join("specs")).unwrap();
        std::fs::write(dir.path().join("specs/probe.json"), spec.to_string()).unwrap();

        let r = call(&store, "factory_plan", json!({ "specId": "probe", "level": "hero" }));
        assert_ne!(r["isError"], true, "{r}");
        assert!(dir.path().join("runs/probe/hero/plan.json").exists());

        let scores = json!([{ "component": "engine_r", "view": "top", "score": 0.3 }]);
        let first = payload(&call(&store, "factory_review", json!({ "specId": "probe", "level": "hero", "scores": scores })));
        let second = payload(&call(&store, "factory_review", json!({ "specId": "probe", "level": "hero", "scores": scores })));
        assert_eq!(first["regenerate"][0]["attempt"], 1);
        assert_eq!(second["regenerate"][0]["attempt"], 2);
        assert_ne!(first["regenerate"][0]["hunyuan"]["seed"], second["regenerate"][0]["hunyuan"]["seed"]);
    }

    #[test]
    fn invalid_spec_is_an_error_result() {
        let store = Store::new(".");
        let mut spec = serde_json::to_value(spec::tests::sample()).unwrap();
        spec["components"][0]["material"] = json!("M42");
        let r = call(&store, "factory_plan", json!({ "spec": spec, "write": false }));
        assert_eq!(r["isError"], true);
    }
}
