//! Turns a design spec into an ordered job list for one of two quality
//! levels. The server never renders, generates or runs Blender itself: it
//! writes jobs with explicit inputs/outputs, and drivers (image model,
//! Hunyuan3D 2.1 API client, Blender scripts) execute them.
//!
//! ```text
//! exploration: master -> hunyuan(whole, no texture) -> normalize -> flat materials -> card
//! hero:        master -> 7 ortho refs -> hunyuan per component -> cleanup -> assembly
//!              -> PBR per material group -> 4-view validation -> card
//! ```

use crate::materials::{material_groups, MaterialLibrary};
use crate::spec::{Component, DesignSpec};
use serde::Serialize;
use serde_json::{json, Value};

pub const LEVELS: [&str; 2] = ["exploration", "hero"];

/// Reference views of the hero pipeline; the design spec proper.
pub const REFERENCE_VIEWS: [&str; 7] =
    ["front", "rear", "left", "right", "top", "bottom", "hero_3q"];

/// Views the assembled base mesh is checked against.
pub const VALIDATION_VIEWS: [&str; 4] = ["front", "left", "top", "hero_3q"];

#[derive(Debug, Serialize)]
pub struct Job {
    pub id: String,
    pub stage: String,
    pub kind: String,
    #[serde(rename = "dependsOn")]
    pub depends_on: Vec<String>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct Plan {
    #[serde(rename = "specId")]
    pub spec_id: String,
    pub level: String,
    pub stages: Vec<String>,
    pub jobs: Vec<Job>,
}

/// FNV-1a; stable seeds per (spec, component, attempt) across runs and
/// machines, so a rerun of the same attempt reproduces the same mesh.
pub fn seed(spec_id: &str, component: &str, attempt: u32) -> u32 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in spec_id
        .bytes()
        .chain([b'/'])
        .chain(component.bytes())
        .chain(attempt.to_le_bytes())
    {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    (h % 2_147_483_647) as u32
}

fn style_block(spec: &DesignSpec) -> String {
    let dl = &spec.design_language;
    let mut parts = vec![format!("{} ({})", spec.asset_class, spec.title)];
    if !dl.primary_form.is_empty() {
        parts.push(format!("primary form: {}", dl.primary_form));
    }
    if !dl.structural.is_empty() {
        parts.push(format!("structural language: {}", dl.structural.join(", ")));
    }
    if !dl.material_language.is_empty() {
        parts.push(format!("materials: {}", dl.material_language.join(", ")));
    }
    if !dl.detail_level.is_empty() {
        parts.push(format!("detail level: {}", dl.detail_level));
    }
    parts.extend(dl.constraints.iter().cloned());
    parts.join("; ")
}

fn view_phrase(view: &str) -> &'static str {
    match view {
        "front" => "orthographic front view, looking along -Y",
        "rear" => "orthographic rear view, looking along +Y",
        "left" => "orthographic left side view",
        "right" => "orthographic right side view",
        "top" => "orthographic top view",
        "bottom" => "orthographic bottom view",
        _ => "three-quarter hero view from front-left, slightly above, 35mm",
    }
}

/// Prompts for the design master and the controlled reference set. Every
/// view shares the style block and a fixed presentation, so the views are
/// one design rather than seven pictures.
pub fn reference_prompts(spec: &DesignSpec) -> Value {
    let style = style_block(spec);
    let d = &spec.dimensions;
    let presentation = format!(
        "same object in every view, neutral grey background, even studio light, no shadows on background, \
         object centred and filling 80% of frame, proportions {:.1} x {:.1} x {:.1} {} (L x W x H)",
        d.length, d.width, d.height, d.unit
    );
    let components: Vec<String> = spec
        .components
        .iter()
        .filter(|c| c.group != "surface_details")
        .map(|c| {
            if c.description.is_empty() {
                c.id.replace('_', " ")
            } else {
                c.description.clone()
            }
        })
        .collect();
    let views: Vec<Value> = REFERENCE_VIEWS
        .iter()
        .map(|v| {
            json!({
                "view": v,
                "prompt": format!("{style}. Components: {}. {}. {presentation}.", components.join("; "), view_phrase(v)),
                "output": format!("refs/{v}.png"),
            })
        })
        .collect();
    json!({
        "specId": spec.id,
        "master": {
            "prompt": format!("design master sheet: {style}. Components: {}. {presentation}.", components.join("; ")),
            "output": "refs/master.png",
        },
        "views": views,
        "negative": "text, watermark, cropped, perspective distortion in orthographic views, decorative greebles, inconsistent details between views",
    })
}

