use crate::app::McpApp;
use crate::rpc::handle_message;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn example_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            if entry.file_name() != "semantic" {
                copy_dir(&entry.path(), &target);
            }
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn temp_app() -> (TempDir, McpApp) {
    let dir = TempDir::new().unwrap();
    copy_dir(
        &example_root().join("projects/acme-app"),
        &dir.path().join("projects/acme-app"),
    );
    copy_dir(&example_root().join("global"), &dir.path().join("global"));
    let app = McpApp::from_repo_root(dir.path());
    (dir, app)
}

fn rpc(app: &McpApp, method: &str, params: Value) -> Value {
    handle_message(
        app,
        &json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }),
    )
    .unwrap()
}

/// Calls a tool and returns its JSON payload; panics on a tool error.
fn call(app: &McpApp, name: &str, arguments: Value) -> Value {
    let response = rpc(
        app,
        "tools/call",
        json!({ "name": name, "arguments": arguments }),
    );
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(
        response["result"]["isError"],
        json!(true),
        "{name} failed: {text}"
    );
    serde_json::from_str(&text).unwrap()
}

fn read(app: &McpApp, uri: &str) -> Value {
    let response = rpc(app, "resources/read", json!({ "uri": uri }));
    let text = response["result"]["contents"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("{uri}: {response}"));
    serde_json::from_str(text).unwrap()
}

#[test]
fn initialize_advertises_resources() {
    let (_d, app) = temp_app();
    let r = rpc(&app, "initialize", json!({}));
    assert!(r["result"]["capabilities"]["resources"].is_object());
}

#[test]
fn tools_list_exposes_the_semantic_pipeline() {
    let (_d, app) = temp_app();
    let r = rpc(&app, "tools/list", json!({}));
    let names: Vec<&str> = r["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for n in [
        "register_artifact",
        "inspect",
        "translate",
        "validate",
        "diff",
        "propose",
        "review_proposal",
        "ingest_repository",
        "resolve_context",
    ] {
        assert!(names.contains(&n), "missing {n}");
    }
}

#[test]
fn resources_list_and_read() {
    let (_d, app) = temp_app();
    let r = rpc(&app, "resources/list", json!({}));
    let uris: Vec<&str> = r["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["uri"].as_str().unwrap())
        .collect();
    for suffix in [
        "",
        "/semantic",
        "/skills",
        "/behaviors",
        "/graph",
        "/policies",
    ] {
        assert!(
            uris.contains(&format!("nexus://projects/acme-app{suffix}").as_str()),
            "missing {suffix}"
        );
    }
    let t = rpc(&app, "resources/templates/list", json!({}));
    assert!(t["result"]["resourceTemplates"].as_array().unwrap().len() >= 6);

    let project = read(&app, "nexus://projects/acme-app");
    assert_eq!(project["project"]["roles"][1], "ux");
    assert_eq!(
        project["project"]["semanticContracts"][0]["id"],
        "checkout-submit"
    );

    let semantic = read(&app, "nexus://projects/acme-app/semantic");
    assert_eq!(semantic["source"], "live");
    assert!(!semantic["model"]["states"].as_array().unwrap().is_empty());

    let skills = read(&app, "nexus://projects/acme-app/skills");
    let ux = skills
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["skill"]["id"] == "checkout-ux")
        .unwrap();
    assert_eq!(
        ux["engineeringContext"]["constraints"]
            .as_array()
            .unwrap()
            .len(),
        4
    );

    let graph = read(&app, "nexus://projects/acme-app/graph");
    assert!(graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["id"] == "checkout-button" && n["canonical"] == true));
    assert!(graph["nodes"][0]["depth"].is_number() && graph["nodes"][0]["cluster"].is_string());

    assert!(!read(&app, "nexus://projects/acme-app/policies")
        .as_array()
        .unwrap()
        .is_empty());
    assert!(!read(&app, "nexus://projects/acme-app/behaviors")
        .as_array()
        .unwrap()
        .is_empty());

    let missing = rpc(
        &app,
        "resources/read",
        json!({ "uri": "nexus://projects/nope/graph" }),
    );
    assert_eq!(missing["error"]["code"], -32002);
    let bad = rpc(
        &app,
        "resources/read",
        json!({ "uri": "nexus://projects/acme-app/unknown" }),
    );
    assert_eq!(bad["error"]["code"], -32002);
    // Reading resources never writes.
    assert!(!app.domain.projects_root.join("acme-app/semantic").exists());
}

