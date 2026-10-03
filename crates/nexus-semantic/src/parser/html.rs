use super::{has_ext, text_of, ArtifactParser};
use crate::error::SemanticResult;
use crate::model::*;
use crate::text::{line_of, slug};
use async_trait::async_trait;
use nexus_domain::types::RelationKind;
use std::collections::BTreeMap;

const NAME: &str = "HtmlParser";

/// HTML prototypes / design specs. Elements present in the markup are
/// explicit; an intent guessed from a button label is only a candidate.
pub struct HtmlParser;

struct Element {
    tag: String,
    attrs: BTreeMap<String, String>,
    text: String,
    line: u32,
}

fn parse_attrs(s: &str) -> BTreeMap<String, String> {
    let mut attrs = BTreeMap::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && (chars[i].is_whitespace() || chars[i] == '/') {
            i += 1;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '=' && chars[i] != '/' {
            i += 1;
        }
        if start == i {
            i += 1;
            continue;
        }
        let name: String = chars[start..i].iter().collect::<String>().to_lowercase();
        let mut value = String::new();
        if i < chars.len() && chars[i] == '=' {
            i += 1;
            if i < chars.len() && (chars[i] == '"' || chars[i] == '\'') {
                let q = chars[i];
                i += 1;
                while i < chars.len() && chars[i] != q {
                    value.push(chars[i]);
                    i += 1;
                }
                i += 1;
            } else {
                while i < chars.len() && !chars[i].is_whitespace() {
                    value.push(chars[i]);
                    i += 1;
                }
            }
        }
        attrs.insert(name, value);
    }
    attrs
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn elements(html: &str) -> Vec<Element> {
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(rel) = lower[pos..].find('<') {
        let start = pos + rel;
        let Some(end_rel) = lower[start..].find('>') else {
            break;
        };
        let end = start + end_rel;
        let inner = &html[start + 1..end];
        pos = end + 1;
        if inner.starts_with('/') || inner.starts_with('!') || inner.starts_with('?') {
            continue;
        }
        let tag_end = inner
            .find(|c: char| c.is_whitespace() || c == '/')
            .unwrap_or(inner.len());
        let tag = inner[..tag_end].to_lowercase();
        let attrs = parse_attrs(&inner[tag_end..]);
        let close = format!("</{tag}>");
        let text = if ["button", "a", "label", "h1", "h2", "h3", "option"].contains(&tag.as_str()) {
            lower[pos..]
                .find(&close)
                .map(|i| strip_tags(&html[pos..pos + i]))
                .unwrap_or_default()
        } else {
            String::new()
        };
        out.push(Element {
            tag,
            attrs,
            text,
            line: line_of(html, start),
        });
    }
    out
}

#[async_trait]
impl ArtifactParser for HtmlParser {
    fn name(&self) -> &'static str {
        NAME
    }

    fn supports(&self, artifact: &NexusArtifact) -> bool {
        has_ext(artifact, &["html", "htm"])
            || artifact.media_type() == Some("text/html")
            || (artifact.kind == ArtifactKind::DesignSpec
                && artifact
                    .text()
                    .is_some_and(|t| t.trim_start().starts_with('<')))
    }

    async fn inspect(&self, artifact: &NexusArtifact) -> SemanticResult<SemanticModel> {
        let html = text_of(artifact)?;
        let mut m = SemanticModel::default();
        let mut form: Option<String> = None;

        for el in elements(html) {
            let at = || Basis::explicit(artifact.at(NAME, Some((el.line, el.line))));
            let interesting = matches!(
                el.tag.as_str(),
                "button" | "form" | "input" | "select" | "textarea" | "a"
            ) || el.attrs.contains_key("data-component");
            if !interesting {
                continue;
            }
            let label = el
                .attrs
                .get("data-component")
                .or_else(|| el.attrs.get("aria-label"))
                .cloned()
                .filter(|s| !s.is_empty())
                .or_else(|| (!el.text.is_empty()).then(|| el.text.clone()))
                .or_else(|| el.attrs.get("name").cloned())
                .or_else(|| el.attrs.get("id").cloned())
                .unwrap_or_else(|| format!("{}-line-{}", el.tag, el.line));
            let id = el
                .attrs
                .get("data-component")
                .or_else(|| el.attrs.get("id"))
                .map(|s| slug(s))
                .unwrap_or_else(|| slug(&label));
            let kind = if el.attrs.contains_key("data-component") {
                "component"
            } else if el.tag == "input" || el.tag == "select" || el.tag == "textarea" {
                "field"
            } else if el.tag == "a" {
                "link"
            } else {
                el.tag.as_str()
            };
            let mut entity = SemanticEntity::new(&id, &label, kind, at());
            entity.role = Some(artifact.role.clone());
            entity
                .attributes
                .insert("tag".into(), el.tag.clone().into());
            m.entities.push(entity);

            if el.tag == "form" {
                form = Some(id.clone());
            } else if let Some(f) = &form {
                m.dependencies
                    .push(Dependency::new(f, RelationKind::Contains, &id, at()));
            }

            let mut states: Vec<String> = Vec::new();
            if let Some(s) = el.attrs.get("data-state") {
                states.push(s.clone());
            }
            if let Some(s) = el.attrs.get("data-states") {
                states.extend(
                    s.split([' ', ','])
                        .filter(|x| !x.is_empty())
                        .map(str::to_string),
                );
            }
            for s in &states {
                m.states.push(State::new(&id, s, at()));
            }
            let when = el
                .attrs
                .get("data-state")
                .map(|s| format!(" while {s}"))
                .unwrap_or_default();

            if el.attrs.contains_key("disabled") {
                m.constraints.push(Constraint::new(
                    Some(&id),
                    &format!("{label} is disabled{when}"),
                    ConstraintKind::Must,
                    at(),
                ));
            }
            if el.attrs.contains_key("required") {
                m.constraints.push(Constraint::new(
                    Some(&id),
                    &format!("{label} is required"),
                    ConstraintKind::Must,
                    at(),
                ));
            }
            for (k, v) in el.attrs.iter().filter(|(k, _)| k.starts_with("aria-")) {
                if k == "aria-label" {
                    continue;
                }
                m.constraints.push(Constraint::new(
                    Some(&id),
                    &format!("{label} has {k}=\"{v}\"{when}"),
                    ConstraintKind::Accessibility,
                    at(),
                ));
            }

            let action = el
                .attrs
                .get("data-action")
                .or_else(|| el.attrs.get("onclick"))
                .cloned();
            if el.tag == "button"
                && (el.attrs.get("type").map(String::as_str) == Some("submit") || action.is_some())
            {
                let effect = action.unwrap_or_else(|| "submit".to_string());
                m.interactions
                    .push(Interaction::new("click", Some(&id), Some(&effect), at()));
            }
            if el.tag == "a" {
                if let Some(href) = el.attrs.get("href") {
                    m.interactions.push(Interaction::new(
                        "click",
                        Some(&id),
                        Some(&format!("navigate {href}")),
                        at(),
                    ));
                }
            }
            if el.tag == "button" && !el.text.is_empty() {
                m.intent.push(Intent::new(
                    &el.text.to_lowercase(),
                    Basis::candidate(
                        artifact.at(NAME, Some((el.line, el.line))),
                        "intent guessed from a button label",
                    ),
                ));
            }
        }
        Ok(m)
    }
}