/// Hunyuan3D 2.1 `api_server.py` request body (image is filled in by the
/// driver from `image` path). Face budget follows the component's role.
pub fn hunyuan_request(
    spec: &DesignSpec,
    c: Option<&Component>,
    level: &str,
    attempt: u32,
) -> Value {
    let (octree, steps, faces) = match (level, c.map(|c| c.group.as_str())) {
        ("exploration", _) => (256, 25, 40_000),
        (_, Some("primary_structure")) => (512, 50, 120_000),
        (_, Some("propulsion")) => (384, 50, 60_000),
        (_, Some("landing")) => (384, 50, 25_000),
        _ => (384, 50, 15_000),
    };
    let name = c.map(|c| c.id.as_str()).unwrap_or("whole");
    let image = match level {
        "exploration" => "refs/master.png".to_string(),
        _ => format!("refs/components/{name}.png"),
    };
    json!({
        "image": image,
        "remove_background": true,
        "texture": false,
        "seed": seed(&spec.id, name, attempt),
        "octree_resolution": octree,
        "num_inference_steps": steps,
        "guidance_scale": 5.0,
        "num_chunks": 8000,
        "face_count": faces,
    })
}

/// Standard camera rig for the comparison card: identical for every asset
/// of a class so cards are comparable. Distances derive from dimensions.
pub fn card_rig(spec: &DesignSpec) -> Value {
    let d = &spec.dimensions;
    let extent = d.length.max(d.width).max(d.height);
    let ortho = extent * 1.15;
    let dist = extent * 2.4;
    json!({
        "specId": spec.id,
        "resolution": [1024, 1024],
        "background": [0.18, 0.18, 0.19],
        "groundPlane": { "z": 0.0, "color": [0.32, 0.32, 0.33], "shadowCatcher": true },
        "scaleBar": { "length": scale_bar(extent), "unit": d.unit },
        // Suns, not area lights: scale-independent, so every asset size is
        // lit the same. `from` is the direction the light comes from.
        "lights": [
            { "name": "key",  "type": "sun", "strength": 3.2, "from": [0.6, 0.5, 0.75],  "angle": 3.0 },
            { "name": "fill", "type": "sun", "strength": 1.0, "from": [-0.8, 0.3, 0.4],  "angle": 10.0 },
            { "name": "rim",  "type": "sun", "strength": 1.8, "from": [0.0, -1.0, 0.55], "angle": 5.0 }
        ],
        // Asset space: +X right, +Y forward (nose), +Z up.
        "cameras": [
            { "name": "front",   "type": "ORTHO", "orthoScale": ortho, "location": [0.0, dist, d.height / 2.0], "lookAt": [0.0, 0.0, d.height / 2.0] },
            { "name": "left",    "type": "ORTHO", "orthoScale": ortho, "location": [-dist, 0.0, d.height / 2.0], "lookAt": [0.0, 0.0, d.height / 2.0] },
            { "name": "top",     "type": "ORTHO", "orthoScale": ortho, "location": [0.0, 0.0, dist], "lookAt": [0.0, 0.0, 0.0] },
            { "name": "hero_3q", "type": "PERSP", "lens": 50.0, "location": [-dist * 0.42, dist * 0.47, dist * 0.3], "lookAt": [0.0, 0.0, d.height / 2.0] }
        ],
        "layout": { "grid": [2, 2], "order": ["hero_3q", "front", "left", "top"], "output": "card.png" }
    })
}

fn scale_bar(extent: f64) -> f64 {
    // 1/2/5 x 10^n, roughly a quarter of the extent.
    let target = extent / 4.0;
    let mag = 10f64.powf(target.log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * mag)
        .min_by(|a, b| (a - target).abs().total_cmp(&(b - target).abs()))
        .unwrap_or(mag)
}

