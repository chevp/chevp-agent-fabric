use crate::errors::{NexusError, NexusResult};
use crate::fs_util::{list_files_recursive, read_file_to_string};
use crate::types::{Skill, SkillFrontmatter, SkillScope};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Splits a Markdown file into its `---`-delimited YAML frontmatter and the
/// remaining body text.
fn split_frontmatter(raw: &str) -> Option<(&str, &str)> {
    let rest = raw.strip_prefix("---")?;
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    let end = rest.find("\n---")?;
    let frontmatter = &rest[..end];
    let after_marker = &rest[end + 4..];
    let body = after_marker.strip_prefix('\n').unwrap_or(after_marker);
    Some((frontmatter, body))
}

fn load_skill_file(
    path: &Path,
    expected_scope: SkillScope,
    project_id: Option<&str>,
) -> NexusResult<Skill> {
    let raw = read_file_to_string(path)?;
    let (frontmatter_yaml, body) =
        split_frontmatter(&raw).ok_or_else(|| NexusError::Validation {
            message: "skill file is missing '---' delimited YAML frontmatter".to_string(),
            path: path.to_path_buf(),
        })?;

    let frontmatter: SkillFrontmatter =
        serde_yaml::from_str(frontmatter_yaml).map_err(|err| NexusError::Validation {
            message: format!("invalid skill frontmatter: {err}"),
            path: path.to_path_buf(),
        })?;

    if frontmatter.scope != expected_scope {
        return Err(NexusError::Validation {
            message: format!(
                "skill \"{}\" declares scope \"{}\" but is located under a \"{}\" directory",
                frontmatter.id,
                frontmatter.scope.as_str(),
                expected_scope.as_str()
            ),
            path: path.to_path_buf(),
        });
    }

    Ok(Skill {
        id: frontmatter.id,
        name: frontmatter.name,
        version: frontmatter.version,
        scope: frontmatter.scope,
        description: frontmatter.description,
        tags: frontmatter.tags,
        depends_on: frontmatter.depends_on,
        body: body.trim().to_string(),
        project_id: if expected_scope == SkillScope::Project {
            project_id.map(|s| s.to_string())
        } else {
            None
        },
        source_path: path.to_path_buf(),
    })
}

fn assert_no_duplicate_ids(skills: &[Skill]) -> NexusResult<()> {
    let mut seen: HashMap<&str, &Path> = HashMap::new();
    for skill in skills {
        if let Some(existing) = seen.get(skill.id.as_str()) {
            return Err(NexusError::VersionConflict(format!(
                "duplicate skill id \"{}\" found at both {} and {}",
                skill.id,
                existing.display(),
                skill.source_path.display()
            )));
        }
        seen.insert(&skill.id, &skill.source_path);
    }
    Ok(())
}

/// Filesystem/Git-backed skill storage: Markdown files with YAML frontmatter
/// under `global_root/skills` and `projects_root/<id>/skills`.
pub struct SkillStore {
    global_root: PathBuf,
    projects_root: PathBuf,
}

impl SkillStore {
    pub fn new(global_root: impl Into<PathBuf>, projects_root: impl Into<PathBuf>) -> Self {
        Self {
            global_root: global_root.into(),
            projects_root: projects_root.into(),
        }
    }

    pub fn list_global_skills(&self) -> NexusResult<Vec<Skill>> {
        let dir = self.global_root.join("skills");
        let files = list_files_recursive(&dir, &["md"])?;
        let skills = files
            .iter()
            .map(|f| load_skill_file(f, SkillScope::Global, None))
            .collect::<NexusResult<Vec<_>>>()?;
        assert_no_duplicate_ids(&skills)?;
        Ok(skills)
    }

    pub fn list_project_skills(&self, project_id: &str) -> NexusResult<Vec<Skill>> {
        let dir = self.projects_root.join(project_id).join("skills");
        let files = list_files_recursive(&dir, &["md"])?;
        let skills = files
            .iter()
            .map(|f| load_skill_file(f, SkillScope::Project, Some(project_id)))
            .collect::<NexusResult<Vec<_>>>()?;
        assert_no_duplicate_ids(&skills)?;
        Ok(skills)
    }

