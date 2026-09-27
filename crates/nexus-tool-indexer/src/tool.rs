use crate::scan::{scan_claude_skills, scan_labs, scan_moana_kits, LabEntity, MoanaKit};
use crate::write::{write_generated, FileOutcome, MARKER};
use nexus_tools::{Tool, ToolContext, ToolError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

pub struct IndexProjectTool;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    project_id: String,
    source_root: String,
    #[serde(default)]
    dry_run: bool,
}

impl Tool for IndexProjectTool {
    fn name(&self) -> &str {
        "index_project"
    }

    fn description(&self) -> &str {
        "Scans a project's real source tree (icc-frost-lib labs, moana kits, .claude/skills) \
         and writes/updates matching graph entities, relations and imported skill files under \
         the project's Nexus directory. Every generated file carries a marker: re-running \
         refreshes files it created and leaves hand-authored files at the same path untouched \
         (reported as skipped). Set dryRun to preview without writing."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["projectId", "sourceRoot"],
            "additionalProperties": false,
            "properties": {
                "projectId": { "type": "string", "minLength": 1 },
                "sourceRoot": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Absolute path to the real repository to scan, e.g. G:/ft (not the .nexus root)."
                },
                "dryRun": { "type": "boolean", "default": false }
            }
        })
    }

    fn call(&self, ctx: &ToolContext<'_>, input: Value) -> Result<Value, ToolError> {
        let input: Input = serde_json::from_value(input)
            .map_err(|err| ToolError::InvalidInput(err.to_string()))?;

        let project = ctx.domain.projects.require_project(&input.project_id)?;
        let source_root = PathBuf::from(&input.source_root);
        if !source_root.is_dir() {
            return Err(ToolError::InvalidInput(format!(
                "sourceRoot \"{}\" is not a directory",
                source_root.display()
            )));
        }

        let entities_dir = project.path.join("graph").join("entities");
        let relations_dir = project.path.join("graph").join("relations");
        let skills_dir = project.path.join("skills");

        let mut outcomes: Vec<FileOutcome> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        let mut written_relations: HashSet<(String, String)> = HashSet::new();

        let labs = scan_labs(&source_root).map_err(|err| io_err("icc-frost-lib/labs", &err))?;
        let lab_ids: HashSet<&str> = labs.iter().map(|l| l.id.as_str()).collect();

        for lab in &labs {
            let path = entities_dir.join(format!("{}.yaml", lab.id));
            outcomes.push(wg(&path, &render_lab_entity(lab), input.dry_run)?);

            if let Some(parent) = &lab.derived_from {
                if !lab_ids.contains(parent.as_str()) {
                    warnings.push(format!(
                        "lab \"{}\" declares derivedFrom \"{parent}\", which was not found among scanned labs",
                        lab.id
                    ));
                }
                if written_relations.insert((lab.id.clone(), parent.clone())) {
                    let rel_path =
                        relations_dir.join(format!("{}-depends-on-{}.yaml", lab.id, parent));
                    let content = render_relation(&lab.id, "depends-on", parent);
                    outcomes.push(wg(&rel_path, &content, input.dry_run)?);
                }
            }
        }

        let kits =
            scan_moana_kits(&source_root).map_err(|err| io_err("icc-frost-lib/moana", &err))?;
        for kit in &kits {
            let path = entities_dir.join(format!("{}.yaml", kit.id));
            outcomes.push(wg(&path, &render_kit_entity(kit), input.dry_run)?);

            let mut members: Vec<&str> = kit.members.iter().map(|m| m.lab.as_str()).collect();
            members.sort_unstable();
            members.dedup();
            for lab_id in members {
                if !lab_ids.contains(lab_id) {
                    warnings.push(format!(
                        "kit \"{}\" references lab \"{lab_id}\", which was not found among scanned labs",
                        kit.id
                    ));
                }
                if written_relations.insert((kit.id.clone(), lab_id.to_string())) {
                    let rel_path = relations_dir.join(format!("{}-uses-{}.yaml", kit.id, lab_id));
                    let content = render_relation(&kit.id, "uses", lab_id);
                    outcomes.push(wg(&rel_path, &content, input.dry_run)?);
                }
            }
        }

        let skills =
            scan_claude_skills(&source_root).map_err(|err| io_err(".claude/skills", &err))?;
        for skill in &skills {
            let path = skills_dir.join(&skill.id).join("skill.md");
            outcomes.push(wg(&path, &render_skill_md(skill), input.dry_run)?);
        }

        let mut summary: BTreeMap<&'static str, usize> = BTreeMap::new();
        for o in &outcomes {
            *summary.entry(o.status).or_default() += 1;
        }

        Ok(json!({
            "projectId": input.project_id,
            "sourceRoot": input.source_root,
            "dryRun": input.dry_run,
            "labsFound": labs.len(),
            "moanaKitsFound": kits.len(),
            "claudeSkillsFound": skills.len(),
            "summary": summary,
            "files": outcomes,
            "warnings": warnings,
        }))
    }
}

