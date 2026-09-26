//! The fixed set of MCP tools that wrap `NexusDomain` directly (projects,
//! skills, behavior, graph, policies, context resolution). These are not
//! "tools" in the `nexus-tools::Tool` sense — they are the Control Plane's
//! own read API, always present. Registered `Tool`s (see `nexus-tools`) are
//! the extension surface on top of this fixed set.

use nexus_domain::graph::{get_related_entities, RelatedEntitiesOptions};
use nexus_domain::types::ClientInfo;
use nexus_domain::{context, NexusDomain, NexusError};
use nexus_tools::ToolDescriptor;
use serde::Deserialize;
use serde_json::{json, Value};

pub fn descriptors() -> Vec<ToolDescriptor> {
    vec![
        ToolDescriptor {
            name: "list_projects".into(),
            description: "Lists all projects known to the Control Plane.".into(),
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDescriptor {
            name: "get_project".into(),
            description: "Gets a single project by id.".into(),
            input_schema: json!({
                "type": "object", "required": ["projectId"],
                "properties": { "projectId": { "type": "string" } }
            }),
        },
        ToolDescriptor {
            name: "list_skills".into(),
            description: "Lists skills. If projectId is given, project skills override global skills of the same id.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "projectId": { "type": "string" } }
            }),
        },
        ToolDescriptor {
            name: "get_skill".into(),
            description: "Gets a single skill by id, resolved within an optional project scope.".into(),
            input_schema: json!({
                "type": "object", "required": ["skillId"],
                "properties": { "skillId": { "type": "string" }, "projectId": { "type": "string" } }
            }),
        },
        ToolDescriptor {
            name: "resolve_skills".into(),
            description: "Resolves the skills relevant to a task, including transitive dependencies.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "projectId": { "type": "string" },
                    "task": { "type": "string" },
                    "explicitIds": { "type": "array", "items": { "type": "string" } }
                }
            }),
        },
        ToolDescriptor {
            name: "list_behaviors".into(),
            description: "Lists the behavior specifications declared by a project.".into(),
            input_schema: json!({
                "type": "object", "required": ["projectId"],
                "properties": { "projectId": { "type": "string" } }
            }),
        },
        ToolDescriptor {
            name: "get_behavior".into(),
            description: "Gets a single behavior specification by id.".into(),
            input_schema: json!({
                "type": "object", "required": ["projectId", "behaviorId"],
                "properties": { "projectId": { "type": "string" }, "behaviorId": { "type": "string" } }
            }),
        },
        ToolDescriptor {
            name: "get_entity".into(),
            description: "Gets a single graph entity by id.".into(),
            input_schema: json!({
                "type": "object", "required": ["projectId", "entityId"],
                "properties": { "projectId": { "type": "string" }, "entityId": { "type": "string" } }
            }),
        },
        ToolDescriptor {
            name: "search_graph".into(),
            description: "Searches graph entities by id, name or type (case-insensitive substring match).".into(),
            input_schema: json!({
                "type": "object", "required": ["projectId", "query"],
                "properties": { "projectId": { "type": "string" }, "query": { "type": "string" } }
            }),
        },
        ToolDescriptor {
            name: "get_related_entities".into(),
            description: "Gets entities related to a given entity, up to a hop depth (default 1).".into(),
            input_schema: json!({
                "type": "object", "required": ["projectId", "entityId"],
                "properties": {
                    "projectId": { "type": "string" },
                    "entityId": { "type": "string" },
                    "depth": { "type": "integer", "minimum": 1 }
                }
            }),
        },
        ToolDescriptor {
            name: "get_project_policy".into(),
            description: "Gets the policy rules granted to a client for a project.".into(),
            input_schema: json!({
                "type": "object", "required": ["clientId", "projectId"],
                "properties": { "clientId": { "type": "string" }, "projectId": { "type": "string" } }
            }),
        },
        ToolDescriptor {
            name: "check_permission".into(),
            description: "Checks whether a client holds a given permission for an optional project.".into(),
            input_schema: json!({
                "type": "object", "required": ["client", "permission"],
                "properties": {
                    "client": { "type": "object", "required": ["id", "type"], "properties": {
                        "id": { "type": "string" }, "type": { "type": "string" }
                    }},
                    "permission": { "type": "string" },
                    "projectId": { "type": "string" }
                }
            }),
        },
        ToolDescriptor {
            name: "resolve_context".into(),
            description: "Resolves the minimal relevant context (skills, behavior, graph entities, policies) for a task.".into(),
            input_schema: json!({
                "type": "object", "required": ["projectId", "task", "client"],
                "properties": {
                    "projectId": { "type": "string" },
                    "task": { "type": "string" },
                    "client": { "type": "object", "required": ["id", "type"], "properties": {
                        "id": { "type": "string" }, "type": { "type": "string" }
                    }},
                    "requestedEntities": { "type": "array", "items": { "type": "string" } }
                }
            }),
        },
    ]
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn names() -> Vec<&'static str> {
    vec![
        "list_projects",
        "get_project",
        "list_skills",
        "get_skill",
        "resolve_skills",
        "list_behaviors",
        "get_behavior",
        "get_entity",
        "search_graph",
        "get_related_entities",
        "get_project_policy",
        "check_permission",
        "resolve_context",
    ]
}

