//! The Semantic Knowledge Layer for Agents: term/alias resolution, kind
//! taxonomy, definitions and content/dependency freshness for
//! `SemanticContract` (`crates/nexus-semantic/src/contract.rs`).
//!
//! This module deliberately does not introduce a second store, a second
//! graph, or a second proposal workflow — it only adds the concepts the
//! existing `contracts/` + `graph/` + `proposal.rs` machinery was missing:
//! a term/alias identity, a kind taxonomy, a structured definition, and
//! deterministic content/dependency hashes for freshness.

use crate::confidence;
use crate::contract::{ContractBehaviors, SemanticContract};
use crate::model::Role;
use nexus_domain::types::BehaviorTransition;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Current version of the semantic schema these hashes/fields are computed
/// against. Bump when the shape of what `semantic_sha256` covers changes.
pub const SCHEMA_VERSION: &str = "1";

/// What kind of concept a knowledge entry names. Closed vocabulary plus a
/// `Custom` escape hatch, mirroring `Role`'s `From<String>` pattern — never
/// hard-code a project-specific concept into this enum, use `Custom` instead.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum ConceptKind {
    Concept,
    Component,
    Behavior,
    Requirement,
    Interaction,
    Policy,
    Skill,
    DesignToken,
    Api,
    DataModel,
    Workflow,
    Actor,
    Domain,
    Feature,
    Artifact,
    Custom(String),
}

impl ConceptKind {
    pub fn as_str(&self) -> &str {
        match self {
            ConceptKind::Concept => "concept",
            ConceptKind::Component => "component",
            ConceptKind::Behavior => "behavior",
            ConceptKind::Requirement => "requirement",
            ConceptKind::Interaction => "interaction",
            ConceptKind::Policy => "policy",
            ConceptKind::Skill => "skill",
            ConceptKind::DesignToken => "design-token",
            ConceptKind::Api => "api",
            ConceptKind::DataModel => "data-model",
            ConceptKind::Workflow => "workflow",
            ConceptKind::Actor => "actor",
            ConceptKind::Domain => "domain",
            ConceptKind::Feature => "feature",
            ConceptKind::Artifact => "artifact",
            ConceptKind::Custom(s) => s,
        }
    }
}

impl From<String> for ConceptKind {
    fn from(s: String) -> Self {
        match s.to_lowercase().replace(['_', ' '], "-").as_str() {
            "concept" => ConceptKind::Concept,
            "component" => ConceptKind::Component,
            "behavior" | "behaviour" => ConceptKind::Behavior,
            "requirement" => ConceptKind::Requirement,
            "interaction" => ConceptKind::Interaction,
            "policy" => ConceptKind::Policy,
            "skill" => ConceptKind::Skill,
            "design-token" => ConceptKind::DesignToken,
            "api" => ConceptKind::Api,
            "data-model" => ConceptKind::DataModel,
            "workflow" => ConceptKind::Workflow,
            "actor" => ConceptKind::Actor,
            "domain" => ConceptKind::Domain,
            "feature" => ConceptKind::Feature,
            "artifact" => ConceptKind::Artifact,
            _ => ConceptKind::Custom(s),
        }
    }
}

impl From<&str> for ConceptKind {
    fn from(s: &str) -> Self {
        ConceptKind::from(s.to_string())
    }
}

impl From<ConceptKind> for String {
    fn from(k: ConceptKind) -> Self {
        k.as_str().to_string()
    }
}

/// A human- and machine-readable definition. `short` is required; `semantic`
/// is a longer, precise statement of meaning for agents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Definition {
    pub short: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic: Option<String>,
}

/// Content and dependency identity of a knowledge entry, used to compute
/// `Freshness` without re-parsing anything the caller hasn't already re-read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentIdentity {
    pub content_sha256: String,
    pub semantic_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parser_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parser_version: Option<String>,
    #[serde(default = "default_schema_version")]
    pub semantic_schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_fingerprint: Option<String>,
}

fn default_schema_version() -> String {
    SCHEMA_VERSION.to_string()
}

