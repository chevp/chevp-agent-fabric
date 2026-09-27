//! Structural and epistemic checks on a SemanticModel, a Translation, or a
//! model against a SemanticContract.

use crate::confidence;
use crate::contract::SemanticContract;
use crate::error::SemanticResult;
use crate::model::*;
use crate::text::{similarity, slug};
use crate::translate::Translation;
use async_trait::async_trait;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidationIssue {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: usize,
    pub warnings: usize,
    pub issues: Vec<ValidationIssue>,
}

impl ValidationResult {
    pub fn from_issues(mut issues: Vec<ValidationIssue>) -> Self {
        issues.sort_by(|a, b| (a.severity, a.code, &a.item).cmp(&(b.severity, b.code, &b.item)));
        let errors = issues.iter().filter(|i| i.severity == Severity::Error).count();
        let warnings = issues.iter().filter(|i| i.severity == Severity::Warning).count();
        Self {
            valid: errors == 0,
            errors,
            warnings,
            issues,
        }
    }

    pub fn has(&self, code: &str) -> bool {
        self.issues.iter().any(|i| i.code == code)
    }
}

#[async_trait]
pub trait Validator: Send + Sync {
    async fn validate(&self, model: &SemanticModel) -> SemanticResult<ValidationResult>;
}

/// Deterministic model validator. `known_entities` are ids that exist
/// outside the model (canonical graph, skills, ...), so references to them
/// are not reported as unknown.
#[derive(Default)]
pub struct ModelValidator {
    pub known_entities: BTreeSet<String>,
}

/// Prefixes under which an authored source may be referenced by bare id
/// (the canonical graph writes `governed-by: checkout-ux`, not `skill:checkout-ux`).
pub const SOURCE_PREFIXES: &[&str] = &["skill:", "behavior:", "contract:", "policy:"];

pub fn is_known(id: &str, known: &BTreeSet<String>) -> bool {
    known.contains(id)
        || SOURCE_PREFIXES
            .iter()
            .any(|p| known.contains(&format!("{p}{id}")) || id.strip_prefix(p).is_some_and(|b| known.contains(b)))
}

fn issue(severity: Severity, code: &'static str, item: &str, message: String) -> ValidationIssue {
    ValidationIssue {
        severity,
        code,
        message,
        item: (!item.is_empty()).then(|| item.to_string()),
    }
}

/// Negation-insensitive core of a constraint, for contradiction checks.
fn core(statement: &str) -> String {
    let words = crate::text::content_words(statement);
    words.into_iter().collect::<Vec<_>>().join(" ")
}