#[test]
fn register_inspect_validate_translate_diff_propose_review_via_mcp() {
    let (_d, app) = temp_app();
    let p = json!("acme-app");

    let reg = call(
        &app,
        "register_artifact",
        json!({ "projectId": p, "kind": "behavior-spec", "name": "checkout-button", "path": "behavior/checkout-button.yaml" }),
    );
    let artifact_id = reg["artifactId"].as_str().unwrap().to_string();
    assert_eq!(reg["status"], "registered");

    let inspected = call(
        &app,
        "inspect",
        json!({ "projectId": p, "artifactId": artifact_id }),
    );
    assert_eq!(inspected["parser"], "BehaviorSpecParser");
    let states = inspected["model"]["states"].as_array().unwrap();
    assert!(states.iter().all(|s| s["evidence"] == "explicit"
        && s["provenance"][0]["source"] == "projects/acme-app/behavior/checkout-button.yaml"));

    let valid = call(
        &app,
        "validate",
        json!({ "projectId": p, "artifactIds": [artifact_id], "contractId": "checkout-submit" }),
    );
    assert_eq!(valid["valid"], true, "{valid}");

    let translated = call(
        &app,
        "translate",
        json!({ "projectId": p, "artifactIds": [artifact_id], "direction": "behavior-to-engineering", "contextSkills": ["skill://checkout-ux"] }),
    );
    let tid = translated["translationId"].as_str().unwrap();
    assert_eq!(translated["translation"]["result"]["kind"], "engineering");
    let tv = call(
        &app,
        "validate",
        json!({ "projectId": p, "translationId": tid }),
    );
    assert!(tv["issues"]
        .as_array()
        .unwrap()
        .iter()
        .all(|i| i["code"] != "untraceable_output"));
    let behavior = call(
        &app,
        "translate",
        json!({ "projectId": p, "artifactIds": [artifact_id], "direction": "engineering-to-behavior" }),
    );
    assert_eq!(
        behavior["translation"]["result"]["machines"][0]["id"],
        "checkout-button"
    );

    let file = app
        .domain
        .projects_root
        .join("acme-app/behavior/checkout-button.yaml");
    let text = std::fs::read_to_string(&file)
        .unwrap()
        .replace("\r\n", "\n")
        .replace("  - disabled\n", "  - disabled\n  - retrying\n");
    std::fs::write(&file, text).unwrap();
    let diff = call(
        &app,
        "diff",
        json!({ "projectId": p, "artifactId": artifact_id }),
    );
    assert_eq!(diff["summary"]["StateAdded"], 1, "{diff}");

    let html = call(
        &app,
        "register_artifact",
        json!({ "projectId": p, "kind": "design-spec", "name": "checkout html", "path": "design/checkout-button.html" }),
    );
    let html_id = html["artifactId"].as_str().unwrap();
    call(
        &app,
        "inspect",
        json!({ "projectId": p, "artifactId": html_id }),
    );
    let proposal = call(
        &app,
        "propose",
        json!({ "projectId": p, "artifactIds": [html_id] }),
    );
    let pid = proposal["proposalId"].as_str().unwrap();
    assert_eq!(proposal["proposal"]["status"], "pending");

    let denied = rpc(
        &app,
        "tools/call",
        json!({ "name": "review_proposal", "arguments": { "projectId": p, "proposalId": pid, "decision": "accept", "client": { "id": "copilot", "type": "coding-agent" } } }),
    );
    assert_eq!(denied["result"]["isError"], true);
    let accepted = call(
        &app,
        "review_proposal",
        json!({ "projectId": p, "proposalId": pid, "decision": "accept", "client": { "id": "human-reviewer", "type": "human" } }),
    );
    assert_eq!(accepted["status"], "accepted");

    let graph = read(&app, "nexus://projects/acme-app/graph");
    let form = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "checkout-form")
        .unwrap();
    assert_eq!(form["canonical"], true);
    let artifact = read(
        &app,
        &format!("nexus://projects/acme-app/artifacts/{artifact_id}"),
    );
    assert!(artifact["model"]["states"].is_array());
    let stored = read(&app, &format!("nexus://projects/acme-app/proposals/{pid}"));
    assert_eq!(stored["status"], "accepted");
}