fn job(
    id: &str,
    stage: &str,
    kind: &str,
    deps: &[&str],
    inputs: Vec<String>,
    outputs: Vec<String>,
    params: Value,
) -> Job {
    Job {
        id: id.into(),
        stage: stage.into(),
        kind: kind.into(),
        depends_on: deps.iter().map(|s| s.to_string()).collect(),
        inputs,
        outputs,
        params,
    }
}

pub fn plan(spec: &DesignSpec, lib: &MaterialLibrary, level: &str) -> Result<Plan, String> {
    match level {
        "exploration" => Ok(exploration(spec, lib)),
        "hero" => Ok(hero(spec, lib)),
        _ => Err(format!("level \"{level}\" must be one of {LEVELS:?}")),
    }
}

fn exploration(spec: &DesignSpec, lib: &MaterialLibrary) -> Plan {
    let prompts = reference_prompts(spec);
    // Whole-asset mesh can only take one material: the one covering the
    // most components of the primary structure.
    let dominant = spec
        .components
        .iter()
        .find(|c| c.group == "primary_structure")
        .map(|c| c.material.clone())
        .unwrap_or_else(|| "M02".into());
    let jobs = vec![
        job(
            "master",
            "design_master",
            "image.generate",
            &[],
            vec![],
            vec!["refs/master.png".into()],
            json!({ "prompt": prompts["master"]["prompt"], "negative": prompts["negative"] }),
        ),
        job(
            "shape.whole",
            "shape",
            "hunyuan.shape",
            &["master"],
            vec!["refs/master.png".into()],
            vec!["meshes/whole.glb".into()],
            hunyuan_request(spec, None, "exploration", 0),
        ),
        job(
            "normalize",
            "cleanup",
            "blender.normalize",
            &["shape.whole"],
            vec!["meshes/whole.glb".into()],
            vec!["meshes/whole.norm.glb".into()],
            json!({ "fitTo": [spec.dimensions.width, spec.dimensions.length, spec.dimensions.height], "groundAtZ0": true, "forward": "+Y" }),
        ),
        job(
            "materials",
            "material",
            "blender.assign_flat",
            &["normalize"],
            vec!["meshes/whole.norm.glb".into()],
            vec!["asset.glb".into()],
            json!({ "material": lib.get(&dominant) }),
        ),
        job(
            "card",
            "presentation",
            "blender.card",
            &["materials"],
            vec!["asset.glb".into()],
            vec!["card.png".into()],
            card_rig(spec),
        ),
    ];
    finish(spec, "exploration", jobs)
}

