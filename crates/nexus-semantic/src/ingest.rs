//! Repository ingestion helpers: walking a tree, classifying files into
//! artifact kinds, and the directory-structure model. `SemanticEngine::ingest`
//! drives the full run (register -> inspect -> consolidate -> validate ->
//! Git history -> semantic diffs -> proposal).

use crate::diff::SemanticChange;
use crate::model::*;
use crate::parser::CodeMetadataParser;
use crate::validate::ValidationResult;
use nexus_domain::types::RelationKind;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Directories never descended into.
pub const SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "dist",
    "build",
    "out",
    ".next",
    ".turbo",
    ".cache",
    "coverage",
    ".idea",
    ".vscode",
    "__pycache__",
    ".venv",
    "venv",
];

pub const MAX_FILE_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone)]
pub struct IngestOptions {
    /// Directory to scan; defaults to the project directory.
    pub root: Option<PathBuf>,
    pub max_commits: usize,
    /// Revision to diff the working tree against; defaults to the parent of
    /// the newest commit that touched the root.
    pub diff_base: Option<String>,
    pub propose: bool,
    pub persist: bool,
}

impl Default for IngestOptions {
    fn default() -> Self {
        Self {
            root: None,
            max_commits: 20,
            diff_base: None,
            propose: true,
            persist: true,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct IngestedArtifact {
    pub id: ArtifactId,
    pub kind: ArtifactKind,
    pub role: Role,
    pub path: String,
    pub parser: String,
    pub statements: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Skipped {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChanges {
    pub path: String,
    pub base: String,
    pub changes: Vec<SemanticChange>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestReport {
    pub project_id: String,
    pub root: String,
    pub git_available: bool,
    pub head: Option<String>,
    pub files_scanned: usize,
    pub directories: usize,
    pub artifacts: Vec<IngestedArtifact>,
    pub skipped: Vec<Skipped>,
    pub commits: usize,
    pub semantic_changes: Vec<FileChanges>,
    pub model: ModelSummary,
    pub validation: ValidationResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub superseded: Vec<String>,
    pub generated_at: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ModelSummary {
    pub intent: usize,
    pub requirements: usize,
    pub behaviors: usize,
    pub entities: usize,
    pub states: usize,
    pub constraints: usize,
    pub interactions: usize,
    pub dependencies: usize,
    pub assumptions: usize,
    pub explicit: usize,
    pub inferred: usize,
    pub candidate: usize,
    pub unknown: usize,
    pub confidence: f32,
}

impl ModelSummary {
    pub fn of(m: &SemanticModel) -> Self {
        let items = m.items();
        let count = |e: Evidence| items.iter().filter(|i| i.basis.evidence == e).count();
        Self {
            intent: m.intent.len(),
            requirements: m.requirements.len(),
            behaviors: m.behaviors.len(),
            entities: m.entities.len(),
            states: m.states.len(),
            constraints: m.constraints.len(),
            interactions: m.interactions.len(),
            dependencies: m.dependencies.len(),
            assumptions: m.assumptions.len(),
            explicit: count(Evidence::Explicit),
            inferred: count(Evidence::Inferred),
            candidate: count(Evidence::Candidate),
            unknown: count(Evidence::Unknown),
            confidence: m.confidence.value,
        }
    }
}

/// Kind and role of a file by path convention. `None` = not an artifact.
pub fn classify(rel: &str) -> Option<(ArtifactKind, Role)> {
    let lower = rel.to_lowercase();
    let file = lower.rsplit('/').next().unwrap_or(&lower);
    let ext = file.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    let in_dir = |d: &str| lower.starts_with(&format!("{d}/")) || lower.contains(&format!("/{d}/"));
    let k = |kind: ArtifactKind| Some((kind, kind.default_role()));

    if file == "project.yaml" && !lower.contains('/') {
        return k(ArtifactKind::Project);
    }
    if file == "skill.md" {
        return k(ArtifactKind::Skill);
    }
    if in_dir("contracts") && matches!(ext, "yaml" | "yml") {
        return k(ArtifactKind::Contract);
    }
    if in_dir("behavior") && matches!(ext, "yaml" | "yml") {
        return k(ArtifactKind::BehaviorSpec);
    }
    if in_dir("policies") && matches!(ext, "yaml" | "yml") {
        return k(ArtifactKind::Policy);
    }
    if lower.contains(".github/workflows/") {
        return k(ArtifactKind::Workflow);
    }
    if file.contains("openapi")
        || file.contains("swagger")
        || matches!(ext, "proto" | "graphql" | "gql")
    {
        return k(ArtifactKind::Api);
    }
    if file.contains("token") && matches!(ext, "json" | "yaml" | "yml") {
        return k(ArtifactKind::DesignSystem);
    }
    if (in_dir("design-system") || in_dir("design")) && matches!(ext, "json" | "yaml" | "yml") {
        return k(ArtifactKind::DesignSystem);
    }
    if matches!(ext, "html" | "htm") {
        return k(ArtifactKind::DesignSpec);
    }
    if file.ends_with(".schema.json") || matches!(ext, "sql" | "prisma") {
        return k(ArtifactKind::DataModel);
    }
    if CodeMetadataParser::is_test_path(&lower) && crate::parser::CODE_EXTENSIONS.contains(&ext) {
        return k(ArtifactKind::Test);
    }
    if crate::parser::CODE_EXTENSIONS.contains(&ext) {
        let kind = if in_dir("components") && matches!(ext, "tsx" | "jsx" | "vue" | "svelte") {
            ArtifactKind::Component
        } else {
            ArtifactKind::Code
        };
        return k(kind);
    }
    if matches!(ext, "md" | "markdown" | "mdx" | "txt" | "rst") {
        let kind = if in_dir("requirements") || file.contains("requirement") {
            ArtifactKind::Requirement
        } else {
            ArtifactKind::Documentation
        };
        return k(kind);
    }
    if matches!(ext, "json" | "yaml" | "yml" | "toml") {
        return k(ArtifactKind::File);
    }
    None
}

/// Files (relative, forward slashes) and directories under `root`.
/// `skip_top` names top-level directories to leave out (e.g. `semantic`, `graph`).
pub fn walk(root: &Path, skip_top: &[&str]) -> (Vec<String>, Vec<String>) {
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let rel = crate::text::rel_path(root, &path);
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                let top = !rel.contains('/');
                if SKIP_DIRS.contains(&name.as_str()) || (top && skip_top.contains(&name.as_str()))
                {
                    continue;
                }
                dirs.push(rel);
                stack.push(path);
            } else if ft.is_file() {
                files.push(rel);
            }
        }
    }
    files.sort();
    dirs.sort();
    (files, dirs)
}

/// `dir:` entities with `contains` edges down to each file's artifact entity.
pub fn structure_model(
    root_label: &str,
    dirs: &[String],
    files: &[(String, String)],
) -> SemanticModel {
    let prov = |source: &str| Provenance {
        source: source.to_string(),
        line_start: None,
        line_end: None,
        commit: None,
        extraction: "RepositoryWalker".to_string(),
        artifact: None,
    };
    let dir_id = |d: &str| {
        if d.is_empty() {
            "dir:.".to_string()
        } else {
            format!("dir:{d}")
        }
    };
    let parent = |p: &str| {
        p.rsplit_once('/')
            .map(|(a, _)| a.to_string())
            .unwrap_or_default()
    };
    let mut m = SemanticModel::default();
    let mut root =
        SemanticEntity::new("dir:.", root_label, "directory", Basis::explicit(prov(".")));
    root.role = Some(Role::System);
    m.entities.push(root);
    for d in dirs {
        let mut e = SemanticEntity::new(&dir_id(d), d, "directory", Basis::explicit(prov(d)));
        e.role = Some(Role::System);
        m.entities.push(e);
        m.dependencies.push(Dependency::new(
            &dir_id(&parent(d)),
            RelationKind::Contains,
            &dir_id(d),
            Basis::explicit(prov(d)),
        ));
    }
    for (path, entity) in files {
        m.dependencies.push(Dependency::new(
            &dir_id(&parent(path)),
            RelationKind::Contains,
            entity,
            Basis::explicit(prov(path)),
        ));
    }
    m.finalize();
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_by_path_convention() {
        let kind = |p: &str| classify(p).map(|(k, _)| k);
        assert_eq!(kind("project.yaml"), Some(ArtifactKind::Project));
        assert_eq!(
            kind("skills/checkout-ux/skill.md"),
            Some(ArtifactKind::Skill)
        );
        assert_eq!(
            kind("behavior/checkout-button.yaml"),
            Some(ArtifactKind::BehaviorSpec)
        );
        assert_eq!(
            kind("contracts/checkout-submit.yaml"),
            Some(ArtifactKind::Contract)
        );
        assert_eq!(kind("design/tokens.json"), Some(ArtifactKind::DesignSystem));
        assert_eq!(kind("design/checkout.html"), Some(ArtifactKind::DesignSpec));
        assert_eq!(
            kind("src/components/Button.tsx"),
            Some(ArtifactKind::Component)
        );
        assert_eq!(kind("src/Button.test.tsx"), Some(ArtifactKind::Test));
        assert_eq!(kind("src/lib.rs"), Some(ArtifactKind::Code));
        assert_eq!(
            kind("context/overview.md"),
            Some(ArtifactKind::Documentation)
        );
        assert_eq!(
            kind(".github/workflows/ci.yml"),
            Some(ArtifactKind::Workflow)
        );
        assert_eq!(kind("logo.png"), None);
    }
}
