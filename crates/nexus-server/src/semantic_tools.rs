//! MCP tools for the semantic translation pipeline. Thin adapters: parse
//! arguments, call `SemanticEngine`, serialize the result. No semantics here.

use nexus_domain::types::{ClientInfo, ContextRequest};
use nexus_domain::NexusDomain;
use nexus_semantic::engine::{DiffAgainst, RegisterRequest, ValidateTarget};
use nexus_semantic::ingest::IngestOptions;
use nexus_semantic::proposal::Decision;
use nexus_semantic::translate::TranslationDirection;
use nexus_semantic::{block_on, ArtifactId, ArtifactKind, Metadata, Role, SemanticEngine};
use nexus_tools::ToolDescriptor;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;

fn tool(name: &str, description: &str, schema: Value) -> ToolDescriptor {
    ToolDescriptor {
        name: name.into(),
        description: description.into(),
        input_schema: schema,
    }
}

fn client_schema() -> Value {
    json!({ "type": "object", "required": ["id", "type"], "properties": {
        "id": { "type": "string" }, "type": { "type": "string" }
    }})
}

pub fn descriptors() -> Vec<ToolDescriptor> {
    let kinds: Vec<&str> = ArtifactKind::ALL.iter().map(|k| k.as_str()).collect();
    let directions: Vec<&str> = TranslationDirection::ALL
        .iter()
        .map(|d| d.as_str())
        .collect();
    let ids = json!({ "type": "array", "items": { "type": "string" } });
    vec![
        tool("register_artifact", "Registers an artifact (a repo file via `path`, or inline `content`) with kind, role and provenance. Returns the NexusArtifact.", json!({
            "type": "object", "required": ["projectId", "kind", "name"],
            "properties": {
                "projectId": { "type": "string" },
                "kind": { "type": "string", "enum": kinds },
                "name": { "type": "string" },
                "role": { "type": "string", "description": "product|ux|design|engineering|qa|architecture|agent|policy|security|system or custom" },
                "path": { "type": "string", "description": "Repo- or project-relative file path." },
                "content": { "type": "string" },
                "mediaType": { "type": "string" },
                "metadata": { "type": "object" }
            }
        })),
        tool("inspect", "Parses a registered artifact into a SemanticModel (intent, requirements, behaviors, entities, states, constraints, interactions, dependencies, assumptions) with evidence, confidence and provenance per statement.", json!({
            "type": "object", "required": ["projectId", "artifactId"],
            "properties": { "projectId": { "type": "string" }, "artifactId": { "type": "string" }, "persist": { "type": "boolean", "default": true } }
        })),
        tool("translate", "Translates the SemanticModel of artifacts (default: whole project) into a target role representation. `contextSkills` adds target-system rules; clashes are reported as conflicts requiring a human decision.", json!({
            "type": "object", "required": ["projectId", "direction"],
            "properties": {
                "projectId": { "type": "string" },
                "direction": { "type": "string", "enum": directions },
                "artifactIds": ids,
                "contextSkills": { "type": "array", "items": { "type": "string" }, "description": "Skill ids (`react`, `skill://react`)." },
                "persist": { "type": "boolean", "default": true }
            }
        })),
        tool("validate", "Validates artifacts' SemanticModel, a stored translation (`translationId`) or the whole project; optionally against a semantic contract.", json!({
            "type": "object", "required": ["projectId"],
            "properties": { "projectId": { "type": "string" }, "artifactIds": ids, "translationId": { "type": "string" }, "contractId": { "type": "string" } }
        })),
        tool("diff", "Semantic diff of an artifact: against its stored model (default), another artifact (`againstArtifact`), or its content at a Git revision (`revision`).", json!({
            "type": "object", "required": ["projectId", "artifactId"],
            "properties": { "projectId": { "type": "string" }, "artifactId": { "type": "string" }, "againstArtifact": { "type": "string" }, "revision": { "type": "string" } }
        })),
        tool("propose", "Creates a pending proposal with the graph changes the SemanticModel implies (default: whole project). Nothing becomes canonical until review_proposal accepts it.", json!({
            "type": "object", "required": ["projectId"],
            "properties": { "projectId": { "type": "string" }, "artifactIds": ids }
        })),
        tool("review_proposal", "Accepts (writes to graph/, evidence unchanged) or rejects a pending proposal. The client needs the `review:proposals` permission.", json!({
            "type": "object", "required": ["projectId", "proposalId", "decision", "client"],
            "properties": {
                "projectId": { "type": "string" }, "proposalId": { "type": "string" },
                "decision": { "type": "string", "enum": ["accept", "reject"] },
                "client": client_schema(), "note": { "type": "string" }
            }
        })),
        tool("ingest_repository", "Scans a directory (default: the project directory) and its Git history: registers and inspects every recognized file, records commits, computes semantic diffs of the newest commit, validates, and creates a pending proposal.", json!({
            "type": "object", "required": ["projectId"],
            "properties": {
                "projectId": { "type": "string" },
                "path": { "type": "string", "description": "Directory inside the repository root." },
                "maxCommits": { "type": "integer", "minimum": 0, "default": 20 },
                "diffBase": { "type": "string", "description": "Revision to diff the working tree against." },
                "propose": { "type": "boolean", "default": true }
            }
        })),
        tool("list_artifacts", "Lists the artifacts registered for a project.", json!({
            "type": "object", "required": ["projectId"], "properties": { "projectId": { "type": "string" } }
        })),
        tool("resolve_semantic_context", "resolve_context plus the EngineeringContext of the resolved skills and the model statements about the resolved entities.", json!({
            "type": "object", "required": ["projectId", "task", "client"],
            "properties": {
                "projectId": { "type": "string" }, "task": { "type": "string" }, "client": client_schema(),
                "requestedEntities": ids
            }
        })),
        tool("export_semantic_graph", "Writes semantic-graph.json and the standalone semantic-project.html explorer (default: projects/<id>/semantic/).", json!({
            "type": "object", "required": ["projectId"],
            "properties": { "projectId": { "type": "string" }, "outDir": { "type": "string" }, "includeStructure": { "type": "boolean", "default": false } }
        })),
    ]
}