fn hero(spec: &DesignSpec, lib: &MaterialLibrary) -> Plan {
    let prompts = reference_prompts(spec);
    let mut jobs = vec![job(
        "master",
        "design_master",
        "image.generate",
        &[],
        vec![],
        vec!["refs/master.png".into()],
        json!({ "prompt": prompts["master"]["prompt"], "negative": prompts["negative"] }),
    )];

    let ref_outputs: Vec<String> = REFERENCE_VIEWS
        .iter()
        .map(|v| format!("refs/{v}.png"))
        .collect();
    for v in prompts["views"].as_array().into_iter().flatten() {
        let view = v["view"].as_str().unwrap_or_default();
        jobs.push(job(
            &format!("ref.{view}"), "reference_views", "image.generate", &["master"],
            vec!["refs/master.png".into()], vec![format!("refs/{view}.png")],
            json!({ "prompt": v["prompt"], "negative": prompts["negative"], "conditionOn": "refs/master.png" }),
        ));
    }
    let ref_ids: Vec<String> = REFERENCE_VIEWS.iter().map(|v| format!("ref.{v}")).collect();
    let ref_deps: Vec<&str> = ref_ids.iter().map(String::as_str).collect();

    let mut assembly_inputs = Vec::new();
    let mut shape_ids = Vec::new();
    for c in spec
        .components
        .iter()
        .filter(|c| c.source == "hunyuan" && c.mirror_of.is_none())
    {
        let crop_id = format!("crop.{}", c.id);
        jobs.push(job(
            &crop_id, "component_refs", "image.component_crop", &ref_deps, ref_outputs.clone(),
            vec![format!("refs/components/{}.png", c.id)],
            json!({ "component": c.id, "description": c.description, "anchor": c.anchor, "size": c.size,
                    "isolatePrompt": format!("isolated {} of the same design, 3/4 view, neutral background", if c.description.is_empty() { c.id.replace('_', " ") } else { c.description.clone() }) }),
        ));
        let shape_id = format!("shape.{}", c.id);
        jobs.push(job(
            &shape_id,
            "shape",
            "hunyuan.shape",
            &[crop_id.as_str()],
            vec![format!("refs/components/{}.png", c.id)],
            vec![format!("meshes/{}.glb", c.id)],
            hunyuan_request(spec, Some(c), "hero", 0),
        ));
        let clean_id = format!("clean.{}", c.id);
        jobs.push(job(
            &clean_id, "cleanup", "blender.cleanup", &[shape_id.as_str()],
            vec![format!("meshes/{}.glb", c.id)], vec![format!("meshes/{}.clean.glb", c.id)],
            json!({ "fitTo": c.size, "mergeDistance": 0.001, "recalcNormals": true, "decimateTo": hunyuan_request(spec, Some(c), "hero", 0)["face_count"] }),
        ));
        assembly_inputs.push(format!("meshes/{}.clean.glb", c.id));
        shape_ids.push(clean_id);
    }
    let procedural: Vec<Value> = spec
        .components
        .iter()
        .filter(|c| c.source == "blender_procedural" && c.mirror_of.is_none())
        .map(|c| json!({ "id": c.id, "description": c.description, "anchor": c.anchor, "size": c.size }))
        .collect();
    let placements: Vec<Value> = spec
        .components
        .iter()
        .filter(|c| c.source != "texture_detail")
        .map(|c| {
            let src = c.mirror_of.as_deref().unwrap_or(&c.id);
            json!({ "id": c.id, "mesh": format!("meshes/{src}.clean.glb"), "anchor": c.anchor, "size": c.size,
                    "mirrorX": c.mirror_of.is_some(), "parent": c.parent, "material": c.material })
        })
        .collect();
    let shape_deps: Vec<&str> = shape_ids.iter().map(String::as_str).collect();
    jobs.push(job(
        "assembly", "assembly", "blender.assemble", &shape_deps, assembly_inputs,
        vec!["assembly.blend".into(), "assembly.glb".into()],
        json!({ "placements": placements, "procedural": procedural, "symmetry": spec.symmetry, "dimensions": spec.dimensions }),
    ));

    let groups = material_groups(spec, lib);
    let details: Vec<Value> = spec
        .components
        .iter()
        .filter(|c| c.source == "texture_detail")
        .map(|c| json!({ "id": c.id, "on": c.parent, "description": c.description, "material": c.material }))
        .collect();
    let mut paint_ids = Vec::new();
    for g in &groups {
        let id = format!("paint.{}", g.material.id);
        let id = if paint_ids.contains(&id) {
            format!("{id}.{}", paint_ids.len())
        } else {
            id
        };
        jobs.push(job(
            &id, "pbr", "hunyuan.paint", &["assembly"],
            vec!["assembly.glb".into(), "refs/hero_3q.png".into()],
            vec![format!("textures/{}/albedo.png", g.key), format!("textures/{}/metallic_roughness.png", g.key), format!("textures/{}/normal.png", g.key)],
            json!({ "materialGroup": g.key, "components": g.components, "material": g.material, "semantic": g.overrides,
                    "surfaceDetails": details.iter().filter(|d| d["material"] == g.material.id.as_str() || g.components.iter().any(|c| d["on"] == c.as_str())).collect::<Vec<_>>(),
                    "clampToLibrary": true }),
        ));
        paint_ids.push(id);
    }
    let paint_deps: Vec<&str> = paint_ids.iter().map(String::as_str).collect();
    jobs.push(job(
        "bake", "pbr", "blender.bake_materials", &paint_deps, vec!["assembly.blend".into()], vec!["asset.glb".into()],
        json!({ "groups": groups.iter().map(|g| &g.key).collect::<Vec<_>>(), "clampToLibrary": true }),
    ));
    let mut val_ids = Vec::new();
    for v in VALIDATION_VIEWS {
        let id = format!("validate.{v}");
        jobs.push(job(
            &id, "validation", "blender.compare_view", &["bake"], vec!["asset.glb".into(), format!("refs/{v}.png")],
            vec![format!("validation/{v}.png"), format!("validation/{v}.json")],
            json!({ "view": v, "metrics": ["silhouette_iou", "proportion_error"], "perComponent": true }),
        ));
        val_ids.push(id);
    }
    let val_deps: Vec<&str> = val_ids.iter().map(String::as_str).collect();
    jobs.push(job(
        "card",
        "presentation",
        "blender.card",
        &val_deps,
        vec!["asset.glb".into()],
        vec!["card.png".into()],
        card_rig(spec),
    ));
    finish(spec, "hero", jobs)
}

