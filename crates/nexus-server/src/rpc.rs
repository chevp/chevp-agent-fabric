//! A minimal, hand-rolled JSON-RPC 2.0 dispatcher for the MCP methods Agent
//! Nexus supports (`initialize`, `tools/list`, `tools/call`, `ping`). This is
//! deliberately not built on a generic RPC framework: the MCP stdio
//! transport is one newline-delimited JSON message in, at most one JSON
//! message out, and that is simple enough to own directly.
//!
//! `handle_message` is a pure function of `(&McpApp, Value) -> Option<Value>`
//! so it can be unit tested without spawning a process or touching stdio.

use crate::app::McpApp;
use crate::domain_tools;
use nexus_tools::{ToolContext, ToolDescriptor};
use serde_json::{json, Value};

const PROTOCOL_VERSION: &str = "2024-11-05";

/// Handles one incoming JSON-RPC message. Returns `None` for notifications
/// (no `id`), which must not be replied to.
pub fn handle_message(app: &McpApp, message: &Value) -> Option<Value> {
    let id = message.get("id").cloned();
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));

    match method {
        "initialize" => Some(success(id, initialize_result())),
        "ping" => Some(success(id, json!({}))),
        "notifications/initialized" | "notifications/cancelled" => None,
        "tools/list" => Some(success(id, json!({ "tools": tool_list_json(app) }))),
        "tools/call" => Some(success(id, call_tool(app, &params))),
        _ => id.map(|id| error(id, -32601, "Method not found")),
    }
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "agent-nexus", "version": env!("CARGO_PKG_VERSION") }
    })
}

fn tool_list_json(app: &McpApp) -> Vec<Value> {
    let mut descriptors: Vec<ToolDescriptor> = domain_tools::descriptors();
    descriptors.extend(app.tools.list());
    descriptors
        .into_iter()
        .map(|d| json!({ "name": d.name, "description": d.description, "inputSchema": d.input_schema }))
        .collect()
}

fn call_tool(app: &McpApp, params: &Value) -> Value {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    if let Some(result) = domain_tools::call(&app.domain, name, arguments.clone()) {
        return match result {
            Ok(value) => text_result(value),
            Err(err) => error_result(err.to_string()),
        };
    }

    let ctx = ToolContext {
        domain: &app.domain,
    };
    match app.tools.call(&ctx, name, arguments) {
        Ok(value) => text_result(value),
        Err(err) => error_result(err.to_string()),
    }
}

fn text_result(value: Value) -> Value {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    json!({ "content": [{ "type": "text", "text": text }] })
}

fn error_result(message: String) -> Value {
    json!({ "content": [{ "type": "text", "text": message }], "isError": true })
}

fn success(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::McpApp;
    use std::path::PathBuf;

    fn test_app() -> McpApp {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        McpApp::from_repo_root(repo_root)
    }

    #[test]
    fn initialize_returns_the_protocol_version() {
        let app = test_app();
        let response = handle_message(
            &app,
            &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }),
        )
        .unwrap();
        assert_eq!(response["result"]["protocolVersion"], PROTOCOL_VERSION);
    }

    #[test]
    fn notifications_get_no_response() {
        let app = test_app();
        let response = handle_message(
            &app,
            &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        );
        assert!(response.is_none());
    }

    #[test]
    fn tools_list_exposes_domain_tools_and_registered_tools() {
        let app = test_app();
        let response = handle_message(
            &app,
            &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
        )
        .unwrap();
        let names: Vec<&str> = response["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();

        assert!(names.contains(&"resolve_context"));
        assert!(names.contains(&"graph_insights"));
        assert_eq!(names.len(), domain_tools::names().len() + 1);
    }

    #[test]
    fn tools_call_round_trips_resolve_context() {
        let app = test_app();
        let response = handle_message(
            &app,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "resolve_context",
                    "arguments": {
                        "projectId": "acme-app",
                        "task": "Implement the checkout button",
                        "client": { "id": "copilot", "type": "coding-agent" }
                    }
                }
            }),
        )
        .unwrap();

        assert_ne!(response["result"]["isError"], json!(true));
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        let payload: Value = serde_json::from_str(text).unwrap();
        assert_eq!(payload["project"]["id"], "acme-app");
    }

    #[test]
    fn tools_call_surfaces_a_domain_error_without_crashing() {
        let app = test_app();
        let response = handle_message(
            &app,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": "get_project", "arguments": { "projectId": "does-not-exist" } }
            }),
        )
        .unwrap();

        assert_eq!(response["result"]["isError"], json!(true));
    }

    #[test]
    fn tools_call_dispatches_to_a_registered_tool() {
        let app = test_app();
        let response = handle_message(
            &app,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": "graph_insights", "arguments": { "projectId": "acme-app" } }
            }),
        )
        .unwrap();

        assert_ne!(response["result"]["isError"], json!(true));
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        let payload: Value = serde_json::from_str(text).unwrap();
        assert_eq!(payload["entities"][0]["id"], "checkout-button");
    }

    #[test]
    fn unknown_method_with_id_gets_a_json_rpc_error() {
        let app = test_app();
        let response = handle_message(
            &app,
            &json!({ "jsonrpc": "2.0", "id": 1, "method": "nope" }),
        )
        .unwrap();
        assert_eq!(response["error"]["code"], -32601);
    }
}
