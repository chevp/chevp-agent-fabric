//! `SemanticEngine`: the service layer MCP tools and resources call.
//! Owns the parser registry; reads through `NexusDomain`; persists under
//! `projects/<id>/semantic/`; writes `graph/` only through proposal review.

use crate::consolidate::consolidate;
use crate::contract::NexusProject;
use crate::diff::{semantic_diff, SemanticChange, SemanticDiff};
use crate::engineering::EngineeringContext;
use crate::error::{SemanticError, SemanticResult};
use crate::graph_view::{self, SemanticGraph};
use crate::ingest::{
    self, FileChanges, IngestOptions, IngestReport, IngestedArtifact, ModelSummary, Skipped,
};
use crate::model::*;
use crate::parser::{ParserRegistry, SkillParser};
use crate::proposal::{self, Decision, Proposal};
use crate::store::{self, SemanticStore};
use crate::text::{now, rel_path, slug};
use crate::translate::{DeterministicTranslator, Translation, TranslationDirection, Translator};
use crate::validate::{self, ValidationResult};
use crate::{confidence, git};
use nexus_domain::policy::require_permission;
use nexus_domain::types::{ClientInfo, ContextRequest, Project, RelationKind, ResolvedContext};
use nexus_domain::NexusDomain;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Permission a client needs to accept/reject proposals.
pub const REVIEW_PERMISSION: &str = "review:proposals";

pub struct SemanticEngine {
    repo_root: PathBuf,
    parsers: ParserRegistry,
}

#[derive(Debug, Clone)]
pub struct RegisterRequest {
    pub project_id: String,
    pub kind: ArtifactKind,
    pub role: Option<Role>,
    pub name: String,
    /// Repo- or project-relative path; mutually exclusive with `content`.
    pub path: Option<String>,
    pub content: Option<String>,
    pub media_type: Option<String>,
    pub metadata: Metadata,
}

#[derive(Debug, Clone, Serialize)]
pub struct InspectOutcome {
    pub artifact: NexusArtifact,
    pub parser: String,
    pub model: SemanticModel,
}

