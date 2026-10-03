use super::{explicit_at, has_ext, parse_yaml, strings, ArtifactParser};
use crate::error::{SemanticError, SemanticResult};
use crate::model::*;
use async_trait::async_trait;
use nexus_domain::types::RelationKind;
use serde_json::Value;

const NAME: &str = "BehaviorSpecParser";

/// Event names treated as user actions; everything else is a system event.
const USER_ACTIONS: &[&str] = &[
    "submit", "click", "tap", "press", "select", "input", "change", "reset", "open", "close",
    "cancel", "retry", "hover", "focus", "blur", "drag", "drop", "toggle", "type", "save",
    "delete", "confirm", "dismiss",
];

/// Behavior YAML (`states`, `transitions`, `rules`) -> states, behaviors,
/// constraints. All explicit, except the subject link and interactions.
pub struct BehaviorSpecParser;

#[async_trait]
impl ArtifactParser for BehaviorSpecParser {
    fn name(&self) -> &'static str {
        NAME
    }

    fn supports(&self, artifact: &NexusArtifact) -> bool {
        if artifact.kind == ArtifactKind::BehaviorSpec {
            return true;
        }
        has_ext(artifact, &["yaml", "yml"])
            && parse_yaml(artifact).is_ok_and(|v| {
                v.get("states").is_some_and(Value::is_array)
                    && v.get("transitions").is_some_and(Value::is_array)
            })
    }

    async fn inspect(&self, artifact: &NexusArtifact) -> SemanticResult<SemanticModel> {
        let doc = parse_yaml(artifact)?;
        let id = doc
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| SemanticError::Parse {
                source_path: artifact.provenance.source.clone(),
                message: "behavior spec is missing an id".to_string(),
            })?;
        let subject = doc.get("subject").and_then(Value::as_str).unwrap_or(id);
        let spec_entity = format!("behavior:{id}");
        let mut m = SemanticModel::default();

        let mut entity = SemanticEntity::new(
            &spec_entity,
            id,
            "behavior-spec",
            explicit_at(artifact, NAME, "id:"),
        );
        entity.artifact = Some(artifact.id.clone());
        if let Some(kind) = doc.get("type").and_then(Value::as_str) {
            entity.attributes.insert("specType".into(), kind.into());
        }
        m.entities.push(entity);

        let subject_basis = if doc.get("subject").is_some() {
            explicit_at(artifact, NAME, "subject:")
        } else {
            Basis::inferred(
                vec![artifact.at(NAME, None)],
                "behavior spec id equals the subject entity id (Context Resolver convention)",
            )
        };
        m.dependencies.push(Dependency::new(
            subject,
            RelationKind::DefinedBy,
            &spec_entity,
            subject_basis,
        ));

        for state in strings(doc.get("states")) {
            let basis = explicit_at(artifact, NAME, &format!("- {state}"));
            m.states.push(State::new(subject, &state, basis));
        }

        for t in doc
            .get("transitions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let field = |k: &str| t.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let (from, event, to) = (field("from"), field("event"), field("to"));
            let basis = explicit_at(artifact, NAME, &format!("event: {event}"));
            m.behaviors
                .push(Behavior::new(subject, &from, &event, &to, basis));
            if USER_ACTIONS.contains(&event.as_str()) {
                m.interactions.push(Interaction::new(
                    &event,
                    Some(subject),
                    Some(&event),
                    Basis::inferred(
                        vec![artifact.at(NAME, None)],
                        "event name is in the user-action lexicon",
                    ),
                ));
            }
        }

        for rule in doc
            .get("rules")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(description) = rule.get("description").and_then(Value::as_str) else {
                continue;
            };
            let basis = explicit_at(artifact, NAME, description);
            let kind = modal_kind(description).unwrap_or(ConstraintKind::Must);
            let mut c = Constraint::new(Some(subject), description, kind, basis);
            if let Some(rule_id) = rule.get("id").and_then(Value::as_str) {
                c.id = format!("constraint:{subject}:{rule_id}");
            }
            m.constraints.push(c);
        }

        for intent in strings(doc.get("intent")) {
            m.intent
                .push(Intent::new(&intent, explicit_at(artifact, NAME, &intent)));
        }
        Ok(m)
    }
}
