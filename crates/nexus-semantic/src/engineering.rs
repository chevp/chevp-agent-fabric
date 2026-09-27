//! SKILL.md -> EngineeringContext.
//!
//! A skill body is read as structure, not as an opaque string: `##`
//! sections are classified by heading (see `classify`), and each bullet (or
//! paragraph, if a section has no bullets) becomes one `ContextItem` with
//! its source lines. Unrecognized sections are kept under `other`, never
//! dropped and never reinterpreted.

use crate::model::{Basis, Provenance};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextItem {
    pub text: String,
    #[serde(flatten)]
    pub basis: Basis,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EngineeringContext {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub purpose: Vec<ContextItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<ContextItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<ContextItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conventions: Vec<ContextItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forbidden: Vec<ContextItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<ContextItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<ContextItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<ContextItem>,
    /// Sections whose heading is not one of the above, keyed by heading.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub other: BTreeMap<String, Vec<ContextItem>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind {
    Purpose,
    Capabilities,
    Constraints,
    Conventions,
    Forbidden,
    Inputs,
    Outputs,
    Examples,
    Other,
}

/// Heading -> section. `Rules`/`Requirements` count as constraints and
/// `Guidelines` as conventions; everything else unknown is `Other`.
pub fn classify(heading: &str) -> SectionKind {
    let h = heading.trim().to_lowercase();
    let h = h.trim_end_matches(':');
    match h {
        "purpose" | "goal" | "goals" | "intent" => SectionKind::Purpose,
        "capabilities" | "capability" | "features" => SectionKind::Capabilities,
        "constraints" | "constraint" | "rules" | "requirements" | "must" => {
            SectionKind::Constraints
        }
        "conventions" | "convention" | "guidelines" | "style" => SectionKind::Conventions,
        "forbidden" | "prohibited" | "do not" | "don't" | "never" | "anti-patterns" => {
            SectionKind::Forbidden
        }
        "inputs" | "input" => SectionKind::Inputs,
        "outputs" | "output" => SectionKind::Outputs,
        "examples" | "example" => SectionKind::Examples,
        _ => SectionKind::Other,
    }
}

#[derive(Debug, Clone)]
pub struct Section {
    pub heading: String,
    pub level: usize,
    pub line: u32,
    /// (text, first line, last line), 1-based within the parsed text.
    pub items: Vec<(String, u32, u32)>,
}

/// Splits Markdown into `#`/`##`/`###` sections with bullet or paragraph items.
/// `line_offset` is added to every line number (e.g. frontmatter length).
pub fn parse_sections(markdown: &str, line_offset: u32) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let mut current = Section {
        heading: String::new(),
        level: 0,
        line: 1 + line_offset,
        items: Vec::new(),
    };
    let mut bullets: Vec<(String, u32, u32)> = Vec::new();
    let mut paragraphs: Vec<(String, u32, u32)> = Vec::new();
    let mut para: Option<(String, u32, u32)> = None;
    let mut fence: Option<(String, u32)> = None;
    let mut in_bullet = false;

    fn flush(
        current: &mut Section,
        bullets: &mut Vec<(String, u32, u32)>,
        paragraphs: &mut Vec<(String, u32, u32)>,
        para: &mut Option<(String, u32, u32)>,
    ) {
        if let Some(p) = para.take() {
            paragraphs.push(p);
        }
        current.items = if bullets.is_empty() {
            std::mem::take(paragraphs)
        } else {
            paragraphs.clear();
            std::mem::take(bullets)
        };
    }

    for (i, raw) in markdown.lines().enumerate() {
        let n = i as u32 + 1 + line_offset;
        let line = raw.trim_end();
        let trimmed = line.trim_start();

        if trimmed.starts_with("```") {
            match fence.take() {
                Some((mut code, start)) => {
                    code.push_str(line);
                    paragraphs.push((code, start, n));
                }
                None => {
                    if let Some(p) = para.take() {
                        paragraphs.push(p);
                    }
                    in_bullet = false;
                    fence = Some((format!("{line}\n"), n));
                }
            }
            continue;
        }
        if let Some((code, _)) = fence.as_mut() {
            code.push_str(line);
            code.push('\n');
            continue;
        }

        let hashes = trimmed.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
            flush(&mut current, &mut bullets, &mut paragraphs, &mut para);
            if !current.heading.is_empty() || !current.items.is_empty() {
                sections.push(current);
            }
            current = Section {
                heading: trimmed[hashes..].trim().to_string(),
                level: hashes,
                line: n,
                items: Vec::new(),
            };
            in_bullet = false;
            continue;
        }

        let bullet = ["- ", "* ", "+ "]
            .iter()
            .find_map(|m| trimmed.strip_prefix(m))
            .or_else(|| {
                let digits = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
                (digits > 0)
                    .then(|| trimmed[digits..].strip_prefix(". "))
                    .flatten()
            });
        let indented = line.starts_with("  ") || line.starts_with('\t');

        if let Some(text) = bullet.filter(|_| !indented || !in_bullet) {
            if let Some(p) = para.take() {
                paragraphs.push(p);
            }
            bullets.push((text.trim().to_string(), n, n));
            in_bullet = true;
        } else if trimmed.is_empty() {
            if let Some(p) = para.take() {
                paragraphs.push(p);
            }
        } else if in_bullet && (indented || bullet.is_some()) {
            if let Some(last) = bullets.last_mut() {
                last.0.push(' ');
                last.0.push_str(bullet.unwrap_or(trimmed).trim());
                last.2 = n;
            }
        } else {
            in_bullet = false;
            match para.as_mut() {
                Some(p) => {
                    p.0.push(' ');
                    p.0.push_str(trimmed);
                    p.2 = n;
                }
                None => para = Some((trimmed.to_string(), n, n)),
            }
        }
    }
    if let Some((code, start)) = fence.take() {
        paragraphs.push((code, start, start));
    }
    flush(&mut current, &mut bullets, &mut paragraphs, &mut para);
    if !current.heading.is_empty() || !current.items.is_empty() {
        sections.push(current);
    }
    sections
}