#[derive(Debug, Clone)]
pub enum DiffAgainst {
    /// The model stored by the last inspect/ingest of the same artifact.
    Stored,
    Artifact(ArtifactId),
    /// The artifact's file content at a Git revision.
    Revision(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffOutcome {
    pub before: String,
    pub after: String,
    #[serde(flatten)]
    pub diff: SemanticDiff,
}

#[derive(Debug, Clone)]
pub enum ValidateTarget {
    Artifacts(Vec<ArtifactId>),
    Translation(String),
    Project,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub graph_json: String,
    pub html: String,
    pub nodes: usize,
    pub edges: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticContext {
    #[serde(flatten)]
    pub resolved: ResolvedContext,
    /// Engineering context of the resolved skills.
    pub engineering: EngineeringContext,
    /// Statements from the project model about the resolved entities.
    pub semantics: SemanticModel,
}

impl SemanticEngine {
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            repo_root: repo_root.into(),
            parsers: ParserRegistry::default(),
        }
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    /// Internal extension point: add a parser ahead of the built-ins.
    pub fn parsers_mut(&mut self) -> &mut ParserRegistry {
        &mut self.parsers
    }

    fn project(&self, domain: &NexusDomain, id: &str) -> SemanticResult<Project> {
        Ok(domain.projects.require_project(id)?)
    }

    pub fn store(&self, project: &Project) -> SemanticStore {
        SemanticStore::for_project(&project.path)
    }

    fn source_of(&self, abs: &Path) -> String {
        rel_path(&self.repo_root, abs)
    }

    fn abs_of(&self, source: &str) -> PathBuf {
        let p = PathBuf::from(source);
        if p.is_absolute() {
            p
        } else {
            self.repo_root.join(p)
        }
    }

    fn file_artifact(
        &self,
        project: &Project,
        abs: &Path,
        kind: ArtifactKind,
        role: Role,
        commit: Option<String>,
    ) -> NexusArtifact {
        let source = self.source_of(abs);
        let name = abs
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| source.clone());
        NexusArtifact {
            id: ArtifactId(format!("{}-{}", kind.as_str(), slug(&source))),
            project_id: ProjectId(project.id.clone()),
            kind,
            role,
            name,
            content: ArtifactContent::File {
                path: source.clone(),
            },
            provenance: Provenance {
                source: source.clone(),
                line_start: None,
                line_end: None,
                commit,
                extraction: "register_artifact".into(),
                artifact: None,
            },
            confidence: Confidence {
                value: confidence::EXPLICIT,
                reason: Some(format!("artifact exists at {source}")),
            },
            metadata: Metadata::new(),
            registered_at: now(),
        }
    }

    // ---- register / load / inspect -------------------------------------

    pub fn register(
        &self,
        domain: &NexusDomain,
        req: RegisterRequest,
    ) -> SemanticResult<NexusArtifact> {
        let project = self.project(domain, &req.project_id)?;
        let role = req.role.clone().unwrap_or_else(|| req.kind.default_role());
        let mut artifact = match (&req.path, &req.content) {
            (Some(path), None) => {
                let candidates = [self.repo_root.join(path), project.path.join(path)];
                let abs = candidates
                    .into_iter()
                    .find(|p| p.is_file())
                    .ok_or_else(|| {
                        SemanticError::InvalidInput(format!(
                            "no file at \"{path}\" (repo- or project-relative)"
                        ))
                    })?;
                let root = self
                    .repo_root
                    .canonicalize()
                    .map_err(|e| SemanticError::io(&self.repo_root, e))?;
                let canon = abs.canonicalize().map_err(|e| SemanticError::io(&abs, e))?;
                if !canon.starts_with(&root) {
                    return Err(SemanticError::InvalidInput(format!(
                        "\"{path}\" is outside the repository root"
                    )));
                }
                let source = self.source_of(&abs);
                let commit = git::last_commit_for(&self.repo_root, &source);
                let mut a = self.file_artifact(&project, &abs, req.kind, role, commit);
                a.name = req.name.clone();
                a
            }
            (None, Some(text)) => NexusArtifact {
                id: ArtifactId(format!("{}-{}", req.kind.as_str(), slug(&req.name))),
                project_id: ProjectId(project.id.clone()),
                kind: req.kind,
                role,
                name: req.name.clone(),
                content: ArtifactContent::Inline {
                    text: text.clone(),
                    media_type: req.media_type.clone(),
                },
                provenance: Provenance {
                    source: format!("inline:{}", req.name),
                    line_start: None,
                    line_end: None,
                    commit: None,
                    extraction: "register_artifact".into(),
                    artifact: None,
                },
                confidence: Confidence {
                    value: confidence::EXPLICIT,
                    reason: Some("content supplied by the registering client".into()),
                },
                metadata: Metadata::new(),
                registered_at: now(),
            },
            _ => {
                return Err(SemanticError::InvalidInput(
                    "give exactly one of \"path\" or \"content\"".into(),
                ))
            }
        };
        artifact.metadata = req.metadata;
        artifact.provenance.artifact = Some(artifact.id.0.clone());
        self.store(&project).save_artifact(&artifact)?;
        Ok(artifact)
    }

    /// Returns the artifact with its file content inlined.
    pub fn load(&self, artifact: &NexusArtifact) -> SemanticResult<NexusArtifact> {
        let ArtifactContent::File { path } = &artifact.content else {
            return Ok(artifact.clone());
        };
        let abs = self.abs_of(path);
        let text = std::fs::read(&abs).map_err(|e| SemanticError::io(&abs, e))?;
        let text = String::from_utf8(text).map_err(|_| SemanticError::Parse {
            source_path: path.clone(),
            message: "not UTF-8 text".into(),
        })?;
        let mut loaded = artifact.clone();
        loaded.content = ArtifactContent::Inline {
            text,
            media_type: None,
        };
        Ok(loaded)
    }

    /// `file:<source>` for file artifacts (also once loaded), else `artifact:<id>`.
    fn artifact_node_id(artifact: &NexusArtifact) -> String {
        let source = &artifact.provenance.source;
        if source.starts_with("inline:") || source.starts_with("git:") {
            format!("artifact:{}", artifact.id)
        } else {
            format!("file:{source}")
        }
    }

    /// Parses a loaded artifact and links its primary entities to the
    /// artifact node (`defined-by`).
    async fn parse(&self, artifact: &NexusArtifact) -> SemanticResult<(String, SemanticModel)> {
        let (parser, mut model) = self.parsers.inspect(artifact).await?;
        let node = Self::artifact_node_id(artifact);
        if model.entity(&node).is_none() {
            let mut e = SemanticEntity::new(
                &node,
                &artifact.name,
                "file",
                Basis::explicit(artifact.at("SemanticEngine", None)),
            );
            e.role = Some(artifact.role.clone());
            e.artifact = Some(artifact.id.clone());
            e.attributes
                .insert("artifactKind".into(), artifact.kind.as_str().into());
            model.entities.push(e);
        }
        let primary: Vec<String> = model
            .entities
            .iter()
            .filter(|e| e.id != node && e.artifact.as_ref() == Some(&artifact.id))
            .map(|e| e.id.clone())
            .collect();
        for id in primary {
            model.dependencies.push(Dependency::new(
                &id,
                RelationKind::DefinedBy,
                &node,
                Basis::explicit(artifact.at("SemanticEngine", None)),
            ));
        }
        model.finalize();
        Ok((parser.to_string(), model))
    }

    pub async fn inspect_artifact(
        &self,
        artifact: &NexusArtifact,
    ) -> SemanticResult<(String, SemanticModel)> {
        self.parse(&self.load(artifact)?).await
    }

    pub async fn inspect(
        &self,
        domain: &NexusDomain,
        project_id: &str,
        artifact_id: &ArtifactId,
        persist: bool,
    ) -> SemanticResult<InspectOutcome> {
        let project = self.project(domain, project_id)?;
        let store = self.store(&project);
        let artifact = store.artifact(artifact_id)?;
        let (parser, model) = self.inspect_artifact(&artifact).await?;
        if persist {
            store.save_model(&artifact.id, &model)?;
        }
        Ok(InspectOutcome {
            artifact,
            parser,
            model,
        })
    }

    /// Stored model if present, otherwise a fresh inspect (not persisted).
    async fn model_of(
        &self,
        store: &SemanticStore,
        id: &ArtifactId,
    ) -> SemanticResult<SemanticModel> {
        if let Some(m) = store.model(id)? {
            return Ok(m);
        }
        let artifact = store.artifact(id)?;
        Ok(self.inspect_artifact(&artifact).await?.1)
    }

    async fn models_of(
        &self,
        store: &SemanticStore,
        ids: &[ArtifactId],
    ) -> SemanticResult<SemanticModel> {
        let mut models = Vec::new();
        for id in ids {
            models.push(self.model_of(store, id).await?);
        }
        Ok(consolidate(models))
    }

    // ---- skills ----------------------------------------------------------

    /// Merged EngineeringContext of the given skills (`frontend`,
    /// `skill:frontend` and `skill://frontend` are equivalent).
    pub fn engineering_context(
        &self,
        domain: &NexusDomain,
        project_id: &str,
        skill_ids: &[String],
    ) -> SemanticResult<EngineeringContext> {
        let project = self.project(domain, project_id)?;
        let mut contexts = Vec::new();
        for raw in skill_ids {
            let id = raw
                .trim_start_matches("skill://")
                .trim_start_matches("skill:");
            let skill = domain.skills.get_skill(id, Some(&project.id))?;
            let artifact = self.file_artifact(
                &project,
                &skill.source_path,
                ArtifactKind::Skill,
                Role::Engineering,
                None,
            );
            contexts.push(SkillParser::engineering_context(&self.load(&artifact)?)?.1);
        }
        Ok(EngineeringContext::merge(contexts))
    }

    // ---- translate / validate / diff -------------------------------------

    pub async fn translate(
        &self,
        domain: &NexusDomain,
        project_id: &str,
        artifact_ids: &[ArtifactId],
        direction: TranslationDirection,
        context_skills: &[String],
        persist: bool,
    ) -> SemanticResult<Translation> {
        let project = self.project(domain, project_id)?;
        let store = self.store(&project);
        let model = if artifact_ids.is_empty() {
            self.project_model(domain, project_id).await?
        } else {
            self.models_of(&store, artifact_ids).await?
        };
        let target_context = if context_skills.is_empty() {
            None
        } else {
            Some(self.engineering_context(domain, project_id, context_skills)?)
        };
        let translator = DeterministicTranslator { target_context };
        let mut t = translator.translate(&model, direction).await?;
        t.project_id = Some(ProjectId(project.id.clone()));
        if persist {
            store.save_translation(&t)?;
        }
        Ok(t)
    }

    /// Everything a model may legitimately reference in this project.
    pub fn known_entities(
        &self,
        domain: &NexusDomain,
        project: &Project,
        store: &SemanticStore,
    ) -> SemanticResult<BTreeSet<String>> {
        let mut known: BTreeSet<String> = domain
            .graph
            .list_entities(&project.id)?
            .into_iter()
            .map(|e| e.id)
            .collect();
        known.extend(
            domain
                .skills
                .list_skills(Some(&project.id))?
                .into_iter()
                .map(|s| format!("skill:{}", s.id)),
        );
        known.extend(
            domain
                .behaviors
                .list_behaviors(&project.id)?
                .into_iter()
                .map(|b| format!("behavior:{}", b.id)),
        );
        for m in store.models()? {
            known.extend(m.entities.into_iter().map(|e| e.id));
        }
        known.insert(format!("project:{}", project.id));
        Ok(known)
    }

    pub async fn validate(
        &self,
        domain: &NexusDomain,
        project_id: &str,
        target: ValidateTarget,
        contract_id: Option<&str>,
    ) -> SemanticResult<ValidationResult> {
        let project = self.project(domain, project_id)?;
        let store = self.store(&project);
        let known = self.known_entities(domain, &project, &store)?;
        let (mut issues, model) = match target {
            ValidateTarget::Translation(id) => {
                let t = store.translation(&id)?;
                (validate::check_translation(&t, &known), t.semantic_model)
            }
            ValidateTarget::Artifacts(ids) => {
                let m = self.models_of(&store, &ids).await?;
                (validate::check_model(&m, &known), m)
            }
            ValidateTarget::Project => {
                let m = self.project_model(domain, project_id).await?;
                (validate::check_model(&m, &known), m)
            }
        };
        if let Some(cid) = contract_id {
            let np = NexusProject::load(&self.repo_root, project.clone())?;
            let contract = np
                .semantic_contracts
                .iter()
                .find(|c| c.id == cid)
                .ok_or_else(|| SemanticError::not_found("Contract", cid))?;
            issues.extend(validate::check_contract(&model, contract));
        }
        Ok(ValidationResult::from_issues(issues))
    }

    pub async fn diff(
        &self,
        domain: &NexusDomain,
        project_id: &str,
        artifact_id: &ArtifactId,
        against: DiffAgainst,
    ) -> SemanticResult<DiffOutcome> {
        let project = self.project(domain, project_id)?;
        let store = self.store(&project);
        let artifact = store.artifact(artifact_id)?;
        let current = self.inspect_artifact(&artifact).await?.1;
        let (before_label, before) = match against {
            DiffAgainst::Stored => (
                format!("stored model of {artifact_id}"),
                store.model(artifact_id)?.ok_or_else(|| {
                    SemanticError::InvalidInput(format!(
                        "no stored model for \"{artifact_id}\"; run inspect first"
                    ))
                })?,
            ),
            DiffAgainst::Artifact(other) => {
                (other.to_string(), self.model_of(&store, &other).await?)
            }
            DiffAgainst::Revision(rev) => {
                let ArtifactContent::File { path } = &artifact.content else {
                    return Err(SemanticError::InvalidInput(
                        "revision diffs need a file artifact".into(),
                    ));
                };
                let old = git::show(&self.repo_root, &rev, path).ok_or_else(|| {
                    SemanticError::InvalidInput(format!(
                        "\"{path}\" does not exist at revision \"{rev}\""
                    ))
                })?;
                let mut old_artifact = artifact.clone();
                old_artifact.content = ArtifactContent::Inline {
                    text: old,
                    media_type: None,
                };
                old_artifact.provenance.commit = Some(rev.clone());
                (format!("{path}@{rev}"), self.parse(&old_artifact).await?.1)
            }
        };
        Ok(DiffOutcome {
            before: before_label,
            after: format!("current {artifact_id}"),
            diff: semantic_diff(&before, &current),
        })
    }

    // ---- proposals --------------------------------------------------------

    fn sources(
        &self,
        domain: &NexusDomain,
        project: &Project,
        model: &SemanticModel,
    ) -> SemanticResult<BTreeSet<String>> {
        let mut sources: BTreeSet<String> = model
            .entities
            .iter()
            .map(|e| e.id.clone())
            .filter(|id| validate::SOURCE_PREFIXES.iter().any(|p| id.starts_with(p)))
            .collect();
        sources.extend(
            domain
                .skills
                .list_skills(Some(&project.id))?
                .into_iter()
                .map(|s| format!("skill:{}", s.id)),
        );
        sources.extend(
            domain
                .behaviors
                .list_behaviors(&project.id)?
                .into_iter()
                .map(|b| format!("behavior:{}", b.id)),
        );
        Ok(sources)
    }

    fn build_proposal(
        &self,
        domain: &NexusDomain,
        project: &Project,
        model: &SemanticModel,
        title: String,
        scope: String,
    ) -> SemanticResult<(Proposal, Vec<String>)> {
        let entities = domain.graph.list_entities(&project.id)?;
        let relations = domain.graph.list_relations(&project.id)?;
        let sources = self.sources(domain, project, model)?;
        let p = proposal::build(proposal::BuildInput {
            project_id: &project.id,
            title,
            scope,
            model,
            canonical_entities: &entities,
            canonical_relations: &relations,
            sources: &sources,
        });
        let superseded = proposal::submit(&self.store(project), &p)?;
        Ok((p, superseded))
    }

    pub async fn propose(
        &self,
        domain: &NexusDomain,
        project_id: &str,
        artifact_ids: &[ArtifactId],
    ) -> SemanticResult<Proposal> {
        let project = self.project(domain, project_id)?;
        let store = self.store(&project);
        let model = if artifact_ids.is_empty() {
            self.project_model(domain, project_id).await?
        } else {
            self.models_of(&store, artifact_ids).await?
        };
        let scope = if artifact_ids.is_empty() {
            "propose:project".to_string()
        } else {
            format!(
                "propose:{}",
                artifact_ids
                    .iter()
                    .map(|a| a.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        let title = format!(
            "Semantic graph update from {} artifact(s)",
            model.artifacts.len()
        );
        Ok(self
            .build_proposal(domain, &project, &model, title, scope)?
            .0)
    }

    pub fn review(
        &self,
        domain: &NexusDomain,
        project_id: &str,
        proposal_id: &str,
        decision: Decision,
        client: &ClientInfo,
        note: Option<String>,
    ) -> SemanticResult<Proposal> {
        let project = self.project(domain, project_id)?;
        require_permission(
            &domain.policies,
            client,
            REVIEW_PERMISSION,
            Some(&project.id),
        )?;
        proposal::review(
            &self.store(&project),
            &project.path,
            &self.repo_root,
            proposal_id,
            decision,
            &client.id,
            note,
        )
    }

    pub fn proposals(
        &self,
        domain: &NexusDomain,
        project_id: &str,
    ) -> SemanticResult<Vec<Proposal>> {
        let project = self.project(domain, project_id)?;
        self.store(&project).proposals()
    }

    pub fn artifacts(
        &self,
        domain: &NexusDomain,
        project_id: &str,
    ) -> SemanticResult<Vec<NexusArtifact>> {
        let project = self.project(domain, project_id)?;
        self.store(&project).artifacts()
    }

    pub fn artifact(
        &self,
        domain: &NexusDomain,
        project_id: &str,
        id: &ArtifactId,
    ) -> SemanticResult<(NexusArtifact, Option<SemanticModel>)> {
        let project = self.project(domain, project_id)?;
        let store = self.store(&project);
        Ok((store.artifact(id)?, store.model(id)?))
    }

    pub fn translation(
        &self,
        domain: &NexusDomain,
        project_id: &str,
        id: &str,
    ) -> SemanticResult<Translation> {
        let project = self.project(domain, project_id)?;
        self.store(&project).translation(id)
    }

    pub fn nexus_project(
        &self,
        domain: &NexusDomain,
        project_id: &str,
    ) -> SemanticResult<NexusProject> {
        NexusProject::load(&self.repo_root, self.project(domain, project_id)?)
    }

    // ---- ingestion -----------------------------------------------------------

    pub async fn ingest(
        &self,
        domain: &NexusDomain,
        project_id: &str,
        opts: IngestOptions,
    ) -> SemanticResult<(IngestReport, SemanticModel)> {
        let project = self.project(domain, project_id)?;
        let store = self.store(&project);
        let root = opts.root.clone().unwrap_or_else(|| project.path.clone());
        if !root.is_dir() {
            return Err(SemanticError::InvalidInput(format!(
                "\"{}\" is not a directory",
                root.display()
            )));
        }
        let is_project_dir = root == project.path;
        let skip_top: &[&str] = if is_project_dir {
            &[store::SEMANTIC_DIR, "graph", "studio"]
        } else {
            &[]
        };
        let (files, dirs) = ingest::walk(&root, skip_top);
        let git_ok = git::available(&root);
        let commits = if git_ok {
            git::log(&root, opts.max_commits)
        } else {
            Vec::new()
        };
        let mut newest_commit_of: BTreeMap<String, String> = BTreeMap::new();
        for c in &commits {
            for f in &c.files {
                newest_commit_of
                    .entry(f.path.clone())
                    .or_insert_with(|| c.sha.clone());
            }
        }

        if opts.persist {
            store.clear_ingested(|a| match &a.content {
                ArtifactContent::File { path } => {
                    self.abs_of(path).starts_with(&root) || a.kind == ArtifactKind::Skill
                }
                ArtifactContent::Inline { .. } => a.kind == ArtifactKind::Commit,
            })?;
        }
        let mut models = Vec::new();
        let mut ingested = Vec::new();
        let mut skipped = Vec::new();
        let mut structure_files = Vec::new();
        let mut parsed: Vec<(NexusArtifact, String)> = Vec::new();

        let mut candidates: Vec<(PathBuf, String, ArtifactKind, Role)> = Vec::new();
        for rel in &files {
            match ingest::classify(rel) {
                Some((kind, role)) => candidates.push((root.join(rel), rel.clone(), kind, role)),
                None => skipped.push(Skipped {
                    path: rel.clone(),
                    reason: "unsupported file type".into(),
                }),
            }
        }
        // Global skills apply to every project; include those outside the root.
        for skill in domain.skills.list_skills(Some(&project.id))? {
            if !skill.source_path.starts_with(&root) {
                let rel = self.source_of(&skill.source_path);
                candidates.push((
                    skill.source_path.clone(),
                    rel,
                    ArtifactKind::Skill,
                    Role::Engineering,
                ));
            }
        }

        for (abs, rel, kind, role) in candidates {
            if std::fs::metadata(&abs).map(|m| m.len()).unwrap_or(0) > ingest::MAX_FILE_BYTES {
                skipped.push(Skipped {
                    path: rel,
                    reason: "larger than 512 KiB".into(),
                });
                continue;
            }
            let commit = newest_commit_of.get(&rel).cloned();
            let artifact = self.file_artifact(&project, &abs, kind, role, commit);
            let loaded = match self.load(&artifact) {
                Ok(l) => l,
                Err(e) => {
                    skipped.push(Skipped {
                        path: rel,
                        reason: e.to_string(),
                    });
                    continue;
                }
            };
            let (parser, model) = match self.parse(&loaded).await {
                Ok(x) => x,
                Err(e) => {
                    skipped.push(Skipped {
                        path: rel,
                        reason: e.to_string(),
                    });
                    continue;
                }
            };
            if opts.persist {
                store.save_artifact(&artifact)?;
                store.save_model(&artifact.id, &model)?;
            }
            if abs.starts_with(&root) {
                structure_files.push((rel.clone(), Self::artifact_node_id(&artifact)));
            }
            ingested.push(IngestedArtifact {
                id: artifact.id.clone(),
                kind,
                role: artifact.role.clone(),
                path: artifact.provenance.source.clone(),
                parser: parser.clone(),
                statements: model.items().len(),
            });
            parsed.push((artifact, rel));
            models.push(model);
        }

        // Git history as commit artifacts; file paths mapped to artifact nodes.
        let node_of_rel: BTreeMap<&str, &str> = structure_files
            .iter()
            .map(|(r, n)| (r.as_str(), n.as_str()))
            .collect();
        for c in &commits {
            let short = &c.sha[..c.sha.len().min(7)];
            let files: Vec<serde_json::Value> = c
                .files
                .iter()
                .filter_map(|f| node_of_rel.get(f.path.as_str()).map(|n| serde_json::json!({ "status": f.status, "path": n.trim_start_matches("file:") })))
                .collect();
            let mut metadata = Metadata::new();
            metadata.insert("sha".into(), c.sha.clone().into());
            metadata.insert("subject".into(), c.subject.clone().into());
            metadata.insert("author".into(), c.author.clone().into());
            metadata.insert("date".into(), c.date.clone().into());
            metadata.insert("files".into(), files.into());
            let artifact = NexusArtifact {
                id: ArtifactId(format!("commit-{short}")),
                project_id: ProjectId(project.id.clone()),
                kind: ArtifactKind::Commit,
                role: Role::System,
                name: c.subject.clone(),
                content: ArtifactContent::Inline {
                    text: c.subject.clone(),
                    media_type: None,
                },
                provenance: Provenance {
                    source: format!("git:{short}"),
                    line_start: None,
                    line_end: None,
                    commit: Some(c.sha.clone()),
                    extraction: "GitIngestion".into(),
                    artifact: Some(format!("commit-{short}")),
                },
                confidence: confidence::explicit(),
                metadata,
                registered_at: now(),
            };
            let (_, model) = self.parsers.inspect(&artifact).await?;
            if opts.persist {
                store.save_artifact(&artifact)?;
                store.save_model(&artifact.id, &model)?;
            }
            models.push(model);
        }

        let root_label = if is_project_dir {
            project.name.clone()
        } else {
            self.source_of(&root)
        };
        let structure = ingest::structure_model(&root_label, &dirs, &structure_files);
        let derived = derive_changed_by(&consolidate(models.clone()), &commits, &node_of_rel);
        if opts.persist {
            store.save_model(&ArtifactId::from(STRUCTURE_MODEL), &structure)?;
            store.save_model(&ArtifactId::from(DERIVED_MODEL), &derived)?;
        }
        models.push(structure);
        models.push(derived);
        let model = consolidate(models);

        // Semantic changes of the newest commit (or since `diff_base`) vs the working tree.
        let mut semantic_changes = Vec::new();
        if let Some(newest) = commits.first() {
            let base = opts
                .diff_base
                .clone()
                .unwrap_or_else(|| format!("{}^", newest.sha));
            let touched: BTreeSet<&str> = newest.files.iter().map(|f| f.path.as_str()).collect();
            for (artifact, rel) in &parsed {
                if opts.diff_base.is_none() && !touched.contains(rel.as_str()) {
                    continue;
                }
                let loaded = self.load(artifact)?;
                let before = match git::show(&root, &base, rel) {
                    Some(old) if Some(old.as_str()) == loaded.text() => continue,
                    Some(old) => {
                        let mut a = artifact.clone();
                        a.content = ArtifactContent::Inline {
                            text: old,
                            media_type: None,
                        };
                        self.parse(&a).await.map(|x| x.1).unwrap_or_default()
                    }
                    None => SemanticModel::default(),
                };
                let after = self.parse(&loaded).await?.1;
                let d = semantic_diff(&before, &after);
                if !d.is_empty() {
                    semantic_changes.push(FileChanges {
                        path: artifact.provenance.source.clone(),
                        base: base.clone(),
                        changes: d.changes,
                    });
                }
            }
        }

        let known = self.known_entities(domain, &project, &store)?;
        let validation = ValidationResult::from_issues(validate::check_model(&model, &known));

        let (proposal, superseded) = if opts.propose && opts.persist {
            let (p, s) = self.build_proposal(
                domain,
                &project,
                &model,
                format!("Ingestion of {}", self.source_of(&root)),
                format!("ingest:{}", self.source_of(&root)),
            )?;
            (Some(p.id), s)
        } else {
            (None, Vec::new())
        };

        let report = IngestReport {
            project_id: project.id.clone(),
            root: self.source_of(&root),
            git_available: git_ok,
            head: if git_ok { git::head(&root) } else { None },
            files_scanned: files.len(),
            directories: dirs.len(),
            artifacts: ingested,
            skipped,
            commits: commits.len(),
            semantic_changes,
            model: ModelSummary::of(&model),
            validation,
            proposal,
            superseded,
            generated_at: now(),
        };
        if opts.persist {
            store::write_json(&store.dir.join("ingest.json"), &report)?;
            store::write_json(&store.dir.join("diff.json"), &report.semantic_changes)?;
        }
        Ok((report, model))
    }

    /// Consolidated model of everything stored for the project; if nothing
    /// was ingested yet, a non-persisting ingestion of the project directory.
    pub async fn project_model(
        &self,
        domain: &NexusDomain,
        project_id: &str,
    ) -> SemanticResult<SemanticModel> {
        let project = self.project(domain, project_id)?;
        let store = self.store(&project);
        let models = store.models()?;
        if !models.is_empty() {
            return Ok(consolidate(models));
        }
        let opts = IngestOptions {
            persist: false,
            propose: false,
            ..Default::default()
        };
        Ok(self.ingest(domain, project_id, opts).await?.1)
    }

    pub async fn graph(
        &self,
        domain: &NexusDomain,
        project_id: &str,
        include_structure: bool,
    ) -> SemanticResult<SemanticGraph> {
        let project = self.project(domain, project_id)?;
        let store = self.store(&project);
        let model = self.project_model(domain, project_id).await?;
        let entities = domain.graph.list_entities(&project.id)?;
        let relations = domain.graph.list_relations(&project.id)?;
        let proposals = store.proposals()?;
        let diff_path = store.dir.join("diff.json");
        let diff: Vec<SemanticChange> = if diff_path.is_file() {
            store::read_json::<Vec<FileChanges>>(&diff_path)?
                .into_iter()
                .flat_map(|f| f.changes)
                .collect()
        } else {
            Vec::new()
        };
        Ok(graph_view::build(graph_view::GraphInput {
            repo_root: &self.repo_root,
            project_id: &project.id,
            project_name: &project.name,
            canonical_entities: &entities,
            canonical_relations: &relations,
            model: &model,
            proposals: &proposals,
            diff,
            include_structure,
        }))
    }

    /// Writes `semantic-graph.json` and `semantic-project.html`.
    pub async fn export(
        &self,
        domain: &NexusDomain,
        project_id: &str,
        out_dir: Option<PathBuf>,
        include_structure: bool,
    ) -> SemanticResult<ExportResult> {
        let project = self.project(domain, project_id)?;
        let dir = out_dir.unwrap_or_else(|| self.store(&project).dir);
        let graph = self.graph(domain, project_id, include_structure).await?;
        let json_path = dir.join("semantic-graph.json");
        let html_path = dir.join("semantic-project.html");
        store::write_json(&json_path, &graph)?;
        std::fs::write(&html_path, crate::viz::render_html(&graph))
            .map_err(|e| SemanticError::io(&html_path, e))?;
        Ok(ExportResult {
            graph_json: self.source_of(&json_path),
            html: self.source_of(&html_path),
            nodes: graph.nodes.len(),
            edges: graph.edges.len(),
        })
    }

    /// The Context Resolver as a later pipeline step: its minimal context
    /// plus the engineering context of the resolved skills and the model
    /// statements about the resolved entities.
    pub async fn semantic_context(
        &self,
        domain: &NexusDomain,
        request: &ContextRequest,
    ) -> SemanticResult<SemanticContext> {
        let resolved = nexus_domain::context::resolve_context(domain, request)?;
        let skill_ids: Vec<String> = resolved.skills.iter().map(|s| s.id.clone()).collect();
        let engineering = self.engineering_context(domain, &request.project_id, &skill_ids)?;
        let model = self.project_model(domain, &request.project_id).await?;
        let mut focus: BTreeSet<String> = resolved.entities.iter().map(|e| e.id.clone()).collect();
        focus.extend(skill_ids.iter().map(|s| format!("skill:{s}")));
        focus.extend(
            resolved
                .behaviors
                .iter()
                .map(|b| format!("behavior:{}", b.id)),
        );
        let about = |s: &Option<String>| s.as_ref().is_some_and(|s| focus.contains(s));
        let semantics = SemanticModel {
            artifacts: model.artifacts.clone(),
            intent: model.intent.clone(),
            requirements: model
                .requirements
                .iter()
                .filter(|r| about(&r.subject))
                .cloned()
                .collect(),
            behaviors: model
                .behaviors
                .iter()
                .filter(|b| focus.contains(&b.subject))
                .cloned()
                .collect(),
            entities: model
                .entities
                .iter()
                .filter(|e| focus.contains(&e.id))
                .cloned()
                .collect(),
            states: model
                .states
                .iter()
                .filter(|s| focus.contains(&s.subject))
                .cloned()
                .collect(),
            constraints: model
                .constraints
                .iter()
                .filter(|c| about(&c.subject))
                .cloned()
                .collect(),
            interactions: model
                .interactions
                .iter()
                .filter(|i| about(&i.target))
                .cloned()
                .collect(),
            dependencies: model
                .dependencies
                .iter()
                .filter(|d| focus.contains(&d.from) || focus.contains(&d.to))
                .cloned()
                .collect(),
            ..SemanticModel::default()
        };
        let mut semantics = semantics;
        let focus_sources: BTreeSet<String> = semantics
            .items()
            .iter()
            .flat_map(|i| i.basis.provenance.iter().map(|p| p.source.clone()))
            .collect();
        semantics.intent.retain(|i| {
            i.basis
                .provenance
                .iter()
                .any(|p| focus_sources.contains(&p.source))
        });
        semantics.finalize();
        Ok(SemanticContext {
            resolved,
            engineering,
            semantics,
        })
    }
}

/// Stored model ids that are not artifacts.
pub const STRUCTURE_MODEL: &str = "_structure";
pub const DERIVED_MODEL: &str = "_derived";

/// `X defined-by file:F` + newest commit C touching F => `X changed-by C`
/// (inferred: the commit touched the file, not necessarily X itself).
fn derive_changed_by(
    model: &SemanticModel,
    commits: &[git::CommitInfo],
    node_of_rel: &BTreeMap<&str, &str>,
) -> SemanticModel {
    let mut newest: BTreeMap<&str, &git::CommitInfo> = BTreeMap::new();
    for c in commits {
        for f in &c.files {
            if let Some(node) = node_of_rel.get(f.path.as_str()) {
                newest.entry(*node).or_insert(c);
            }
        }
    }
    let mut derived = Vec::new();
    for d in &model.dependencies {
        if d.relation != RelationKind::DefinedBy || !proposal::graphable(&d.from) {
            continue;
        }
        if let Some(c) = newest.get(d.to.as_str()) {
            let short = &c.sha[..c.sha.len().min(7)];
            let mut provenance = d.basis.provenance.clone();
            provenance.push(Provenance {
                source: format!("git:{short}"),
                line_start: None,
                line_end: None,
                commit: Some(c.sha.clone()),
                extraction: "GitIngestion".into(),
                artifact: None,
            });
            derived.push(Dependency::new(
                &d.from,
                RelationKind::ChangedBy,
                &format!("commit:{short}"),
                Basis {
                    evidence: Evidence::Inferred,
                    confidence: confidence::inferred(
                        1,
                        "entity is defined in a file this commit touched",
                    ),
                    provenance,
                },
            ));
        }
    }
    let mut out = SemanticModel {
        dependencies: derived,
        ..SemanticModel::default()
    };
    out.finalize();
    out
}
