//! Proposals: the only path from extracted/inferred knowledge into the
//! canonical graph (`graph/entities`, `graph/relations`).
//!
//! `build` compares a model with the canonical graph and lists what is new.
//! Nothing is written until `review` accepts it. Accepting writes the
//! statements with their original evidence, confidence and provenance: an
//! inferred relation becomes part of the canonical graph *as inferred*.

use crate::contract::SemanticContract;
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
    /// A Semantic Knowledge Layer entry, written to `contracts/<id>.yaml` on
    /// acceptance — never to `graph/`, `contracts/` is its own canonical home.
    /// Boxed: `SemanticContract` is much larger than the other variants.
    AddContract {
        contract: Box<SemanticContract>,
        #[serde(flatten)]
        basis: Basis,
    },
}

impl ProposedChange {
    pub fn basis(&self) -> &Basis {
        match self {
            ProposedChange::AddEntity { basis, .. }
            | ProposedChange::AddRelation { basis, .. }
            | ProposedChange::AddContract { basis, .. } => basis,
        }
    }

    pub fn key(&self) -> String {
        match self {
            ProposedChange::AddEntity { id, .. } => format!("entity:{id}"),
            ProposedChange::AddRelation {
                from, relation, to, ..
            } => {
                format!("relation:{from}|{}|{to}", relation.as_str())
            }
            ProposedChange::AddContract { contract, .. } => format!("contract:{}", contract.id),
        }
    }
}

/// Wraps a single already-built candidate contract into a pending proposal.
/// Unlike `build`, this never diffs against the canonical graph — the caller
/// (a human or an agent, via the `propose_knowledge_entry` MCP tool) decided
/// exactly what the contract should contain.
pub fn from_contract(project_id: &str, scope: String, contract: SemanticContract, basis: Basis) -> Proposal {
    let change = ProposedChange::AddContract {
        contract: Box::new(contract),
        basis: basis.clone(),
    };
    let created_at = now();
    let key = change.key();
    Proposal {
        id: format!("prop-{created_at}-{}{}", short_hash(&key, 4), crate::text::nonce()),
        project_id: ProjectId::from(project_id),
        title: format!("Knowledge entry: {key}"),
        scope,
        confidence: basis.confidence,
        provenance: basis.provenance,
        changes: vec![change],
        status: ProposalStatus::Pending,
        created_at,
        reviewed_at: None,
        reviewer: None,
        note: None,
        superseded_by: None,
        written: Vec::new(),
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
    "file:",
    "dir:",
    "doc:",
    "module:",
    "package:",
    "skill:",
    "behavior:",
    "contract:",
    "policy:",
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
    let entity_ids: BTreeSet<String> = input
        .canonical_entities
        .iter()
        .map(|e| e.id.clone())
        .collect();
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
        if to.starts_with("commit:")
            && d.relation == RelationKind::ChangedBy
            && endpoint_ok(&from, &proposed)
        {
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
        id: format!(
            "prop-{created_at}-{}{}",
            short_hash(&keys.join(","), 4),
            crate::text::nonce()
        ),
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
        if old.status == ProposalStatus::Pending
            && old.scope == proposal.scope
            && old.id != proposal.id
        {
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
    let text =
        serde_yaml::to_string(value).map_err(|e| SemanticError::InvalidInput(e.to_string()))?;
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
                ProposedChange::AddEntity {
                    id,
                    name,
                    kind,
                    description,
                    artifact,
                    basis,
                } => (
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
                ProposedChange::AddRelation {
                    from,
                    relation,
                    to,
                    basis,
                } => (
                    graph.join("relations").join(format!(
                        "{}-{}-{}.yaml",
                        slug(from),
                        relation.as_str(),
                        slug(to)
                    )),
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
                ProposedChange::AddContract { contract, basis } => (
                    project_dir
                        .join("contracts")
                        .join(format!("{}.yaml", slug(&contract.id))),
                    serde_json::json!({
                        "id": contract.id,
                        "name": contract.name,
                        "role": contract.role,
                        "subject": contract.subject,
                        "term": contract.term,
                        "kind": contract.kind,
                        "definition": contract.definition,
                        "aliases": contract.aliases,
                        "intent": contract.intent,
                        "requirements": contract.requirements,
                        "behaviors": contract.behaviors,
                        "constraints": contract.constraints,
                        "confidence": basis.confidence,
                        "identity": contract.identity,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::{build_contract, ContractInput, Definition};
    use tempfile::TempDir;

    #[test]
    fn contract_proposal_round_trips_through_submit_and_review() {
        let dir = TempDir::new().unwrap();
        let project_dir = dir.path().join("acme-app");
        std::fs::create_dir_all(&project_dir).unwrap();
        let store = SemanticStore::for_project(&project_dir);

        let contract = build_contract(ContractInput {
            definition: Some(Definition {
                short: "test".into(),
                semantic: None,
            }),
            ..ContractInput::new("checkout-cancel", "CheckoutCancel", Role::from("behavior"))
        });
        let basis = Basis::candidate(
            Provenance {
                source: "test".into(),
                line_start: None,
                line_end: None,
                commit: None,
                extraction: "test".into(),
                artifact: None,
            },
            "test",
        );
        let proposal = from_contract(
            "acme-app",
            "propose-knowledge:checkout-cancel".to_string(),
            contract,
            basis,
        );
        assert_eq!(proposal.status, ProposalStatus::Pending);

        submit(&store, &proposal).unwrap();

        let reviewed = review(
            &store,
            &project_dir,
            dir.path(),
            &proposal.id,
            Decision::Accept,
            "human-reviewer",
            None,
        )
        .unwrap();
        assert_eq!(reviewed.status, ProposalStatus::Accepted);
        assert_eq!(reviewed.written.len(), 1);

        let written = project_dir.join("contracts/checkout-cancel.yaml");
        assert!(written.exists());
        let text = std::fs::read_to_string(&written).unwrap();
        assert!(text.contains("CheckoutCancel"));

        // Reviewing again is rejected: only pending proposals can be reviewed.
        assert!(review(
            &store,
            &project_dir,
            dir.path(),
            &proposal.id,
            Decision::Accept,
            "human-reviewer",
            None,
        )
        .is_err());
    }

    #[test]
    fn rejecting_a_contract_proposal_writes_nothing() {
        let dir = TempDir::new().unwrap();
        let project_dir = dir.path().join("acme-app");
        std::fs::create_dir_all(&project_dir).unwrap();
        let store = SemanticStore::for_project(&project_dir);

        let contract = build_contract(ContractInput {
            definition: Some(Definition {
                short: "test".into(),
                semantic: None,
            }),
            ..ContractInput::new("checkout-cancel", "CheckoutCancel", Role::from("behavior"))
        });
        let basis = Basis::candidate(
            Provenance {
                source: "test".into(),
                line_start: None,
                line_end: None,
                commit: None,
                extraction: "test".into(),
                artifact: None,
            },
            "test",
        );
        let proposal = from_contract(
            "acme-app",
            "propose-knowledge:checkout-cancel".to_string(),
            contract,
            basis,
        );
        submit(&store, &proposal).unwrap();

        let reviewed = review(
            &store,
            &project_dir,
            dir.path(),
            &proposal.id,
            Decision::Reject,
            "human-reviewer",
            None,
        )
        .unwrap();
        assert_eq!(reviewed.status, ProposalStatus::Rejected);
        assert!(reviewed.written.is_empty());
        assert!(!project_dir.join("contracts/checkout-cancel.yaml").exists());
    }
}
