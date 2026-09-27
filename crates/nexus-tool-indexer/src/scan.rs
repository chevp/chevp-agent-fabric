//! Reads the three real, existing source formats this indexer understands.
//! Nothing here writes anything — it only turns files under `source_root`
//! into plain structs for `tool.rs` to render and write.

use serde::Deserialize;
use std::collections::HashMap;
use std::io;
use std::path::Path;

// ------------------------------------------------------------------- labs

pub struct LabEntity {
    pub id: String,
    pub name: String,
    pub relative_path: String,
    pub class: Option<String>,
    pub derived_from: Option<String>,
}

/// Scans `icc-frost-lib/labs/<aN>-<slug>` directories. `labs/INDEX.md`, if
/// present, additionally supplies `class` and `derivedFrom` per lab id — it
/// is enrichment, not the source of truth for which labs exist.
pub fn scan_labs(source_root: &Path) -> io::Result<Vec<LabEntity>> {
    let labs_dir = source_root.join("icc-frost-lib").join("labs");
    let annotations = std::fs::read_to_string(labs_dir.join("INDEX.md"))
        .map(|text| parse_index_md(&text))
        .unwrap_or_default();

    let entries = match std::fs::read_dir(&labs_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };

    let mut labs = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let dir_name = entry.file_name().to_string_lossy().to_string();
        let Some((id, rest)) = parse_lab_dir_name(&dir_name) else {
            continue;
        };
        let (derived_from, class) = annotations.get(&id).cloned().unwrap_or((None, None));
        labs.push(LabEntity {
            id,
            name: rest,
            relative_path: format!("icc-frost-lib/labs/{dir_name}"),
            class,
            derived_from,
        });
    }
    labs.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(labs)
}

/// `a1-animal-lab.lab` -> `("a1", "animal-lab.lab")`; `a27-content.settlers4`
/// -> `("a27", "content.settlers4")` (the `.lab` suffix is not universal).
fn parse_lab_dir_name(name: &str) -> Option<(String, String)> {
    let (id_part, rest) = name.split_once('-')?;
    is_lab_id(id_part).then(|| (id_part.to_string(), rest.to_string()))
}

/// An `aN` id: `a`, then digits, then optional lowercase letters (`a1`,
/// `a12a`, `a27aa`).
fn is_lab_id(s: &str) -> bool {
    let Some(rest) = s.strip_prefix('a') else {
        return false;
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return false;
    }
    rest[digits.len()..].chars().all(|c| c.is_ascii_lowercase())
}

/// `| \`a2a\` | \`content.audio-lab.lab\` | \`a2\` | A* | 1 .mproj |` ->
/// `id -> (derivedFrom, class)`. Header/separator rows are filtered out
/// because their first cell is never a valid lab id.
fn parse_index_md(text: &str) -> HashMap<String, (Option<String>, Option<String>)> {
    let mut map = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            continue;
        }
        let cells: Vec<String> = line
            .trim_matches('|')
            .split('|')
            .map(|c| strip_backticks(c).to_string())
            .collect();
        if cells.len() < 4 {
            continue;
        }
        let id = cells[0].clone();
        if !is_lab_id(&id) {
            continue;
        }
        let derived_from = (!cells[2].is_empty()).then(|| cells[2].clone());
        let class = (!cells[3].is_empty()).then(|| cells[3].clone());
        map.insert(id, (derived_from, class));
    }
    map
}

fn strip_backticks(s: &str) -> &str {
    s.trim().trim_matches('`')
}

// ------------------------------------------------------------- moana kits

#[derive(Deserialize)]
struct KitsFile {
    #[serde(default)]
    kits: Vec<KitEntry>,
}

#[derive(Deserialize)]
struct KitEntry {
    name: String,
    folder: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    members: Vec<KitMember>,
}

#[derive(Deserialize, Clone)]
pub struct KitMember {
    pub lab: String,
}

pub struct MoanaKit {
    pub id: String,
    pub title: String,
    pub relative_path: String,
    pub members: Vec<KitMember>,
}

/// Reads `icc-frost-lib/moana/kits.json`. Each kit member names a lab id
/// directly (`kits.json` already speaks the `aN` scheme), so no id lookup is
/// needed to relate a kit to the labs it draws from.
pub fn scan_moana_kits(source_root: &Path) -> io::Result<Vec<MoanaKit>> {
    let path = source_root
        .join("icc-frost-lib")
        .join("moana")
        .join("kits.json");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let parsed: KitsFile = serde_json::from_str(&text)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    Ok(parsed
        .kits
        .into_iter()
        .map(|k| MoanaKit {
            id: format!("moana-{}", k.name),
            title: if k.title.is_empty() {
                k.name.clone()
            } else {
                k.title
            },
            relative_path: format!("icc-frost-lib/moana/{}", k.folder),
            members: k.members,
        })
        .collect())
}

// --------------------------------------------------------- .claude/skills

pub struct ClaudeSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub body: String,
    pub relative_path: String,
}

/// Scans `.claude/skills/<id>/SKILL.md`. That frontmatter only has
/// `name`/`description` — not a valid Nexus skill on its own (missing
/// `id`/`version`/`scope`) — so this hands back plain data for `tool.rs` to
/// wrap into one.
pub fn scan_claude_skills(source_root: &Path) -> io::Result<Vec<ClaudeSkill>> {
    let dir = source_root.join(".claude").join("skills");
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };

    let mut skills = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        let Ok(raw) = std::fs::read_to_string(entry.path().join("SKILL.md")) else {
            continue;
        };
        let Some((name, description, body)) = parse_claude_frontmatter(&raw) else {
            continue;
        };
        skills.push(ClaudeSkill {
            relative_path: format!(".claude/skills/{id}/SKILL.md"),
            id,
            name,
            description,
            body,
        });
    }
    skills.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(skills)
}

fn parse_claude_frontmatter(raw: &str) -> Option<(String, String, String)> {
    let rest = raw.strip_prefix("---")?;
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    let end = rest.find("\n---")?;
    let frontmatter = &rest[..end];
    let after = &rest[end + 4..];
    let body = after.strip_prefix('\n').unwrap_or(after).trim().to_string();

    #[derive(Deserialize)]
    struct Frontmatter {
        name: String,
        description: String,
    }
    let fm: Frontmatter = serde_yaml::from_str(frontmatter).ok()?;
    Some((fm.name, fm.description, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_lab_ids_with_letter_suffixes() {
        assert!(is_lab_id("a1"));
        assert!(is_lab_id("a12a"));
        assert!(is_lab_id("a27aa"));
        assert!(!is_lab_id("moana"));
        assert!(!is_lab_id("a"));
        assert!(!is_lab_id("ID"));
    }

    #[test]
    fn parses_index_md_table_rows() {
        let text = "\
| ID | Name (ohne ID) | Ableitung von | Klasse | Inhalt |
|---|---|---|---|---|
| `a1` | `content.animal-lab.lab` |  | A* | 1 .mproj |
| `a2a` | `content.audio-lab.lab` | `a2` | A* | 1 .mproj |
";
        let map = parse_index_md(text);
        assert_eq!(map.get("a1"), Some(&(None, Some("A*".to_string()))));
        assert_eq!(
            map.get("a2a"),
            Some(&(Some("a2".to_string()), Some("A*".to_string())))
        );
        assert!(!map.contains_key("ID"));
    }
}
