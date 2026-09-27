//! Proposals: the only path from extracted/inferred knowledge into the
//! canonical graph (`graph/entities`, `graph/relations`).
//!
//! `build` compares a model with the canonical graph and lists what is new.
//! Nothing is written until `review` accepts it. Accepting writes the
//! statements with their original evidence, confidence and provenance: an
//! inferred relation becomes part of the canonical graph *as inferred*.

use crate::error::{SemanticError, SemanticResult};
use crate::model::*;
use crate::store::SemanticStore;
use crate::text::{now, short_hash, slug};
use nexus_domain::types::{GraphEntity, GraphRelation, RelationKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProposalStatus {
    Pending,
    Accepted,
    Rejected,
    Superseded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum ProposedChange {
    AddEntity {
        id: String,
        name: String,
        kind: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact: Option<ArtifactId>,
        #[serde(flatten)]
        basis: Basis,
    },
    AddRelation {
        from: String,
        relation: RelationKind,
        to: String,
        #[serde(flatten)]
        basis: Basis,
    },
}

impl ProposedChange {
    pub fn basis(&self) -> &Basis {
        match self {
            ProposedChange::AddEntity { basis, .. } | ProposedChange::AddRelation { basis, .. } => {
                basis
            }
        }
    }

    pub fn key(&self) -> String {
        match self {
            ProposedChange::AddEntity { id, .. } => format!("entity:{id}"),
            ProposedChange::AddRelation { from, relation, to, .. } => {
                format!("relation:{from}|{}|{to}", relation.as_str())
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    pub id: String,
    pub project_id: ProjectId,
    pub title: String,
    /// What produced it (`ingest:<path>`, `propose:<artifact ids>`); a newer
    /// pending proposal with the same scope supersedes older ones.
    pub scope: String,
    pub changes: Vec<ProposedChange>,
    pub confidence: Confidence,
    pub provenance: Vec<Provenance>,
    pub status: ProposalStatus,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    /// Files written on acceptance.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub written: Vec<String>,
}

/// Entities that stay out of the canonical graph: structure (files,
/// directories, modules) and authored sources that are canonical as files.
pub const NON_GRAPH_PREFIXES: &[&str] = &[
    "file:", "dir:", "doc:", "module:", "package:", "skill:", "behavior:", "contract:", "policy:",
    "project:",
];

pub fn graphable(id: &str) -> bool {
    !NON_GRAPH_PREFIXES.iter().any(|p| id.starts_with(p))
}

/// Maps a bare reference to an authored source id (`checkout-ux` ->
/// `skill:checkout-ux`) when that is what it denotes.
pub fn normalize_ref(id: &str, entities: &BTreeSet<String>, sources: &BTreeSet<String>) -> String {
    if entities.contains(id) || id.contains(':') {
        return id.to_string();
    }
    crate::validate::SOURCE_PREFIXES
        .iter()
        .map(|p| format!("{p}{id}"))
        .find(|p| sources.contains(p))
        .unwrap_or_else(|| id.to_string())
}

pub struct BuildInput<'a> {
    pub project_id: &'a str,
    pub title: String,
    pub scope: String,
    pub model: &'a SemanticModel,
    pub canonical_entities: &'a [GraphEntity],
    pub canonical_relations: &'a [GraphRelation],
    /// Ids of authored sources (`skill:…`, `behavior:…`, `contract:…`, `policy:…`).
    pub sources: &'a BTreeSet<String>,
}

pub fn build(input: BuildInput<'_>) -> Proposal {
    let entity_ids: BTreeSet<String> = input.canonical_entities.iter().map(|e| e.id.clone()).collect();
    let norm = |id: &str| normalize_ref(id, &entity_ids, input.sources);
    let canonical_edges: BTreeSet<(String, RelationKind, String)> = input
        .canonical_relations
        .iter()
        .map(|r| (norm(&r.from), r.relation, norm(&r.to)))
        .collect();

    let mut proposed: BTreeSet<String> = input
        .model
        .entities
        .iter()
        .filter(|e| graphable(&e.id) && !e.id.starts_with("commit:"))
        .filter(|e| !entity_ids.contains(&e.id) && e.basis.evidence != Evidence::Unknown)
        .map(|e| e.id.clone())
        .collect();

    let endpoint_ok = |id: &str, proposed: &BTreeSet<String>| {
        entity_ids.contains(id) || proposed.contains(id) || input.sources.contains(id)
    };
    let mut relations = Vec::new();
    for d in &input.model.dependencies {
        if d.basis.evidence == Evidence::Unknown {
            continue;
        }
        let (from, to) = (norm(&d.from), norm(&d.to));
        if !graphable(&from) && !graphable(&to) {
            continue;
        }
        if canonical_edges.contains(&(from.clone(), d.relation, to.clone())) {
            continue;
        }
        // Commits enter only as endpoints of changed-by relations.
        if to.starts_with("commit:") && d.relation == RelationKind::ChangedBy && endpoint_ok(&from, &proposed) {
            proposed.insert(to.clone());
        }
        if endpoint_ok(&from, &proposed) && endpoint_ok(&to, &proposed) {
            relations.push(ProposedChange::AddRelation {
                from,
                relation: d.relation,
                to,
                basis: d.basis.clone(),
            });
        }
    }

    let mut changes: Vec<ProposedChange> = input
        .model
        .entities
        .iter()
        .filter(|e| proposed.contains(&e.id))
        .map(|e| ProposedChange::AddEntity {
            id: e.id.clone(),
            name: e.name.clone(),
            kind: e.kind.clone(),
            description: e.description.clone(),
            artifact: e.artifact.clone(),
            basis: e.basis.clone(),
        })
        .collect();
    changes.extend(relations);
    changes.sort_by_key(|c| c.key());
    changes.dedup_by_key(|c| c.key());

    let bases: Vec<&Basis> = changes.iter().map(|c| c.basis()).collect();
    let mut provenance: Vec<Provenance> = bases.iter().flat_map(|b| b.provenance.clone()).collect();
    provenance.sort();
    provenance.dedup();
    let created_at = now();
    let keys: Vec<String> = changes.iter().map(|c| c.key()).collect();
    Proposal {
        id: format!("prop-{created_at}-{}{}", short_hash(&keys.join(","), 4), crate::text::nonce()),
        project_id: ProjectId::from(input.project_id),
        title: input.title,
        scope: input.scope,
        confidence: crate::confidence::aggregate(&bases),
        provenance,
        changes,
        status: ProposalStatus::Pending,
        created_at,
        reviewed_at: None,
        reviewer: None,
        note: None,
        superseded_by: None,
        written: Vec::new(),
    }
}

/// Persists a new pending proposal and supersedes older pending ones of
/// the same scope.
pub fn submit(store: &SemanticStore, proposal: &Proposal) -> SemanticResult<Vec<String>> {
    let mut superseded = Vec::new();
    for mut old in store.proposals()? {
        if old.status == ProposalStatus::Pending && old.scope == proposal.scope && old.id != proposal.id {
            old.status = ProposalStatus::Superseded;
            old.superseded_by = Some(proposal.id.clone());
            store.save_proposal(&old)?;
            superseded.push(old.id);
        }
    }
    store.save_proposal(proposal)?;
    Ok(superseded)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Accept,
    Reject,
}

fn yaml_write(path: &Path, value: &Value) -> SemanticResult<bool> {
    if path.exists() {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| SemanticError::io(parent, e))?;
    }
    let text = serde_yaml::to_string(value).map_err(|e| SemanticError::InvalidInput(e.to_string()))?;
    fs::write(path, text).map_err(|e| SemanticError::io(path, e))?;
    Ok(true)
}

/// Accepts (writes to `graph/`) or rejects a pending proposal.
pub fn review(
    store: &SemanticStore,
    project_dir: &Path,
    repo_root: &Path,
    proposal_id: &str,
    decision: Decision,
    reviewer: &str,
    note: Option<String>,
) -> SemanticResult<Proposal> {
    let mut p = store.proposal(proposal_id)?;
    if p.status != ProposalStatus::Pending {
        return Err(SemanticError::InvalidInput(format!(
            "proposal \"{}\" is {:?}; only pending proposals can be reviewed",
            p.id, p.status
        )));
    }
    if decision == Decision::Accept {
        let graph = project_dir.join("graph");
        for change in &p.changes {
            let (path, value) = match change {
                ProposedChange::AddEntity { id, name, kind, description, artifact, basis } => (
                    graph.join("entities").join(format!("{}.yaml", slug(id))),
                    serde_json::json!({
                        "id": id,
                        "type": kind,
                        "name": name,
                        "description": description,
                        "artifact": artifact,
                        "evidence": basis.evidence,
                        "confidence": basis.confidence,
                        "provenance": basis.provenance,
                        "proposal": p.id,
                    }),
                ),
                ProposedChange::AddRelation { from, relation, to, basis } => (
                    graph
                        .join("relations")
                        .join(format!("{}-{}-{}.yaml", slug(from), relation.as_str(), slug(to))),
                    serde_json::json!({
                        "from": from,
                        "relation": relation,
                        "to": to,
                        "evidence": basis.evidence,
                        "confidence": basis.confidence,
                        "provenance": basis.provenance,
                        "proposal": p.id,
                    }),
                ),
            };
            let mut value = value;
            if let Value::Object(o) = &mut value {
                o.retain(|_, v| !v.is_null());
            }
            if yaml_write(&path, &value)? {
                p.written.push(crate::text::rel_path(repo_root, &path));
            }
        }
    }
    p.status = match decision {
        Decision::Accept => ProposalStatus::Accepted,
        Decision::Reject => ProposalStatus::Rejected,
    };
    p.reviewed_at = Some(now());
    p.reviewer = Some(reviewer.to_string());
    p.note = note;
    store.save_proposal(&p)?;
    Ok(p)
}