pub fn names() -> Vec<&'static str> {
    vec![
        "register_artifact",
        "inspect",
        "translate",
        "validate",
        "diff",
        "propose",
        "review_proposal",
        "ingest_repository",
        "list_artifacts",
        "resolve_semantic_context",
        "export_semantic_graph",
    ]
}

fn parse<T: DeserializeOwned>(args: Value) -> Result<T, String> {
    serde_json::from_value(args).map_err(|e| format!("invalid arguments: {e}"))
}

fn ids(v: Vec<String>) -> Vec<ArtifactId> {
    v.into_iter().map(ArtifactId).collect()
}

/// Resolves a user-supplied directory; it must stay inside the repository.
fn inside_repo(engine: &SemanticEngine, path: &str) -> Result<PathBuf, String> {
    let root = engine
        .repo_root()
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let abs = engine.repo_root().join(path);
    let canon = abs
        .canonicalize()
        .map_err(|_| format!("no directory at \"{path}\""))?;
    if !canon.starts_with(&root) {
        return Err(format!("\"{path}\" is outside the repository root"));
    }
    Ok(abs)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Project {
    project_id: String,
}

/// Dispatches a semantic tool; `None` if `name` is not one of them.
pub fn call(
    engine: &SemanticEngine,
    domain: &NexusDomain,
    name: &str,
    args: Value,
) -> Option<Result<Value, String>> {
    let result = match name {
        "register_artifact" => (|| {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                project_id: String,
                kind: String,
                name: String,
                role: Option<String>,
                path: Option<String>,
                content: Option<String>,
                media_type: Option<String>,
                #[serde(default)]
                metadata: Metadata,
            }
            let a: A = parse(args)?;
            let kind = ArtifactKind::parse(&a.kind)
                .ok_or_else(|| format!("unknown artifact kind \"{}\"", a.kind))?;
            let artifact = engine
                .register(
                    domain,
                    RegisterRequest {
                        project_id: a.project_id,
                        kind,
                        role: a.role.map(Role::from),
                        name: a.name,
                        path: a.path,
                        content: a.content,
                        media_type: a.media_type,
                        metadata: a.metadata,
                    },
                )
                .map_err(|e| e.to_string())?;
            Ok(json!({ "artifactId": artifact.id, "status": "registered", "artifact": artifact }))
        })(),
        "inspect" => (|| {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                project_id: String,
                artifact_id: String,
                persist: Option<bool>,
            }
            let a: A = parse(args)?;
            let out = block_on(engine.inspect(
                domain,
                &a.project_id,
                &ArtifactId(a.artifact_id),
                a.persist.unwrap_or(true),
            ))
            .map_err(|e| e.to_string())?;
            Ok(json!(out))
        })(),
        "translate" => (|| {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                project_id: String,
                direction: String,
                #[serde(default)]
                artifact_ids: Vec<String>,
                #[serde(default)]
                context_skills: Vec<String>,
                persist: Option<bool>,
            }
            let a: A = parse(args)?;
            let direction = TranslationDirection::parse(&a.direction)
                .ok_or_else(|| format!("unknown direction \"{}\"", a.direction))?;
            let t = block_on(engine.translate(
                domain,
                &a.project_id,
                &ids(a.artifact_ids),
                direction,
                &a.context_skills,
                a.persist.unwrap_or(true),
            ))
            .map_err(|e| e.to_string())?;
            Ok(json!({ "translationId": t.id, "translation": t }))
        })(),
        "validate" => (|| {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                project_id: String,
                #[serde(default)]
                artifact_ids: Vec<String>,
                translation_id: Option<String>,
                contract_id: Option<String>,
            }
            let a: A = parse(args)?;
            let target = match (a.translation_id, a.artifact_ids.is_empty()) {
                (Some(t), _) => ValidateTarget::Translation(t),
                (None, false) => ValidateTarget::Artifacts(ids(a.artifact_ids)),
                (None, true) => ValidateTarget::Project,
            };
            let r =
                block_on(engine.validate(domain, &a.project_id, target, a.contract_id.as_deref()))
                    .map_err(|e| e.to_string())?;
            Ok(json!(r))
        })(),
        "diff" => (|| {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                project_id: String,
                artifact_id: String,
                against_artifact: Option<String>,
                revision: Option<String>,
            }
            let a: A = parse(args)?;
            let against = match (a.against_artifact, a.revision) {
                (Some(other), None) => DiffAgainst::Artifact(ArtifactId(other)),
                (None, Some(rev)) => DiffAgainst::Revision(rev),
                (None, None) => DiffAgainst::Stored,
                _ => return Err("give at most one of againstArtifact / revision".to_string()),
            };
            let d =
                block_on(engine.diff(domain, &a.project_id, &ArtifactId(a.artifact_id), against))
                    .map_err(|e| e.to_string())?;
            Ok(json!(d))
        })(),
        "propose" => (|| {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                project_id: String,
                #[serde(default)]
                artifact_ids: Vec<String>,
            }
            let a: A = parse(args)?;
            let p = block_on(engine.propose(domain, &a.project_id, &ids(a.artifact_ids)))
                .map_err(|e| e.to_string())?;
            Ok(json!({ "proposalId": p.id, "proposal": p }))
        })(),
        "review_proposal" => (|| {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                project_id: String,
                proposal_id: String,
                decision: Decision,
                client: ClientInfo,
                note: Option<String>,
            }
            let a: A = parse(args)?;
            let p = engine
                .review(
                    domain,
                    &a.project_id,
                    &a.proposal_id,
                    a.decision,
                    &a.client,
                    a.note,
                )
                .map_err(|e| e.to_string())?;
            Ok(json!(p))
        })(),
        "ingest_repository" => (|| {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                project_id: String,
                path: Option<String>,
                max_commits: Option<usize>,
                diff_base: Option<String>,
                propose: Option<bool>,
            }
            let a: A = parse(args)?;
            let root = a
                .path
                .as_deref()
                .map(|p| inside_repo(engine, p))
                .transpose()?;
            let opts = IngestOptions {
                root,
                max_commits: a.max_commits.unwrap_or(20),
                diff_base: a.diff_base,
                propose: a.propose.unwrap_or(true),
                persist: true,
            };
            let (report, _) =
                block_on(engine.ingest(domain, &a.project_id, opts)).map_err(|e| e.to_string())?;
            Ok(json!(report))
        })(),
        "list_artifacts" => (|| {
            let a: Project = parse(args)?;
            Ok(json!(engine
                .artifacts(domain, &a.project_id)
                .map_err(|e| e.to_string())?))
        })(),
        "resolve_semantic_context" => (|| {
            let request: ContextRequest = parse(args)?;
            Ok(json!(
                block_on(engine.semantic_context(domain, &request)).map_err(|e| e.to_string())?
            ))
        })(),
        "export_semantic_graph" => (|| {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                project_id: String,
                out_dir: Option<String>,
                include_structure: Option<bool>,
            }
            let a: A = parse(args)?;
            if let Some(d) = &a.out_dir {
                if std::path::Path::new(d).is_absolute() || d.split(['/', '\\']).any(|c| c == "..")
                {
                    return Err(format!(
                        "outDir \"{d}\" must be a relative path inside the repository"
                    ));
                }
            }
            let out_dir = a.out_dir.map(|d| engine.repo_root().join(d));
            let out = block_on(engine.export(
                domain,
                &a.project_id,
                out_dir,
                a.include_structure.unwrap_or(false),
            ))
            .map_err(|e| e.to_string())?;
            Ok(json!(out))
        })(),
        _ => return None,
    };
    Some(result)
}