#[derive(Debug, thiserror::Error)]
pub enum DomainToolError {
    #[error("invalid arguments: {0}")]
    InvalidArguments(String),
    #[error(transparent)]
    Domain(#[from] NexusError),
}

fn parse<T: for<'de> Deserialize<'de>>(args: Value) -> Result<T, DomainToolError> {
    serde_json::from_value(args).map_err(|e| DomainToolError::InvalidArguments(e.to_string()))
}

/// Dispatches one of the fixed domain tool names. Returns `None` if `name`
/// isn't one of them, so the caller can fall through to the tool registry.
pub fn call(
    domain: &NexusDomain,
    name: &str,
    args: Value,
) -> Option<Result<Value, DomainToolError>> {
    let result = match name {
        "list_projects" => (|| Ok(json!(domain.projects.list_projects()?)))(),
        "get_project" => (|| {
            #[derive(Deserialize)]
            struct Args {
                #[serde(rename = "projectId")]
                project_id: String,
            }
            let args: Args = parse(args)?;
            Ok(json!(domain.projects.require_project(&args.project_id)?))
        })(),
        "list_skills" => (|| {
            #[derive(Deserialize, Default)]
            struct Args {
                #[serde(rename = "projectId")]
                project_id: Option<String>,
            }
            let args: Args = parse(args)?;
            Ok(json!(domain
                .skills
                .list_skills(args.project_id.as_deref())?))
        })(),
        "get_skill" => (|| {
            #[derive(Deserialize)]
            struct Args {
                #[serde(rename = "skillId")]
                skill_id: String,
                #[serde(rename = "projectId")]
                project_id: Option<String>,
            }
            let args: Args = parse(args)?;
            Ok(json!(domain
                .skills
                .get_skill(&args.skill_id, args.project_id.as_deref())?))
        })(),
        "resolve_skills" => (|| {
            #[derive(Deserialize, Default)]
            struct Args {
                #[serde(rename = "projectId")]
                project_id: Option<String>,
                task: Option<String>,
                #[serde(rename = "explicitIds", default)]
                explicit_ids: Vec<String>,
            }
            let args: Args = parse(args)?;
            Ok(json!(nexus_domain::skill::resolve_skills(
                &domain.skills,
                args.project_id.as_deref(),
                args.task.as_deref(),
                &args.explicit_ids,
            )?))
        })(),
        "list_behaviors" => (|| {
            #[derive(Deserialize)]
            struct Args {
                #[serde(rename = "projectId")]
                project_id: String,
            }
            let args: Args = parse(args)?;
            Ok(json!(domain.behaviors.list_behaviors(&args.project_id)?))
        })(),
        "get_behavior" => (|| {
            #[derive(Deserialize)]
            struct Args {
                #[serde(rename = "projectId")]
                project_id: String,
                #[serde(rename = "behaviorId")]
                behavior_id: String,
            }
            let args: Args = parse(args)?;
            Ok(json!(domain
                .behaviors
                .require_behavior(&args.project_id, &args.behavior_id)?))
        })(),
        "get_entity" => (|| {
            #[derive(Deserialize)]
            struct Args {
                #[serde(rename = "projectId")]
                project_id: String,
                #[serde(rename = "entityId")]
                entity_id: String,
            }
            let args: Args = parse(args)?;
            Ok(json!(domain
                .graph
                .get_entity(&args.project_id, &args.entity_id)?))
        })(),
        "search_graph" => (|| {
            #[derive(Deserialize)]
            struct Args {
                #[serde(rename = "projectId")]
                project_id: String,
                query: String,
            }
            let args: Args = parse(args)?;
            Ok(json!(nexus_domain::graph::search_graph(
                &domain.graph,
                &args.project_id,
                &args.query
            )?))
        })(),
        "get_related_entities" => (|| {
            #[derive(Deserialize)]
            struct Args {
                #[serde(rename = "projectId")]
                project_id: String,
                #[serde(rename = "entityId")]
                entity_id: String,
                depth: Option<usize>,
            }
            let args: Args = parse(args)?;
            let (entities, relations) = get_related_entities(
                &domain.graph,
                &args.project_id,
                &args.entity_id,
                RelatedEntitiesOptions {
                    depth: args.depth.unwrap_or(1),
                    ..Default::default()
                },
            )?;
            Ok(json!({ "entities": entities, "relations": relations }))
        })(),
        "get_project_policy" => (|| {
            #[derive(Deserialize)]
            struct Args {
                #[serde(rename = "clientId")]
                client_id: String,
                #[serde(rename = "projectId")]
                project_id: String,
            }
            let args: Args = parse(args)?;
            Ok(json!(domain.policies.policies_for_client(
                &args.client_id,
                Some(&args.project_id)
            )?))
        })(),
        "check_permission" => (|| {
            #[derive(Deserialize)]
            struct Args {
                client: ClientInfo,
                permission: String,
                #[serde(rename = "projectId")]
                project_id: Option<String>,
            }
            let args: Args = parse(args)?;
            let allowed = domain.policies.has_permission(
                &args.client.id,
                &args.permission,
                args.project_id.as_deref(),
            )?;
            Ok(json!({ "allowed": allowed }))
        })(),
        "resolve_context" => (|| {
            let request: nexus_domain::types::ContextRequest = parse(args)?;
            Ok(json!(context::resolve_context(domain, &request)?))
        })(),
        _ => return None,
    };
    Some(result)
}