/// Splits `---` YAML frontmatter off a Markdown file. Returns
/// (frontmatter, body, number of lines before the body).
pub fn split_frontmatter(raw: &str) -> (Option<&str>, &str, u32) {
    let Some(rest) = raw.strip_prefix("---") else {
        return (None, raw, 0);
    };
    let rest = rest.strip_prefix("\r\n").or_else(|| rest.strip_prefix('\n')).unwrap_or(rest);
    let Some(end) = rest.find("\n---") else {
        return (None, raw, 0);
    };
    let frontmatter = &rest[..end];
    let after = &rest[end + 4..];
    let body = after
        .strip_prefix("\r\n")
        .or_else(|| after.strip_prefix('\n'))
        .unwrap_or(after);
    let offset = raw[..raw.len() - body.len()].matches('\n').count() as u32;
    (Some(frontmatter), body, offset)
}

impl EngineeringContext {
    /// Parses a Markdown body. `provenance` is the template for every item's
    /// provenance; line numbers are filled in per item.
    pub fn from_markdown(body: &str, line_offset: u32, provenance: &Provenance) -> Self {
        let mut ctx = EngineeringContext::default();
        for section in parse_sections(body, line_offset) {
            let items: Vec<ContextItem> = section
                .items
                .iter()
                .map(|(text, start, end)| ContextItem {
                    text: text.clone(),
                    basis: Basis::explicit(Provenance {
                        line_start: Some(*start),
                        line_end: Some(*end),
                        ..provenance.clone()
                    }),
                })
                .collect();
            if items.is_empty() {
                continue;
            }
            let target = match classify(&section.heading) {
                SectionKind::Purpose => &mut ctx.purpose,
                SectionKind::Capabilities => &mut ctx.capabilities,
                SectionKind::Constraints => &mut ctx.constraints,
                SectionKind::Conventions => &mut ctx.conventions,
                SectionKind::Forbidden => &mut ctx.forbidden,
                SectionKind::Inputs => &mut ctx.inputs,
                SectionKind::Outputs => &mut ctx.outputs,
                SectionKind::Examples => &mut ctx.examples,
                SectionKind::Other if section.level <= 1 => continue,
                SectionKind::Other => ctx.other.entry(section.heading.clone()).or_default(),
            };
            target.extend(items);
        }
        ctx
    }

    /// Union of several contexts (e.g. every skill a translation targets).
    pub fn merge(contexts: impl IntoIterator<Item = EngineeringContext>) -> Self {
        let mut out = EngineeringContext::default();
        for c in contexts {
            out.skills.extend(c.skills);
            out.purpose.extend(c.purpose);
            out.capabilities.extend(c.capabilities);
            out.constraints.extend(c.constraints);
            out.conventions.extend(c.conventions);
            out.forbidden.extend(c.forbidden);
            out.inputs.extend(c.inputs);
            out.outputs.extend(c.outputs);
            out.examples.extend(c.examples);
            for (k, v) in c.other {
                out.other.entry(k).or_default().extend(v);
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.purpose.is_empty()
            && self.capabilities.is_empty()
            && self.constraints.is_empty()
            && self.conventions.is_empty()
            && self.forbidden.is_empty()
            && self.inputs.is_empty()
            && self.outputs.is_empty()
            && self.examples.is_empty()
            && self.other.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prov() -> Provenance {
        Provenance {
            source: "skills/react/skill.md".to_string(),
            line_start: None,
            line_end: None,
            commit: None,
            extraction: "SkillParser".to_string(),
            artifact: None,
        }
    }

    const SKILL: &str = "---\nid: react\nname: React\nversion: 1.0.0\nscope: global\ndescription: React frontend\n---\n\n# React Frontend\n\n## Purpose\n\nBuild the customer-facing UI.\n\n## Capabilities\n\n- Functional React components\n- Server state via TanStack Query\n\n## Constraints\n\n- All API calls must use the generated API client,\n  never raw fetch.\n\n## Conventions\n\n- Use the existing design system.\n\n## Forbidden\n\n- Do not introduce new component libraries.\n\n## Examples\n\n```tsx\nconst q = useQuery(...)\n```\n\n## Notes\n\nSomething else.\n";

    #[test]
    fn skill_md_becomes_an_engineering_context_with_line_provenance() {
        let (_, body, offset) = split_frontmatter(SKILL);
        let ctx = EngineeringContext::from_markdown(body, offset, &prov());

        assert_eq!(ctx.purpose[0].text, "Build the customer-facing UI.");
        assert_eq!(ctx.capabilities.len(), 2);
        assert_eq!(
            ctx.constraints[0].text,
            "All API calls must use the generated API client, never raw fetch."
        );
        let p = &ctx.constraints[0].basis.provenance[0];
        assert_eq!((p.line_start, p.line_end), (Some(22), Some(23)));
        assert_eq!(ctx.conventions[0].text, "Use the existing design system.");
        assert_eq!(ctx.forbidden[0].text, "Do not introduce new component libraries.");
        assert!(ctx.examples[0].text.contains("useQuery"));
        assert_eq!(ctx.other["Notes"][0].text, "Something else.");
        assert!(ctx.inputs.is_empty() && ctx.outputs.is_empty());
    }

    #[test]
    fn missing_sections_stay_empty() {
        let ctx = EngineeringContext::from_markdown("# Title\n\nJust prose.\n", 0, &prov());
        assert!(ctx.is_empty());
    }
}
