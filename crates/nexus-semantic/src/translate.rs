//! SemanticModel -> target-role representation.
//!
//! `DeterministicTranslator` only regroups statements that already exist in
//! the model; every output item keeps the evidence and provenance of the
//! statement it came from. It never adds facts. `LlmTranslator` may add
//! suggestions, but only as `candidate` items next to the deterministic result.

use crate::confidence;
use crate::engineering::{ContextItem, EngineeringContext};
use crate::error::{SemanticError, SemanticResult};
use crate::model::*;
use crate::text::{now, pascal, short_hash, similarity};
use async_trait::async_trait;
use nexus_domain::types::{BehaviorRule, BehaviorTransition};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TranslationDirection {
    DesignToEngineering,
    EngineeringToDesign,
    ProductToEngineering,
    BehaviorToEngineering,
    EngineeringToBehavior,
    SkillToEngineering,
}

impl TranslationDirection {
    pub const ALL: [TranslationDirection; 6] = [
        TranslationDirection::DesignToEngineering,
        TranslationDirection::EngineeringToDesign,
        TranslationDirection::ProductToEngineering,
        TranslationDirection::BehaviorToEngineering,
        TranslationDirection::EngineeringToBehavior,
        TranslationDirection::SkillToEngineering,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            TranslationDirection::DesignToEngineering => "design-to-engineering",
            TranslationDirection::EngineeringToDesign => "engineering-to-design",
            TranslationDirection::ProductToEngineering => "product-to-engineering",
            TranslationDirection::BehaviorToEngineering => "behavior-to-engineering",
            TranslationDirection::EngineeringToBehavior => "engineering-to-behavior",
            TranslationDirection::SkillToEngineering => "skill-to-engineering",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let key = s.to_lowercase().replace(['_', ' '], "-");
        Self::ALL.into_iter().find(|d| d.as_str() == key)
    }

    pub fn source_role(&self) -> Role {
        match self {
            TranslationDirection::DesignToEngineering => Role::Design,
            TranslationDirection::ProductToEngineering => Role::Product,
            TranslationDirection::BehaviorToEngineering => Role::Ux,
            TranslationDirection::EngineeringToDesign
            | TranslationDirection::EngineeringToBehavior
            | TranslationDirection::SkillToEngineering => Role::Engineering,
        }
    }

    pub fn target_role(&self) -> Role {
        match self {
            TranslationDirection::EngineeringToDesign => Role::Design,
            TranslationDirection::EngineeringToBehavior => Role::Ux,
            _ => Role::Engineering,
        }
    }
}

/// A state machine in the same shape as `behavior/*.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorRepresentation {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub version: u32,
    pub states: Vec<String>,
    pub transitions: Vec<BehaviorTransition>,
    pub rules: Vec<BehaviorRule>,
    /// Provenance of every state/transition/rule used.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provenance: Vec<Provenance>,
}

impl BehaviorRepresentation {
    /// YAML accepted by `BehaviorStore` (provenance omitted).
    pub fn to_yaml(&self) -> String {
        let mut copy = self.clone();
        copy.provenance.clear();
        serde_yaml::to_string(&copy).unwrap_or_default()
    }