fn io_err(context: &str, err: &std::io::Error) -> ToolError {
    ToolError::Other(format!("failed scanning {context}: {err}"))
}

fn wg(path: &Path, content: &str, dry_run: bool) -> Result<FileOutcome, ToolError> {
    write_generated(path, content, dry_run)
        .map_err(|err| ToolError::Other(format!("I/O error at {}: {err}", path.display())))
}

fn render_lab_entity(lab: &LabEntity) -> String {
    #[derive(Serialize)]
    struct Doc<'a> {
        id: &'a str,
        #[serde(rename = "type")]
        kind: &'a str,
        name: &'a str,
        path: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        class: Option<&'a str>,
        #[serde(rename = "derivedFrom", skip_serializing_if = "Option::is_none")]
        derived_from: Option<&'a str>,
    }
    let doc = Doc {
        id: &lab.id,
        kind: "lab",
        name: &lab.name,
        path: &lab.relative_path,
        class: lab.class.as_deref(),
        derived_from: lab.derived_from.as_deref(),
    };
    format!(
        "# {MARKER}\n{}",
        serde_yaml::to_string(&doc).unwrap_or_default()
    )
}

fn render_kit_entity(kit: &MoanaKit) -> String {
    #[derive(Serialize)]
    struct Doc<'a> {
        id: &'a str,
        #[serde(rename = "type")]
        kind: &'a str,
        name: &'a str,
        path: &'a str,
        #[serde(rename = "memberCount")]
        member_count: usize,
    }
    let doc = Doc {
        id: &kit.id,
        kind: "moana-kit",
        name: &kit.title,
        path: &kit.relative_path,
        member_count: kit.members.len(),
    };
    format!(
        "# {MARKER}\n{}",
        serde_yaml::to_string(&doc).unwrap_or_default()
    )
}

fn render_relation(from: &str, relation: &str, to: &str) -> String {
    #[derive(Serialize)]
    struct Doc<'a> {
        from: &'a str,
        relation: &'a str,
        to: &'a str,
    }
    let doc = Doc { from, relation, to };
    format!(
        "# {MARKER}\n{}",
        serde_yaml::to_string(&doc).unwrap_or_default()
    )
}

