use super::{explicit_at, strings, structured, ArtifactParser};
use crate::error::{SemanticError, SemanticResult};
use crate::model::*;
use crate::text::slug;
use async_trait::async_trait;
use nexus_domain::types::RelationKind;
use serde_json::{Map, Value};

const NAME: &str = "DesignSystemParser";

/// Design tokens (W3C `$value` / Style Dictionary `value` leaves) and
/// component specs (`component`, `variants`, `states`, `tokens`,
/// `interactions`, `accessibility`, `documentation`, `tests`, `dependencies`).
pub struct DesignSystemParser;

fn is_component_spec(v: &Value) -> bool {
    v.get("component").is_some_and(Value::is_string)
        && [
            "variants",
            "states",
            "tokens",
            "interactions",
            "accessibility",
        ]
        .iter()
        .any(|k| v.get(k).is_some())
}

fn collect_tokens(v: &Value, path: &mut Vec<String>, out: &mut Vec<(String, Map<String, Value>)>) {
    let Value::Object(o) = v else { return };
    if let Some(value) = o.get("$value").or_else(|| o.get("value")) {
        if !value.is_object() {
            let mut attrs = Map::new();
            attrs.insert("value".into(), value.clone());
            if let Some(t) = o.get("$type").or_else(|| o.get("type")) {
                attrs.insert("type".into(), t.clone());
            }
            out.push((path.join("."), attrs));
            return;
        }
    }
    for (k, child) in o {
        if k.starts_with('$') {
            continue;
        }
        path.push(k.clone());
        collect_tokens(child, path, out);
        path.pop();
    }
}

fn tokens_of(v: &Value) -> Vec<(String, Map<String, Value>)> {
    let mut out = Vec::new();
    collect_tokens(v, &mut Vec::new(), &mut out);
    out
}

#[async_trait]
impl ArtifactParser for DesignSystemParser {
    fn name(&self) -> &'static str {
        NAME
    }

    fn supports(&self, artifact: &NexusArtifact) -> bool {
        let Some(v) = structured(artifact) else {
            return false;
        };
        artifact.kind == ArtifactKind::DesignSystem
            || is_component_spec(&v)
            || !tokens_of(&v).is_empty()
    }

    async fn inspect(&self, artifact: &NexusArtifact) -> SemanticResult<SemanticModel> {
        let v = structured(artifact).ok_or_else(|| SemanticError::Parse {
            source_path: artifact.provenance.source.clone(),
            message: "design system artifacts must be YAML or JSON".to_string(),
        })?;
        let at = |needle: &str| explicit_at(artifact, NAME, needle);
        let mut m = SemanticModel::default();

        if is_component_spec(&v) {
            let name = v["component"].as_str().unwrap_or_default();
            let id = slug(name);
            let mut entity = SemanticEntity::new(&id, name, "component", at("component:"));
            entity.role = Some(Role::Design);
            entity.artifact = Some(artifact.id.clone());
            entity.description = v
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_string);
            m.entities.push(entity);

            for variant in strings(v.get("variants")) {
                let vid = format!("variant:{id}/{}", slug(&variant));
                m.entities
                    .push(SemanticEntity::new(&vid, &variant, "variant", at(&variant)));
                m.dependencies.push(Dependency::new(
                    &id,
                    RelationKind::HasVariant,
                    &vid,
                    at(&variant),
                ));
            }
            for state in strings(v.get("states")) {
                m.states.push(State::new(&id, &slug(&state), at(&state)));
            }
            for token in strings(v.get("tokens")) {
                m.dependencies.push(Dependency::new(
                    &id,
                    RelationKind::Uses,
                    &format!("token:{token}"),
                    at(&token),
                ));
            }
            for interaction in strings(v.get("interactions")) {
                let (trigger, effect) = interaction
                    .split_once("->")
                    .or_else(|| interaction.split_once('→'))
                    .map(|(a, b)| (a.trim().to_string(), Some(b.trim().to_string())))
                    .unwrap_or((interaction.trim().to_string(), None));
                m.interactions.push(Interaction::new(
                    &trigger,
                    Some(&id),
                    effect.as_deref(),
                    at(&interaction),
                ));
            }
            for rule in strings(v.get("accessibility")) {
                m.constraints.push(Constraint::new(
                    Some(&id),
                    &rule,
                    ConstraintKind::Accessibility,
                    at(&rule),
                ));
            }
            for doc in strings(v.get("documentation")) {
                m.dependencies.push(Dependency::new(
                    &format!("doc:{doc}"),
                    RelationKind::Documents,
                    &id,
                    at(&doc),
                ));
            }
            for test in strings(v.get("tests")) {
                m.dependencies.push(Dependency::new(
                    &id,
                    RelationKind::TestedBy,
                    &format!("test:{test}"),
                    at(&test),
                ));
            }
            for dep in strings(v.get("dependencies")) {
                m.dependencies.push(Dependency::new(
                    &id,
                    RelationKind::DependsOn,
                    &slug(&dep),
                    at(&dep),
                ));
            }
        }

        let tokens = tokens_of(&v);
        if !tokens.is_empty() {
            let ds_id = format!("design-system:{}", slug(&artifact.name));
            let mut ds = SemanticEntity::new(&ds_id, &artifact.name, "design-system", at("{"));
            ds.artifact = Some(artifact.id.clone());
            m.entities.push(ds);
            for (path, attrs) in tokens {
                let tid = format!("token:{path}");
                let leaf = path.rsplit('.').next().unwrap_or(&path).to_string();
                let mut token =
                    SemanticEntity::new(&tid, &path, "token", at(&format!("\"{leaf}\"")));
                token.attributes = attrs.into_iter().collect();
                m.entities.push(token);
                m.dependencies.push(Dependency::new(
                    &ds_id,
                    RelationKind::Contains,
                    &tid,
                    Basis::explicit(artifact.at(NAME, None)),
                ));
            }
        }
        Ok(m)
    }
}
