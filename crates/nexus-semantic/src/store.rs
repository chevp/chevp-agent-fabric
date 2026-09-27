//! Filesystem persistence for the semantic layer, next to the project's
//! authored files so it lives in Git like everything else:
//!
//! ```text
//! projects/<id>/semantic/
//! ├── artifacts/<artifact-id>.json     registered NexusArtifacts
//! ├── models/<artifact-id>.json        last SemanticModel per artifact
//! ├── translations/<id>.json
//! ├── proposals/<id>.json              pending | accepted | rejected | superseded
//! ├── ingest.json                      last ingestion report
//! ├── semantic-graph.json              exported graph view
//! └── semantic-project.html            exported explorer
//! ```

use crate::error::{SemanticError, SemanticResult};
use crate::model::{ArtifactId, NexusArtifact, SemanticModel};
use crate::proposal::Proposal;
use crate::translate::Translation;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

pub const SEMANTIC_DIR: &str = "semantic";

pub struct SemanticStore {
    pub dir: PathBuf,
}

fn file_name(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' })
        .collect()
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> SemanticResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| SemanticError::io(parent, e))?;
    }
    let mut text = serde_json::to_string_pretty(value)
        .map_err(|e| SemanticError::InvalidInput(e.to_string()))?;
    text.push('\n');
    fs::write(path, text).map_err(|e| SemanticError::io(path, e))
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> SemanticResult<T> {
    let raw = fs::read_to_string(path).map_err(|e| SemanticError::io(path, e))?;
    serde_json::from_str(&raw).map_err(|e| SemanticError::Parse {
        source_path: path.display().to_string(),
        message: e.to_string(),
    })
}

fn read_all<T: DeserializeOwned>(dir: &Path) -> SemanticResult<Vec<T>> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    paths.iter().map(|p| read_json(p)).collect()
}

impl SemanticStore {
    pub fn for_project(project_dir: &Path) -> Self {
        Self {
            dir: project_dir.join(SEMANTIC_DIR),
        }
    }

    fn path(&self, sub: &str, id: &str) -> PathBuf {
        self.dir.join(sub).join(format!("{}.json", file_name(id)))
    }

    pub fn save_artifact(&self, a: &NexusArtifact) -> SemanticResult<()> {
        write_json(&self.path("artifacts", a.id.as_str()), a)
    }

    pub fn artifact(&self, id: &ArtifactId) -> SemanticResult<NexusArtifact> {
        let p = self.path("artifacts", id.as_str());
        if !p.exists() {
            return Err(SemanticError::not_found("Artifact", id.as_str()));
        }
        read_json(&p)
    }

    pub fn artifacts(&self) -> SemanticResult<Vec<NexusArtifact>> {
        read_all(&self.dir.join("artifacts"))
    }

    /// Removes stored artifacts (and their models) matching `produced_by_ingest`,
    /// plus the non-artifact ingest models, before a re-ingest.
    pub fn clear_ingested(&self, produced_by_ingest: impl Fn(&NexusArtifact) -> bool) -> SemanticResult<()> {
        for a in self.artifacts()? {
            if produced_by_ingest(&a) {
                for sub in ["artifacts", "models"] {
                    let p = self.path(sub, a.id.as_str());
                    if p.exists() {
                        fs::remove_file(&p).map_err(|e| SemanticError::io(&p, e))?;
                    }
                }
            }
        }
        for id in [crate::engine::STRUCTURE_MODEL, crate::engine::DERIVED_MODEL] {
            let p = self.path("models", id);
            if p.exists() {
                fs::remove_file(&p).map_err(|e| SemanticError::io(&p, e))?;
            }
        }
        Ok(())
    }

    pub fn save_model(&self, id: &ArtifactId, m: &SemanticModel) -> SemanticResult<()> {
        write_json(&self.path("models", id.as_str()), m)
    }

    pub fn model(&self, id: &ArtifactId) -> SemanticResult<Option<SemanticModel>> {
        let p = self.path("models", id.as_str());
        p.exists().then(|| read_json(&p)).transpose()
    }

    pub fn models(&self) -> SemanticResult<Vec<SemanticModel>> {
        read_all(&self.dir.join("models"))
    }

    pub fn save_translation(&self, t: &Translation) -> SemanticResult<()> {
        write_json(&self.path("translations", &t.id), t)
    }

    pub fn translation(&self, id: &str) -> SemanticResult<Translation> {
        let p = self.path("translations", id);
        if !p.exists() {
            return Err(SemanticError::not_found("Translation", id));
        }
        read_json(&p)
    }

    pub fn save_proposal(&self, p: &Proposal) -> SemanticResult<()> {
        write_json(&self.path("proposals", &p.id), p)
    }

    pub fn proposal(&self, id: &str) -> SemanticResult<Proposal> {
        let p = self.path("proposals", id);
        if !p.exists() {
            return Err(SemanticError::not_found("Proposal", id));
        }
        read_json(&p)
    }

    pub fn proposals(&self) -> SemanticResult<Vec<Proposal>> {
        read_all(&self.dir.join("proposals"))
    }
}