/// Whether a knowledge entry is still accurate relative to its own source
/// file and its dependencies, computed on demand — never cached, since the
/// point is to catch drift the entry's own file can't know about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Freshness {
    Fresh,
    StaleContent,
    StaleDependency,
    StaleSchema,
    /// `identity.parser_id`/`parser_version` were recorded and no longer
    /// match the parser that would extract this entry now.
    StaleParser,
    /// No `identity` was ever recorded for this entry.
    Unknown,
}

/// Sha256 of the raw source bytes (hex-encoded).
pub fn content_sha256(raw: &str) -> String {
    hex(Sha256::digest(raw.as_bytes()))
}

/// Sha256 of the semantically-relevant subset of a contract (term, kind,
/// definition, behaviors, constraints, requirements) — so a formatting-only
/// edit to the YAML file doesn't count as a semantic change.
pub fn semantic_sha256(contract: &SemanticContract) -> String {
    let mut canonical = String::new();
    canonical.push_str(contract.term());
    canonical.push('\n');
    canonical.push_str(contract.kind.as_ref().map(ConceptKind::as_str).unwrap_or(""));
    canonical.push('\n');
    if let Some(def) = &contract.definition {
        canonical.push_str(&def.short);
        canonical.push('\n');
        canonical.push_str(def.semantic.as_deref().unwrap_or(""));
        canonical.push('\n');
    }
    for state in &contract.behaviors.states {
        canonical.push_str(state);
        canonical.push('\n');
    }
    for t in &contract.behaviors.transitions {
        canonical.push_str(&format!("{}--{}->{}\n", t.from, t.event, t.to));
    }
    for c in &contract.constraints {
        canonical.push_str(c);
        canonical.push('\n');
    }
    for r in &contract.requirements {
        canonical.push_str(r);
        canonical.push('\n');
    }
    hex(Sha256::digest(canonical.as_bytes()))
}

/// Deterministic fingerprint of a set of dependency ids: sha256 of the
/// sorted, newline-joined ids. Order-independent by construction.
pub fn dependency_fingerprint(related_ids: &[String]) -> String {
    let mut sorted: Vec<&str> = related_ids.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    sorted.dedup();
    hex(Sha256::digest(sorted.join("\n").as_bytes()))
}

fn hex(digest: impl AsRef<[u8]>) -> String {
    digest.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// Compares a contract's stored `identity` against freshly computed values.
/// `current_raw` is the entry's source file re-read by the caller;
/// `current_related_ids` are the subject's currently related graph entity
/// ids (from `nexus_domain::graph::get_related_entities`); `current_parser`
/// is the `(id, version)` of the parser that would extract this entry now,
/// when the caller has one to compare against (`None` skips the check, same
/// as an unset `dependency_fingerprint`).
pub fn compute_freshness(
    contract: &SemanticContract,
    current_raw: &str,
    current_related_ids: &[String],
    current_parser: Option<(&str, &str)>,
) -> Freshness {
    let Some(identity) = &contract.identity else {
        return Freshness::Unknown;
    };
    if identity.semantic_schema_version != SCHEMA_VERSION {
        return Freshness::StaleSchema;
    }
    if let (Some(id), Some(version)) = (&identity.parser_id, &identity.parser_version) {
        if let Some((current_id, current_version)) = current_parser {
            if id != current_id || version != current_version {
                return Freshness::StaleParser;
            }
        }
    }
    if content_sha256(current_raw) != identity.content_sha256 {
        return Freshness::StaleContent;
    }
    if let Some(expected) = &identity.dependency_fingerprint {
        if dependency_fingerprint(current_related_ids) != *expected {
            return Freshness::StaleDependency;
        }
    }
    Freshness::Fresh
}

/// Resolves free text to a canonical entry: exact `id`, then exact `term`
/// (case-insensitive), then an alias (case-insensitive). This is identity
/// resolution, not task-relevance matching — `nexus_domain::skill::resolve_skills`
/// solves that different problem and is untouched.
pub fn resolve_term<'a>(contracts: &'a [SemanticContract], text: &str) -> Option<&'a SemanticContract> {
    let needle = text.trim();
    if needle.is_empty() {
        return None;
    }
    if let Some(c) = contracts.iter().find(|c| c.id == needle) {
        return Some(c);
    }
    let lower = needle.to_lowercase();
    if let Some(c) = contracts.iter().find(|c| c.term().to_lowercase() == lower) {
        return Some(c);
    }
    contracts
        .iter()
        .find(|c| c.aliases.iter().any(|a| a.to_lowercase() == lower))
}

