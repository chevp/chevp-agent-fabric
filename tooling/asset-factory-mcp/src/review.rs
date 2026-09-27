//! Validation feedback: when a view does not match its reference, only the
//! offending component block is regenerated (new attempt -> new seed), never
//! the whole asset.

use crate::plan::{hunyuan_request, VALIDATION_VIEWS};
use crate::spec::DesignSpec;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
pub struct Score {
    pub component: String,
    pub view: String,
    pub score: f64,
    #[serde(default)]
    pub note: String,
}

/// After this many attempts a component goes to a human instead of another
/// seed: the spec or its reference crop is the problem, not the dice.
pub const MAX_ATTEMPTS: u32 = 4;

/// `history` = earlier review records (reviews.jsonl) for this run.
pub fn decide(spec: &DesignSpec, level: &str, scores: &[Score], threshold: f64, history: &[Value]) -> Result<Value, String> {
    let mut worst: BTreeMap<&str, (&Score, usize)> = BTreeMap::new();
    for s in scores {
        let c = spec
            .component(&s.component)
            .ok_or_else(|| format!("unknown component \"{}\"", s.component))?;
        if !VALIDATION_VIEWS.contains(&s.view.as_str()) {
            return Err(format!("view \"{}\" must be one of {VALIDATION_VIEWS:?}", s.view));
        }
        if !(0.0..=1.0).contains(&s.score) {
            return Err(format!("score for {}/{} must be in [0,1]", s.component, s.view));
        }
        // A bad mirror means its source mesh is bad.
        let target = c.mirror_of.as_deref().unwrap_or(&c.id);
        let target = spec.component(target).map(|c| c.id.as_str()).unwrap_or(target);
        let entry = worst.entry(target).or_insert((s, 0));
        entry.1 += 1;
        if s.score < entry.0.score {
            entry.0 = s;
        }
    }

    let mut regenerate = Vec::new();
    let mut escalate = Vec::new();
    let mut keep = Vec::new();
    for (component, (s, _)) in &worst {
        if s.score >= threshold {
            keep.push(json!(component));
            continue;
        }
        let c = spec.component(component).expect("checked above");
        let previous = history
            .iter()
            .flat_map(|h| h["regenerate"].as_array().cloned().unwrap_or_default())
            .filter(|r| r["component"] == *component)
            .count() as u32;
        let attempt = previous + 1;
        let reason = json!({ "view": s.view, "score": s.score, "note": s.note });
        if attempt > MAX_ATTEMPTS || c.source != "hunyuan" {
            escalate.push(json!({ "component": component, "attempts": previous, "worst": reason,
                "why": if c.source != "hunyuan" { "not a Hunyuan component; fix procedural/texture step" } else { "attempt budget used up; revise spec or reference crop" } }));
        } else {
            regenerate.push(json!({ "component": component, "attempt": attempt, "worst": reason,
                "jobs": [format!("shape.{component}"), format!("clean.{component}")],
                "hunyuan": hunyuan_request(spec, Some(c), level, attempt) }));
        }
    }
    let rerun_downstream = !regenerate.is_empty();
    Ok(json!({
        "specId": spec.id,
        "level": level,
        "threshold": threshold,
        "accepted": regenerate.is_empty() && escalate.is_empty(),
        "keep": keep,
        "regenerate": regenerate,
        "escalate": escalate,
        "rerunAfter": if rerun_downstream { json!(["assembly", "paint.*", "bake", "validate.*", "card"]) } else { json!([]) },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::tests::sample;

    fn s(component: &str, view: &str, score: f64) -> Score {
        Score { component: component.into(), view: view.into(), score, note: String::new() }
    }

    #[test]
    fn only_failing_components_regenerate_and_mirrors_map_to_source() {
        let spec = sample();
        let out = decide(&spec, "hero", &[s("main_hull", "front", 0.9), s("engine_r", "top", 0.4)], 0.7, &[]).unwrap();
        assert_eq!(out["keep"], json!(["main_hull"]));
        assert_eq!(out["regenerate"][0]["component"], "engine_l");
        assert_eq!(out["regenerate"][0]["attempt"], 1);
        assert_eq!(out["accepted"], false);
    }

    #[test]
    fn attempts_accumulate_and_then_escalate() {
        let spec = sample();
        let prior = json!({ "regenerate": [{ "component": "engine_l" }] });
        let history = vec![prior; MAX_ATTEMPTS as usize];
        let out = decide(&spec, "hero", &[s("engine_l", "front", 0.1)], 0.7, &history).unwrap();
        assert!(out["regenerate"].as_array().unwrap().is_empty());
        assert_eq!(out["escalate"][0]["component"], "engine_l");
    }

    #[test]
    fn unknown_view_is_rejected() {
        assert!(decide(&sample(), "hero", &[s("main_hull", "rear", 0.5)], 0.7, &[]).is_err());
    }
}
