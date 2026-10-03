use super::{has_ext, text_of, ArtifactParser};
use crate::error::SemanticResult;
use crate::model::*;
use crate::text::slug;
use async_trait::async_trait;
use nexus_domain::types::RelationKind;
use serde_json::Value;
use std::collections::BTreeSet;

const NAME: &str = "CodeMetadataParser";

pub const CODE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "py", "go", "java", "kt", "cs", "cpp", "cc", "h", "hpp",
    "swift", "rb", "php", "vue", "svelte",
];

/// Line-based source metadata: imports, declared symbols, components,
/// tests, HTTP calls and variant props. No type checking, no execution.
pub struct CodeMetadataParser;

impl CodeMetadataParser {
    pub fn is_test_path(path: &str) -> bool {
        path.contains(".test.")
            || path.contains(".spec.")
            || path.contains("/tests/")
            || path.starts_with("tests/")
            || path.contains("_test.")
            || path
                .rsplit('/')
                .next()
                .is_some_and(|f| f.starts_with("test_"))
    }
}

fn ident(s: &str) -> String {
    s.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

fn quoted(s: &str) -> Option<String> {
    let start = s.find(['"', '\'', '`'])?;
    let q = s[start..].chars().next()?;
    let rest = &s[start + 1..];
    let end = rest.find(q)?;
    Some(rest[..end].to_string())
}

fn after<'a>(line: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes.iter().find_map(|p| line.strip_prefix(p))
}