pub fn check_model(model: &SemanticModel, known: &BTreeSet<String>) -> Vec<ValidationIssue> {
    use Severity::*;
    let mut out = Vec::new();

    // Ids.
    let mut seen: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for item in model.items() {
        if item.id.trim().is_empty() {
            out.push(issue(Error, "missing_id", "", format!("a {} has an empty id", item.category)));
        }
        *seen.entry((item.category, item.id)).or_default() += 1;
    }
    for ((cat, id), n) in seen {
        if n > 1 {
            out.push(issue(Error, "duplicate_id", id, format!("{cat} id \"{id}\" occurs {n} times")));
        }
    }

    // Required fields.
    for x in &model.intent {
        if x.statement.trim().is_empty() {
            out.push(issue(Error, "missing_field", &x.id, "intent has no statement".into()));
        }
    }
    for x in &model.requirements {
        if x.statement.trim().is_empty() {
            out.push(issue(Error, "missing_field", &x.id, "requirement has no statement".into()));
        }
    }
    for x in &model.constraints {
        if x.statement.trim().is_empty() {
            out.push(issue(Error, "missing_field", &x.id, "constraint has no statement".into()));
        }
    }
    for x in &model.entities {
        if x.name.trim().is_empty() || x.kind.trim().is_empty() {
            out.push(issue(Error, "missing_field", &x.id, "entity needs a name and a kind".into()));
        }
    }
    for x in &model.behaviors {
        if x.event.trim().is_empty() || x.from.trim().is_empty() || x.to.trim().is_empty() {
            out.push(issue(Error, "missing_field", &x.id, "behavior needs from, event and to".into()));
        }
    }

    // State transitions.
    let states: BTreeSet<(&str, &str)> = model
        .states
        .iter()
        .map(|s| (s.subject.as_str(), s.name.as_str()))
        .collect();
    for b in &model.behaviors {
        for end in [&b.from, &b.to] {
            if !end.is_empty() && !states.contains(&(b.subject.as_str(), end.as_str())) {
                out.push(issue(
                    Error,
                    "invalid_transition",
                    &b.id,
                    format!("transition {} --{}--> {} references undeclared state \"{end}\" of \"{}\"", b.from, b.event, b.to, b.subject),
                ));
            }
        }
    }

    // References.
    let mut ids: BTreeSet<String> = model.entities.iter().map(|e| e.id.clone()).collect();
    ids.extend(known.iter().cloned());
    let external = |id: &str| {
        ["module:", "package:", "file:", "dir:", "commit:", "test:", "token:", "doc:"]
            .iter()
            .any(|p| id.starts_with(p))
    };
    let check_ref = |id: &str, item: &str, what: &str, out: &mut Vec<ValidationIssue>| {
        if id.trim().is_empty() {
            out.push(issue(Error, "invalid_reference", item, format!("{what} is empty")));
        } else if !is_known(id, &ids) && !external(id) {
            let (sev, code) = if model.items().iter().any(|i| i.id == item && i.basis.evidence == Evidence::Candidate) {
                (Info, "unknown_entity")
            } else {
                (Warning, "unknown_entity")
            };
            out.push(issue(sev, code, item, format!("{what} \"{id}\" is not a known entity")));
        }
    };
    for d in &model.dependencies {
        check_ref(&d.from, &d.id, "relation source", &mut out);
        check_ref(&d.to, &d.id, "relation target", &mut out);
    }
    for s in &model.states {
        check_ref(&s.subject, &s.id, "state subject", &mut out);
    }
    for b in &model.behaviors {
        check_ref(&b.subject, &b.id, "behavior subject", &mut out);
    }
    for i in &model.interactions {
        if let Some(t) = &i.target {
            if !t.contains(' ') {
                check_ref(t, &i.id, "interaction target", &mut out);
            }
        }
    }

    // Inconsistent relations: the same pair related in both directions by an
    // asymmetric relation kind.
    let edges: BTreeSet<(&str, &str, &str)> = model
        .dependencies
        .iter()
        .map(|d| (d.from.as_str(), d.relation.as_str(), d.to.as_str()))
        .collect();
    for d in &model.dependencies {
        let asymmetric = !matches!(d.relation.as_str(), "related-to");
        if asymmetric && d.from < d.to && edges.contains(&(d.to.as_str(), d.relation.as_str(), d.from.as_str())) {
            out.push(issue(
                Warning,
                "inconsistent_relation",
                &d.id,
                format!("\"{}\" and \"{}\" {} each other", d.from, d.to, d.relation.as_str()),
            ));
        }
        if d.from == d.to {
            out.push(issue(Error, "inconsistent_relation", &d.id, "relation points to itself".into()));
        }
    }

    // Contradicting constraints: same subject, same core, must vs must-not.
    for (i, a) in model.constraints.iter().enumerate() {
        for b in &model.constraints[i + 1..] {
            let opposite = matches!(
                (a.kind, b.kind),
                (ConstraintKind::Must, ConstraintKind::MustNot) | (ConstraintKind::MustNot, ConstraintKind::Must)
            );
            if opposite && a.subject == b.subject && (core(&a.statement) == core(&b.statement) || similarity(&a.statement, &b.statement) >= 0.8) {
                out.push(issue(
                    Error,
                    "conflicting_constraints",
                    &a.id,
                    format!("\"{}\" contradicts \"{}\" ({}); requires a human decision", a.statement, b.statement, b.id),
                ));
            }
        }
    }

    // Evidence / confidence consistency.
    for item in model.items() {
        let b = item.basis;
        let v = b.confidence.value;
        if !(0.0..=1.0).contains(&v) {
            out.push(issue(Error, "confidence_out_of_range", item.id, format!("confidence {v} is outside 0..=1")));
        }
        if b.provenance.is_empty() && b.evidence != Evidence::Unknown {
            out.push(issue(Error, "missing_provenance", item.id, format!("{} statement has no provenance", item.category)));
        }
        if b.provenance.iter().any(|p| p.source.trim().is_empty()) {
            out.push(issue(Error, "invalid_reference", item.id, "provenance with an empty source".into()));
        }
        match b.evidence {
            Evidence::Explicit if v < confidence::EXPLICIT => out.push(issue(Warning, "confidence_mismatch", item.id, format!("explicit statement with confidence {v}"))),
            Evidence::Inferred if v > confidence::INFERRED_MAX => out.push(issue(Error, "confidence_mismatch", item.id, format!("inferred statement claims confidence {v} > {}", confidence::INFERRED_MAX))),
            Evidence::Candidate if v > 0.5 => out.push(issue(Warning, "confidence_mismatch", item.id, format!("candidate statement claims confidence {v}"))),
            Evidence::Inferred if b.confidence.reason.is_none() => out.push(issue(Warning, "missing_reason", item.id, "inferred statement without a rule/reason".into())),
            _ => {}
        }
    }
    if model.intent.is_empty() && !model.is_empty() {
        out.push(issue(Info, "no_intent", "", "no explicit intent found; none was invented".into()));
    }
    out
}

#[async_trait]
impl Validator for ModelValidator {
    async fn validate(&self, model: &SemanticModel) -> SemanticResult<ValidationResult> {
        Ok(ValidationResult::from_issues(check_model(model, &self.known_entities)))
    }
}

