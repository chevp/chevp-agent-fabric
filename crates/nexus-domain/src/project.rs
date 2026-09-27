use crate::errors::{NexusError, NexusResult};
use crate::fs_util::{list_subdirectories, path_exists, read_yaml_file};
use crate::types::Project;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct ProjectFile {
    id: String,
    name: String,
    version: u32,
    description: Option<String>,
    #[serde(default)]
    roles: Vec<String>,
}

fn load_project_file(project_dir: &Path) -> NexusResult<Project> {
    let file_path = project_dir.join("project.yaml");
    let data: ProjectFile = read_yaml_file(&file_path)?;
    Ok(Project {
        id: data.id,
        name: data.name,
        version: data.version,
        description: data.description,
        path: project_dir.to_path_buf(),
        roles: data.roles,
    })
}

/// Filesystem/Git-backed project storage: one directory per project under
/// `projects_root`, each with a `project.yaml`.
pub struct ProjectStore {
    projects_root: PathBuf,
}

impl ProjectStore {
    pub fn new(projects_root: impl Into<PathBuf>) -> Self {
        Self {
            projects_root: projects_root.into(),
        }
    }

    pub fn list_projects(&self) -> NexusResult<Vec<Project>> {
        let mut projects = Vec::new();
        for dir_name in list_subdirectories(&self.projects_root)? {
            let project_dir = self.projects_root.join(&dir_name);
            if path_exists(&project_dir.join("project.yaml")) {
                projects.push(load_project_file(&project_dir)?);
            }
        }
        Ok(projects)
    }

    pub fn get_project(&self, id: &str) -> NexusResult<Option<Project>> {
        let project_dir = self.projects_root.join(id);
        if !path_exists(&project_dir.join("project.yaml")) {
            return Ok(None);
        }
        Ok(Some(load_project_file(&project_dir)?))
    }

    pub fn require_project(&self, id: &str) -> NexusResult<Project> {
        self.get_project(id)?.ok_or_else(|| NexusError::NotFound {
            kind: "Project",
            id: id.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::{write_file, TmpRepo};

    #[test]
    fn loads_a_project_from_project_yaml() {
        let repo = TmpRepo::new();
        write_file(
            &repo.projects_root.join("acme-app").join("project.yaml"),
            "id: acme-app\nname: Acme App\nversion: 1\n",
        );

        let store = ProjectStore::new(&repo.projects_root);
        let project = store.get_project("acme-app").unwrap().unwrap();

        assert_eq!(project.id, "acme-app");
        assert_eq!(project.name, "Acme App");
        assert_eq!(project.version, 1);
        assert_eq!(project.description, None);
    }

    #[test]
    fn lists_every_project_directory_that_has_a_project_yaml() {
        let repo = TmpRepo::new();
        write_file(
            &repo.projects_root.join("a").join("project.yaml"),
            "id: a\nname: A\nversion: 1\n",
        );
        write_file(
            &repo.projects_root.join("b").join("project.yaml"),
            "id: b\nname: B\nversion: 1\n",
        );
        write_file(
            &repo.projects_root.join("not-a-project").join("notes.md"),
            "hi",
        );

        let store = ProjectStore::new(&repo.projects_root);
        let mut ids: Vec<String> = store
            .list_projects()
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        ids.sort();

        assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn returns_none_for_a_missing_project() {
        let repo = TmpRepo::new();
        let store = ProjectStore::new(&repo.projects_root);
        assert!(store.get_project("nope").unwrap().is_none());
    }

    #[test]
    fn require_project_errors_for_a_missing_project() {
        let repo = TmpRepo::new();
        let store = ProjectStore::new(&repo.projects_root);
        assert!(matches!(
            store.require_project("nope"),
            Err(NexusError::NotFound { .. })
        ));
    }

    #[test]
    fn rejects_invalid_project_data() {
        let repo = TmpRepo::new();
        write_file(
            &repo.projects_root.join("broken").join("project.yaml"),
            "id: broken\nname: Broken\n",
        );

        let store = ProjectStore::new(&repo.projects_root);
        assert!(matches!(
            store.get_project("broken"),
            Err(NexusError::Validation { .. })
        ));
    }
}
