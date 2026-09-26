use crate::errors::{NexusError, NexusResult};
use crate::fs_util::list_files_recursive;
use crate::types::{GraphEntity, GraphRelation, RelationKind};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

fn load_entity_file(path: &Path, project_id: &str) -> NexusResult<GraphEntity> {
    let raw: Value = crate::fs_util::read_yaml_file(path)?;
    let mut object = raw
        .as_object()
        .cloned()
        .ok_or_else(|| NexusError::Validation {
            message: "graph entity must be a YAML mapping".to_string(),
            path: path.to_path_buf(),
        })?;

    let id = take_string(&mut object, "id", path)?;
    let kind = take_string(&mut object, "type", path)?;
    let name = take_string(&mut object, "name", path)?;

    Ok(GraphEntity {
        id,
        kind,
        name,
        attributes: object,
        project_id: project_id.to_string(),
        source_path: path.to_path_buf(),
    })
}

fn take_string(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    path: &Path,
) -> NexusResult<String> {
    object
        .remove(key)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .ok_or_else(|| NexusError::Validation {
            message: format!("graph entity is missing required string field \"{key}\""),
            path: path.to_path_buf(),
        })
}

#[derive(serde::Deserialize)]
struct RelationFile {
    from: String,
    relation: RelationKind,
    to: String,
}

fn load_relation_file(path: &Path, project_id: &str) -> NexusResult<GraphRelation> {
    let data: RelationFile = crate::fs_util::read_yaml_file(path)?;
    Ok(GraphRelation {
        from: data.from,
        relation: data.relation,
        to: data.to,
        project_id: project_id.to_string(),
        source_path: path.to_path_buf(),
    })
}

fn assert_no_duplicate_entity_ids(entities: &[GraphEntity]) -> NexusResult<()> {
    let mut seen: HashMap<&str, &Path> = HashMap::new();
    for entity in entities {
        if let Some(existing) = seen.get(entity.id.as_str()) {
            return Err(NexusError::VersionConflict(format!(
                "duplicate entity id \"{}\" found at both {} and {}",
                entity.id,
                existing.display(),
                entity.source_path.display()
            )));
        }
        seen.insert(&entity.id, &entity.source_path);
    }
    Ok(())
}

/// A relation may point at an entity, a skill, or another graph object
/// outside this store's view; we only fail closed when neither endpoint
/// resolves to a known entity.
fn assert_endpoint_is_resolvable(
    relation: &GraphRelation,
    entity_ids: &HashSet<&str>,
) -> NexusResult<()> {
    if !entity_ids.contains(relation.from.as_str()) && !entity_ids.contains(relation.to.as_str()) {
        return Err(NexusError::Validation {
            message: format!(
                "relation references neither a known entity as \"from\" nor \"to\" ({} -> {})",
                relation.from, relation.to
            ),
            path: relation.source_path.clone(),
        });
    }
    Ok(())
}

/// Filesystem/Git-backed storage for the Semantic Content Graph:
/// `projects_root/<id>/graph/{entities,relations}`.
pub struct GraphStore {
    projects_root: PathBuf,
}

impl GraphStore {
    pub fn new(projects_root: impl Into<PathBuf>) -> Self {
        Self {
            projects_root: projects_root.into(),
        }
    }

    pub fn list_entities(&self, project_id: &str) -> NexusResult<Vec<GraphEntity>> {
        let dir = self
            .projects_root
            .join(project_id)
            .join("graph")
            .join("entities");
        let files = list_files_recursive(&dir, &["yaml", "yml"])?;
        let entities = files
            .iter()
            .map(|f| load_entity_file(f, project_id))
            .collect::<NexusResult<Vec<_>>>()?;
        assert_no_duplicate_entity_ids(&entities)?;
        Ok(entities)
    }

    pub fn list_relations(&self, project_id: &str) -> NexusResult<Vec<GraphRelation>> {
        let dir = self
            .projects_root
            .join(project_id)
            .join("graph")
            .join("relations");
        let files = list_files_recursive(&dir, &["yaml", "yml"])?;
        let relations = files
            .iter()
            .map(|f| load_relation_file(f, project_id))
            .collect::<NexusResult<Vec<_>>>()?;
        let entities = self.list_entities(project_id)?;
        let entity_id_set: HashSet<&str> = entities.iter().map(|e| e.id.as_str()).collect();
        for relation in &relations {
            assert_endpoint_is_resolvable(relation, &entity_id_set)?;
        }
        Ok(relations)
    }

    pub fn get_entity(&self, project_id: &str, id: &str) -> NexusResult<Option<GraphEntity>> {
        Ok(self
            .list_entities(project_id)?
            .into_iter()
            .find(|e| e.id == id))
    }
}

pub struct RelatedEntitiesOptions {
    pub depth: usize,
    pub relation_kinds: Option<Vec<RelationKind>>,
}

impl Default for RelatedEntitiesOptions {
    fn default() -> Self {
        Self {
            depth: 1,
            relation_kinds: None,
        }
    }
}

