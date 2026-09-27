//! Small, dependency-free text helpers shared by the parsers.

use std::collections::BTreeSet;
use std::path::Path;

/// `CheckoutButton`, `checkout button`, `checkout_button` -> `checkout-button`.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in s.chars() {
        if c.is_alphanumeric() {
            if let Some(p) = prev {
                let boundary = (p.is_lowercase() || p.is_ascii_digit()) && c.is_uppercase();
                if boundary && !out.ends_with('-') {
                    out.push('-');
                }
            }
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
        prev = Some(c);
    }
    out.trim_matches('-').to_string()
}

/// `checkout-button` -> `CheckoutButton`.
pub fn pascal(s: &str) -> String {
    slug(s)
        .split('-')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "of", "to", "in", "on", "is", "be", "it", "for", "with", "while",
    "must", "should", "shall", "not", "never", "no", "can", "cannot", "do", "does", "don", "t",
    "user", "users",
];

/// Content words, lowercased, without stopwords and negations.
pub fn content_words(s: &str) -> BTreeSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !STOPWORDS.contains(w))
        .map(|w| w.trim_end_matches('s').to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

/// Jaccard similarity of the content words of two statements.
pub fn similarity(a: &str, b: &str) -> f32 {
    let (a, b) = (content_words(a), content_words(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(&b).count() as f32;
    let union = a.union(&b).count() as f32;
    inter / union
}

/// 1-based line number of a byte offset.
pub fn line_of(text: &str, offset: usize) -> u32 {
    text[..offset.min(text.len())].matches('\n').count() as u32 + 1
}

/// 1-based line of the first line containing `needle`.
pub fn find_line(text: &str, needle: &str) -> Option<u32> {
    text.lines()
        .position(|l| l.contains(needle))
        .map(|i| i as u32 + 1)
}

/// Repo-relative path with forward slashes; absolute (forward slashes) if
/// `path` is outside `root`.
pub fn rel_path(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let s = rel.to_string_lossy().replace('\\', "/");
    s.trim_start_matches("./").to_string()
}

/// Identifiers written in backticks that look like references
/// (`checkout-button`, `PaymentFlow`), with their 1-based line.
pub fn backtick_refs(text: &str) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for (i, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let mut parts = line.split('`');
        parts.next();
        while let (Some(inner), Some(_)) = (parts.next(), parts.next()) {
            let looks_like_ref = !inner.is_empty()
                && inner.len() <= 64
                && inner
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                && (inner.contains('-') || inner.chars().next().is_some_and(|c| c.is_uppercase()));
            if looks_like_ref {
                out.push((slug(inner), i as u32 + 1));
            }
        }
    }
    out
}

/// Stable 64-bit FNV-1a hash, hex encoded (first `len` chars).
pub fn short_hash(s: &str, len: usize) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")[..len.min(16)].to_string()
}

/// Unique-enough suffix for ids created within the same second.
pub fn nonce() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    short_hash(&format!("{nanos}-{}", COUNTER.fetch_add(1, Ordering::Relaxed)), 6)
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_and_pascal_cases() {
        assert_eq!(slug("CheckoutButton"), "checkout-button");
        assert_eq!(slug("Save profile!"), "save-profile");
        assert_eq!(slug("checkout_button"), "checkout-button");
        assert_eq!(pascal("checkout-button"), "CheckoutButton");
    }

    #[test]
    fn finds_backtick_references_outside_code_fences() {
        let refs = backtick_refs("Use `checkout-button` and `x`.\n```\n`ignored-ref`\n```\n`PaymentFlow`");
        assert_eq!(
            refs,
            vec![("checkout-button".to_string(), 1), ("payment-flow".to_string(), 5)]
        );
    }

    #[test]
    fn similarity_ignores_negation_and_stopwords() {
        assert!(similarity("must not submit twice", "submit twice") > 0.99);
    }
}
