//! The SemanticModel: the intermediate representation every artifact is
//! translated into, and every target representation is translated out of.

use crate::confidence;
use nexus_domain::types::RelationKind;
pub use nexus_domain::types::{Confidence, Evidence, Provenance};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub type Metadata = BTreeMap<String, Value>;

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }
    };
}

string_id!(ArtifactId);
string_id!(ProjectId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactKind {
    Project,
    Requirement,
    DesignSpec,
    BehaviorSpec,
    Component,
    DesignSystem,
    Skill,
    Policy,
    Code,
    Api,
    DataModel,
    Test,
    Documentation,
    Workflow,
    Commit,
    File,
    Contract,
    Directory,
}

impl ArtifactKind {
    pub const ALL: [ArtifactKind; 18] = [
        ArtifactKind::Project,
        ArtifactKind::Requirement,
        ArtifactKind::DesignSpec,
        ArtifactKind::BehaviorSpec,
        ArtifactKind::Component,
        ArtifactKind::DesignSystem,
        ArtifactKind::Skill,
        ArtifactKind::Policy,
        ArtifactKind::Code,
        ArtifactKind::Api,
        ArtifactKind::DataModel,
        ArtifactKind::Test,
        ArtifactKind::Documentation,
        ArtifactKind::Workflow,
        ArtifactKind::Commit,
        ArtifactKind::File,
        ArtifactKind::Contract,
        ArtifactKind::Directory,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            ArtifactKind::Project => "project",
            ArtifactKind::Requirement => "requirement",
            ArtifactKind::DesignSpec => "design-spec",
            ArtifactKind::BehaviorSpec => "behavior-spec",
            ArtifactKind::Component => "component",
            ArtifactKind::DesignSystem => "design-system",
            ArtifactKind::Skill => "skill",
            ArtifactKind::Policy => "policy",
            ArtifactKind::Code => "code",
            ArtifactKind::Api => "api",
            ArtifactKind::DataModel => "data-model",
            ArtifactKind::Test => "test",
            ArtifactKind::Documentation => "documentation",
            ArtifactKind::Workflow => "workflow",
            ArtifactKind::Commit => "commit",
            ArtifactKind::File => "file",
            ArtifactKind::Contract => "contract",
            ArtifactKind::Directory => "directory",
        }
    }

    pub fn parse(s: &str) -> Option<ArtifactKind> {
        let key = s.to_lowercase().replace(['_', ' '], "-");
        ArtifactKind::ALL.into_iter().find(|k| k.as_str() == key)
    }

    /// The role that usually owns this kind of artifact.
    pub fn default_role(&self) -> Role {
        match self {
            ArtifactKind::Project | ArtifactKind::Requirement => Role::Product,
            ArtifactKind::DesignSpec | ArtifactKind::DesignSystem => Role::Design,
            ArtifactKind::BehaviorSpec | ArtifactKind::Contract => Role::Ux,
            ArtifactKind::Component
            | ArtifactKind::Skill
            | ArtifactKind::Code
            | ArtifactKind::Api
            | ArtifactKind::DataModel => Role::Engineering,
            ArtifactKind::Test => Role::Qa,
            ArtifactKind::Policy => Role::Policy,
            ArtifactKind::Documentation => Role::Product,
            ArtifactKind::Workflow
            | ArtifactKind::Commit
            | ArtifactKind::File
            | ArtifactKind::Directory => Role::System,
        }
    }
}

/// The semantic role an artifact speaks for. Independent of the
/// game-studio org roles (`director`/`agent`), which model reporting lines.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum Role {
    Product,
    Ux,
    Design,
    Engineering,
    Qa,
    Architecture,
    Agent,
    Policy,
    Security,
    System,
    Custom(String),
}

impl Role {
    pub fn as_str(&self) -> &str {
        match self {
            Role::Product => "product",
            Role::Ux => "ux",
            Role::Design => "design",
            Role::Engineering => "engineering",
            Role::Qa => "qa",
            Role::Architecture => "architecture",
            Role::Agent => "agent",
            Role::Policy => "policy",
            Role::Security => "security",
            Role::System => "system",
            Role::Custom(s) => s,
        }
    }
}

