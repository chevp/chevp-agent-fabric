use crate::errors::{NexusError, NexusResult};
use crate::fs_util::{list_files_recursive, read_yaml_file};
use crate::types::{ClientInfo, PolicyRule};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct PolicyFile {
    client: String,
    permissions: Vec<String>,
}

fn load_policy_file(path: &Path, project_id: Option<&str>) -> NexusResult<PolicyRule> {
    let data: PolicyFile = read_yaml_file(path)?;
    if data.permissions.is_empty() {
        return Err(NexusError::Validation {
            message: "policy must declare at least one permission".to_string(),
            path: path.to_path_buf(),
        });
    }
    Ok(PolicyRule {
        client: data.client,
        permissions: data.permissions,
        project_id: project_id.map(|s| s.to_string()),
        source_path: path.to_path_buf(),
    })
}

/// Filesystem/Git-backed policy storage: `global_root/policies` and
/// `projects_root/<id>/policies`.
pub struct PolicyStore {
    global_root: PathBuf,
    projects_root: PathBuf,
}

impl PolicyStore {
    pub fn new(global_root: impl Into<PathBuf>, projects_root: impl Into<PathBuf>) -> Self {
        Self {
            global_root: global_root.into(),
            projects_root: projects_root.into(),
        }
    }

    pub fn list_global_policies(&self) -> NexusResult<Vec<PolicyRule>> {
        let files = list_files_recursive(&self.global_root.join("policies"), &["yaml", "yml"])?;
        files.iter().map(|f| load_policy_file(f, None)).collect()
    }

    pub fn list_project_policies(&self, project_id: &str) -> NexusResult<Vec<PolicyRule>> {
        let dir = self.projects_root.join(project_id).join("policies");
        let files = list_files_recursive(&dir, &["yaml", "yml"])?;
        files
            .iter()
            .map(|f| load_policy_file(f, Some(project_id)))
            .collect()
    }
}

/// Independent of MCP: a plain domain service resolving and evaluating
/// permissions for a (client, project) pair.
pub struct PolicyEngine {
    store: PolicyStore,
}

impl PolicyEngine {
    pub fn new(store: PolicyStore) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &PolicyStore {
        &self.store
    }

    /// Project-scoped rules for a client replace (not merge with) its global
    /// rules; a client with no project-specific rule falls back to global.
    pub fn policies_for_client(
        &self,
        client_id: &str,
        project_id: Option<&str>,
    ) -> NexusResult<Vec<PolicyRule>> {
        let global: Vec<PolicyRule> = self
            .store
            .list_global_policies()?
            .into_iter()
            .filter(|p| p.client == client_id)
            .collect();
        let Some(project_id) = project_id else {
            return Ok(global);
        };
        let project: Vec<PolicyRule> = self
            .store
            .list_project_policies(project_id)?
            .into_iter()
            .filter(|p| p.client == client_id)
            .collect();
        Ok(if project.is_empty() { global } else { project })
    }

    pub fn permissions(
        &self,
        client_id: &str,
        project_id: Option<&str>,
    ) -> NexusResult<HashSet<String>> {
        Ok(self
            .policies_for_client(client_id, project_id)?
            .into_iter()
            .flat_map(|p| p.permissions)
            .collect())
    }

    pub fn has_permission(
        &self,
        client_id: &str,
        permission: &str,
        project_id: Option<&str>,
    ) -> NexusResult<bool> {
        Ok(self
            .permissions(client_id, project_id)?
            .contains(permission))
    }
}

/// Returns `PermissionDenied` unless `client` holds `permission` for `project_id`.
pub fn require_permission(
    engine: &PolicyEngine,
    client: &ClientInfo,
    permission: &str,
    project_id: Option<&str>,
) -> NexusResult<()> {
    if engine.has_permission(&client.id, permission, project_id)? {
        Ok(())
    } else {
        Err(NexusError::PermissionDenied {
            client: client.id.clone(),
            permission: permission.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::{write_file, TmpRepo};

    #[test]
    fn grants_permissions_declared_for_a_client() {
        let repo = TmpRepo::new();
        write_file(
            &repo.projects_root.join("acme-app/policies/copilot.yaml"),
            "client: copilot\npermissions:\n  - read:project\n  - write:code-reference\n",
        );

        let engine = PolicyEngine::new(PolicyStore::new(&repo.global_root, &repo.projects_root));

        assert!(engine
            .has_permission("copilot", "read:project", Some("acme-app"))
            .unwrap());
        assert!(!engine
            .has_permission("copilot", "write:decision", Some("acme-app"))
            .unwrap());
    }

    #[test]
    fn falls_back_to_global_policy_when_no_project_rule_exists_for_a_client() {
        let repo = TmpRepo::new();
        write_file(
            &repo.global_root.join("policies/ide-agent.yaml"),
            "client: ide-agent\npermissions:\n  - read:project\n",
        );

        let engine = PolicyEngine::new(PolicyStore::new(&repo.global_root, &repo.projects_root));

        assert!(engine
            .has_permission("ide-agent", "read:project", Some("acme-app"))
            .unwrap());
    }

    #[test]
    fn project_scoped_rules_for_a_client_override_its_global_rules() {
        let repo = TmpRepo::new();
        write_file(
            &repo.global_root.join("policies/copilot.yaml"),
            "client: copilot\npermissions:\n  - read:project\n",
        );
        write_file(
            &repo.projects_root.join("acme-app/policies/copilot.yaml"),
            "client: copilot\npermissions:\n  - read:project\n  - write:code-reference\n",
        );

        let engine = PolicyEngine::new(PolicyStore::new(&repo.global_root, &repo.projects_root));
        let mut permissions: Vec<String> = engine
            .permissions("copilot", Some("acme-app"))
            .unwrap()
            .into_iter()
            .collect();
        permissions.sort();

        assert_eq!(
            permissions,
            vec![
                "read:project".to_string(),
                "write:code-reference".to_string()
            ]
        );
    }

    #[test]
    fn require_permission_errors_when_unauthorized() {
        let repo = TmpRepo::new();
        let engine = PolicyEngine::new(PolicyStore::new(&repo.global_root, &repo.projects_root));
        let client = ClientInfo {
            id: "unknown-client".to_string(),
            kind: "coding-agent".to_string(),
        };

        assert!(matches!(
            require_permission(&engine, &client, "read:project", None),
            Err(NexusError::PermissionDenied { .. })
        ));
    }
}
