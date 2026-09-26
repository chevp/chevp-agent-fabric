use crate::errors::{NexusError, NexusResult};
use crate::fs_util::{list_files_recursive, read_yaml_file};
use crate::types::{BehaviorRule, BehaviorSpec, BehaviorTransition};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct BehaviorFile {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    version: u32,
    states: Vec<String>,
    #[serde(default)]
    transitions: Vec<BehaviorTransition>,
    #[serde(default)]
    rules: Vec<BehaviorRule>,
}

fn load_behavior_file(path: &Path, project_id: &str) -> NexusResult<BehaviorSpec> {
    let data: BehaviorFile = read_yaml_file(path)?;
    if data.states.is_empty() {
        return Err(NexusError::Validation {
            message: "behavior spec must declare at least one state".to_string(),
            path: path.to_path_buf(),
        });
    }
    let state_set: HashSet<&str> = data.states.iter().map(|s| s.as_str()).collect();
    for transition in &data.transitions {
        if !state_set.contains(transition.from.as_str())
            || !state_set.contains(transition.to.as_str())
        {
            return Err(NexusError::Validation {
                message: format!("transition {:?} references an undeclared state", transition),
                path: path.to_path_buf(),
            });
        }
    }
    Ok(BehaviorSpec {
        id: data.id,
        kind: data.kind,
        version: data.version,
        states: data.states,
        transitions: data.transitions,
        rules: data.rules,
        project_id: project_id.to_string(),
        source_path: path.to_path_buf(),
    })
}

/// Filesystem/Git-backed behavior spec storage under
/// `projects_root/<id>/behavior`.
pub struct BehaviorStore {
    projects_root: PathBuf,
}

impl BehaviorStore {
    pub fn new(projects_root: impl Into<PathBuf>) -> Self {
        Self {
            projects_root: projects_root.into(),
        }
    }

    pub fn list_behaviors(&self, project_id: &str) -> NexusResult<Vec<BehaviorSpec>> {
        let dir = self.projects_root.join(project_id).join("behavior");
        let files = list_files_recursive(&dir, &["yaml", "yml"])?;
        files
            .iter()
            .map(|f| load_behavior_file(f, project_id))
            .collect()
    }

    pub fn get_behavior(&self, project_id: &str, id: &str) -> NexusResult<Option<BehaviorSpec>> {
        Ok(self
            .list_behaviors(project_id)?
            .into_iter()
            .find(|b| b.id == id))
    }

    pub fn require_behavior(&self, project_id: &str, id: &str) -> NexusResult<BehaviorSpec> {
        self.get_behavior(project_id, id)?
            .ok_or_else(|| NexusError::NotFound {
                kind: "Behavior",
                id: id.to_string(),
            })
    }
}

fn tokenize(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(|w| w.to_string())
        .collect()
}

/// Resolves the behavior specs relevant to a task: those whose id matches a
/// resolved graph entity id, or overlaps the task text by keyword.
pub fn resolve_behaviors(
    store: &BehaviorStore,
    project_id: &str,
    task: Option<&str>,
    behavior_ids: &[String],
) -> NexusResult<Vec<BehaviorSpec>> {
    let all = store.list_behaviors(project_id)?;
    let mut relevant: HashSet<String> = behavior_ids.iter().cloned().collect();

    if let Some(task) = task {
        let task_words = tokenize(task);
        for behavior in &all {
            let id_words = tokenize(&behavior.id);
            if !id_words.is_empty() && id_words.iter().any(|w| task_words.contains(w)) {
                relevant.insert(behavior.id.clone());
            }
        }
    }

    Ok(all
        .into_iter()
        .filter(|b| relevant.contains(&b.id))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::{write_file, TmpRepo};

    const VALID_BEHAVIOR: &str = "
id: checkout-button
type: ui-behavior
version: 1
states:
  - idle
  - loading
  - success
  - error
transitions:
  - from: idle
    event: submit
    to: loading
  - from: loading
    event: success
    to: success
  - from: loading
    event: failure
    to: error
rules:
  - id: prevent-double-submit
    description: User cannot submit twice while loading
";

    #[test]
    fn parses_a_valid_behavior_spec() {
        let repo = TmpRepo::new();
        write_file(
            &repo
                .projects_root
                .join("acme-app/behavior/checkout-button.yaml"),
            VALID_BEHAVIOR,
        );

        let store = BehaviorStore::new(&repo.projects_root);
        let behavior = store
            .get_behavior("acme-app", "checkout-button")
            .unwrap()
            .unwrap();

        assert_eq!(behavior.states, vec!["idle", "loading", "success", "error"]);
        assert_eq!(behavior.transitions.len(), 3);
        assert_eq!(behavior.rules[0].id, "prevent-double-submit");
    }

    #[test]
    fn rejects_a_transition_referencing_an_undeclared_state() {
        let repo = TmpRepo::new();
        write_file(
            &repo.projects_root.join("acme-app/behavior/broken.yaml"),
            "id: broken\ntype: ui-behavior\nversion: 1\nstates:\n  - idle\ntransitions:\n  - from: idle\n    event: submit\n    to: nonexistent-state\n",
        );

        let store = BehaviorStore::new(&repo.projects_root);
        assert!(matches!(
            store.list_behaviors("acme-app"),
            Err(NexusError::Validation { .. })
        ));
    }

    #[test]
    fn errors_for_a_missing_behavior() {
        let repo = TmpRepo::new();
        let store = BehaviorStore::new(&repo.projects_root);
        assert!(matches!(
            store.require_behavior("acme-app", "nope"),
            Err(NexusError::NotFound { .. })
        ));
    }
}
