//! Semantic diff: compares two SemanticModels statement by statement.
//! Provenance (line numbers, commits) and confidence are ignored, so moving
//! text around is not a change; changing evidence is.

use crate::model::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticChange {
    /// e.g. `BehaviorChanged`, `StateRemoved`, `ConstraintAdded`.
    #[serde(rename = "type")]
    pub change_type: String,
    pub category: String,
    pub id: String,
    /// Entity the statement is about, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<String>,
    /// Changed fields: `{ field: { before, after } }`, or the whole
    /// statement for additions/removals.
    pub change: Value,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SemanticDiff {
    pub changes: Vec<SemanticChange>,
    pub summary: BTreeMap<String, usize>,
}

impl SemanticDiff {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn has(&self, change_type: &str) -> bool {
        self.changes.iter().any(|c| c.change_type == change_type)
    }
}

fn payload<T: Serialize>(item: &T) -> Map<String, Value> {
    let mut v = match serde_json::to_value(item) {
        Ok(Value::Object(o)) => o,
        _ => Map::new(),
    };
    v.remove("provenance");
    v.remove("confidence");
    v
}

fn entity_of(category: &str, p: &Map<String, Value>) -> Option<String> {
    let key = match category {
        "Entity" => "id",
        "Dependency" => "from",
        "Interaction" => "target",
        _ => "subject",
    };
    p.get(key).and_then(Value::as_str).map(str::to_string)
}

fn diff_category<T: Serialize>(
    category: &str,
    left: &[T],
    right: &[T],
    id: impl Fn(&T) -> &str,
    out: &mut Vec<SemanticChange>,
) {
    let l: BTreeMap<&str, Map<String, Value>> = left.iter().map(|x| (id(x), payload(x))).collect();
    let r: BTreeMap<&str, Map<String, Value>> = right.iter().map(|x| (id(x), payload(x))).collect();
    for (k, before) in &l {
        match r.get(k) {
            None => out.push(SemanticChange {
                change_type: format!("{category}Removed"),
                category: category.to_string(),
                id: k.to_string(),
                entity: entity_of(category, before),
                change: Value::Object(before.clone()),
            }),
            Some(after) if after != before => {
                let mut fields = Map::new();
                for key in before.keys().chain(after.keys()) {
                    let (b, a) = (before.get(key), after.get(key));
                    if b != a && !fields.contains_key(key) {
                        fields.insert(key.clone(), json!({ "before": b, "after": a }));
                    }
                }
                out.push(SemanticChange {
                    change_type: format!("{category}Changed"),
                    category: category.to_string(),
                    id: k.to_string(),
                    entity: entity_of(category, after),
                    change: Value::Object(fields),
                });
            }
            Some(_) => {}
        }
    }
    for (k, after) in &r {
        if !l.contains_key(k) {
            out.push(SemanticChange {
                change_type: format!("{category}Added"),
                category: category.to_string(),
                id: k.to_string(),
                entity: entity_of(category, after),
                change: Value::Object(after.clone()),
            });
        }
    }
}

/// Folds a removed + added dependency on the same pair into one
/// `DependencyChanged` (the relation kind changed).
fn fold_relation_changes(changes: Vec<SemanticChange>) -> Vec<SemanticChange> {
    let pair = |c: &SemanticChange| {
        (
            c.change.get("from").cloned().unwrap_or(Value::Null),
            c.change.get("to").cloned().unwrap_or(Value::Null),
        )
    };
    let mut out: Vec<SemanticChange> = Vec::new();
    let mut added: Vec<SemanticChange> = Vec::new();
    let mut removed: Vec<SemanticChange> = Vec::new();
    for c in changes {
        match c.change_type.as_str() {
            "DependencyAdded" => added.push(c),
            "DependencyRemoved" => removed.push(c),
            _ => out.push(c),
        }
    }
    for r in removed {
        if let Some(i) = added.iter().position(|a| pair(a) == pair(&r)) {
            let a = added.remove(i);
            out.push(SemanticChange {
                change_type: "DependencyChanged".into(),
                category: "Dependency".into(),
                id: a.id.clone(),
                entity: a.entity.clone(),
                change: json!({
                    "relation": { "before": r.change.get("relation"), "after": a.change.get("relation") },
                    "previousId": r.id,
                }),
            });
        } else {
            out.push(r);
        }
    }
    out.extend(added);
    out
}

pub fn semantic_diff(left: &SemanticModel, right: &SemanticModel) -> SemanticDiff {
    let mut changes = Vec::new();
    diff_category(
        "Intent",
        &left.intent,
        &right.intent,
        |x| &x.id,
        &mut changes,
    );
    diff_category(
        "Requirement",
        &left.requirements,
        &right.requirements,
        |x| &x.id,
        &mut changes,
    );
    diff_category(
        "Behavior",
        &left.behaviors,
        &right.behaviors,
        |x| &x.id,
        &mut changes,
    );
    diff_category(
        "Entity",
        &left.entities,
        &right.entities,
        |x| &x.id,
        &mut changes,
    );
    diff_category(
        "State",
        &left.states,
        &right.states,
        |x| &x.id,
        &mut changes,
    );
    diff_category(
        "Constraint",
        &left.constraints,
        &right.constraints,
        |x| &x.id,
        &mut changes,
    );
    diff_category(
        "Interaction",
        &left.interactions,
        &right.interactions,
        |x| &x.id,
        &mut changes,
    );
    diff_category(
        "Dependency",
        &left.dependencies,
        &right.dependencies,
        |x| &x.id,
        &mut changes,
    );
    diff_category(
        "Assumption",
        &left.assumptions,
        &right.assumptions,
        |x| &x.id,
        &mut changes,
    );
    let changes = fold_relation_changes(changes);
    let mut summary = BTreeMap::new();
    for c in &changes {
        *summary.entry(c.change_type.clone()).or_insert(0) += 1;
    }
    SemanticDiff { changes, summary }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_domain::types::RelationKind;

    fn prov(line: u32) -> Provenance {
        Provenance {
            source: "b.yaml".into(),
            line_start: Some(line),
            line_end: Some(line),
            commit: None,
            extraction: "test".into(),
            artifact: None,
        }
    }

    fn base() -> SemanticModel {
        let mut m = SemanticModel::default();
        m.states
            .push(State::new("btn", "idle", Basis::explicit(prov(1))));
        m.states
            .push(State::new("btn", "loading", Basis::explicit(prov(2))));
        m.constraints.push(Constraint {
            id: "constraint:btn:double".into(),
            statement: "no double submit".into(),
            kind: ConstraintKind::MustNot,
            subject: Some("btn".into()),
            basis: Basis::explicit(prov(3)),
        });
        m.dependencies.push(Dependency::new(
            "btn",
            RelationKind::Uses,
            "flow",
            Basis::explicit(prov(4)),
        ));
        m
    }

    #[test]
    fn moved_lines_are_not_changes() {
        let mut moved = base();
        moved.states[0].basis = Basis::explicit(prov(40));
        assert!(semantic_diff(&base(), &moved).is_empty());
    }

    #[test]
    fn state_added_and_removed() {
        let mut after = base();
        after.states.retain(|s| s.name != "loading");
        after
            .states
            .push(State::new("btn", "error", Basis::explicit(prov(9))));
        let d = semantic_diff(&base(), &after);
        assert!(d.has("StateAdded"));
        assert!(d.has("StateRemoved"));
        let removed = d
            .changes
            .iter()
            .find(|c| c.change_type == "StateRemoved")
            .unwrap();
        assert_eq!(removed.entity.as_deref(), Some("btn"));
    }

    #[test]
    fn constraint_changed() {
        let mut after = base();
        after.constraints[0].statement = "never allow a double submit".into();
        let d = semantic_diff(&base(), &after);
        assert!(d.has("ConstraintChanged"));
        assert_eq!(
            d.changes[0].change["statement"]["before"],
            "no double submit"
        );
    }

    #[test]
    fn relationship_changed() {
        let mut after = base();
        after.dependencies = vec![Dependency::new(
            "btn",
            RelationKind::DependsOn,
            "flow",
            Basis::explicit(prov(4)),
        )];
        let d = semantic_diff(&base(), &after);
        assert_eq!(d.changes.len(), 1, "{:?}", d.changes);
        assert_eq!(d.changes[0].change_type, "DependencyChanged");
        assert_eq!(d.changes[0].change["relation"]["after"], "depends-on");
    }
}