/// Case-insensitive substring search over id/term/display name/definition/aliases.
pub fn search<'a>(contracts: &'a [SemanticContract], query: &str) -> Vec<&'a SemanticContract> {
    let needle = query.to_lowercase();
    contracts
        .iter()
        .filter(|c| {
            c.id.to_lowercase().contains(&needle)
                || c.term().to_lowercase().contains(&needle)
                || c.display_name().to_lowercase().contains(&needle)
                || c.definition
                    .as_ref()
                    .map(|d| d.short.to_lowercase().contains(&needle))
                    .unwrap_or(false)
                || c.aliases.iter().any(|a| a.to_lowercase().contains(&needle))
        })
        .collect()
}

/// Direct-input constructor for a candidate knowledge entry — the narrow,
/// deterministic "semantic extraction integration": a human or an agent
/// (e.g. after calling the `inspect` MCP tool and reading the resulting
/// `SemanticModel`) distills a term's definition/behavior/constraints and
/// calls this; nothing here guesses at what belongs in a contract.
#[derive(Debug, Clone)]
pub struct ContractInput {
    pub id: String,
    pub name: Option<String>,
    pub role: Role,
    pub subject: Option<String>,
    pub term: Option<String>,
    pub kind: Option<ConceptKind>,
    pub definition: Option<Definition>,
    pub aliases: Vec<String>,
    pub intent: Vec<String>,
    pub requirements: Vec<String>,
    pub behavior_states: Vec<String>,
    pub behavior_transitions: Vec<BehaviorTransition>,
    pub constraints: Vec<String>,
}

impl ContractInput {
    /// A bare entry with just an id, term and role — everything else empty,
    /// meant to be filled in with struct-update syntax by callers.
    pub fn new(id: impl Into<String>, term: impl Into<String>, role: Role) -> Self {
        Self {
            id: id.into(),
            name: None,
            role,
            subject: None,
            term: Some(term.into()),
            kind: None,
            definition: None,
            aliases: Vec::new(),
            intent: Vec::new(),
            requirements: Vec::new(),
            behavior_states: Vec::new(),
            behavior_transitions: Vec::new(),
            constraints: Vec::new(),
        }
    }
}

