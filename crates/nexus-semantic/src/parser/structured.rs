use super::{explicit_at, has_ext, parse_json, parse_yaml, strings, ArtifactParser};
use crate::error::SemanticResult;
use crate::model::*;
use crate::text::slug;
use async_trait::async_trait;
use nexus_domain::types::RelationKind;
use serde_json::Value;

/// Generic YAML documents: recognized keys only (`intent`, `requirements`,
/// `constraints`, `states`, `dependsOn`, ...) plus known shapes (project,
/// policy, workflow, OpenAPI). Unrecognized keys are ignored, not guessed.
pub struct YamlParser;

/// Generic JSON documents; same rules as `YamlParser`, plus `package.json`.
pub struct JsonParser;

#[async_trait]
impl ArtifactParser for YamlParser {
    fn name(&self) -> &'static str {
        "YamlParser"
    }

    fn supports(&self, artifact: &NexusArtifact) -> bool {
        has_ext(artifact, &["yaml", "yml"])
    }

    async fn inspect(&self, artifact: &NexusArtifact) -> SemanticResult<SemanticModel> {
        Ok(extract(artifact, &parse_yaml(artifact)?, self.name()))
    }
}

#[async_trait]
impl ArtifactParser for JsonParser {
    fn name(&self) -> &'static str {
        "JsonParser"
    }

    fn supports(&self, artifact: &NexusArtifact) -> bool {
        has_ext(artifact, &["json"])
    }

    async fn inspect(&self, artifact: &NexusArtifact) -> SemanticResult<SemanticModel> {
        Ok(extract(artifact, &parse_json(artifact)?, self.name()))
    }
}

fn str_field<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

fn extract(artifact: &NexusArtifact, v: &Value, parser: &str) -> SemanticModel {
    let at = |needle: &str| explicit_at(artifact, parser, needle);
    let mut m = SemanticModel::default();
    if !v.is_object() {
        return m;
    }

    let entity_id = if artifact.kind == ArtifactKind::Project {
        str_field(v, "id").map(|id| (format!("project:{id}"), "project"))
    } else if artifact.kind == ArtifactKind::Policy || v.get("permissions").is_some() {
        str_field(v, "client").map(|c| (format!("policy:{c}"), "policy"))
    } else if artifact.kind == ArtifactKind::Workflow
        || (v.get("jobs").is_some() && v.get("on").is_some())
    {
        Some((format!("workflow:{}", slug(&artifact.name)), "workflow"))
    } else if v.get("openapi").is_some() || v.get("swagger").is_some() {
        let title = v
            .pointer("/info/title")
            .and_then(Value::as_str)
            .unwrap_or(&artifact.name);
        Some((format!("api:{}", slug(title)), "api"))
    } else if artifact.name == "package.json"
        || artifact.provenance.source.ends_with("package.json")
    {
        str_field(v, "name").map(|n| (format!("package:{n}"), "package"))
    } else {
        match (
            str_field(v, "id"),
            str_field(v, "type").or(str_field(v, "kind")),
        ) {
            (Some(id), Some(kind)) => Some((slug(id), kind)),
            _ => None,
        }
    };

    let subject = entity_id.as_ref().map(|(id, _)| id.clone());
    if let Some((id, kind)) = &entity_id {
        let name = str_field(v, "name")
            .or_else(|| str_field(v, "client"))
            .or_else(|| v.pointer("/info/title").and_then(Value::as_str))
            .unwrap_or(&artifact.name);
        let mut e = SemanticEntity::new(id, name, kind, at(name));
        e.artifact = Some(artifact.id.clone());
        e.role = Some(artifact.role.clone());
        e.description = str_field(v, "description").map(|s| s.trim().to_string());
        for key in ["permissions", "roles", "version"] {
            if let Some(x) = v.get(key) {
                e.attributes.insert(key.into(), x.clone());
            }
        }
        if *kind == "workflow" {
            if let Some(jobs) = v.get("jobs").and_then(Value::as_object) {
                e.attributes.insert(
                    "jobs".into(),
                    jobs.keys()
                        .cloned()
                        .map(Value::from)
                        .collect::<Vec<_>>()
                        .into(),
                );
            }
        }
        m.entities.push(e);
    }
    let subject = subject.as_deref();

    for intent in strings(v.get("intent")) {
        m.intent.push(Intent::new(&intent, at(&intent)));
    }
    for req in strings(v.get("requirements")) {
        m.requirements.push(Requirement::new(
            subject,
            &req,
            RequirementKind::Functional,
            at(&req),
        ));
    }
    for c in strings(v.get("constraints")) {
        let kind = modal_kind(&c)
            .filter(|k| *k == ConstraintKind::MustNot)
            .unwrap_or(ConstraintKind::Must);
        m.constraints
            .push(Constraint::new(subject, &c, kind, at(&c)));
    }
    if let Some(s) = subject {
        for state in strings(v.get("states")) {
            m.states
                .push(State::new(s, &state, at(&format!("- {state}"))));
        }
        for dep in strings(v.get("dependsOn").or_else(|| v.get("depends_on"))) {
            m.dependencies.push(Dependency::new(
                s,
                RelationKind::DependsOn,
                &slug(&dep),
                at(&dep),
            ));
        }
        if let Some(deps) = v.get("dependencies").and_then(Value::as_object) {
            for name in deps.keys() {
                m.dependencies.push(Dependency::new(
                    s,
                    RelationKind::DependsOn,
                    &format!("module:{name}"),
                    at(&format!("\"{name}\"")),
                ));
            }
        }
    }

    if let (Some(api), Some(paths)) = (subject, v.get("paths").and_then(Value::as_object)) {
        for (path, ops) in paths {
            for (method, op) in ops.as_object().into_iter().flatten() {
                let method = method.to_uppercase();
                if !["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&method.as_str()) {
                    continue;
                }
                let name = format!("{method} {path}");
                let eid = format!("endpoint:{}", slug(&name));
                let mut e = SemanticEntity::new(&eid, &name, "endpoint", at(path));
                e.description = str_field(op, "summary").map(str::to_string);
                m.entities.push(e);
                m.dependencies
                    .push(Dependency::new(api, RelationKind::Contains, &eid, at(path)));
                m.interactions.push(Interaction::new(
                    "http",
                    Some(&eid),
                    str_field(op, "operationId"),
                    at(path),
                ));
            }
        }
    }
    m
}
