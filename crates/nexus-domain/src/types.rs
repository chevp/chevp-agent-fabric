use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::PathBuf;

/// A client/agent talking to the Control Plane. Never hard-coded per-vendor;
/// `id` is whatever a policy file says it is (e.g. "copilot", "figma-make").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientInfo {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
}

/// The bounded workspace and source of truth for a body of work.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub version: u32,
    pub description: Option<String>,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillScope {
    Global,
    Project,
}

impl SkillScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            SkillScope::Global => "global",
            SkillScope::Project => "project",
        }
    }
}

/// Machine-readable frontmatter of a Markdown skill file.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillFrontmatter {
    pub id: String,
    pub name: String,
    pub version: String,
    pub scope: SkillScope,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, rename = "dependsOn")]
    pub depends_on: Vec<String>,
}

/// A fully loaded skill: frontmatter plus its Markdown instruction body.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub version: String,
    pub scope: SkillScope,
    pub description: String,
    pub tags: Vec<String>,
    pub depends_on: Vec<String>,
    pub body: String,
    pub project_id: Option<String>,
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorTransition {
    pub from: String,
    pub event: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorRule {
    pub id: String,
    pub description: String,
}

/// A structured, technology-agnostic UX/product behavior specification.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BehaviorSpec {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub version: u32,
    pub states: Vec<String>,
    pub transitions: Vec<BehaviorTransition>,
    pub rules: Vec<BehaviorRule>,
    pub project_id: String,
    pub source_path: PathBuf,
}

/// A node in the Semantic Content Graph.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEntity {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    /// Any additional descriptive fields declared on the entity file.
    pub attributes: Map<String, Value>,
    pub project_id: String,
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RelationKind {
    Implements,
    DependsOn,
    Uses,
    DefinedBy,
    GovernedBy,
    RepresentedBy,
    ImplementedBy,
    ValidatedBy,
    RelatedTo,
}

/// A directed edge in the Semantic Content Graph.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphRelation {
    pub from: String,
    pub relation: RelationKind,
    pub to: String,
    pub project_id: String,
    pub source_path: PathBuf,
}

/// A single client's permission grant, global or for one project.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyRule {
    pub client: String,
    pub permissions: Vec<String>,
    pub project_id: Option<String>,
    pub source_path: PathBuf,
}

/// A single Markdown document under a project's `context/` directory.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextDocument {
    pub id: String,
    pub title: String,
    pub body: String,
    pub source_path: PathBuf,
}

/// Input to the Context Resolver.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextRequest {
    pub project_id: String,
    pub task: String,
    pub client: ClientInfo,
    #[serde(default)]
    pub requested_entities: Vec<String>,
}

/// The minimal relevant context package returned by the Context Resolver.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedContext {
    pub project: Project,
    pub context_documents: Vec<ContextDocument>,
    pub skills: Vec<Skill>,
    pub behaviors: Vec<BehaviorSpec>,
    pub entities: Vec<GraphEntity>,
    pub relations: Vec<GraphRelation>,
    pub policies: Vec<PolicyRule>,
}