impl From<String> for Role {
    fn from(s: String) -> Self {
        match s.to_lowercase().as_str() {
            "product" => Role::Product,
            "ux" => Role::Ux,
            "design" => Role::Design,
            "engineering" | "frontend" | "backend" => Role::Engineering,
            "qa" | "test" => Role::Qa,
            "architecture" => Role::Architecture,
            "agent" => Role::Agent,
            "policy" => Role::Policy,
            "security" => Role::Security,
            "system" => Role::System,
            _ => Role::Custom(s),
        }
    }
}

impl From<Role> for String {
    fn from(r: Role) -> Self {
        r.as_str().to_string()
    }
}

impl From<&str> for Role {
    fn from(s: &str) -> Self {
        Role::from(s.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ArtifactContent {
    /// Repo-relative path, forward slashes.
    File { path: String },
    Inline {
        text: String,
        #[serde(default, rename = "mediaType", skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
    },
}

/// Normalized representation of anything Nexus can reason about.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NexusArtifact {
    pub id: ArtifactId,
    pub project_id: ProjectId,
    pub kind: ArtifactKind,
    pub role: Role,
    pub name: String,
    pub content: ArtifactContent,
    pub provenance: Provenance,
    pub confidence: Confidence,
    #[serde(default)]
    pub metadata: Metadata,
    #[serde(default)]
    pub registered_at: u64,
}

impl NexusArtifact {
    /// Inline text, once the engine has loaded file content.
    pub fn text(&self) -> Option<&str> {
        match &self.content {
            ArtifactContent::Inline { text, .. } => Some(text),
            ArtifactContent::File { .. } => None,
        }
    }

    /// Lowercase extension of the source path or name, if any.
    pub fn extension(&self) -> Option<String> {
        let candidate = match &self.content {
            ArtifactContent::File { path } => path.as_str(),
            ArtifactContent::Inline { .. } => self.provenance.source.as_str(),
        };
        let file = candidate.rsplit('/').next().unwrap_or(candidate);
        let file = if file.contains('.') { file } else { self.name.as_str() };
        file.rsplit_once('.').map(|(_, ext)| ext.to_lowercase())
    }

    pub fn media_type(&self) -> Option<&str> {
        match &self.content {
            ArtifactContent::Inline { media_type, .. } => media_type.as_deref(),
            ArtifactContent::File { .. } => None,
        }
    }

    /// Provenance for a statement at `line_start..=line_end` of this artifact.
    pub fn at(&self, extraction: &str, lines: Option<(u32, u32)>) -> Provenance {
        Provenance {
            source: self.provenance.source.clone(),
            line_start: lines.map(|l| l.0),
            line_end: lines.map(|l| l.1),
            commit: self.provenance.commit.clone(),
            extraction: extraction.to_string(),
            artifact: Some(self.id.0.clone()),
        }
    }
}

/// Evidence, confidence and provenance shared by every semantic statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Basis {
    pub evidence: Evidence,
    pub confidence: Confidence,
    #[serde(default)]
    pub provenance: Vec<Provenance>,
}

impl Basis {
    pub fn explicit(provenance: Provenance) -> Self {
        Self {
            evidence: Evidence::Explicit,
            confidence: confidence::explicit(),
            provenance: vec![provenance],
        }
    }

    pub fn inferred(provenance: Vec<Provenance>, reason: &str) -> Self {
        let sources = distinct_sources(&provenance);
        Self {
            evidence: Evidence::Inferred,
            confidence: confidence::inferred(sources, reason),
            provenance,
        }
    }

    pub fn candidate(provenance: Provenance, reason: &str) -> Self {
        Self {
            evidence: Evidence::Candidate,
            confidence: confidence::candidate(reason),
            provenance: vec![provenance],
        }
    }

