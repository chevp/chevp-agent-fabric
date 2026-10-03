use super::{text_of, ArtifactParser};
use crate::engineering::{split_frontmatter, ContextItem, EngineeringContext};
use crate::error::{SemanticError, SemanticResult};
use crate::model::*;
use crate::text::backtick_refs;
use async_trait::async_trait;
use nexus_domain::types::RelationKind;
use serde::Deserialize;

const NAME: &str = "SkillParser";

fn item(i: &ContextItem) -> (&str, Basis) {
    (i.text.as_str(), i.basis.clone())
}

#[derive(Deserialize)]
struct Frontmatter {
    id: Option<String>,
    name: Option<String>,
    description: Option<String>,
    #[serde(default, rename = "dependsOn")]
    depends_on: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
}

/// SKILL.md -> EngineeringContext -> SemanticModel. Section items are
/// explicit; backtick mentions of other ids are candidates.
pub struct SkillParser;

impl SkillParser {
    /// The skill id plus its engineering context, with line provenance.
    pub fn engineering_context(
        artifact: &NexusArtifact,
    ) -> SemanticResult<(String, EngineeringContext)> {
        let raw = text_of(artifact)?;
        let (fm, body, offset) = split_frontmatter(raw);
        let fm: Option<Frontmatter> = fm
            .map(|f| {
                serde_yaml::from_str(f).map_err(|e| SemanticError::Parse {
                    source_path: artifact.provenance.source.clone(),
                    message: format!("invalid skill frontmatter: {e}"),
                })
            })
            .transpose()?;
        let id = fm
            .and_then(|f| f.id)
            .unwrap_or_else(|| crate::text::slug(&artifact.name));
        let mut ctx = EngineeringContext::from_markdown(body, offset, &artifact.at(NAME, None));
        ctx.skills = vec![id.clone()];
        Ok((id, ctx))
    }
}

#[async_trait]
impl ArtifactParser for SkillParser {
    fn name(&self) -> &'static str {
        NAME
    }

    fn supports(&self, artifact: &NexusArtifact) -> bool {
        artifact.kind == ArtifactKind::Skill
    }

    async fn inspect(&self, artifact: &NexusArtifact) -> SemanticResult<SemanticModel> {
        let raw = text_of(artifact)?;
        let (fm_raw, _, _) = split_frontmatter(raw);
        let fm: Option<Frontmatter> = fm_raw.and_then(|f| serde_yaml::from_str(f).ok());
        let (id, ctx) = Self::engineering_context(artifact)?;
        let entity_id = format!("skill:{id}");
        let mut m = SemanticModel::default();

        let mut entity = SemanticEntity::new(
            &entity_id,
            fm.as_ref().and_then(|f| f.name.as_deref()).unwrap_or(&id),
            "skill",
            super::explicit_at(artifact, NAME, "id:"),
        );
        entity.role = Some(Role::Engineering);
        entity.artifact = Some(artifact.id.clone());
        entity.description = fm.as_ref().and_then(|f| f.description.clone());
        if let Some(fm) = &fm {
            if !fm.tags.is_empty() {
                entity
                    .attributes
                    .insert("tags".into(), fm.tags.clone().into());
            }
        }
        m.entities.push(entity);

        for dep in fm
            .as_ref()
            .map(|f| f.depends_on.clone())
            .unwrap_or_default()
        {
            m.dependencies.push(Dependency::new(
                &entity_id,
                RelationKind::DependsOn,
                &format!("skill:{dep}"),
                super::explicit_at(artifact, NAME, &format!("- {dep}")),
            ));
        }

        let subject = Some(entity_id.as_str());
        for (text, basis) in ctx.purpose.iter().map(item) {
            m.intent.push(Intent::new(text, basis));
        }
        for (list, kind) in [
            (&ctx.capabilities, RequirementKind::Capability),
            (&ctx.inputs, RequirementKind::Input),
            (&ctx.outputs, RequirementKind::Output),
        ] {
            for (text, basis) in list.iter().map(item) {
                m.requirements
                    .push(Requirement::new(subject, text, kind, basis));
            }
        }
        for (text, basis) in ctx.constraints.iter().map(item) {
            let kind = modal_kind(text)
                .filter(|k| *k == ConstraintKind::MustNot)
                .unwrap_or(ConstraintKind::Must);
            m.constraints
                .push(Constraint::new(subject, text, kind, basis));
        }
        for (text, basis) in ctx.conventions.iter().map(item) {
            m.constraints.push(Constraint::new(
                subject,
                text,
                ConstraintKind::Convention,
                basis,
            ));
        }
        for (text, basis) in ctx.forbidden.iter().map(item) {
            m.constraints.push(Constraint::new(
                subject,
                text,
                ConstraintKind::MustNot,
                basis,
            ));
        }

        let deps: Vec<String> = fm.map(|f| f.depends_on).unwrap_or_default();
        for (reference, line) in backtick_refs(raw) {
            if reference == id || deps.contains(&reference) {
                continue;
            }
            m.dependencies.push(Dependency::new(
                &entity_id,
                RelationKind::RelatedTo,
                &reference,
                Basis::candidate(
                    artifact.at(NAME, Some((line, line))),
                    "identifier mentioned in backticks",
                ),
            ));
        }
        Ok(m)
    }
}
