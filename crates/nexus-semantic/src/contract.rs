//! Semantic contracts (`projects/<id>/contracts/*.yaml`) and the
//! `NexusProject` view that adds roles and contracts to a domain `Project`.

use crate::error::{SemanticError, SemanticResult};
use crate::model::{Provenance, Role};
use crate::text::rel_path;
use nexus_domain::fs_util::{list_files_recursive, read_file_to_string};
use nexus_domain::types::{BehaviorTransition, Project};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContractBehaviors {
    #[serde(default)]
    pub states: Vec<String>,
    #[serde(default)]
    pub transitions: Vec<BehaviorTransition>,
}

/// Something that must stay stable across roles: UX visualizes it,
/// engineering implements it, QA derives tests from it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticContract {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub role: Role,
    /// Entity the contract is about (e.g. `checkout-button`).
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub intent: Vec<String>,
    #[serde(default)]
    pub requirements: Vec<String>,
    #[serde(default)]
    pub behaviors: ContractBehaviors,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default, skip_deserializing)]
    pub provenance: Option<Provenance>,
}

impl SemanticContract {
    pub fn parse(raw: &str, source: &str) -> SemanticResult<Self> {
        let mut contract: SemanticContract =
            serde_yaml::from_str(raw).map_err(|e| SemanticError::Parse {
                source_path: source.to_string(),
                message: e.to_string(),
            })?;
        if contract.id.trim().is_empty() {
            return Err(SemanticError::Parse {
                source_path: source.to_string(),
                message: "contract is missing an id".to_string(),
            });
        }
        contract.provenance = Some(Provenance {
            source: source.to_string(),
            line_start: None,
            line_end: None,
            commit: None,
            extraction: "ContractParser".to_string(),
            artifact: None,
        });
        Ok(contract)
    }

    pub fn subject(&self) -> &str {
        self.subject.as_deref().unwrap_or(&self.id)
    }
}

pub fn load_contracts(repo_root: &Path, project_dir: &Path) -> SemanticResult<Vec<SemanticContract>> {
    let files = list_files_recursive(&project_dir.join("contracts"), &["yaml", "yml"])?;
    files
        .iter()
        .map(|f| {
            let raw = read_file_to_string(f)?;
            SemanticContract::parse(&raw, &rel_path(repo_root, f))
        })
        .collect()
}

/// A domain `Project` plus the semantic roles and contracts it declares.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NexusProject {
    pub id: String,
    pub name: String,
    pub version: u32,
    pub description: Option<String>,
    pub path: PathBuf,
    pub roles: Vec<Role>,
    pub semantic_contracts: Vec<SemanticContract>,
}

impl NexusProject {
    pub fn load(repo_root: &Path, project: Project) -> SemanticResult<Self> {
        let semantic_contracts = load_contracts(repo_root, &project.path)?;
        Ok(Self {
            roles: project.roles.iter().map(|r| Role::from(r.as_str())).collect(),
            id: project.id,
            name: project.name,
            version: project.version,
            description: project.description,
            path: project.path,
            semantic_contracts,
        })
    }
}