#[async_trait]
impl ArtifactParser for CodeMetadataParser {
    fn name(&self) -> &'static str {
        NAME
    }

    fn supports(&self, artifact: &NexusArtifact) -> bool {
        has_ext(artifact, CODE_EXTENSIONS)
    }

    async fn inspect(&self, artifact: &NexusArtifact) -> SemanticResult<SemanticModel> {
        let text = text_of(artifact)?;
        let ext = artifact.extension().unwrap_or_default();
        let path = artifact.provenance.source.clone();
        let file_id = format!("file:{path}");
        let jsx = matches!(ext.as_str(), "tsx" | "jsx");
        let is_test_file = Self::is_test_path(&path);
        let mut m = SemanticModel::default();
        let mut symbols: BTreeSet<String> = BTreeSet::new();
        let mut modules: BTreeSet<String> = BTreeSet::new();
        let mut components: Vec<String> = Vec::new();
        let mut next_is_test = false;

        for (i, raw) in text.lines().enumerate() {
            let n = i as u32 + 1;
            let line = raw.trim();
            let at = || Basis::explicit(artifact.at(NAME, Some((n, n))));

            // Imports.
            let module = if let Some(rest) = after(line, &["use "]) {
                let root = rest.split("::").next().unwrap_or("").trim_end_matches(';');
                (!["crate", "self", "super"].contains(&root)).then(|| root.to_string())
            } else if line.starts_with("import ") || line.starts_with("export * from") {
                quoted(line.rsplit("from").next().unwrap_or(line)).or_else(|| {
                    line.strip_prefix("import ")
                        .map(|r| r.split([' ', ',', ';']).next().unwrap_or("").to_string())
                })
            } else if line.starts_with("} from") {
                quoted(line)
            } else if let Some(rest) = after(line, &["from "]) {
                rest.split_whitespace().next().map(str::to_string)
            } else if line.contains("require(") {
                quoted(line.split("require(").nth(1).unwrap_or(""))
            } else {
                None
            };
            if let Some(module) = module.filter(|m| !m.is_empty() && !m.starts_with('.')) {
                if modules.insert(module.clone()) {
                    m.dependencies.push(Dependency::new(
                        &file_id,
                        RelationKind::DependsOn,
                        &format!("module:{module}"),
                        at(),
                    ));
                }
            }

            // Declarations.
            if line.starts_with("#[test]") || line.starts_with("#[tokio::test]") {
                next_is_test = true;
                continue;
            }
            let decl = after(
                line,
                &[
                    "pub fn ",
                    "fn ",
                    "pub async fn ",
                    "async fn ",
                    "pub(crate) fn ",
                    "export default function ",
                    "export function ",
                    "export async function ",
                    "function ",
                    "def ",
                    "async def ",
                    "func ",
                ],
            )
            .map(|r| ("function", ident(r)))
            .or_else(|| {
                after(
                    line,
                    &[
                        "pub struct ",
                        "struct ",
                        "pub enum ",
                        "enum ",
                        "pub trait ",
                        "trait ",
                        "export class ",
                        "export default class ",
                        "class ",
                        "export interface ",
                        "interface ",
                        "export type ",
                        "type ",
                    ],
                )
                .map(|r| ("type", ident(r)))
            })
            .or_else(|| {
                after(line, &["export const ", "export let "]).map(|r| ("value", ident(r)))
            });
            if let Some((kind, name)) = decl.filter(|(_, n)| !n.is_empty()) {
                symbols.insert(name.clone());
                let test_name = next_is_test
                    || name.starts_with("test_")
                    || (kind == "function" && ext == "go" && name.starts_with("Test"));
                if test_name {
                    let tid = format!("test:{path}#{name}");
                    m.entities
                        .push(SemanticEntity::new(&tid, &name, "test", at()));
                    m.dependencies.push(Dependency::new(
                        &file_id,
                        RelationKind::TestedBy,
                        &tid,
                        at(),
                    ));
                } else if jsx
                    && kind != "type"
                    && name.chars().next().is_some_and(char::is_uppercase)
                {
                    let cid = slug(&name);
                    let mut c = SemanticEntity::new(
                        &cid,
                        &name,
                        "component",
                        Basis::inferred(
                            vec![artifact.at(NAME, Some((n, n)))],
                            "PascalCase function/const declared in a JSX file",
                        ),
                    );
                    c.role = Some(Role::Engineering);
                    m.entities.push(c);
                    m.dependencies.push(Dependency::new(
                        &cid,
                        RelationKind::ImplementedBy,
                        &file_id,
                        at(),
                    ));
                    components.push(cid);
                }
                next_is_test = false;
            }

            // JS/TS test blocks.
            if let Some(rest) = after(line, &["describe(", "it(", "test("]) {
                if let Some(name) = quoted(rest) {
                    let tid = format!("test:{path}#{}", slug(&name));
                    m.entities
                        .push(SemanticEntity::new(&tid, &name, "test", at()));
                    m.dependencies.push(Dependency::new(
                        &file_id,
                        RelationKind::TestedBy,
                        &tid,
                        at(),
                    ));
                }
            }

            // HTTP calls.
            let lower = line.to_ascii_lowercase();
            for (needle, method) in [
                ("fetch(", None),
                (".get(", Some("GET")),
                (".post(", Some("POST")),
                (".put(", Some("PUT")),
                (".patch(", Some("PATCH")),
                (".delete(", Some("DELETE")),
            ] {
                let Some(idx) = lower.find(needle) else {
                    continue;
                };
                let Some(url) = quoted(&line[idx + needle.len()..]) else {
                    continue;
                };
                if !url.starts_with('/') && !url.starts_with("http") {
                    continue;
                }
                let method = method.map(str::to_string).unwrap_or_else(|| {
                    ["POST", "PUT", "PATCH", "DELETE"]
                        .into_iter()
                        .find(|meth| {
                            line.contains(&format!("'{meth}'"))
                                || line.contains(&format!("\"{meth}\""))
                        })
                        .unwrap_or("GET")
                        .to_string()
                });
                m.interactions.push(Interaction::new(
                    "http",
                    Some(&format!("{method} {url}")),
                    None,
                    Basis::inferred(
                        vec![artifact.at(NAME, Some((n, n)))],
                        "string literal passed to an HTTP call",
                    ),
                ));
            }

            // `variant?: 'primary' | 'secondary'` in a component file.
            if jsx {
                if let Some((prop, values)) = line.split_once(':') {
                    let prop = prop.trim().trim_end_matches('?');
                    if prop == "variant" && values.contains('|') {
                        if let Some(owner) = components.last().cloned() {
                            for v in values.split('|').filter_map(quoted) {
                                let vid = format!("variant:{owner}/{}", slug(&v));
                                m.entities
                                    .push(SemanticEntity::new(&vid, &v, "variant", at()));
                                m.dependencies.push(Dependency::new(
                                    &owner,
                                    RelationKind::HasVariant,
                                    &vid,
                                    at(),
                                ));
                            }
                        }
                    }
                }
            }
        }

        let mut file = SemanticEntity::new(
            &file_id,
            &path,
            if is_test_file {
                "test-file"
            } else {
                "code-file"
            },
            Basis::explicit(artifact.at(NAME, None)),
        );
        file.artifact = Some(artifact.id.clone());
        file.attributes.insert("language".into(), ext.into());
        file.attributes
            .insert("lines".into(), Value::from(text.lines().count()));
        if !symbols.is_empty() {
            file.attributes.insert(
                "symbols".into(),
                symbols
                    .into_iter()
                    .map(Value::from)
                    .collect::<Vec<_>>()
                    .into(),
            );
        }
        m.entities.push(file);
        Ok(m)
    }
}