    pub fn unknown(provenance: Provenance, reason: &str) -> Self {
        Self {
            evidence: Evidence::Unknown,
            confidence: confidence::unknown(reason),
            provenance: vec![provenance],
        }
    }
}

pub fn distinct_sources(provenance: &[Provenance]) -> usize {
    provenance
        .iter()
        .map(|p| p.source.as_str())
        .collect::<BTreeSet<_>>()
        .len()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Intent {
    pub id: String,
    pub statement: String,
    #[serde(flatten)]
    pub basis: Basis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RequirementKind {
    Functional,
    Capability,
    Input,
    Output,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Requirement {
    pub id: String,
    pub statement: String,
    pub kind: RequirementKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(flatten)]
    pub basis: Basis,
}

/// One state transition: `from --event--> to` on `subject`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Behavior {
    pub id: String,
    pub subject: String,
    pub from: String,
    pub event: String,
    pub to: String,
    #[serde(flatten)]
    pub basis: Basis,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticEntity {
    pub id: String,
    pub name: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<ArtifactId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: Metadata,
    #[serde(flatten)]
    pub basis: Basis,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub id: String,
    pub name: String,
    pub subject: String,
    #[serde(flatten)]
    pub basis: Basis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConstraintKind {
    Must,
    MustNot,
    Convention,
    Accessibility,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Constraint {
    pub id: String,
    pub statement: String,
    pub kind: ConstraintKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(flatten)]
    pub basis: Basis,
}

/// Something that happens to an entity from outside: a click, an HTTP call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Interaction {
    pub id: String,
    pub trigger: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
    #[serde(flatten)]
    pub basis: Basis,
}

/// A directed semantic relation between two entities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dependency {
    pub id: String,
    pub from: String,
    pub relation: RelationKind,
    pub to: String,
    #[serde(flatten)]
    pub basis: Basis,
}

impl Dependency {
    pub fn new(from: &str, relation: RelationKind, to: &str, basis: Basis) -> Self {
        Self {
            id: format!("{from}|{}|{to}", relation.as_str()),
            from: from.to_string(),
            relation,
            to: to.to_string(),
            basis,
        }
    }
}

/// Something a parser/translator had to assume, stated so it is never
/// mistaken for a fact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assumption {
    pub id: String,
    pub statement: String,
    pub reason: String,
    #[serde(flatten)]
    pub basis: Basis,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticModel {
    #[serde(default)]
    pub artifacts: Vec<ArtifactId>,
    #[serde(default)]
    pub intent: Vec<Intent>,
    #[serde(default)]
    pub requirements: Vec<Requirement>,
    #[serde(default)]
    pub behaviors: Vec<Behavior>,
    #[serde(default)]
    pub entities: Vec<SemanticEntity>,
    #[serde(default)]
    pub states: Vec<State>,
    #[serde(default)]
    pub constraints: Vec<Constraint>,
    #[serde(default)]
    pub interactions: Vec<Interaction>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    pub confidence: Confidence,
    #[serde(default)]
    pub provenance: Vec<Provenance>,
    #[serde(default)]
    pub assumptions: Vec<Assumption>,
}

impl Default for SemanticModel {
    fn default() -> Self {
        Self {
            artifacts: Vec::new(),
            intent: Vec::new(),
            requirements: Vec::new(),
            behaviors: Vec::new(),
            entities: Vec::new(),
            states: Vec::new(),
            constraints: Vec::new(),
            interactions: Vec::new(),
            dependencies: Vec::new(),
            confidence: confidence::unknown("no statements extracted"),
            provenance: Vec::new(),
            assumptions: Vec::new(),
        }
    }
}

/// A borrowed view of one statement, for code that treats all categories alike.
pub struct Item<'a> {
    pub category: &'static str,
    pub id: &'a str,
    pub basis: &'a Basis,
}

impl SemanticModel {
    pub fn items(&self) -> Vec<Item<'_>> {
        let mut out = Vec::new();
        macro_rules! push {
            ($field:ident, $cat:literal) => {
                for x in &self.$field {
                    out.push(Item {
                        category: $cat,
                        id: &x.id,
                        basis: &x.basis,
                    });
                }
            };
        }
        push!(intent, "intent");
        push!(requirements, "requirement");
        push!(behaviors, "behavior");
        push!(entities, "entity");
        push!(states, "state");
        push!(constraints, "constraint");
        push!(interactions, "interaction");
        push!(dependencies, "dependency");
        push!(assumptions, "assumption");
        out
    }

    pub fn is_empty(&self) -> bool {
        self.items().is_empty()
    }

    pub fn entity(&self, id: &str) -> Option<&SemanticEntity> {
        self.entities.iter().find(|e| e.id == id)
    }

