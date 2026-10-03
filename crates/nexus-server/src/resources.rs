//! MCP resources: read-only `nexus://` views of a project. Tools act,
//! resources give context.
//!
//! ```text
//! nexus://projects/{projectId}                      NexusProject (roles, contracts) + counts
//! nexus://projects/{projectId}/semantic             consolidated SemanticModel
//! nexus://projects/{projectId}/skills               skills with their EngineeringContext
//! nexus://projects/{projectId}/behaviors            behavior specs
//! nexus://projects/{projectId}/graph                SemanticGraph (nodes, edges, layout metrics)
//! nexus://projects/{projectId}/policies             global + project policy rules
//! nexus://projects/{projectId}/proposals[/{id}]
//! nexus://projects/{projectId}/artifacts[/{id}]     artifact + stored model
//! nexus://projects/{projectId}/translations/{id}
//! ```

use crate::app::McpApp;
use nexus_semantic::{block_on, ArtifactId};
use serde_json::{json, Value};

pub const SCHEME: &str = "nexus://projects/";

const VIEWS: &[(&str, &str)] = &[
    ("", "Project with semantic roles and contracts"),
    ("/semantic", "Consolidated SemanticModel of the project"),
    ("/skills", "Skills with their EngineeringContext"),
    ("/behaviors", "Behavior specifications"),
    ("/graph", "Semantic graph with layout metrics"),
    ("/policies", "Policy rules"),
    ("/proposals", "Semantic graph proposals"),
    ("/artifacts", "Registered artifacts"),
];

pub fn list(app: &McpApp) -> Value {
    let projects = app.domain.projects.list_projects().unwrap_or_default();
    let resources: Vec<Value> = projects
        .iter()
        .flat_map(|p| {
            VIEWS.iter().map(move |(suffix, description)| {
                json!({
                    "uri": format!("{SCHEME}{}{suffix}", p.id),
                    "name": format!("{}{suffix}", p.name),
                    "description": description,
                    "mimeType": "application/json"
                })
            })
        })
        .collect();
    json!({ "resources": resources })
}

pub fn templates() -> Value {
    let mut templates: Vec<Value> = VIEWS
        .iter()
        .map(|(suffix, description)| {
            json!({
                "uriTemplate": format!("{SCHEME}{{projectId}}{suffix}"),
                "name": format!("project{suffix}"),
                "description": description,
                "mimeType": "application/json"
            })
        })
        .collect();
    for (segment, description) in [
        (
            "artifacts/{artifactId}",
            "One artifact and its stored SemanticModel",
        ),
        ("translations/{translationId}", "One stored translation"),
        ("proposals/{proposalId}", "One proposal"),
    ] {
        templates.push(json!({
            "uriTemplate": format!("{SCHEME}{{projectId}}/{segment}"),
            "name": segment.split('/').next().unwrap_or(segment),
            "description": description,
            "mimeType": "application/json"
        }));
    }
    json!({ "resourceTemplates": templates })
}

pub enum ReadError {
    NotFound(String),
    Internal(String),
}

fn err(e: impl std::fmt::Display) -> ReadError {
    let message = e.to_string();
    if message.contains("was not found") {
        ReadError::NotFound(message)
    } else {
        ReadError::Internal(message)
    }
}

fn to_json(v: impl serde::Serialize) -> Result<Value, ReadError> {
    serde_json::to_value(v).map_err(err)
}

fn view(app: &McpApp, project_id: &str, rest: &[&str]) -> Result<Value, ReadError> {
    let (domain, engine) = (&app.domain, &app.semantic);
    match rest {
        [] => {
            let project = engine.nexus_project(domain, project_id).map_err(err)?;
            let artifacts = engine.artifacts(domain, project_id).map_err(err)?.len();
            let proposals = engine.proposals(domain, project_id).map_err(err)?;
            let pending = proposals
                .iter()
                .filter(|p| p.status == nexus_semantic::proposal::ProposalStatus::Pending)
                .count();
            Ok(json!({
                "project": project,
                "artifacts": artifacts,
                "proposals": { "total": proposals.len(), "pending": pending }
            }))
        }
        ["semantic"] => {
            let stored = !engine
                .artifacts(domain, project_id)
                .map_err(err)?
                .is_empty();
            let model = block_on(engine.project_model(domain, project_id)).map_err(err)?;
            Ok(json!({ "source": if stored { "stored" } else { "live" }, "model": model }))
        }
        ["skills"] => {
            let skills = domain.skills.list_skills(Some(project_id)).map_err(err)?;
            let mut out = Vec::new();
            for s in skills {
                let ctx = engine
                    .engineering_context(domain, project_id, std::slice::from_ref(&s.id))
                    .map_err(err)?;
                out.push(json!({ "skill": s, "engineeringContext": ctx }));
            }
            Ok(json!(out))
        }
        ["behaviors"] => to_json(domain.behaviors.list_behaviors(project_id).map_err(err)?),
        ["graph"] => to_json(block_on(engine.graph(domain, project_id, false)).map_err(err)?),
        ["policies"] => {
            let store = domain.policies.store();
            let mut rules = store.list_global_policies().map_err(err)?;
            rules.extend(store.list_project_policies(project_id).map_err(err)?);
            to_json(rules)
        }
        ["proposals"] => to_json(engine.proposals(domain, project_id).map_err(err)?),
        ["proposals", id] => {
            let all = engine.proposals(domain, project_id).map_err(err)?;
            let p = all
                .into_iter()
                .find(|p| p.id == *id)
                .ok_or_else(|| ReadError::NotFound(format!("Proposal \"{id}\" was not found")))?;
            to_json(p)
        }
        ["artifacts"] => to_json(engine.artifacts(domain, project_id).map_err(err)?),
        ["artifacts", id] => {
            let (artifact, model) = engine
                .artifact(domain, project_id, &ArtifactId::from(*id))
                .map_err(err)?;
            Ok(json!({ "artifact": artifact, "model": model }))
        }
        ["translations", id] => to_json(engine.translation(domain, project_id, id).map_err(err)?),
        _ => Err(ReadError::NotFound(format!(
            "unknown resource path \"{}\"",
            rest.join("/")
        ))),
    }
}

pub fn read(app: &McpApp, uri: &str) -> Result<Value, ReadError> {
    let path = uri
        .strip_prefix(SCHEME)
        .ok_or_else(|| ReadError::NotFound(format!("unsupported resource URI \"{uri}\"")))?;
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let (project_id, rest) = segments
        .split_first()
        .ok_or_else(|| ReadError::NotFound(format!("resource URI \"{uri}\" names no project")))?;
    app.domain
        .projects
        .require_project(project_id)
        .map_err(err)?;
    let value = view(app, project_id, rest)?;
    let text = serde_json::to_string_pretty(&value).map_err(err)?;
    Ok(json!({ "contents": [{ "uri": uri, "mimeType": "application/json", "text": text }] }))
}
