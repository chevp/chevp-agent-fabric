use super::{has_ext, text_of, ArtifactParser};
use crate::engineering::{classify, parse_sections, split_frontmatter, SectionKind};
use crate::error::SemanticResult;
use crate::model::*;
use crate::text::{backtick_refs, slug};
use async_trait::async_trait;
use nexus_domain::types::RelationKind;

const NAME: &str = "MarkdownParser";

/// Markdown documents. Items under recognized headings (Purpose,
/// Constraints, Forbidden, ...) are explicit; modal sentences elsewhere
/// ("must", "never") are inferred; backtick mentions are candidates.
pub struct MarkdownParser;

#[async_trait]
impl ArtifactParser for MarkdownParser {
    fn name(&self) -> &'static str {
        NAME
    }

    fn supports(&self, artifact: &NexusArtifact) -> bool {
        has_ext(artifact, &["md", "markdown", "mdx"])
            || artifact.media_type() == Some("text/markdown")
    }

    async fn inspect(&self, artifact: &NexusArtifact) -> SemanticResult<SemanticModel> {
        let raw = text_of(artifact)?;
        let (_, body, offset) = split_frontmatter(raw);
        let doc_id = format!("doc:{}", slug(&artifact.provenance.source));
        let subject = Some(doc_id.as_str());
        let sections = parse_sections(body, offset);
        let title = sections
            .iter()
            .find(|s| s.level == 1)
            .map(|s| s.heading.clone())
            .unwrap_or_else(|| artifact.name.clone());
        let mut m = SemanticModel::default();

        let mut doc = SemanticEntity::new(
            &doc_id,
            &title,
            "document",
            Basis::explicit(artifact.at(NAME, None)),
        );
        doc.role = Some(artifact.role.clone());
        doc.artifact = Some(artifact.id.clone());
        m.entities.push(doc);

        for section in &sections {
            let kind = classify(&section.heading);
            for (text, start, end) in &section.items {
                let prov = artifact.at(NAME, Some((*start, *end)));
                let explicit = || Basis::explicit(prov.clone());
                match kind {
                    SectionKind::Purpose => m.intent.push(Intent::new(text, explicit())),
                    SectionKind::Capabilities => m.requirements.push(Requirement::new(
                        subject,
                        text,
                        RequirementKind::Capability,
                        explicit(),
                    )),
                    SectionKind::Inputs => m.requirements.push(Requirement::new(
                        subject,
                        text,
                        RequirementKind::Input,
                        explicit(),
                    )),
                    SectionKind::Outputs => m.requirements.push(Requirement::new(
                        subject,
                        text,
                        RequirementKind::Output,
                        explicit(),
                    )),
                    SectionKind::Constraints => {
                        let k = modal_kind(text)
                            .filter(|k| *k == ConstraintKind::MustNot)
                            .unwrap_or(ConstraintKind::Must);
                        m.constraints
                            .push(Constraint::new(subject, text, k, explicit()));
                    }
                    SectionKind::Conventions => m.constraints.push(Constraint::new(
                        subject,
                        text,
                        ConstraintKind::Convention,
                        explicit(),
                    )),
                    SectionKind::Forbidden => m.constraints.push(Constraint::new(
                        subject,
                        text,
                        ConstraintKind::MustNot,
                        explicit(),
                    )),
                    SectionKind::Examples => {}
                    SectionKind::Other => {
                        if text.starts_with("```") {
                            continue;
                        }
                        if let Some(k) = modal_kind(text) {
                            m.constraints.push(Constraint::new(
                                subject,
                                text,
                                k,
                                Basis::inferred(vec![prov], "modal keyword in prose"),
                            ));
                        }
                    }
                }
            }
        }

        for (reference, line) in backtick_refs(raw) {
            m.dependencies.push(Dependency::new(
                &doc_id,
                RelationKind::Documents,
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