fn render_skill_md(skill: &crate::scan::ClaudeSkill) -> String {
    #[derive(Serialize)]
    struct Frontmatter<'a> {
        id: &'a str,
        name: &'a str,
        version: &'a str,
        scope: &'a str,
        description: &'a str,
        tags: Vec<&'a str>,
    }
    let fm = Frontmatter {
        id: &skill.id,
        name: &skill.name,
        version: "1.0.0",
        scope: "project",
        description: &skill.description,
        tags: vec!["imported-claude-skill"],
    };
    let yaml = serde_yaml::to_string(&fm).unwrap_or_default();
    format!(
        "---\n{yaml}---\n\n<!-- {MARKER}; source: {} -->\n\n{}\n",
        skill.relative_path, skill.body
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_domain::NexusDomain;
    use std::fs;
    use tempfile::TempDir;

    struct Fixture {
        _dir: TempDir,
        source_root: PathBuf,
        domain: NexusDomain,
    }

    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    impl Fixture {
        fn new() -> Self {
            let dir = TempDir::new().unwrap();
            let source_root = dir.path().join("source");
            let nexus_root = dir.path().join("nexus");

            write(
                &source_root.join("icc-frost-lib/labs/a1-animal-lab.lab/animal-lab.mproj"),
                "",
            );
            write(
                &source_root.join("icc-frost-lib/labs/a1a-animal-lab-sub/sub.mproj"),
                "",
            );
            write(
                &source_root.join("icc-frost-lib/labs/INDEX.md"),
                "| ID | Name (ohne ID) | Ableitung von | Klasse | Inhalt |\n\
                 |---|---|---|---|---|\n\
                 | `a1` | `content.animal-lab.lab` |  | A* | 1 .mproj |\n\
                 | `a1a` | `content.animal-lab-sub.lab` | `a1` | A | 1 .mproj |\n",
            );
            write(
                &source_root.join("icc-frost-lib/moana/kits.json"),
                r#"{ "kits": [ { "name": "animals", "folder": "animals", "title": "Animals Kit",
                     "members": [ { "lab": "a1", "descriptor": "x.mproj" } ] } ] }"#,
            );
            write(
                &source_root.join(".claude/skills/demo-skill/SKILL.md"),
                "---\nname: demo-skill\ndescription: A demo skill.\n---\n\nBody text.\n",
            );

            write(
                &nexus_root.join("projects/demo/project.yaml"),
                "id: demo\nname: Demo\nversion: 1\n",
            );
            fs::create_dir_all(nexus_root.join("global")).unwrap();

            let domain = NexusDomain::from_repo_root(&nexus_root);
            Self {
                _dir: dir,
                source_root,
                domain,
            }
        }

        fn run(&self, dry_run: bool) -> Value {
            let ctx = ToolContext {
                domain: &self.domain,
            };
            IndexProjectTool
                .call(
                    &ctx,
                    json!({
                        "projectId": "demo",
                        "sourceRoot": self.source_root.to_string_lossy(),
                        "dryRun": dry_run,
                    }),
                )
                .unwrap()
        }
    }

    #[test]
    fn first_run_creates_entities_relations_and_skills() {
        let f = Fixture::new();
        let out = f.run(false);

        assert_eq!(out["labsFound"], 2);
        assert_eq!(out["moanaKitsFound"], 1);
        assert_eq!(out["claudeSkillsFound"], 1);
        // a1, a1a entities + depends-on relation + moana kit entity + uses relation + skill file
        assert_eq!(out["summary"]["created"], 6);

        let lab_entity =
            fs::read_to_string(f.domain.projects_root.join("demo/graph/entities/a1a.yaml"))
                .unwrap();
        assert!(lab_entity.contains("derivedFrom: a1"));
        assert!(lab_entity.contains("class: A"));

        let skill = fs::read_to_string(
            f.domain
                .projects_root
                .join("demo/skills/demo-skill/skill.md"),
        )
        .unwrap();
        assert!(skill.contains("scope: project"));
        assert!(skill.contains("Body text."));

        let skills = f.domain.skills.list_skills(Some("demo")).unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].description, "A demo skill.");

        let entities = f.domain.graph.list_entities("demo").unwrap();
        assert_eq!(entities.len(), 3);
        let relations = f.domain.graph.list_relations("demo").unwrap();
        assert_eq!(relations.len(), 2);
    }

    #[test]
    fn second_run_is_a_no_op() {
        let f = Fixture::new();
        f.run(false);
        let out = f.run(false);
        assert_eq!(out["summary"]["unchanged"], 6);
        assert!(out["summary"].get("created").is_none());
    }

    #[test]
    fn dry_run_writes_nothing() {
        let f = Fixture::new();
        let out = f.run(true);
        assert_eq!(out["summary"]["would_write"], 6);
        assert!(!f
            .domain
            .projects_root
            .join("demo/graph/entities/a1.yaml")
            .exists());
    }

    #[test]
    fn hand_authored_files_are_never_overwritten() {
        let f = Fixture::new();
        let hand_written = f.domain.projects_root.join("demo/graph/entities/a1.yaml");
        write(&hand_written, "id: a1\ntype: lab\nname: Hand Written\n");

        let out = f.run(false);
        let files = out["files"].as_array().unwrap();
        let a1 = files
            .iter()
            .find(|f| f["path"].as_str().unwrap().ends_with("a1.yaml"))
            .unwrap();
        assert_eq!(a1["status"], "skipped");
        assert_eq!(
            fs::read_to_string(&hand_written).unwrap(),
            "id: a1\ntype: lab\nname: Hand Written\n"
        );
    }
}