/// A translation is valid when its source model is valid, every carried
/// item has provenance, and no conflict is silently resolved.
pub fn check_translation(t: &Translation, known: &BTreeSet<String>) -> Vec<ValidationIssue> {
    let mut out = check_model(&t.semantic_model, known);
    for (section, item) in t.result.items() {
        if item.basis.provenance.is_empty() {
            out.push(issue(Severity::Error, "untraceable_output", section, format!("\"{}\" has no provenance", item.text)));
        }
    }
    for c in &t.conflicts {
        out.push(issue(Severity::Warning, "unresolved_conflict", &c.id, format!("{} vs {}: {}", c.source, c.target, c.resolution)));
    }
    for u in &t.unresolved {
        out.push(issue(Severity::Info, "unresolved", &u.item, u.reason.clone()));
    }
    out
}

/// Checks that a model honours a semantic contract. A requirement is only
/// reported as *unverified* (not as missing) when no statement matches it.
pub fn check_contract(model: &SemanticModel, contract: &SemanticContract) -> Vec<ValidationIssue> {
    let mut out = Vec::new();
    let subject = contract.subject();
    let states: BTreeSet<String> = model
        .states
        .iter()
        .filter(|s| s.subject == subject)
        .map(|s| slug(&s.name))
        .collect();
    for s in &contract.behaviors.states {
        if !states.contains(&slug(s)) {
            out.push(issue(Severity::Error, "contract_state_missing", &contract.id, format!("contract \"{}\" requires state \"{s}\" on \"{subject}\"", contract.id)));
        }
    }
    let statements: Vec<&str> = model
        .requirements
        .iter()
        .map(|r| r.statement.as_str())
        .chain(model.constraints.iter().map(|c| c.statement.as_str()))
        .collect();
    for req in contract.requirements.iter().chain(&contract.constraints) {
        let best = statements.iter().map(|s| similarity(req, s)).fold(0.0, f32::max);
        if best < 0.34 {
            out.push(issue(Severity::Warning, "contract_requirement_unverified", &contract.id, format!("no statement in the model matches \"{req}\" (best similarity {:.2})", best)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prov() -> Provenance {
        Provenance {
            source: "spec.yaml".into(),
            line_start: None,
            line_end: None,
            commit: None,
            extraction: "test".into(),
            artifact: None,
        }
    }

    fn valid_model() -> SemanticModel {
        let e = || Basis::explicit(prov());
        let mut m = SemanticModel::default();
        m.entities.push(SemanticEntity::new("btn", "Button", "component", e()));
        m.intent.push(Intent::new("submit order", e()));
        m.states.push(State::new("btn", "idle", e()));
        m.states.push(State::new("btn", "loading", e()));
        m.behaviors.push(Behavior::new("btn", "idle", "submit", "loading", e()));
        m.constraints.push(Constraint::new(Some("btn"), "disabled while loading", ConstraintKind::Must, e()));
        m.finalize();
        m
    }

    fn validate(m: &SemanticModel) -> ValidationResult {
        crate::block_on(ModelValidator::default().validate(m)).unwrap()
    }

    #[test]
    fn a_consistent_model_is_valid() {
        let r = validate(&valid_model());
        assert!(r.valid, "{:?}", r.issues);
        assert_eq!(r.warnings, 0, "{:?}", r.issues);
    }

    #[test]
    fn detects_an_invalid_transition() {
        let mut m = valid_model();
        m.behaviors.push(Behavior::new("btn", "loading", "done", "success", Basis::explicit(prov())));
        assert!(validate(&m).has("invalid_transition"));
    }

    #[test]
    fn detects_a_missing_entity() {
        let mut m = valid_model();
        m.dependencies.push(Dependency::new("btn", nexus_domain::types::RelationKind::Uses, "ghost", Basis::explicit(prov())));
        let r = validate(&m);
        assert!(r.has("unknown_entity"));
        let mut known = BTreeSet::new();
        known.insert("ghost".to_string());
        assert!(!ValidationResult::from_issues(check_model(&m, &known)).has("unknown_entity"));
    }

    #[test]
    fn detects_conflicting_constraints() {
        let mut m = valid_model();
        m.constraints.push(Constraint::new(Some("btn"), "must not be disabled while loading", ConstraintKind::MustNot, Basis::explicit(prov())));
        let r = validate(&m);
        assert!(!r.valid);
        assert!(r.has("conflicting_constraints"));
    }

    #[test]
    fn reports_confidence_problems() {
        let mut m = valid_model();
        m.intent[0].basis = Basis {
            evidence: Evidence::Inferred,
            confidence: Confidence { value: 0.95, reason: Some("x".into()) },
            provenance: vec![prov()],
        };
        assert!(validate(&m).has("confidence_mismatch"));
    }
}
