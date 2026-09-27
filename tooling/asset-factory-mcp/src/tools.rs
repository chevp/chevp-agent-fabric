//! MCP tool table. Every tool takes either `specId` (loaded from
//! `<root>/specs/<id>.json`) or an inline `spec` object.

use crate::materials::material_groups;
use crate::plan::{self, LEVELS};
use crate::review::{self, Score};
use crate::spec::{self, DesignSpec};
use crate::store::Store;
use serde_json::{json, Value};

pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: fn() -> Value,
    pub call: fn(&Store, &Value) -> Result<Value, String>,
}

fn spec_schema(extra: Value) -> Value {
    let mut props = json!({
        "specId": { "type": "string", "description": "file stem under <root>/specs" },
        "spec": { "type": "object", "description": "inline design spec (instead of specId)" }
    });
    if let (Some(p), Some(e)) = (props.as_object_mut(), extra.as_object()) {
        p.extend(e.clone());
    }
    json!({ "type": "object", "properties": props })
}

fn load(store: &Store, args: &Value) -> Result<DesignSpec, String> {
    match (args.get("spec"), args.get("specId").and_then(Value::as_str)) {
        (Some(s), _) => serde_json::from_value(s.clone()).map_err(|e| format!("spec: {e}")),
        (None, Some(id)) => store.load_spec(id),
        (None, None) => Err("pass specId or spec".into()),
    }
}

/// Loads and validates; invalid specs never reach the planner.
fn load_valid(store: &Store, args: &Value) -> Result<DesignSpec, String> {
    let spec = load(store, args)?;
    let report = spec::validate(&spec, &store.library()?);
    if !report.ok {
        return Err(format!("spec {} is invalid: {}", spec.id, report.errors.join("; ")));
    }
    Ok(spec)
}

fn level(args: &Value) -> Result<String, String> {
    let l = args.get("level").and_then(Value::as_str).unwrap_or("exploration");
    if LEVELS.contains(&l) { Ok(l.into()) } else { Err(format!("level must be one of {LEVELS:?}")) }
}

pub fn all() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "factory_materials",
            description: "Returns the shared material library (M01..M10 with base, color, roughness, metallic). Components reference these IDs only, so all assets share one material world.",
            schema: || json!({ "type": "object", "properties": {} }),
            call: |store, _| Ok(serde_json::to_value(store.library()?).unwrap_or_default()),
        },
        ToolDef {
            name: "factory_list_specs",
            description: "Lists the design specs in <root>/specs with their component counts and validity.",
            schema: || json!({ "type": "object", "properties": {} }),
            call: |store, _| {
                let lib = store.library()?;
                let mut out = Vec::new();
                for id in store.spec_ids()? {
                    match store.load_spec(&id) {
                        Ok(s) => {
                            let r = spec::validate(&s, &lib);
                            out.push(json!({ "specId": id, "assetClass": s.asset_class, "title": s.title,
                                "components": s.components.len(), "valid": r.ok, "errors": r.errors.len(), "warnings": r.warnings.len() }));
                        }
                        Err(e) => out.push(json!({ "specId": id, "valid": false, "parseError": e })),
                    }
                }
                Ok(json!({ "root": store.root.display().to_string(), "specs": out }))
            },
        },
        ToolDef {
            name: "factory_validate_spec",
            description: "Validates a design spec against the component hierarchy (groups, sources, parents, mirrors) and the material library. Returns errors (blocking) and warnings (too many parts, Hunyuan surface details, off-centre parts without mirror).",
            schema: || spec_schema(json!({})),
            call: |store, args| {
                let s = load(store, args)?;
                Ok(serde_json::to_value(spec::validate(&s, &store.library()?)).unwrap_or_default())
            },
        },
        ToolDef {
            name: "factory_reference_prompts",
            description: "Builds the design-master prompt and the 7 controlled reference views (front, rear, left, right, top, bottom, 3/4 hero) that share one style block and presentation.",
            schema: || spec_schema(json!({})),
            call: |store, args| Ok(plan::reference_prompts(&load_valid(store, args)?)),
        },
        ToolDef {
            name: "factory_material_groups",
            description: "Groups components by material ID + semantic overrides. Each group is one PBR paint unit, so neighbouring parts never get separately generated versions of the same surface.",
            schema: || spec_schema(json!({})),
            call: |store, args| {
                let s = load_valid(store, args)?;
                Ok(json!({ "specId": s.id, "groups": material_groups(&s, &store.library()?) }))
            },
        },
        ToolDef {
            name: "factory_plan",
            description: "Plans the job DAG for level 'exploration' (master image -> one Hunyuan mesh -> normalize -> flat material -> card) or 'hero' (master -> 7 refs -> Hunyuan per component, mirrors generated once -> cleanup -> Blender assembly -> Hunyuan-Paint per material group -> bake -> 4-view validation -> card). Writes runs/<id>/<level>/plan.json unless write=false.",
            schema: || spec_schema(json!({
                "level": { "type": "string", "enum": LEVELS },
                "write": { "type": "boolean", "default": true }
            })),
            call: |store, args| {
                let s = load_valid(store, args)?;
                let level = level(args)?;
                let p = serde_json::to_value(plan::plan(&s, &store.library()?, &level)?).unwrap_or_default();
                let mut out = json!({ "plan": p });
                if args.get("write").and_then(Value::as_bool).unwrap_or(true) {
                    let path = store.run_dir(&s.id, &level).join("plan.json");
                    store.write_json(&path, &out["plan"])?;
                    out["written"] = json!(path.display().to_string());
                }
                Ok(out)
            },
        },
        ToolDef {
            name: "factory_card_rig",
            description: "Standard comparison-card rig for a spec: ortho front/left/top + 35mm 3/4 camera, 3 area lights, ground shadow catcher, scale bar, 2x2 layout. Identical per asset class so cards compare.",
            schema: || spec_schema(json!({})),
            call: |store, args| Ok(plan::card_rig(&load_valid(store, args)?)),
        },
        ToolDef {
            name: "factory_review",
            description: "Takes per-component, per-view validation scores (0..1) and decides: keep, regenerate only the failing component block (next attempt, new seed, mirrors map to their source), or escalate after 4 attempts. Appends the decision to runs/<id>/<level>/reviews.jsonl.",
            schema: || spec_schema(json!({
                "level": { "type": "string", "enum": LEVELS },
                "threshold": { "type": "number", "default": 0.7 },
                "scores": { "type": "array", "items": { "type": "object", "required": ["component", "view", "score"],
                    "properties": { "component": { "type": "string" }, "view": { "type": "string" }, "score": { "type": "number" }, "note": { "type": "string" } } } }
            })),
            call: |store, args| {
                let s = load_valid(store, args)?;
                let level = level(args)?;
                let scores: Vec<Score> = serde_json::from_value(args.get("scores").cloned().unwrap_or(json!([])))
                    .map_err(|e| format!("scores: {e}"))?;
                if scores.is_empty() {
                    return Err("scores must not be empty".into());
                }
                let threshold = args.get("threshold").and_then(Value::as_f64).unwrap_or(0.7);
                let log = store.run_dir(&s.id, &level).join("reviews.jsonl");
                let history = store.read_jsonl(&log)?;
                let decision = review::decide(&s, &level, &scores, threshold, &history)?;
                store.append_jsonl(&log, &decision)?;
                Ok(decision)
            },
        },
    ]
}