/// Breadth-first traversal from `entity_id` up to `depth` hops, in either
/// direction. The origin entity itself is never included in the result.
pub fn get_related_entities(
    store: &GraphStore,
    project_id: &str,
    entity_id: &str,
    options: RelatedEntitiesOptions,
) -> NexusResult<(Vec<GraphEntity>, Vec<GraphRelation>)> {
    let all_entities = store.list_entities(project_id)?;
    let all_relations = store.list_relations(project_id)?;
    let entity_by_id: HashMap<&str, &GraphEntity> =
        all_entities.iter().map(|e| (e.id.as_str(), e)).collect();

    let relations_to_use: Vec<&GraphRelation> = match &options.relation_kinds {
        Some(kinds) => all_relations
            .iter()
            .filter(|r| kinds.contains(&r.relation))
            .collect(),
        None => all_relations.iter().collect(),
    };

    let mut visited: HashSet<String> = HashSet::from([entity_id.to_string()]);
    let mut found_entities: HashMap<String, GraphEntity> = HashMap::new();
    let mut found_relations: Vec<GraphRelation> = Vec::new();
    let mut frontier: Vec<String> = vec![entity_id.to_string()];

    for _ in 0..options.depth {
        if frontier.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for current in &frontier {
            for relation in &relations_to_use {
                let neighbor = if relation.from == *current {
                    Some(relation.to.as_str())
                } else if relation.to == *current {
                    Some(relation.from.as_str())
                } else {
                    None
                };
                let Some(neighbor) = neighbor else { continue };
                found_relations.push((*relation).clone());
                if let Some(entity) = entity_by_id.get(neighbor) {
                    found_entities.insert(neighbor.to_string(), (*entity).clone());
                }
                if visited.insert(neighbor.to_string()) {
                    next.push(neighbor.to_string());
                }
            }
        }
        frontier = next;
    }

    found_entities.remove(entity_id);
    Ok((
        found_entities.into_values().collect(),
        dedupe_relations(found_relations),
    ))
}

/// Shortest path (by hop count) between two entities, following relations in
/// either direction. `None` if no path exists.
pub fn find_path(
    store: &GraphStore,
    project_id: &str,
    from_id: &str,
    to_id: &str,
) -> NexusResult<Option<Vec<GraphRelation>>> {
    if from_id == to_id {
        return Ok(Some(Vec::new()));
    }
    let relations = store.list_relations(project_id)?;
    let mut adjacency: HashMap<&str, Vec<&GraphRelation>> = HashMap::new();
    for relation in &relations {
        adjacency
            .entry(relation.from.as_str())
            .or_default()
            .push(relation);
        adjacency
            .entry(relation.to.as_str())
            .or_default()
            .push(relation);
    }

    let mut visited: HashSet<&str> = HashSet::from([from_id]);
    let mut queue: VecDeque<(&str, Vec<GraphRelation>)> = VecDeque::from([(from_id, Vec::new())]);

    while let Some((node, path)) = queue.pop_front() {
        let Some(edges) = adjacency.get(node) else {
            continue;
        };
        for relation in edges {
            let neighbor = if relation.from == node {
                relation.to.as_str()
            } else {
                relation.from.as_str()
            };
            if visited.contains(neighbor) {
                continue;
            }
            let mut new_path = path.clone();
            new_path.push((*relation).clone());
            if neighbor == to_id {
                return Ok(Some(new_path));
            }
            visited.insert(neighbor);
            queue.push_back((neighbor, new_path));
        }
    }
    Ok(None)
}

/// Case-insensitive substring search over entity id, name and type.
pub fn search_graph(
    store: &GraphStore,
    project_id: &str,
    query: &str,
) -> NexusResult<Vec<GraphEntity>> {
    let needle = query.to_lowercase();
    Ok(store
        .list_entities(project_id)?
        .into_iter()
        .filter(|e| {
            e.id.to_lowercase().contains(&needle)
                || e.name.to_lowercase().contains(&needle)
                || e.kind.to_lowercase().contains(&needle)
        })
        .collect())
}

fn dedupe_relations(relations: Vec<GraphRelation>) -> Vec<GraphRelation> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for relation in relations {
        let key = (
            relation.from.clone(),
            relation.relation,
            relation.to.clone(),
        );
        if seen.insert(key) {
            result.push(relation);
        }
    }
    result
}

