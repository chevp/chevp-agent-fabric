//! The consolidated semantic graph of a project: the canonical graph
//! (`graph/`), authored sources (skills, behaviors, contracts, policies) and
//! the extracted model, with per-node layout metrics (depth, cluster,
//! parent/children, centrality, distance). The metrics are plain data; any
//! renderer (the radial "semantic flower" explorer, or something else) can
//! use them.

use crate::diff::SemanticChange;
use crate::model::*;
use crate::proposal::{normalize_ref, Proposal, ProposalStatus, ProposedChange};
use nexus_domain::types::{GraphEntity, GraphRelation};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub cluster: String,
    /// Evidence level of the node itself.
    pub status: Evidence,
    /// In the canonical graph or an authored Git source.
    pub canonical: bool,
    pub confidence: f32,
    pub provenance: Vec<Provenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<ArtifactId>,
    pub depth: usize,
    pub distance: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub children: Vec<String>,
    pub centrality: f32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pending_proposals: Vec<String>,
    /// States/constraints/intent attached to this node, for detail views.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub facets: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub relation: String,
    pub status: Evidence,
    pub canonical: bool,
    pub confidence: f32,
    pub provenance: Vec<Provenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClusterInfo {
    pub id: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticGraph {
    pub project_id: String,
    pub project_name: String,
    pub root: String,
    pub generated_at: u64,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub clusters: Vec<ClusterInfo>,
    pub diff: Vec<SemanticChange>,
    pub model_confidence: Confidence,
}

pub fn cluster_of(kind: &str) -> &'static str {
    match kind {
        "project" => "core",
        "skill" => "skills",
        "policy" => "policies",
        "test" | "test-file" => "tests",
        "behavior-spec" => "behaviors",
        "contract" => "contracts",
        "token" | "design-system" | "variant" | "button" | "field" | "link" | "form" => "design",
        "component" => "features",
        "flow" | "domain" | "service" => "domains",
        "endpoint" | "api" => "apis",
        "code-file" | "module" | "package" | "workflow" => "code",
        "document" => "docs",
        "commit" => "history",
        "directory" | "file" => "files",
        _ => "other",
    }
}

fn kind_from_id(id: &str) -> &'static str {
    match id.split_once(':').map(|(p, _)| p) {
        Some("skill") => "skill",
        Some("behavior") => "behavior-spec",
        Some("contract") => "contract",
        Some("policy") => "policy",
        Some("module") => "module",
        Some("package") => "package",
        Some("token") => "token",
        Some("test") => "test",
        Some("doc") => "document",
        Some("file") => "file",
        Some("dir") => "directory",
        Some("commit") => "commit",
        Some("variant") => "variant",
        Some("endpoint") => "endpoint",
        _ => "unresolved",
    }
}

pub struct GraphInput<'a> {
    /// Provenance paths are made relative to this.
    pub repo_root: &'a std::path::Path,
    pub project_id: &'a str,
    pub project_name: &'a str,
    pub canonical_entities: &'a [GraphEntity],
    pub canonical_relations: &'a [GraphRelation],
    pub model: &'a SemanticModel,
    pub proposals: &'a [Proposal],
    pub diff: Vec<SemanticChange>,
    /// Hide `file:`/`dir:` structure nodes.
    pub include_structure: bool,
}

