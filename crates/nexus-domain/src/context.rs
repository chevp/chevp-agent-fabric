use crate::errors::NexusResult;
use crate::fs_util::{list_files_recursive, read_file_to_string};
use crate::graph::resolve_graph_context;
use crate::policy::require_permission;
use crate::types::{ContextDocument, ContextRequest, ResolvedContext};
use crate::{behavior, skill, NexusDomain};
use std::collections::HashSet;

/// Loads all project knowledge/documentation Markdown files under `context/`.
fn load_context_documents(
    projects_root: &std::path::Path,
    project_id: &str,
) -> NexusResult<Vec<ContextDocument>> {
    let dir = projects_root.join(project_id).join("context");
    let files = list_files_recursive(&dir, &["md"])?;
    files
        .into_iter()
        .map(|path| {
            let raw = read_file_to_string(&path)?;
            let title = raw
                .lines()
                .find(|l| l.starts_with("# "))
                .map(|l| l.trim_start_matches("# ").trim().to_string())
                .unwrap_or_else(|| {
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("untitled")
                        .to_string()
                });
            let id = path
                .strip_prefix(&dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            Ok(ContextDocument {
                id,
                title,
                body: raw.trim().to_string(),
                source_path: path,
            })
        })
        .collect()
}

fn tokenize(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(|w| w.to_string())
        .collect()
}

fn relevant_context_documents(
    projects_root: &std::path::Path,
    project_id: &str,
    task: &str,
) -> NexusResult<Vec<ContextDocument>> {
    let documents = load_context_documents(projects_root, project_id)?;
    let task_words = tokenize(task);
    Ok(documents
        .into_iter()
        .filter(|doc| {
            let haystack = tokenize(&format!("{} {}", doc.title, doc.body));
            task_words.iter().any(|w| haystack.contains(w))
        })
        .collect())
}

/// The Context Resolver is the heart of the Control Plane: given a task, a
/// project and a client, it determines the *minimal* relevant context by
/// cross-referencing skills, behavior specs, the semantic graph and policies
/// — never the whole project.
pub fn resolve_context(
    domain: &NexusDomain,
    request: &ContextRequest,
) -> NexusResult<ResolvedContext> {
    let project = domain.projects.require_project(&request.project_id)?;
    require_permission(
        &domain.policies,
        &request.client,
        "read:project",
        Some(project.id.as_str()),
    )?;

    let context_documents =
        relevant_context_documents(&domain.projects_root, &project.id, &request.task)?;

    let (entities, relations) = resolve_graph_context(
        &domain.graph,
        &project.id,
        &request.requested_entities,
        Some(request.task.as_str()),
        1,
    )?;

    // Skills governing the resolved entities are pulled in explicitly, in
    // addition to whatever the task text itself matches.
    let governing_skill_ids: Vec<String> = relations
        .iter()
        .filter(|r| {
            matches!(
                r.relation,
                crate::types::RelationKind::GovernedBy | crate::types::RelationKind::ImplementedBy
            )
        })
        .map(|r| r.to.clone())
        .collect();

    let skills = skill::resolve_skills(
        &domain.skills,
        Some(project.id.as_str()),
        Some(request.task.as_str()),
        &governing_skill_ids,
    )?;

    let behavior_ids: Vec<String> = entities.iter().map(|e| e.id.clone()).collect();
    let behaviors = behavior::resolve_behaviors(
        &domain.behaviors,
        &project.id,
        Some(request.task.as_str()),
        &behavior_ids,
    )?;

    let policies = domain
        .policies
        .policies_for_client(&request.client.id, Some(project.id.as_str()))?;

    Ok(ResolvedContext {
        project,
        context_documents,
        skills,
        behaviors,
        entities,
        relations,
        policies,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ClientInfo;
    use std::path::PathBuf;

    /// Repo root two levels up from this crate: `<repo>/crates/nexus-domain`.
    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn resolves_the_minimal_relevant_context_for_implement_the_checkout_button() {
        let domain = NexusDomain::from_repo_root(repo_root());
        let request = ContextRequest {
            project_id: "acme-app".to_string(),
            task: "Implement the checkout button".to_string(),
            client: ClientInfo {
                id: "copilot".to_string(),
                kind: "coding-agent".to_string(),
            },
            requested_entities: Vec::new(),
        };

        let context = resolve_context(&domain, &request).unwrap();

        assert_eq!(context.project.id, "acme-app");

        let mut skill_ids: Vec<&str> = context.skills.iter().map(|s| s.id.as_str()).collect();
        skill_ids.sort();
        assert_eq!(skill_ids, vec!["accessibility", "checkout-ux"]);
        assert!(!skill_ids.contains(&"code-style"));

        assert_eq!(
            context
                .behaviors
                .iter()
                .map(|b| b.id.as_str())
                .collect::<Vec<_>>(),
            vec!["checkout-button"]
        );

        let mut entity_ids: Vec<&str> = context.entities.iter().map(|e| e.id.as_str()).collect();
        entity_ids.sort();
        assert_eq!(
            entity_ids,
            vec!["checkout-button", "checkout-summary", "payment-flow"]
        );

        assert!(context
            .policies
            .iter()
            .any(|p| p.permissions.iter().any(|perm| perm == "read:project")));
    }

    #[test]
    fn does_not_resolve_context_for_a_client_without_read_project_permission() {
        let domain = NexusDomain::from_repo_root(repo_root());
        let request = ContextRequest {
            project_id: "acme-app".to_string(),
            task: "Implement the checkout button".to_string(),
            client: ClientInfo {
                id: "no-such-client".to_string(),
                kind: "coding-agent".to_string(),
            },
            requested_entities: Vec::new(),
        };

        assert!(matches!(
            resolve_context(&domain, &request),
            Err(crate::errors::NexusError::PermissionDenied { .. })
        ));
    }

    #[test]
    fn errors_for_an_unknown_project() {
        let domain = NexusDomain::from_repo_root(repo_root());
        let request = ContextRequest {
            project_id: "does-not-exist".to_string(),
            task: "anything".to_string(),
            client: ClientInfo {
                id: "copilot".to_string(),
                kind: "coding-agent".to_string(),
            },
            requested_entities: Vec::new(),
        };

        assert!(matches!(
            resolve_context(&domain, &request),
            Err(crate::errors::NexusError::NotFound { .. })
        ));
    }
}
