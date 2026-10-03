//! Parsers turn one loaded `NexusArtifact` into a `SemanticModel` by
//! deterministic extraction. `ParserRegistry` picks the first parser whose
//! `supports` accepts the artifact; `FallbackParser` accepts everything.
//!
//! This trait is an internal domain extension point (like `Translator` and
//! `Validator`), separate from the MCP-level `Tool` extension point.

mod behavior;
mod code;
mod commit;
mod contract;
mod design_system;
mod fallback;
mod html;
mod markdown;
mod skill;
mod structured;

pub use behavior::BehaviorSpecParser;
pub use code::{CodeMetadataParser, CODE_EXTENSIONS};
pub use commit::CommitParser;
pub use contract::ContractParser;
pub use design_system::DesignSystemParser;
pub use fallback::FallbackParser;
pub use html::HtmlParser;
pub use markdown::MarkdownParser;
pub use skill::SkillParser;
pub use structured::{JsonParser, YamlParser};

use crate::error::{SemanticError, SemanticResult};
use crate::model::{Basis, NexusArtifact, SemanticModel};
use async_trait::async_trait;
use serde_json::Value;

#[async_trait]
pub trait ArtifactParser: Send + Sync {
    fn name(&self) -> &'static str;
    /// Called with a loaded artifact (inline content).
    fn supports(&self, artifact: &NexusArtifact) -> bool;
    async fn inspect(&self, artifact: &NexusArtifact) -> SemanticResult<SemanticModel>;
}

pub struct ParserRegistry {
    parsers: Vec<Box<dyn ArtifactParser>>,
}

impl Default for ParserRegistry {
    fn default() -> Self {
        Self {
            parsers: vec![
                Box::new(SkillParser),
                Box::new(ContractParser),
                Box::new(BehaviorSpecParser),
                Box::new(DesignSystemParser),
                Box::new(CommitParser),
                Box::new(HtmlParser),
                Box::new(CodeMetadataParser),
                Box::new(MarkdownParser),
                Box::new(YamlParser),
                Box::new(JsonParser),
            ],
        }
    }
}

impl ParserRegistry {
    /// Registers a parser ahead of the built-ins.
    pub fn register(&mut self, parser: Box<dyn ArtifactParser>) {
        self.parsers.insert(0, parser);
    }

    pub fn select(&self, artifact: &NexusArtifact) -> &dyn ArtifactParser {
        self.parsers
            .iter()
            .find(|p| p.supports(artifact))
            .map(|p| p.as_ref())
            .unwrap_or(&FallbackParser)
    }

    /// Runs the selected parser; returns its name and the finalized model.
    pub async fn inspect(
        &self,
        artifact: &NexusArtifact,
    ) -> SemanticResult<(&'static str, SemanticModel)> {
        let parser = self.select(artifact);
        let mut model = parser.inspect(artifact).await?;
        model.artifacts.push(artifact.id.clone());
        model.finalize();
        Ok((parser.name(), model))
    }
}

pub(crate) fn text_of(artifact: &NexusArtifact) -> SemanticResult<&str> {
    artifact.text().ok_or_else(|| {
        SemanticError::InvalidInput(format!("artifact \"{}\" has not been loaded", artifact.id))
    })
}

pub(crate) fn has_ext(artifact: &NexusArtifact, exts: &[&str]) -> bool {
    artifact
        .extension()
        .is_some_and(|e| exts.contains(&e.as_str()))
}

pub(crate) fn parse_yaml(artifact: &NexusArtifact) -> SemanticResult<Value> {
    serde_yaml::from_str(text_of(artifact)?).map_err(|e| SemanticError::Parse {
        source_path: artifact.provenance.source.clone(),
        message: e.to_string(),
    })
}

pub(crate) fn parse_json(artifact: &NexusArtifact) -> SemanticResult<Value> {
    serde_json::from_str(text_of(artifact)?).map_err(|e| SemanticError::Parse {
        source_path: artifact.provenance.source.clone(),
        message: e.to_string(),
    })
}

/// Structured YAML/JSON content if the artifact is YAML or JSON and parses.
pub(crate) fn structured(artifact: &NexusArtifact) -> Option<Value> {
    if has_ext(artifact, &["yaml", "yml"]) {
        parse_yaml(artifact).ok()
    } else if has_ext(artifact, &["json"]) {
        parse_json(artifact).ok()
    } else {
        None
    }
}

/// A string or a list of strings.
pub(crate) fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| match x {
                Value::String(s) => Some(s.clone()),
                Value::Object(o) => o
                    .get("description")
                    .or_else(|| o.get("statement"))
                    .or_else(|| o.get("name"))
                    .or_else(|| o.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Explicit basis pointing at the first line mentioning `needle`.
pub(crate) fn explicit_at(artifact: &NexusArtifact, parser: &str, needle: &str) -> Basis {
    let line = artifact
        .text()
        .and_then(|t| crate::text::find_line(t, needle));
    Basis::explicit(artifact.at(parser, line.map(|l| (l, l))))
}
