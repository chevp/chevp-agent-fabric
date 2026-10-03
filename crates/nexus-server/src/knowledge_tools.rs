//! MCP tools for the Semantic Knowledge Layer for Agents: term/alias
//! resolution, search, relations, provenance and freshness over
//! `SemanticContract` entries (`projects/<id>/contracts/*.yaml`). Thin
//! adapters only — the model lives in `nexus_semantic::knowledge` and
//! `nexus_semantic::contract`, the graph delegation reuses
//! `nexus_domain::graph` directly, and proposing a new entry reuses the
//! existing `proposal`/`review_proposal` lifecycle (a contract proposal is
//! reviewed with the same `review_proposal` tool as any other proposal).

use nexus_domain::graph::{get_related_entities, RelatedEntitiesOptions};
use nexus_domain::NexusDomain;
use nexus_semantic::knowledge::{self, ConceptKind, ContractInput, Definition};
use nexus_semantic::proposal;
use nexus_semantic::{Role, SemanticEngine};
use nexus_tools::ToolDescriptor;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

fn tool(name: &str, description: &str, schema: Value) -> ToolDescriptor {
    ToolDescriptor {
        name: name.into(),
        description: description.into(),
        input_schema: schema,
    }
}

pub fn descriptors() -> Vec<ToolDescriptor> {
    vec![
        tool("get_knowledge_entry", "Gets one Semantic Knowledge Layer entry by exact id.", json!({
            "type": "object", "required": ["projectId", "id"],
            "properties": { "projectId": { "type": "string" }, "id": { "type": "string" } }
        })),
        tool("list_knowledge_entries", "Lists every knowledge entry declared by a project.", json!({
            "type": "object", "required": ["projectId"],
            "properties": { "projectId": { "type": "string" } }
        })),
        tool("search_knowledge", "Case-insensitive substring search over id, term, display name, definition and aliases.", json!({
            "type": "object", "required": ["projectId", "query"],
            "properties": { "projectId": { "type": "string" }, "query": { "type": "string" } }
        })),
        tool("resolve_term", "Resolves free text to a canonical knowledge entry: exact id, then exact term, then an alias (case-insensitive).", json!({
            "type": "object", "required": ["projectId", "text"],
            "properties": { "projectId": { "type": "string" }, "text": { "type": "string" } }
        })),
        tool("get_term_relations", "Gets the Semantic Content Graph entities/relations related to a knowledge entry's subject, up to a hop depth (default 1).", json!({
            "type": "object", "required": ["projectId", "id"],
            "properties": {
                "projectId": { "type": "string" }, "id": { "type": "string" },
                "depth": { "type": "integer", "minimum": 1 }
            }
        })),
        tool("get_term_provenance", "Gets the source file provenance of a knowledge entry.", json!({
            "type": "object", "required": ["projectId", "id"],
            "properties": { "projectId": { "type": "string" }, "id": { "type": "string" } }
        })),
        tool("get_term_freshness", "Compares a knowledge entry's recorded content/dependency identity against its current source file and graph relations.", json!({
            "type": "object", "required": ["projectId", "id"],
            "properties": { "projectId": { "type": "string" }, "id": { "type": "string" } }
        })),
        tool("propose_knowledge_entry", "Creates a pending proposal for a new or revised knowledge entry from direct input (term, kind, definition, ...). Nothing becomes canonical until review_proposal accepts it.", json!({
            "type": "object", "required": ["projectId", "term", "definition"],
            "properties": {
                "projectId": { "type": "string" },
                "id": { "type": "string", "description": "Defaults to a slug of term." },
                "subject": { "type": "string", "description": "Graph entity this entry is about, if any." },
                "term": { "type": "string" },
                "displayName": { "type": "string" },
                "role": { "type": "string" },
                "kind": { "type": "string", "description": "concept|component|behavior|requirement|interaction|policy|skill|design-token|api|data-model|workflow|actor|domain|feature|artifact or custom" },
                "definition": { "type": "object", "required": ["short"], "properties": {
                    "short": { "type": "string" }, "semantic": { "type": "string" }
                }},
                "aliases": { "type": "array", "items": { "type": "string" } },
                "intent": { "type": "array", "items": { "type": "string" } },
                "requirements": { "type": "array", "items": { "type": "string" } },
                "constraints": { "type": "array", "items": { "type": "string" } },
                "behaviorStates": { "type": "array", "items": { "type": "string" } },
                "behaviorTransitions": { "type": "array", "items": { "type": "object", "required": ["from", "event", "to"], "properties": {
                    "from": { "type": "string" }, "event": { "type": "string" }, "to": { "type": "string" }
                }}}
            }
        })),
    ]
}

