//! Merges per-artifact models into one project model. Statements with the
//! same id are unified; their bases are merged by `confidence::merge`, so a
//! statement seen in several sources carries all of their provenance.

use crate::confidence;
use crate::model::*;
use std::collections::BTreeMap;

fn merge_category<T: Clone>(
    items: impl IntoIterator<Item = T>,
    id: impl Fn(&T) -> String,
    basis: impl Fn(&mut T) -> &mut Basis,
) -> Vec<T> {
    let mut groups: BTreeMap<String, Vec<T>> = BTreeMap::new();
    for item in items {
        groups.entry(id(&item)).or_default().push(item);
    }
    groups
        .into_values()
        .map(|mut group| {
            // The strongest observation supplies the non-basis fields.
            group.sort_by_key(|x| {
                let mut x = x.clone();
                basis(&mut x).evidence
            });
            let merged = {
                let mut bases: Vec<Basis> = Vec::new();
                for x in group.iter_mut() {
                    bases.push(basis(x).clone());
                }
                let refs: Vec<&Basis> = bases.iter().collect();
                confidence::merge(&refs)
            };
            let mut first = group.swap_remove(0);
            *basis(&mut first) = merged;
            first
        })
        .collect()
}

pub fn consolidate(models: impl IntoIterator<Item = SemanticModel>) -> SemanticModel {
    let mut all = SemanticModel::default();
    for m in models {
        all.artifacts.extend(m.artifacts);
        all.intent.extend(m.intent);
        all.requirements.extend(m.requirements);
        all.behaviors.extend(m.behaviors);
        all.entities.extend(m.entities);
        all.states.extend(m.states);
        all.constraints.extend(m.constraints);
        all.interactions.extend(m.interactions);
        all.dependencies.extend(m.dependencies);
        all.assumptions.extend(m.assumptions);
    }
    let mut out = SemanticModel {
        artifacts: all.artifacts,
        intent: merge_category(all.intent, |x| x.id.clone(), |x| &mut x.basis),
        requirements: merge_category(all.requirements, |x| x.id.clone(), |x| &mut x.basis),
        behaviors: merge_category(all.behaviors, |x| x.id.clone(), |x| &mut x.basis),
        entities: merge_entities(all.entities),
        states: merge_category(all.states, |x| x.id.clone(), |x| &mut x.basis),
        constraints: merge_category(all.constraints, |x| x.id.clone(), |x| &mut x.basis),
        interactions: merge_category(all.interactions, |x| x.id.clone(), |x| &mut x.basis),
        dependencies: merge_category(all.dependencies, |x| x.id.clone(), |x| &mut x.basis),
        assumptions: merge_category(all.assumptions, |x| x.id.clone(), |x| &mut x.basis),
        ..SemanticModel::default()
    };
    out.finalize();
    out
}

/// Entities additionally union their attributes; a kind disagreement is
/// kept visible under `conflictingKinds` instead of being silently resolved.
fn merge_entities(entities: Vec<SemanticEntity>) -> Vec<SemanticEntity> {
    let mut by_id: BTreeMap<String, Vec<SemanticEntity>> = BTreeMap::new();
    for e in entities {
        by_id.entry(e.id.clone()).or_default().push(e);
    }
    by_id
        .into_values()
        .flat_map(|group| {
            let kinds: Vec<String> = {
                let mut k: Vec<String> = group.iter().map(|e| e.kind.clone()).collect();
                k.sort();
                k.dedup();
                k
            };
            let mut attrs = Metadata::new();
            let mut description = None;
            let mut artifact = None;
            let mut role = None;
            for e in &group {
                for (k, v) in &e.attributes {
                    attrs.entry(k.clone()).or_insert_with(|| v.clone());
                }
                description = description.or_else(|| e.description.clone());
                artifact = artifact.or_else(|| e.artifact.clone());
                role = role.or_else(|| e.role.clone());
            }
            let mut merged = merge_category(group, |x| x.id.clone(), |x| &mut x.basis);
            for e in merged.iter_mut() {
                e.attributes = attrs.clone();
                e.description = e.description.clone().or(description.clone());
                e.artifact = e.artifact.clone().or(artifact.clone());
                e.role = e.role.clone().or(role.clone());
                if kinds.len() > 1 {
                    e.attributes
                        .insert("conflictingKinds".into(), kinds.clone().into());
                }
            }
            merged
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prov(source: &str) -> Provenance {
        Provenance {
            source: source.to_string(),
            line_start: None,
            line_end: None,
            commit: None,
            extraction: "test".to_string(),
            artifact: None,
        }
    }

    #[test]
    fn same_statement_from_two_sources_keeps_both_provenances() {
        let mut a = SemanticModel::default();
        a.states
            .push(State::new("btn", "idle", Basis::explicit(prov("a.yaml"))));
        let mut b = SemanticModel::default();
        b.states.push(State::new(
            "btn",
            "idle",
            Basis::candidate(prov("b.html"), "guess"),
        ));
        let merged = consolidate([a, b]);
        assert_eq!(merged.states.len(), 1);
        assert_eq!(merged.states[0].basis.evidence, Evidence::Explicit);
        assert_eq!(merged.states[0].basis.provenance.len(), 2);
    }
}
