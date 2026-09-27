use super::ArtifactParser;
use crate::error::SemanticResult;
use crate::model::*;
use async_trait::async_trait;

/// Accepts anything and extracts nothing: it only records that no parser
/// understood the artifact, so the gap is visible instead of guessed over.
pub struct FallbackParser;

#[async_trait]
impl ArtifactParser for FallbackParser {
    fn name(&self) -> &'static str {
        "FallbackParser"
    }

    fn supports(&self, _artifact: &NexusArtifact) -> bool {
        true
    }

    async fn inspect(&self, artifact: &NexusArtifact) -> SemanticResult<SemanticModel> {
        let mut m = SemanticModel::default();
        m.assumptions.push(Assumption::new(
            &format!("no semantics extracted from {}", artifact.provenance.source),
            "no parser supports this artifact kind/format; only its existence is recorded",
            Basis::unknown(artifact.at("FallbackParser", None), "unsupported format"),
        ));
        Ok(m)
    }
}