pub fn names() -> Vec<&'static str> {
    vec![
        "get_knowledge_entry",
        "list_knowledge_entries",
        "search_knowledge",
        "resolve_term",
        "get_term_relations",
        "get_term_provenance",
        "get_term_freshness",
        "propose_knowledge_entry",
    ]
}

fn parse<T: DeserializeOwned>(args: Value) -> Result<T, String> {
    serde_json::from_value(args).map_err(|e| format!("invalid arguments: {e}"))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectId {
    project_id: String,
}

/// Dispatches a knowledge-layer tool; `None` if `name` is not one of them.
pub fn call(engine: &SemanticEngine, domain: &NexusDomain, name: &str, args: Value) -> Option<Result<Value, String>> {
    let result = match name {
        "get_knowledge_entry" => (|| {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                project_id: String,
                id: String,
            }
            let a: A = parse(args)?;
            let project = engine.nexus_project(domain, &a.project_id).map_err(|e| e.to_string())?;
            let entry = project
                .semantic_contracts
                .iter()
                .find(|c| c.id == a.id)
                .ok_or_else(|| format!("no knowledge entry \"{}\" in project \"{}\"", a.id, a.project_id))?;
            Ok(json!(entry))
        })(),
        "list_knowledge_entries" => (|| {
            let a: ProjectId = parse(args)?;
            let project = engine.nexus_project(domain, &a.project_id).map_err(|e| e.to_string())?;
            Ok(json!(project.semantic_contracts))
        })(),
        "search_knowledge" => (|| {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                project_id: String,
                query: String,
            }
            let a: A = parse(args)?;
            let project = engine.nexus_project(domain, &a.project_id).map_err(|e| e.to_string())?;
            Ok(json!(knowledge::search(&project.semantic_contracts, &a.query)))
        })(),
        "resolve_term" => (|| {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                project_id: String,
                text: String,
            }
            let a: A = parse(args)?;
            let project = engine.nexus_project(domain, &a.project_id).map_err(|e| e.to_string())?;
            match knowledge::resolve_term(&project.semantic_contracts, &a.text) {
                Some(entry) => Ok(json!({ "resolved": true, "entry": entry })),
                None => Ok(json!({ "resolved": false, "entry": null })),
            }
        })(),
        "get_term_relations" => (|| {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                project_id: String,
                id: String,
                depth: Option<usize>,
            }
            let a: A = parse(args)?;
            let project = engine.nexus_project(domain, &a.project_id).map_err(|e| e.to_string())?;
            let entry = project
                .semantic_contracts
                .iter()
                .find(|c| c.id == a.id)
                .ok_or_else(|| format!("no knowledge entry \"{}\" in project \"{}\"", a.id, a.project_id))?;
            let (entities, relations) = get_related_entities(
                &domain.graph,
                &a.project_id,
                entry.subject(),
                RelatedEntitiesOptions {
                    depth: a.depth.unwrap_or(1),
                    relation_kinds: None,
                },
            )
            .map_err(|e| e.to_string())?;
            Ok(json!({ "subject": entry.subject(), "entities": entities, "relations": relations }))
        })(),
        "get_term_provenance" => (|| {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                project_id: String,
                id: String,
            }
            let a: A = parse(args)?;
            let project = engine.nexus_project(domain, &a.project_id).map_err(|e| e.to_string())?;
            let entry = project
                .semantic_contracts
                .iter()
                .find(|c| c.id == a.id)
                .ok_or_else(|| format!("no knowledge entry \"{}\" in project \"{}\"", a.id, a.project_id))?;
            Ok(json!(entry.provenance))
        })(),
        "get_term_freshness" => (|| {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                project_id: String,
                id: String,
            }
            let a: A = parse(args)?;
            let project = engine.nexus_project(domain, &a.project_id).map_err(|e| e.to_string())?;
            let entry = project
                .semantic_contracts
                .iter()
                .find(|c| c.id == a.id)
                .ok_or_else(|| format!("no knowledge entry \"{}\" in project \"{}\"", a.id, a.project_id))?;
            let Some(provenance) = &entry.provenance else {
                return Ok(json!({ "freshness": "unknown", "reason": "no source provenance recorded" }));
            };
            let path = engine.repo_root().join(&provenance.source);
            let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let (related, _) = get_related_entities(
                &domain.graph,
                &a.project_id,
                entry.subject(),
                RelatedEntitiesOptions { depth: 1, relation_kinds: None },
            )
            .map_err(|e| e.to_string())?;
            let related_ids: Vec<String> = related.into_iter().map(|e| e.id).collect();
            // No parser re-extracts knowledge entries yet, so there is no
            // "current" parser id/version to compare — this only checks
            // content/dependency/schema drift, same as before.
            let freshness = knowledge::compute_freshness(entry, &raw, &related_ids, None);
            Ok(json!({ "freshness": freshness }))
        })(),
        "propose_knowledge_entry" => (|| {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                project_id: String,
                id: Option<String>,
                subject: Option<String>,
                term: String,
                display_name: Option<String>,
                role: Option<String>,
                kind: Option<String>,
                definition: Definition,
                #[serde(default)]
                aliases: Vec<String>,
                #[serde(default)]
                intent: Vec<String>,
                #[serde(default)]
                requirements: Vec<String>,
                #[serde(default)]
                constraints: Vec<String>,
                #[serde(default)]
                behavior_states: Vec<String>,
                #[serde(default)]
                behavior_transitions: Vec<nexus_domain::types::BehaviorTransition>,
            }
            let a: A = parse(args)?;
            let project = domain.projects.require_project(&a.project_id).map_err(|e| e.to_string())?;
            let id = a.id.unwrap_or_else(|| nexus_semantic::text::slug(&a.term));
            let contract = knowledge::build_contract(ContractInput {
                name: a.display_name,
                subject: a.subject,
                kind: a.kind.map(ConceptKind::from),
                aliases: a.aliases,
                intent: a.intent,
                requirements: a.requirements,
                behavior_states: a.behavior_states,
                behavior_transitions: a.behavior_transitions,
                constraints: a.constraints,
                definition: Some(a.definition),
                ..ContractInput::new(id.clone(), a.term, a.role.map(Role::from).unwrap_or_else(|| Role::from("concept")))
            });
            let basis = nexus_semantic::Basis::candidate(
                nexus_semantic::Provenance {
                    source: format!("mcp:propose_knowledge_entry:{}", a.project_id),
                    line_start: None,
                    line_end: None,
                    commit: None,
                    extraction: "propose_knowledge_entry".to_string(),
                    artifact: None,
                },
                "proposed via propose_knowledge_entry",
            );
            let scope = format!("propose-knowledge:{id}");
            let proposal = proposal::from_contract(&a.project_id, scope, contract, basis);
            proposal::submit(&engine.store(&project), &proposal).map_err(|e| e.to_string())?;
            Ok(json!({ "proposalId": proposal.id, "proposal": proposal }))
        })(),
        _ => return None,
    };
    Some(result)
}