    /// Sorts every category by id, drops exact duplicate ids (first wins),
    /// and recomputes the model-level confidence and provenance.
    pub fn finalize(&mut self) {
        macro_rules! tidy {
            ($field:ident) => {
                self.$field.sort_by(|a, b| a.id.cmp(&b.id));
                self.$field.dedup_by(|a, b| a.id == b.id);
            };
        }
        tidy!(intent);
        tidy!(requirements);
        tidy!(behaviors);
        tidy!(entities);
        tidy!(states);
        tidy!(constraints);
        tidy!(interactions);
        tidy!(dependencies);
        tidy!(assumptions);
        self.artifacts.sort();
        self.artifacts.dedup();

        let items = self.items();
        let mut provenance: BTreeSet<Provenance> = BTreeSet::new();
        for item in &items {
            provenance.extend(item.basis.provenance.iter().cloned());
        }
        let bases: Vec<&Basis> = items.iter().map(|i| i.basis).collect();
        self.confidence = confidence::aggregate(&bases);
        self.provenance = provenance.into_iter().collect();
    }
}

fn key(statement: &str) -> String {
    let s = crate::text::slug(statement);
    match s.char_indices().nth(80) {
        Some((i, _)) => s[..i].trim_end_matches('-').to_string(),
        None => s,
    }
}

impl Intent {
    pub fn new(statement: &str, basis: Basis) -> Self {
        Self {
            id: format!("intent:{}", key(statement)),
            statement: statement.trim().to_string(),
            basis,
        }
    }
}

impl Requirement {
    pub fn new(subject: Option<&str>, statement: &str, kind: RequirementKind, basis: Basis) -> Self {
        Self {
            id: format!("requirement:{}:{}", subject.unwrap_or("-"), key(statement)),
            statement: statement.trim().to_string(),
            kind,
            subject: subject.map(str::to_string),
            basis,
        }
    }
}

impl Behavior {
    pub fn new(subject: &str, from: &str, event: &str, to: &str, basis: Basis) -> Self {
        Self {
            id: format!("{subject}:{from}--{event}->{to}"),
            subject: subject.to_string(),
            from: from.to_string(),
            event: event.to_string(),
            to: to.to_string(),
            basis,
        }
    }
}

impl SemanticEntity {
    pub fn new(id: &str, name: &str, kind: &str, basis: Basis) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            kind: kind.to_string(),
            description: None,
            role: None,
            artifact: None,
            attributes: Metadata::new(),
            basis,
        }
    }
}

impl State {
    pub fn new(subject: &str, name: &str, basis: Basis) -> Self {
        Self {
            id: format!("{subject}/{name}"),
            name: name.to_string(),
            subject: subject.to_string(),
            basis,
        }
    }
}

impl Constraint {
    pub fn new(subject: Option<&str>, statement: &str, kind: ConstraintKind, basis: Basis) -> Self {
        Self {
            id: format!("constraint:{}:{}", subject.unwrap_or("-"), key(statement)),
            statement: statement.trim().to_string(),
            kind,
            subject: subject.map(str::to_string),
            basis,
        }
    }
}

impl Interaction {
    pub fn new(trigger: &str, target: Option<&str>, effect: Option<&str>, basis: Basis) -> Self {
        Self {
            id: format!(
                "interaction:{}:{}:{}",
                target.unwrap_or("-"),
                trigger,
                effect.unwrap_or("-")
            ),
            trigger: trigger.to_string(),
            target: target.map(str::to_string),
            effect: effect.map(str::to_string),
            basis,
        }
    }
}

impl Assumption {
    pub fn new(statement: &str, reason: &str, basis: Basis) -> Self {
        Self {
            id: format!("assumption:{}", key(statement)),
            statement: statement.to_string(),
            reason: reason.to_string(),
            basis,
        }
    }
}

/// Classifies prose by modal keyword (RFC 2119 style).
pub fn modal_kind(statement: &str) -> Option<ConstraintKind> {
    let s = format!(" {} ", statement.to_lowercase());
    const MUST_NOT: &[&str] = &[
        " must not ", " must never ", " never ", " do not ", " don't ", " cannot ", " can't ",
        " shall not ", " may not ", " should not ",
    ];
    const MUST: &[&str] = &[" must ", " always ", " shall ", " required ", " requires "];
    if MUST_NOT.iter().any(|k| s.contains(k)) {
        Some(ConstraintKind::MustNot)
    } else if MUST.iter().any(|k| s.contains(k)) {
        Some(ConstraintKind::Must)
    } else {
        None
    }
}
