//! Deterministic confidence rules. Every value produced here is a pure
//! function of evidence type and the number of distinct sources.
//!
//! | evidence  | sources | value |
//! |-----------|---------|-------|
//! | explicit  | any     | 1.0   |
//! | inferred  | 1       | 0.5   |
//! | inferred  | 2       | 0.7   |
//! | inferred  | >= 3    | 0.8   |
//! | candidate | 1       | 0.3   |
//! | unknown   | -       | 0.0   |

use crate::model::{distinct_sources, Basis, Confidence, Evidence};

pub const EXPLICIT: f32 = 1.0;
pub const CANDIDATE: f32 = 0.3;
pub const INFERRED_MAX: f32 = 0.8;

pub fn explicit() -> Confidence {
    Confidence {
        value: EXPLICIT,
        reason: Some("stated explicitly in the source".to_string()),
    }
}

pub fn inferred_value(sources: usize) -> f32 {
    match sources {
        0 | 1 => 0.5,
        2 => 0.7,
        _ => INFERRED_MAX,
    }
}

pub fn inferred(sources: usize, reason: &str) -> Confidence {
    Confidence {
        value: inferred_value(sources),
        reason: Some(format!("{reason} ({} source(s))", sources.max(1))),
    }
}

pub fn candidate(reason: &str) -> Confidence {
    Confidence {
        value: CANDIDATE,
        reason: Some(reason.to_string()),
    }
}

pub fn unknown(reason: &str) -> Confidence {
    Confidence {
        value: 0.0,
        reason: Some(reason.to_string()),
    }
}

/// Merges the bases of one statement observed several times.
///
/// Explicit wins. Otherwise inferred confidence grows with distinct sources,
/// and a candidate seen in two or more independent sources becomes inferred.
/// Nothing is ever promoted to explicit without an explicit source.
pub fn merge(bases: &[&Basis]) -> Basis {
    let mut provenance: Vec<_> = bases.iter().flat_map(|b| b.provenance.clone()).collect();
    provenance.sort();
    provenance.dedup();
    let sources = distinct_sources(&provenance);
    let strongest = bases
        .iter()
        .map(|b| b.evidence)
        .min()
        .unwrap_or(Evidence::Unknown);

    let (evidence, confidence) = match strongest {
        Evidence::Explicit => (Evidence::Explicit, explicit()),
        Evidence::Inferred => (
            Evidence::Inferred,
            inferred(sources, &first_reason(bases, Evidence::Inferred)),
        ),
        Evidence::Candidate if sources >= 2 => (
            Evidence::Inferred,
            inferred(
                sources,
                "corroborated by independent candidate observations",
            ),
        ),
        Evidence::Candidate => (
            Evidence::Candidate,
            candidate(&first_reason(bases, Evidence::Candidate)),
        ),
        Evidence::Unknown => (Evidence::Unknown, unknown("no evidence")),
    };
    Basis {
        evidence,
        confidence,
        provenance,
    }
}

fn first_reason(bases: &[&Basis], evidence: Evidence) -> String {
    bases
        .iter()
        .find(|b| b.evidence == evidence)
        .and_then(|b| b.confidence.reason.clone())
        .map(|r| match r.rfind(" (") {
            Some(i) if r.ends_with("source(s))") => r[..i].to_string(),
            _ => r,
        })
        .unwrap_or_else(|| "derived".to_string())
}

/// Model-level confidence: mean of all statement confidences.
pub fn aggregate(bases: &[&Basis]) -> Confidence {
    if bases.is_empty() {
        return unknown("no statements extracted");
    }
    let sum: f32 = bases.iter().map(|b| b.confidence.value).sum();
    let count = |e: Evidence| bases.iter().filter(|b| b.evidence == e).count();
    Confidence {
        value: round(sum / bases.len() as f32),
        reason: Some(format!(
            "mean of {} statements: {} explicit, {} inferred, {} candidate, {} unknown",
            bases.len(),
            count(Evidence::Explicit),
            count(Evidence::Inferred),
            count(Evidence::Candidate),
            count(Evidence::Unknown)
        )),
    }
}

pub fn round(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Provenance;

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
    fn inferred_confidence_grows_with_sources_and_is_capped() {
        assert_eq!(inferred_value(1), 0.5);
        assert_eq!(inferred_value(2), 0.7);
        assert_eq!(inferred_value(9), 0.8);
    }

    #[test]
    fn merging_never_promotes_inferred_to_explicit() {
        let a = Basis::inferred(vec![prov("a.md")], "rule");
        let b = Basis::inferred(vec![prov("b.md")], "rule");
        let merged = merge(&[&a, &b]);
        assert_eq!(merged.evidence, Evidence::Inferred);
        assert_eq!(merged.confidence.value, 0.7);
    }

    #[test]
    fn corroborated_candidates_become_inferred_single_ones_do_not() {
        let a = Basis::candidate(prov("a.md"), "mention");
        let b = Basis::candidate(prov("b.md"), "mention");
        assert_eq!(merge(&[&a]).evidence, Evidence::Candidate);
        assert_eq!(merge(&[&a, &b]).evidence, Evidence::Inferred);
    }

    #[test]
    fn explicit_wins_when_present() {
        let a = Basis::candidate(prov("a.md"), "mention");
        let b = Basis::explicit(prov("b.yaml"));
        let merged = merge(&[&a, &b]);
        assert_eq!(merged.evidence, Evidence::Explicit);
        assert_eq!(merged.provenance.len(), 2);
    }
}