pub fn build(input: GraphInput<'_>) -> SemanticGraph {
    let root = format!("project:{}", input.project_id);
    let entity_ids: BTreeSet<String> = input
        .canonical_entities
        .iter()
        .map(|e| e.id.clone())
        .collect();
    let sources: BTreeSet<String> = input
        .model
        .entities
        .iter()
        .map(|e| e.id.clone())
        .filter(|id| {
            crate::validate::SOURCE_PREFIXES
                .iter()
                .any(|p| id.starts_with(p))
        })
        .collect();
    let norm = |id: &str| normalize_ref(id, &entity_ids, &sources);
    let structural = |id: &str| id.starts_with("file:") || id.starts_with("dir:");
    let keep = |id: &str| input.include_structure || !structural(id);

    let mut nodes: BTreeMap<String, GraphNode> = BTreeMap::new();
    let node =
        |id: &str, label: &str, kind: &str, status: Evidence, canonical: bool, confidence: f32| {
            GraphNode {
                id: id.to_string(),
                label: label.to_string(),
                kind: kind.to_string(),
                role: None,
                description: None,
                cluster: cluster_of(kind).to_string(),
                status,
                canonical,
                confidence,
                provenance: Vec::new(),
                artifact: None,
                depth: 0,
                distance: 0,
                parent: None,
                children: Vec::new(),
                centrality: 0.0,
                pending_proposals: Vec::new(),
                facets: BTreeMap::new(),
            }
        };

    nodes.insert(
        root.clone(),
        node(
            &root,
            input.project_name,
            "project",
            Evidence::Explicit,
            true,
            1.0,
        ),
    );

    for e in input.canonical_entities {
        let mut n = node(
            &e.id,
            &e.name,
            &e.kind,
            e.evidence,
            true,
            e.confidence.as_ref().map(|c| c.value).unwrap_or(1.0),
        );
        n.description = e
            .attributes
            .get("description")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        n.provenance = if e.provenance.is_empty() {
            vec![Provenance {
                source: crate::text::rel_path(input.repo_root, &e.source_path),
                line_start: None,
                line_end: None,
                commit: None,
                extraction: "GraphStore".into(),
                artifact: None,
            }]
        } else {
            e.provenance.clone()
        };
        n.artifact = e.artifact.clone().map(ArtifactId::from);
        nodes.insert(e.id.clone(), n);
    }

    for e in &input.model.entities {
        if e.id == root {
            if let Some(n) = nodes.get_mut(&root) {
                n.provenance = e.basis.provenance.clone();
                n.description = e.description.clone();
            }
            continue;
        }
        if !keep(&e.id) {
            continue;
        }
        let canonical = e.basis.evidence == Evidence::Explicit;
        let n = nodes.entry(e.id.clone()).or_insert_with(|| {
            node(
                &e.id,
                &e.name,
                &e.kind,
                e.basis.evidence,
                canonical,
                e.basis.confidence.value,
            )
        });
        n.role = n.role.clone().or(e.role.clone());
        n.description = n.description.clone().or(e.description.clone());
        n.artifact = n.artifact.clone().or(e.artifact.clone());
        for p in &e.basis.provenance {
            if !n.provenance.contains(p) {
                n.provenance.push(p.clone());
            }
        }
    }

    // Facets: model statements attached to their subject node.
    let mut facet = |subject: &str, key: &str, text: String| {
        let id = norm(subject);
        if let Some(n) = nodes.get_mut(&id) {
            n.facets.entry(key.to_string()).or_default().push(text);
        }
    };
    for s in &input.model.states {
        facet(&s.subject, "states", s.name.clone());
    }
    for b in &input.model.behaviors {
        facet(
            &b.subject,
            "transitions",
            format!("{} --{}--> {}", b.from, b.event, b.to),
        );
    }
    for c in &input.model.constraints {
        if let Some(s) = &c.subject {
            facet(
                s,
                "constraints",
                format!("[{:?}] {}", c.basis.evidence, c.statement).to_lowercase(),
            );
        }
    }
    for r in &input.model.requirements {
        if let Some(s) = &r.subject {
            facet(s, "requirements", r.statement.clone());
        }
    }

    let mut edges: BTreeMap<String, GraphEdge> = BTreeMap::new();
    let add_edge = |edges: &mut BTreeMap<String, GraphEdge>,
                    from: String,
                    relation: &str,
                    to: String,
                    status: Evidence,
                    canonical: bool,
                    confidence: f32,
                    provenance: Vec<Provenance>,
                    proposal: Option<String>| {
        let id = format!("{from}|{relation}|{to}");
        let e = edges.entry(id.clone()).or_insert(GraphEdge {
            id,
            from,
            to,
            relation: relation.to_string(),
            status,
            canonical,
            confidence,
            provenance: Vec::new(),
            proposal: proposal.clone(),
        });
        e.canonical |= canonical;
        if canonical && proposal.is_some() {
            e.proposal = proposal;
        }
        for p in provenance {
            if !e.provenance.contains(&p) {
                e.provenance.push(p);
            }
        }
    };
    for r in input.canonical_relations {
        let prov = if r.provenance.is_empty() {
            vec![Provenance {
                source: crate::text::rel_path(input.repo_root, &r.source_path),
                line_start: None,
                line_end: None,
                commit: None,
                extraction: "GraphStore".into(),
                artifact: None,
            }]
        } else {
            r.provenance.clone()
        };
        add_edge(
            &mut edges,
            norm(&r.from),
            r.relation.as_str(),
            norm(&r.to),
            r.evidence,
            true,
            r.confidence.as_ref().map(|c| c.value).unwrap_or(1.0),
            prov,
            r.proposal.clone(),
        );
    }
    for d in &input.model.dependencies {
        let (from, to) = (norm(&d.from), norm(&d.to));
        if !keep(&from) || !keep(&to) {
            continue;
        }
        add_edge(
            &mut edges,
            from,
            d.relation.as_str(),
            to,
            d.basis.evidence,
            d.basis.evidence == Evidence::Explicit,
            d.basis.confidence.value,
            d.basis.provenance.clone(),
            None,
        );
    }

    // Placeholder nodes for edge endpoints nothing else describes.
    for e in edges.values() {
        for (id, status, conf) in [
            (&e.from, e.status, e.confidence),
            (&e.to, e.status, e.confidence),
        ] {
            if !nodes.contains_key(id) {
                let kind = kind_from_id(id);
                let label = id.split_once(':').map(|(_, l)| l).unwrap_or(id);
                nodes.insert(id.clone(), node(id, label, kind, status, false, conf));
            }
        }
    }

    // Pending proposals.
    for p in input
        .proposals
        .iter()
        .filter(|p| p.status == ProposalStatus::Pending)
    {
        for c in &p.changes {
            match c {
                ProposedChange::AddEntity { id, .. } => {
                    if let Some(n) = nodes.get_mut(id) {
                        n.pending_proposals.push(p.id.clone());
                    }
                }
                ProposedChange::AddRelation {
                    from, relation, to, ..
                } => {
                    if let Some(e) = edges.get_mut(&format!("{from}|{}|{to}", relation.as_str())) {
                        e.proposal = Some(p.id.clone());
                    }
                }
                // Contracts live in `contracts/`, not the graph; nothing to annotate here.
                ProposedChange::AddContract { .. } => {}
            }
        }
    }

    // Seeds: canonical entities and authored sources hang off the root.
    let mut adjacency: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for e in edges.values() {
        adjacency.entry(&e.from).or_default().insert(&e.to);
        adjacency.entry(&e.to).or_default().insert(&e.from);
    }
    let seeds: Vec<String> = nodes
        .values()
        .filter(|n| {
            n.id != root
                && (entity_ids.contains(&n.id) || sources.contains(&n.id) || n.id == "dir:.")
        })
        .map(|n| n.id.clone())
        .collect();
    let mut dist: BTreeMap<String, (usize, Option<String>)> = BTreeMap::new();
    dist.insert(root.clone(), (0, None));
    let mut queue: VecDeque<String> = VecDeque::new();
    for s in seeds {
        dist.insert(s.clone(), (1, Some(root.clone())));
        queue.push_back(s);
    }
    while let Some(cur) = queue.pop_front() {
        let d = dist[&cur].0;
        let neighbours: Vec<String> = adjacency
            .get(cur.as_str())
            .map(|s| s.iter().map(|x| x.to_string()).collect())
            .unwrap_or_default();
        for next in neighbours {
            if !dist.contains_key(&next) {
                dist.insert(next.clone(), (d + 1, Some(cur.clone())));
                queue.push_back(next);
            }
        }
    }
    let max = dist.values().map(|(d, _)| *d).max().unwrap_or(0);
    // Unreachable nodes: attach to the root one ring outside everything else.
    let unreachable: Vec<String> = nodes
        .keys()
        .filter(|k| !dist.contains_key(*k))
        .cloned()
        .collect();
    for id in unreachable {
        dist.insert(id, (max + 1, Some(root.clone())));
    }

    let n_total = nodes.len().max(2) as f32;
    let degree = |id: &str| adjacency.get(id).map(|s| s.len()).unwrap_or(0) as f32;
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (id, (_, parent)) in &dist {
        if let Some(p) = parent {
            children.entry(p.clone()).or_default().push(id.clone());
        }
    }
    for n in nodes.values_mut() {
        let (d, parent) = dist[&n.id].clone();
        n.depth = d;
        n.distance = d;
        n.parent = parent;
        n.children = children.remove(&n.id).unwrap_or_default();
        n.centrality = crate::confidence::round(degree(&n.id) / (n_total - 1.0));
        n.pending_proposals.sort();
        n.pending_proposals.dedup();
    }

    let mut clusters: BTreeMap<String, usize> = BTreeMap::new();
    for n in nodes.values() {
        *clusters.entry(n.cluster.clone()).or_default() += 1;
    }
    SemanticGraph {
        project_id: input.project_id.to_string(),
        project_name: input.project_name.to_string(),
        root,
        generated_at: crate::text::now(),
        nodes: nodes.into_values().collect(),
        edges: edges.into_values().collect(),
        clusters: clusters
            .into_iter()
            .map(|(id, count)| ClusterInfo { id, count })
            .collect(),
        diff: input.diff,
        model_confidence: input.model.confidence.clone(),
    }
}