fn finish(spec: &DesignSpec, level: &str, jobs: Vec<Job>) -> Plan {
    let mut stages: Vec<String> = Vec::new();
    for j in &jobs {
        if !stages.contains(&j.stage) {
            stages.push(j.stage.clone());
        }
    }
    Plan {
        spec_id: spec.id.clone(),
        level: level.into(),
        stages,
        jobs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::tests::sample;
    use std::collections::BTreeSet;

    fn check_dag(p: &Plan) {
        let mut seen = BTreeSet::new();
        for j in &p.jobs {
            for d in &j.depends_on {
                assert!(
                    seen.contains(d.as_str()),
                    "{} depends on later/unknown {}",
                    j.id,
                    d
                );
            }
            assert!(seen.insert(j.id.as_str()), "duplicate job {}", j.id);
        }
    }

    #[test]
    fn seeds_are_stable_and_attempt_sensitive() {
        assert_eq!(seed("a", "b", 0), seed("a", "b", 0));
        assert_ne!(seed("a", "b", 0), seed("a", "b", 1));
        assert_ne!(seed("a", "b", 0), seed("a", "c", 0));
    }

    #[test]
    fn exploration_is_a_short_chain() {
        let p = plan(
            &sample(),
            &MaterialLibrary::default_library(),
            "exploration",
        )
        .unwrap();
        check_dag(&p);
        assert_eq!(p.jobs.len(), 5);
        assert_eq!(p.jobs[1].params["octree_resolution"], 256);
    }

    #[test]
    fn hero_generates_mirrors_once_and_paints_per_group() {
        let p = plan(&sample(), &MaterialLibrary::default_library(), "hero").unwrap();
        check_dag(&p);
        let shapes: Vec<&str> = p
            .jobs
            .iter()
            .filter(|j| j.kind == "hunyuan.shape")
            .map(|j| j.id.as_str())
            .collect();
        assert_eq!(shapes, ["shape.main_hull", "shape.engine_l"]);
        let paints = p.jobs.iter().filter(|j| j.kind == "hunyuan.paint").count();
        assert_eq!(paints, 2); // M01 hull (+ M02 panel details), M05 engines (shared)
        let hull = p.jobs.iter().find(|j| j.id == "paint.M01").unwrap();
        assert_eq!(hull.params["surfaceDetails"][0]["id"], "panels");
        let assembly = p.jobs.iter().find(|j| j.id == "assembly").unwrap();
        let right = assembly.params["placements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == "engine_r")
            .unwrap();
        assert_eq!(right["mesh"], "meshes/engine_l.clean.glb");
        assert_eq!(right["mirrorX"], true);
    }

    #[test]
    fn unknown_level_is_rejected() {
        assert!(plan(&sample(), &MaterialLibrary::default_library(), "ultra").is_err());
    }

    #[test]
    fn scale_bar_is_round() {
        assert_eq!(scale_bar(24.0), 5.0);
        assert_eq!(scale_bar(2.0), 0.5);
    }
}