    /// Global skills merged with a project's skills; a project skill
    /// overrides a global skill declared under the same id.
    pub fn list_skills(&self, project_id: Option<&str>) -> NexusResult<Vec<Skill>> {
        let global = self.list_global_skills()?;
        let Some(project_id) = project_id else {
            return Ok(global);
        };
        let project = self.list_project_skills(project_id)?;
        Ok(merge_with_project_overrides(global, project))
    }

    pub fn get_skill(&self, id: &str, project_id: Option<&str>) -> NexusResult<Skill> {
        self.list_skills(project_id)?
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| NexusError::NotFound {
                kind: "Skill",
                id: id.to_string(),
            })
    }
}

fn merge_with_project_overrides(global: Vec<Skill>, project: Vec<Skill>) -> Vec<Skill> {
    let mut merged: HashMap<String, Skill> = HashMap::new();
    for skill in global {
        merged.insert(skill.id.clone(), skill);
    }
    for skill in project {
        merged.insert(skill.id.clone(), skill);
    }
    merged.into_values().collect()
}

// Function words that would otherwise match nearly every skill description.
const STOPWORDS: &[&str] = &["the", "and", "for", "with", "from", "into", "this", "that", "how"];

fn tokenize(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() > 2 && !STOPWORDS.contains(w))
        .map(|w| w.to_string())
        .collect()
}

fn is_relevant(skill: &Skill, task_words: &HashSet<String>) -> bool {
    let mut haystack_text = format!("{} {} {}", skill.id, skill.name, skill.description);
    for tag in &skill.tags {
        haystack_text.push(' ');
        haystack_text.push_str(tag);
    }
    let haystack = tokenize(&haystack_text);
    task_words.iter().any(|w| haystack.contains(w))
}

