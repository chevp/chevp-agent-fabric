use super::ArtifactParser;
use crate::error::SemanticResult;
use crate::model::*;
use async_trait::async_trait;
use nexus_domain::types::RelationKind;
use serde_json::Value;

const NAME: &str = "CommitParser";

/// Commit artifact (metadata: `sha`, `subject`, `author`, `date`, `files`)
/// -> commit entity and explicit `changed-by` edges from each touched file.
pub struct CommitParser;

#[async_trait]
impl ArtifactParser for CommitParser {
    fn name(&self) -> &'static str {
        NAME
    }

    fn supports(&self, artifact: &NexusArtifact) -> bool {
        artifact.kind == ArtifactKind::Commit
    }

    async fn inspect(&self, artifact: &NexusArtifact) -> SemanticResult<SemanticModel> {
        let meta = |k: &str| artifact.metadata.get(k).and_then(Value::as_str).unwrap_or("");
        let sha = meta("sha");
        let short = &sha[..sha.len().min(7)];
        let id = format!("commit:{short}");
        let basis = || Basis::explicit(artifact.at(NAME, None));
        let mut m = SemanticModel::default();

        let mut entity = SemanticEntity::new(&id, meta("subject"), "commit", basis());
        entity.artifact = Some(artifact.id.clone());
        for k in ["sha", "author", "date"] {
            entity.attributes.insert(k.into(), meta(k).into());
        }
        m.entities.push(entity);

        for f in artifact
            .metadata
            .get("files")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(path) = f.get("path").and_then(Value::as_str) else {
                continue;
            };
            m.dependencies.push(Dependency::new(
                &format!("file:{path}"),
                RelationKind::ChangedBy,
                &id,
                basis(),
            ));
        }
        Ok(m)
    }
}