    pub fn from_model(model: &SemanticModel) -> Vec<BehaviorRepresentation> {
        let mut subjects: BTreeSet<&str> =
            model.states.iter().map(|s| s.subject.as_str()).collect();
        subjects.extend(model.behaviors.iter().map(|b| b.subject.as_str()));
        subjects
            .into_iter()
            .map(|subject| {
                let mut provenance = Vec::new();
                let states = model
                    .states
                    .iter()
                    .filter(|s| s.subject == subject)
                    .map(|s| {
                        provenance.extend(s.basis.provenance.clone());
                        s.name.clone()
                    })
                    .collect();
                let transitions = model
                    .behaviors
                    .iter()
                    .filter(|b| b.subject == subject)
                    .map(|b| {
                        provenance.extend(b.basis.provenance.clone());
                        BehaviorTransition {
                            from: b.from.clone(),
                            event: b.event.clone(),
                            to: b.to.clone(),
                        }
                    })
                    .collect();
                let rules = model
                    .constraints
                    .iter()
                    .filter(|c| c.subject.as_deref() == Some(subject))
                    .map(|c| {
                        provenance.extend(c.basis.provenance.clone());
                        BehaviorRule {
                            id: c.id.rsplit(':').next().unwrap_or(&c.id).to_string(),
                            description: c.statement.clone(),
                        }
                    })
                    .collect();
                provenance.sort();
                provenance.dedup();
                BehaviorRepresentation {
                    id: subject.to_string(),
                    kind: "ui-behavior".to_string(),
                    version: 1,
                    states,
                    transitions,
                    rules,
                    provenance,
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EngineeringSpec {
    pub context: EngineeringContext,
    pub components: Vec<ContextItem>,
    pub state_machines: Vec<BehaviorRepresentation>,
    /// Checks derived one-to-one from requirements and constraints.
    pub acceptance: Vec<ContextItem>,
    /// Constraints of the target system (skills passed as context).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_context: Option<EngineeringContext>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DesignRepresentation {
    pub components: Vec<ContextItem>,
    pub states: BTreeMap<String, Vec<String>>,
    pub interactions: Vec<ContextItem>,
    pub accessibility: Vec<ContextItem>,
    pub rules: Vec<ContextItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TranslationResult {
    Engineering(Box<EngineeringSpec>),
    Behavior {
        machines: Vec<BehaviorRepresentation>,
    },
    Design(DesignRepresentation),
}

impl TranslationResult {
    /// Every output item with the section it belongs to.
    pub fn items(&self) -> Vec<(&'static str, &ContextItem)> {
        let mut out = Vec::new();
        match self {
            TranslationResult::Engineering(e) => {
                let c = &e.context;
                for (s, list) in [
                    ("purpose", &c.purpose),
                    ("capabilities", &c.capabilities),
                    ("constraints", &c.constraints),
                    ("conventions", &c.conventions),
                    ("forbidden", &c.forbidden),
                    ("inputs", &c.inputs),
                    ("outputs", &c.outputs),
                    ("components", &e.components),
                    ("acceptance", &e.acceptance),
                ] {
                    out.extend(list.iter().map(|i| (s, i)));
                }
            }
            TranslationResult::Design(d) => {
                for (s, list) in [
                    ("components", &d.components),
                    ("interactions", &d.interactions),
                    ("accessibility", &d.accessibility),
                    ("rules", &d.rules),
                ] {
                    out.extend(list.iter().map(|i| (s, i)));
                }
            }
            TranslationResult::Behavior { .. } => {}
        }
        out
    }
}

/// A source statement that clashes with a target-system rule. Never
/// resolved by Nexus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub source: String,
    pub target: String,
    pub resolution: String,
    pub provenance: Vec<Provenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Unresolved {
    pub item: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Translation {
    pub id: String,
    #[serde(default)]
    pub project_id: Option<ProjectId>,
    pub direction: TranslationDirection,
    pub source: Vec<ArtifactId>,
    pub source_role: Role,
    pub target: Role,
    pub semantic_model: SemanticModel,
    pub result: TranslationResult,
    pub assumptions: Vec<Assumption>,
    pub conflicts: Vec<Conflict>,
    pub unresolved: Vec<Unresolved>,
    pub confidence: Confidence,
    pub provenance: Vec<Provenance>,
    pub translator: String,
    pub created_at: u64,
}

#[async_trait]
pub trait Translator: Send + Sync {
    fn name(&self) -> &str;
    async fn translate(
        &self,
        model: &SemanticModel,
        direction: TranslationDirection,
    ) -> SemanticResult<Translation>;
}

#[derive(Default)]
pub struct DeterministicTranslator {
    /// Rules of the target system (merged skills), checked for conflicts.
    pub target_context: Option<EngineeringContext>,
}

fn item(text: String, basis: &Basis) -> ContextItem {
    ContextItem {
        text,
        basis: basis.clone(),
    }
}

const COMPONENT_KINDS: &[&str] = &["component", "button", "form", "field", "link", "flow"];

fn engineering(model: &SemanticModel) -> EngineeringSpec {
    let mut ctx = EngineeringContext::default();
    for i in &model.intent {
        ctx.purpose.push(item(i.statement.clone(), &i.basis));
    }
    for r in &model.requirements {
        let target = match r.kind {
            RequirementKind::Input => &mut ctx.inputs,
            RequirementKind::Output => &mut ctx.outputs,
            _ => &mut ctx.capabilities,
        };
        target.push(item(r.statement.clone(), &r.basis));
    }
    for c in &model.constraints {
        let target = match c.kind {
            ConstraintKind::Must | ConstraintKind::Accessibility => &mut ctx.constraints,
            ConstraintKind::MustNot => &mut ctx.forbidden,
            ConstraintKind::Convention => &mut ctx.conventions,
        };
        target.push(item(c.statement.clone(), &c.basis));
    }
    for i in &model.interactions {
        let text = match (&i.target, &i.effect) {
            (Some(t), Some(e)) => format!("{} on {t} -> {e}", i.trigger),
            (Some(t), None) => format!("{} {t}", i.trigger),
            _ => i.trigger.clone(),
        };
        ctx.inputs.push(item(text, &i.basis));
    }
    let components = model
        .entities
        .iter()
        .filter(|e| COMPONENT_KINDS.contains(&e.kind.as_str()))
        .map(|e| {
            item(
                format!("{} ({}, id {})", pascal(&e.id), e.kind, e.id),
                &e.basis,
            )
        })
        .collect();
    let acceptance = model
        .requirements
        .iter()
        .filter(|r| r.kind != RequirementKind::Input)
        .map(|r| item(format!("verify: {}", r.statement), &r.basis))
        .chain(
            model
                .constraints
                .iter()
                .filter(|c| c.kind != ConstraintKind::Convention)
                .map(|c| item(format!("verify: {}", c.statement), &c.basis)),
        )
        .collect();
    EngineeringSpec {
        context: ctx,
        components,
        state_machines: BehaviorRepresentation::from_model(model),
        acceptance,
        target_context: None,
    }
}

fn design(model: &SemanticModel) -> DesignRepresentation {
    let mut d = DesignRepresentation::default();
    for e in model
        .entities
        .iter()
        .filter(|e| COMPONENT_KINDS.contains(&e.kind.as_str()) || e.kind == "variant")
    {
        d.components
            .push(item(format!("{} ({})", e.name, e.kind), &e.basis));
    }
    for s in &model.states {
        d.states
            .entry(s.subject.clone())
            .or_default()
            .push(s.name.clone());
    }
    for i in &model.interactions {
        let text = format!(
            "{} {} {}",
            i.trigger,
            i.target.as_deref().unwrap_or(""),
            i.effect
                .as_deref()
                .map(|e| format!("-> {e}"))
                .unwrap_or_default()
        );
        d.interactions.push(item(
            text.split_whitespace().collect::<Vec<_>>().join(" "),
            &i.basis,
        ));
    }
    for c in &model.constraints {
        let target = if c.kind == ConstraintKind::Accessibility {
            &mut d.accessibility
        } else {
            &mut d.rules
        };
        target.push(item(c.statement.clone(), &c.basis));
    }
    d
}

fn unresolved(model: &SemanticModel, direction: TranslationDirection) -> Vec<Unresolved> {
    let mut out = Vec::new();
    for rep in BehaviorRepresentation::from_model(model) {
        let touched: BTreeSet<&str> = rep
            .transitions
            .iter()
            .flat_map(|t| [t.from.as_str(), t.to.as_str()])
            .collect();
        if !rep.transitions.is_empty() {
            for s in &rep.states {
                if !touched.contains(s.as_str()) {
                    out.push(Unresolved {
                        item: format!("{}/{s}", rep.id),
                        reason: "state has no transition in or out; its behavior is unspecified"
                            .into(),
                    });
                }
            }
        }
    }
    let events: BTreeSet<&str> = model.behaviors.iter().map(|b| b.event.as_str()).collect();
    for i in &model.interactions {
        if let Some(effect) = &i.effect {
            if i.trigger != "http"
                && !effect.starts_with("navigate")
                && !events.contains(effect.as_str())
            {
                out.push(Unresolved {
                    item: i.id.clone(),
                    reason: format!("interaction effect \"{effect}\" matches no transition event"),
                });
            }
        }
    }
    if direction == TranslationDirection::EngineeringToBehavior && model.states.is_empty() {
        out.push(Unresolved {
            item: "states".into(),
            reason: "source contains no states; no behavior can be derived".into(),
        });
    }
    out
}

fn conflicts(model: &SemanticModel, target: &EngineeringContext) -> Vec<Conflict> {
    let mut out = Vec::new();
    let mut check = |source: &Constraint, rule: &ContextItem, target_is_forbidden: bool| {
        let clash = match source.kind {
            ConstraintKind::Must | ConstraintKind::Accessibility => target_is_forbidden,
            ConstraintKind::MustNot => !target_is_forbidden,
            ConstraintKind::Convention => false,
        };
        if clash && similarity(&source.statement, &rule.text) >= 0.5 {
            let mut provenance = source.basis.provenance.clone();
            provenance.extend(rule.basis.provenance.clone());
            out.push(Conflict {
                id: format!(
                    "conflict:{}",
                    short_hash(&format!("{}|{}", source.id, rule.text), 8)
                ),
                kind: "constraint_conflict".into(),
                source: source.statement.clone(),
                target: rule.text.clone(),
                resolution: "requires_human_decision".into(),
                provenance,
            });
        }
    };
    for c in &model.constraints {
        for rule in &target.forbidden {
            check(c, rule, true);
        }
        for rule in &target.constraints {
            check(c, rule, false);
        }
    }
    out
}

#[async_trait]
impl Translator for DeterministicTranslator {
    fn name(&self) -> &str {
        "DeterministicTranslator"
    }

    async fn translate(
        &self,
        model: &SemanticModel,
        direction: TranslationDirection,
    ) -> SemanticResult<Translation> {
        let result = match direction {
            TranslationDirection::EngineeringToDesign => TranslationResult::Design(design(model)),
            TranslationDirection::EngineeringToBehavior => TranslationResult::Behavior {
                machines: BehaviorRepresentation::from_model(model),
            },
            _ => {
                let mut spec = engineering(model);
                spec.target_context = self.target_context.clone();
                TranslationResult::Engineering(Box::new(spec))
            }
        };
        let conflicts = match (&self.target_context, &result) {
            (Some(target), TranslationResult::Engineering(_)) => conflicts(model, target),
            _ => Vec::new(),
        };
        Ok(finish(
            model,
            direction,
            result,
            conflicts,
            Vec::new(),
            self.name(),
        ))
    }
}

fn finish(
    model: &SemanticModel,
    direction: TranslationDirection,
    result: TranslationResult,
    conflicts: Vec<Conflict>,
    extra_assumptions: Vec<Assumption>,
    translator: &str,
) -> Translation {
    let bases: Vec<&Basis> = result.items().into_iter().map(|(_, i)| &i.basis).collect();
    let confidence = if bases.is_empty() {
        model.confidence.clone()
    } else {
        confidence::aggregate(&bases)
    };
    let mut provenance: Vec<Provenance> = result
        .items()
        .into_iter()
        .flat_map(|(_, i)| i.basis.provenance.clone())
        .chain(model.provenance.iter().cloned())
        .collect();
    provenance.sort();
    provenance.dedup();
    let created_at = now();
    let mut assumptions = model.assumptions.clone();
    assumptions.extend(extra_assumptions);
    Translation {
        id: format!("tr-{}-{}", direction.as_str(), crate::text::nonce()),
        project_id: None,
        direction,
        source: model.artifacts.clone(),
        source_role: direction.source_role(),
        target: direction.target_role(),
        semantic_model: model.clone(),
        unresolved: unresolved(model, direction),
        result,
        assumptions,
        conflicts,
        confidence,
        provenance,
        translator: translator.to_string(),
        created_at,
    }
}

/// Anything that turns a prompt into text: a hosted model, a local one, a stub.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn complete(&self, prompt: &str) -> SemanticResult<String>;
}

/// Runs the deterministic translation, then asks an LLM for additional
/// suggestions. Suggestions are added only as `candidate` items (0.3) with
/// provenance `llm:<provider>`; they never replace deterministic output.
pub struct LlmTranslator<P: LlmProvider> {
    pub provider: P,
    pub base: DeterministicTranslator,
}

#[derive(Deserialize)]
struct Suggestion {
    section: String,
    text: String,
}

pub fn build_prompt(model: &SemanticModel, direction: TranslationDirection) -> String {
    format!(
        "Translate this semantic model ({}). Reply with a JSON array of {{\"section\": \"constraints|capabilities|conventions|forbidden|inputs|outputs\", \"text\": \"...\"}} containing ONLY statements that are missing from the model. Model:\n{}",
        direction.as_str(),
        serde_json::to_string(model).unwrap_or_default()
    )
}

#[async_trait]
impl<P: LlmProvider> Translator for LlmTranslator<P> {
    fn name(&self) -> &str {
        "LlmTranslator"
    }

    async fn translate(
        &self,
        model: &SemanticModel,
        direction: TranslationDirection,
    ) -> SemanticResult<Translation> {
        let mut t = self.base.translate(model, direction).await?;
        let reply = self
            .provider
            .complete(&build_prompt(model, direction))
            .await?;
        let suggestions: Vec<Suggestion> = serde_json::from_str(reply.trim())
            .map_err(|e| SemanticError::Provider(format!("unparseable LLM reply: {e}")))?;
        if let TranslationResult::Engineering(spec) = &mut t.result {
            for s in suggestions {
                let basis = Basis::candidate(
                    Provenance {
                        source: format!("llm:{}", self.provider.name()),
                        line_start: None,
                        line_end: None,
                        commit: None,
                        extraction: "LlmTranslator".into(),
                        artifact: None,
                    },
                    "suggested by a language model; unverified",
                );
                let c = &mut spec.context;
                let target = match s.section.as_str() {
                    "constraints" => &mut c.constraints,
                    "conventions" => &mut c.conventions,
                    "forbidden" => &mut c.forbidden,
                    "inputs" => &mut c.inputs,
                    "outputs" => &mut c.outputs,
                    _ => &mut c.capabilities,
                };
                target.push(ContextItem {
                    text: s.text,
                    basis,
                });
            }
        }
        t.translator = format!("LlmTranslator({})", self.provider.name());
        Ok(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on;

    fn prov() -> Provenance {
        Provenance {
            source: "behavior/checkout-button.yaml".into(),
            line_start: Some(1),
            line_end: Some(1),
            commit: None,
            extraction: "test".into(),
            artifact: None,
        }
    }

    fn model() -> SemanticModel {
        let e = || Basis::explicit(prov());
        let mut m = SemanticModel::default();
        m.entities.push(SemanticEntity::new(
            "checkout-button",
            "Checkout Button",
            "component",
            e(),
        ));
        m.intent.push(Intent::new("submit checkout order", e()));
        for s in ["idle", "loading", "success"] {
            m.states.push(State::new("checkout-button", s, e()));
        }
        m.behaviors.push(Behavior::new(
            "checkout-button",
            "idle",
            "submit",
            "loading",
            e(),
        ));
        m.behaviors.push(Behavior::new(
            "checkout-button",
            "loading",
            "success",
            "success",
            e(),
        ));
        m.constraints.push(Constraint::new(
            Some("checkout-button"),
            "Use a custom loading state",
            ConstraintKind::Must,
            e(),
        ));
        m.finalize();
        m
    }

    #[test]
    fn semantic_model_to_engineering_context_keeps_provenance() {
        let t = block_on(
            DeterministicTranslator::default()
                .translate(&model(), TranslationDirection::BehaviorToEngineering),
        )
        .unwrap();
        let TranslationResult::Engineering(spec) = &t.result else {
            panic!()
        };
        assert_eq!(spec.context.purpose[0].text, "submit checkout order");
        assert_eq!(
            spec.components[0].text,
            "CheckoutButton (component, id checkout-button)"
        );
        assert_eq!(
            spec.state_machines[0].states,
            vec!["idle", "loading", "success"]
        );
        assert!(t
            .result
            .items()
            .iter()
            .all(|(_, i)| !i.basis.provenance.is_empty()));
        assert_eq!(t.confidence.value, 1.0);
    }

    #[test]
    fn semantic_model_to_behavior_representation_round_trips_as_yaml() {
        let t = block_on(
            DeterministicTranslator::default()
                .translate(&model(), TranslationDirection::EngineeringToBehavior),
        )
        .unwrap();
        let TranslationResult::Behavior { machines } = &t.result else {
            panic!()
        };
        let yaml = machines[0].to_yaml();
        assert!(yaml.contains("id: checkout-button"));
        assert!(yaml.contains("event: submit"));
        let back: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(back["states"][1], "loading");
    }

    #[test]
    fn conflicts_with_target_rules_require_a_human_decision() {
        let target = EngineeringContext {
            forbidden: vec![ContextItem {
                text: "No custom loading state management".into(),
                basis: Basis::explicit(prov()),
            }],
            ..Default::default()
        };
        let tr = DeterministicTranslator {
            target_context: Some(target),
        };
        let t =
            block_on(tr.translate(&model(), TranslationDirection::DesignToEngineering)).unwrap();
        assert_eq!(t.conflicts.len(), 1);
        assert_eq!(t.conflicts[0].resolution, "requires_human_decision");
    }

    struct Stub;
    #[async_trait]
    impl LlmProvider for Stub {
        fn name(&self) -> &str {
            "stub"
        }
        async fn complete(&self, _prompt: &str) -> SemanticResult<String> {
            Ok(r#"[{"section":"constraints","text":"debounce submit"}]"#.into())
        }
    }

    #[test]
    fn llm_suggestions_are_only_candidates() {
        let tr = LlmTranslator {
            provider: Stub,
            base: DeterministicTranslator::default(),
        };
        let t =
            block_on(tr.translate(&model(), TranslationDirection::BehaviorToEngineering)).unwrap();
        let TranslationResult::Engineering(spec) = &t.result else {
            panic!()
        };
        let s = spec
            .context
            .constraints
            .iter()
            .find(|c| c.text == "debounce submit")
            .unwrap();
        assert_eq!(s.basis.evidence, Evidence::Candidate);
    }
}