#[test]
fn ingest_and_export_via_mcp() {
    let (dir, app) = temp_app();
    let report = call(
        &app,
        "ingest_repository",
        json!({ "projectId": "acme-app" }),
    );
    assert!(report["artifacts"].as_array().unwrap().len() >= 10);
    assert!(report["proposal"].is_string());
    let outside = rpc(
        &app,
        "tools/call",
        json!({ "name": "ingest_repository", "arguments": { "projectId": "acme-app", "path": ".." } }),
    );
    assert_eq!(outside["result"]["isError"], true);

    let ctx = call(
        &app,
        "resolve_semantic_context",
        json!({ "projectId": "acme-app", "task": "Implement the checkout button", "client": { "id": "copilot", "type": "coding-agent" } }),
    );
    assert!(!ctx["engineering"]["constraints"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(ctx["semantics"]["states"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["subject"] == "checkout-button"));

    let out = call(
        &app,
        "export_semantic_graph",
        json!({ "projectId": "acme-app" }),
    );
    assert!(dir.path().join(out["html"].as_str().unwrap()).is_file());
    assert!(dir
        .path()
        .join(out["graphJson"].as_str().unwrap())
        .is_file());
    let semantic = read(&app, "nexus://projects/acme-app/semantic");
    assert_eq!(semantic["source"], "stored");
}

#[test]
fn knowledge_layer_read_tools_via_mcp() {
    let (_d, app) = temp_app();
    let p = json!("acme-app");

    let entry = call(
        &app,
        "get_knowledge_entry",
        json!({ "projectId": p, "id": "checkout-submit" }),
    );
    assert_eq!(entry["term"], "CheckoutSubmit");
    assert_eq!(entry["kind"], "behavior");

    let list = call(&app, "list_knowledge_entries", json!({ "projectId": p }));
    assert!(list.as_array().unwrap().iter().any(|e| e["id"] == "checkout-submit"));

    let found = call(
        &app,
        "search_knowledge",
        json!({ "projectId": p, "query": "submit" }),
    );
    assert!(found.as_array().unwrap().iter().any(|e| e["id"] == "checkout-submit"));

    let resolved = call(
        &app,
        "resolve_term",
        json!({ "projectId": p, "text": "submit checkout" }),
    );
    assert_eq!(resolved["resolved"], true);
    assert_eq!(resolved["entry"]["id"], "checkout-submit");

    let unresolved = call(
        &app,
        "resolve_term",
        json!({ "projectId": p, "text": "does not exist" }),
    );
    assert_eq!(unresolved["resolved"], false);

    let relations = call(
        &app,
        "get_term_relations",
        json!({ "projectId": p, "id": "checkout-submit" }),
    );
    assert_eq!(relations["subject"], "checkout-button");
    assert!(relations["entities"].as_array().unwrap().iter().any(|e| e["id"] == "payment-flow"));

    let provenance = call(
        &app,
        "get_term_provenance",
        json!({ "projectId": p, "id": "checkout-submit" }),
    );
    assert_eq!(provenance["source"], "projects/acme-app/contracts/checkout-submit.yaml");

    // No `identity` was ever recorded on the hand-authored fixture.
    let freshness = call(
        &app,
        "get_term_freshness",
        json!({ "projectId": p, "id": "checkout-submit" }),
    );
    assert_eq!(freshness["freshness"], "unknown");

    let missing = rpc(
        &app,
        "tools/call",
        json!({ "name": "get_knowledge_entry", "arguments": { "projectId": p, "id": "nope" } }),
    );
    assert_eq!(missing["result"]["isError"], true);
}

#[test]
fn propose_and_review_a_knowledge_entry_via_mcp() {
    let (_d, app) = temp_app();
    let p = json!("acme-app");

    let proposed = call(
        &app,
        "propose_knowledge_entry",
        json!({
            "projectId": p,
            "term": "CheckoutCancel",
            "kind": "behavior",
            "subject": "checkout-button",
            "definition": { "short": "Cancels an in-progress checkout." },
            "aliases": ["cancel checkout"],
            "constraints": ["confirm before discarding entered data"]
        }),
    );
    let pid = proposed["proposalId"].as_str().unwrap().to_string();
    assert_eq!(proposed["proposal"]["status"], "pending");

    let denied = rpc(
        &app,
        "tools/call",
        json!({ "name": "review_proposal", "arguments": { "projectId": p, "proposalId": pid, "decision": "accept", "client": { "id": "copilot", "type": "coding-agent" } } }),
    );
    assert_eq!(denied["result"]["isError"], true);

    let accepted = call(
        &app,
        "review_proposal",
        json!({ "projectId": p, "proposalId": pid, "decision": "accept", "client": { "id": "human-reviewer", "type": "human" } }),
    );
    assert_eq!(accepted["status"], "accepted");

    let entry = call(
        &app,
        "get_knowledge_entry",
        json!({ "projectId": p, "id": "checkout-cancel" }),
    );
    assert_eq!(entry["term"], "CheckoutCancel");
    let confidence = entry["confidence"]["value"].as_f64().unwrap();
    assert!((confidence - 0.3).abs() < 0.001, "{confidence}");

    let resolved = call(
        &app,
        "resolve_term",
        json!({ "projectId": p, "text": "cancel checkout" }),
    );
    assert_eq!(resolved["entry"]["id"], "checkout-cancel");
}