/// Resolves the set of skills relevant to a task: skills matched by keyword
/// overlap with `task`, plus `explicit_ids`, plus the transitive `dependsOn`
/// closure of both.
pub fn resolve_skills(
    store: &SkillStore,
    project_id: Option<&str>,
    task: Option<&str>,
    explicit_ids: &[String],
) -> NexusResult<Vec<Skill>> {
    let all = store.list_skills(project_id)?;
    let by_id: HashMap<&str, &Skill> = all.iter().map(|s| (s.id.as_str(), s)).collect();

    let mut relevant: HashSet<String> = explicit_ids.iter().cloned().collect();

    if let Some(task) = task {
        let task_words = tokenize(task);
        for skill in &all {
            if is_relevant(skill, &task_words) {
                relevant.insert(skill.id.clone());
            }
        }
    }

    let mut queue: Vec<String> = relevant.iter().cloned().collect();
    while let Some(id) = queue.pop() {
        let skill = by_id.get(id.as_str()).ok_or_else(|| NexusError::NotFound {
            kind: "Skill",
            id: id.clone(),
        })?;
        for dep in &skill.depends_on {
            if relevant.insert(dep.clone()) {
                queue.push(dep.clone());
            }
        }
    }

    relevant
        .into_iter()
        .map(|id| {
            by_id
                .get(id.as_str())
                .map(|s| (*s).clone())
                .ok_or(NexusError::NotFound { kind: "Skill", id })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::{write_file, TmpRepo};

    fn skill_md(frontmatter: &str, body: &str) -> String {
        format!("---\n{frontmatter}\n---\n\n{body}\n")
    }

    #[test]
    fn discovers_global_and_project_skills() {
        let repo = TmpRepo::new();
        write_file(
            &repo.global_root.join("skills/accessibility/skill.md"),
            &skill_md(
                "id: accessibility\nname: Accessibility\nversion: 1.0.0\nscope: global\ndescription: Baseline a11y",
                "Body.",
            ),
        );
        write_file(
            &repo.projects_root.join("acme-app/skills/checkout-ux/skill.md"),
            &skill_md(
                "id: checkout-ux\nname: Checkout UX\nversion: 1.0.0\nscope: project\ndescription: Checkout rules",
                "Body.",
            ),
        );

        let store = SkillStore::new(&repo.global_root, &repo.projects_root);
        let mut ids: Vec<String> = store
            .list_skills(Some("acme-app"))
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        ids.sort();

        assert_eq!(
            ids,
            vec!["accessibility".to_string(), "checkout-ux".to_string()]
        );
    }

    #[test]
    fn project_skill_overrides_global_skill_of_the_same_id() {
        let repo = TmpRepo::new();
        write_file(
            &repo.global_root.join("skills/code-style/skill.md"),
            &skill_md(
                "id: code-style\nname: Code Style\nversion: 1.0.0\nscope: global\ndescription: Generic style",
                "Generic rules.",
            ),
        );
        write_file(
            &repo.projects_root.join("acme-app/skills/code-style/skill.md"),
            &skill_md(
                "id: code-style\nname: Code Style (Acme)\nversion: 2.0.0\nscope: project\ndescription: Acme style",
                "Acme rules.",
            ),
        );

        let store = SkillStore::new(&repo.global_root, &repo.projects_root);
        let skill = store.get_skill("code-style", Some("acme-app")).unwrap();

        assert_eq!(skill.version, "2.0.0");
        assert_eq!(skill.body, "Acme rules.");
    }

    #[test]
    fn rejects_a_skill_whose_frontmatter_scope_does_not_match_its_directory() {
        let repo = TmpRepo::new();
        write_file(
            &repo.global_root.join("skills/mismatched/skill.md"),
            &skill_md(
                "id: mismatched\nname: Mismatched\nversion: 1.0.0\nscope: project\ndescription: Should fail",
                "Body.",
            ),
        );

        let store = SkillStore::new(&repo.global_root, &repo.projects_root);
        assert!(matches!(
            store.list_skills(None),
            Err(NexusError::Validation { .. })
        ));
    }

    #[test]
    fn errors_for_an_unknown_skill_id() {
        let repo = TmpRepo::new();
        let store = SkillStore::new(&repo.global_root, &repo.projects_root);
        assert!(matches!(
            store.get_skill("nope", None),
            Err(NexusError::NotFound { .. })
        ));
    }

    #[test]
    fn resolves_transitive_depends_on_closures() {
        let repo = TmpRepo::new();
        write_file(
            &repo.global_root.join("skills/a/skill.md"),
            &skill_md(
                "id: a\nname: A\nversion: 1.0.0\nscope: global\ndescription: Depends on b\ndependsOn:\n  - b",
                "Body.",
            ),
        );
        write_file(
            &repo.global_root.join("skills/b/skill.md"),
            &skill_md(
                "id: b\nname: B\nversion: 1.0.0\nscope: global\ndescription: Leaf",
                "Body.",
            ),
        );

        let store = SkillStore::new(&repo.global_root, &repo.projects_root);
        let mut ids: Vec<String> = resolve_skills(&store, None, None, &["a".to_string()])
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        ids.sort();

        assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn matches_skills_relevant_to_a_task_by_keyword() {
        let repo = TmpRepo::new();
        write_file(
            &repo.global_root.join("skills/checkout-ux/skill.md"),
            &skill_md(
                "id: checkout-ux\nname: Checkout UX\nversion: 1.0.0\nscope: global\ndescription: Rules for the checkout button\ntags:\n  - checkout",
                "Body.",
            ),
        );
        write_file(
            &repo.global_root.join("skills/unrelated/skill.md"),
            &skill_md(
                "id: unrelated\nname: Unrelated\nversion: 1.0.0\nscope: global\ndescription: Something else entirely",
                "Body.",
            ),
        );

        let store = SkillStore::new(&repo.global_root, &repo.projects_root);
        let ids: Vec<String> =
            resolve_skills(&store, None, Some("Implement the checkout button"), &[])
                .unwrap()
                .into_iter()
                .map(|s| s.id)
                .collect();

        assert_eq!(ids, vec!["checkout-ux".to_string()]);
    }
}