/// Resolves the minimal relevant slice of the Semantic Content Graph for a
/// task: seed entities (requested, or found via a task-keyword search) plus
/// their immediate neighborhood.
pub fn resolve_graph_context(
    store: &GraphStore,
    project_id: &str,
    requested_entities: &[String],
    task: Option<&str>,
    depth: usize,
) -> NexusResult<(Vec<GraphEntity>, Vec<GraphRelation>)> {
    let mut seeds: HashSet<String> = requested_entities.iter().cloned().collect();

    if seeds.is_empty() {
        if let Some(task) = task {
            for word in tokenize(task) {
                for entity in search_graph(store, project_id, &word)? {
                    seeds.insert(entity.id);
                }
            }
        }
    }

    let mut entities: HashMap<String, GraphEntity> = HashMap::new();
    let mut relations: Vec<GraphRelation> = Vec::new();
    for seed_id in &seeds {
        if let Some(seed_entity) = store.get_entity(project_id, seed_id)? {
            entities.insert(seed_entity.id.clone(), seed_entity);
        }
        let (related_entities, related_relations) = get_related_entities(
            store,
            project_id,
            seed_id,
            RelatedEntitiesOptions {
                depth,
                ..Default::default()
            },
        )?;
        for entity in related_entities {
            entities.insert(entity.id.clone(), entity);
        }
        relations.extend(related_relations);
    }

    Ok((entities.into_values().collect(), relations))
}

fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(|w| w.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::{write_file, TmpRepo};

    fn seed_chain_graph(projects_root: &Path) {
        let dir = projects_root.join("acme-app/graph");
        write_file(
            &dir.join("entities/a.yaml"),
            "id: a\ntype: component\nname: A\n",
        );
        write_file(
            &dir.join("entities/b.yaml"),
            "id: b\ntype: component\nname: B\n",
        );
        write_file(
            &dir.join("entities/c.yaml"),
            "id: c\ntype: component\nname: C\n",
        );
        write_file(
            &dir.join("relations/a-uses-b.yaml"),
            "from: a\nrelation: uses\nto: b\n",
        );
        write_file(
            &dir.join("relations/b-uses-c.yaml"),
            "from: b\nrelation: uses\nto: c\n",
        );
    }

    #[test]
    fn traverses_related_entities_up_to_a_given_depth() {
        let repo = TmpRepo::new();
        seed_chain_graph(&repo.projects_root);
        let store = GraphStore::new(&repo.projects_root);

        let (entities, _) = get_related_entities(
            &store,
            "acme-app",
            "a",
            RelatedEntitiesOptions {
                depth: 1,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            entities.into_iter().map(|e| e.id).collect::<Vec<_>>(),
            vec!["b".to_string()]
        );

        let (entities, _) = get_related_entities(
            &store,
            "acme-app",
            "a",
            RelatedEntitiesOptions {
                depth: 2,
                ..Default::default()
            },
        )
        .unwrap();
        let mut ids: Vec<String> = entities.into_iter().map(|e| e.id).collect();
        ids.sort();
        assert_eq!(ids, vec!["b".to_string(), "c".to_string()]);
    }

    #[test]
    fn finds_a_path_between_two_entities() {
        let repo = TmpRepo::new();
        seed_chain_graph(&repo.projects_root);
        let store = GraphStore::new(&repo.projects_root);

        let path = find_path(&store, "acme-app", "a", "c").unwrap().unwrap();
        let hops: Vec<String> = path
            .iter()
            .map(|r| format!("{}->{}", r.from, r.to))
            .collect();
        assert_eq!(hops, vec!["a->b".to_string(), "b->c".to_string()]);
    }

    #[test]
    fn returns_none_when_no_path_exists() {
        let repo = TmpRepo::new();
        write_file(
            &repo
                .projects_root
                .join("acme-app/graph/entities/isolated.yaml"),
            "id: isolated\ntype: component\nname: Isolated\n",
        );
        seed_chain_graph(&repo.projects_root);
        let store = GraphStore::new(&repo.projects_root);

        assert!(find_path(&store, "acme-app", "a", "isolated")
            .unwrap()
            .is_none());
    }

    #[test]
    fn searches_entities_by_id_name_and_type() {
        let repo = TmpRepo::new();
        seed_chain_graph(&repo.projects_root);
        let store = GraphStore::new(&repo.projects_root);

        let mut ids: Vec<String> = search_graph(&store, "acme-app", "component")
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect();
        ids.sort();
        assert_eq!(ids, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    }

    #[test]
    fn rejects_a_relation_whose_endpoints_are_both_unknown_entities() {
        let repo = TmpRepo::new();
        write_file(
            &repo.projects_root.join("acme-app/graph/entities/a.yaml"),
            "id: a\ntype: component\nname: A\n",
        );
        write_file(
            &repo.projects_root.join("acme-app/graph/relations/bad.yaml"),
            "from: ghost-one\nrelation: uses\nto: ghost-two\n",
        );

        let store = GraphStore::new(&repo.projects_root);
        assert!(matches!(
            store.list_relations("acme-app"),
            Err(NexusError::Validation { .. })
        ));
    }

    #[test]
    fn rejects_duplicate_entity_ids() {
        let repo = TmpRepo::new();
        write_file(
            &repo.projects_root.join("acme-app/graph/entities/one.yaml"),
            "id: dup\ntype: component\nname: One\n",
        );
        write_file(
            &repo.projects_root.join("acme-app/graph/entities/two.yaml"),
            "id: dup\ntype: component\nname: Two\n",
        );

        let store = GraphStore::new(&repo.projects_root);
        assert!(matches!(
            store.list_entities("acme-app"),
            Err(NexusError::VersionConflict(_))
        ));
    }
}