pub fn build_contract(input: ContractInput) -> SemanticContract {
    SemanticContract {
        id: input.id,
        name: input.name,
        role: input.role,
        subject: input.subject,
        intent: input.intent,
        requirements: input.requirements,
        behaviors: ContractBehaviors {
            states: input.behavior_states,
            transitions: input.behavior_transitions,
        },
        constraints: input.constraints,
        provenance: None,
        term: input.term,
        kind: input.kind,
        definition: input.definition,
        aliases: input.aliases,
        confidence: Some(confidence::candidate("proposed via propose_knowledge_entry")),
        identity: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::SemanticContract;

    fn contract(term: &str, aliases: &[&str]) -> SemanticContract {
        build_contract(ContractInput {
            aliases: aliases.iter().map(|s| s.to_string()).collect(),
            definition: Some(Definition {
                short: format!("{term} definition"),
                semantic: None,
            }),
            ..ContractInput::new(term.to_lowercase(), term, Role::from("concept"))
        })
    }

    #[test]
    fn resolve_term_prefers_id_over_term_over_alias() {
        let entries = vec![contract("CheckoutButton", &["checkout submit", "submit checkout"])];
        assert_eq!(resolve_term(&entries, "checkoutbutton").unwrap().term(), "CheckoutButton");
        assert_eq!(resolve_term(&entries, "CheckoutButton").unwrap().term(), "CheckoutButton");
        assert_eq!(resolve_term(&entries, "submit checkout").unwrap().term(), "CheckoutButton");
        assert!(resolve_term(&entries, "nope").is_none());
        assert!(resolve_term(&entries, "  ").is_none());
    }

    #[test]
    fn search_matches_term_definition_and_aliases() {
        let entries = vec![contract("CheckoutButton", &["submit checkout"])];
        assert_eq!(search(&entries, "checkout").len(), 1);
        assert_eq!(search(&entries, "definition").len(), 1);
        assert_eq!(search(&entries, "submit").len(), 1);
        assert!(search(&entries, "nope").is_empty());
    }

    #[test]
    fn content_hash_is_deterministic_and_change_sensitive() {
        assert_eq!(content_sha256("abc"), content_sha256("abc"));
        assert_ne!(content_sha256("abc"), content_sha256("abd"));
    }

    #[test]
    fn semantic_hash_ignores_nothing_but_the_semantic_fields() {
        let a = contract("CheckoutButton", &[]);
        let mut b = a.clone();
        b.aliases.push("extra alias, not part of the semantic hash".into());
        assert_eq!(semantic_sha256(&a), semantic_sha256(&b));

        let mut c = a.clone();
        c.constraints.push("preserve input on error".into());
        assert_ne!(semantic_sha256(&a), semantic_sha256(&c));
    }

    #[test]
    fn dependency_fingerprint_is_order_independent() {
        let a = dependency_fingerprint(&["b".into(), "a".into()]);
        let b = dependency_fingerprint(&["a".into(), "b".into()]);
        assert_eq!(a, b);
        assert_ne!(a, dependency_fingerprint(&["a".into()]));
    }

    #[test]
    fn freshness_transitions() {
        let mut entry = contract("CheckoutButton", &[]);
        assert_eq!(compute_freshness(&entry, "raw", &[], None), Freshness::Unknown);

        entry.identity = Some(ContentIdentity {
            content_sha256: content_sha256("raw"),
            semantic_sha256: semantic_sha256(&entry),
            parser_id: None,
            parser_version: None,
            semantic_schema_version: SCHEMA_VERSION.to_string(),
            dependency_fingerprint: Some(dependency_fingerprint(&["a".into()])),
        });
        assert_eq!(compute_freshness(&entry, "raw", &["a".into()], None), Freshness::Fresh);
        assert_eq!(
            compute_freshness(&entry, "edited", &["a".into()], None),
            Freshness::StaleContent
        );
        assert_eq!(
            compute_freshness(&entry, "raw", &["b".into()], None),
            Freshness::StaleDependency
        );

        entry.identity.as_mut().unwrap().semantic_schema_version = "0".to_string();
        assert_eq!(
            compute_freshness(&entry, "raw", &["a".into()], None),
            Freshness::StaleSchema
        );
    }

    #[test]
    fn stale_parser_is_detected_before_content_is_even_checked() {
        let mut entry = contract("CheckoutButton", &[]);
        entry.identity = Some(ContentIdentity {
            content_sha256: content_sha256("raw"),
            semantic_sha256: semantic_sha256(&entry),
            parser_id: Some("typescript-component-parser".to_string()),
            parser_version: Some("1.4.2".to_string()),
            semantic_schema_version: SCHEMA_VERSION.to_string(),
            dependency_fingerprint: None,
        });

        // Same parser identity: content is still checked normally.
        assert_eq!(
            compute_freshness(&entry, "raw", &[], Some(("typescript-component-parser", "1.4.2"))),
            Freshness::Fresh
        );
        // No current parser given: the check is skipped (same as an unset dependency_fingerprint).
        assert_eq!(compute_freshness(&entry, "raw", &[], None), Freshness::Fresh);
        // Parser upgraded: stale even though the file content itself is unchanged.
        assert_eq!(
            compute_freshness(&entry, "raw", &[], Some(("typescript-component-parser", "1.5.0"))),
            Freshness::StaleParser
        );
        // Different parser entirely: also stale.
        assert_eq!(
            compute_freshness(&entry, "raw", &[], Some(("other-parser", "1.4.2"))),
            Freshness::StaleParser
        );
    }
}
