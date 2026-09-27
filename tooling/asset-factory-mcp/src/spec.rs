//! The design spec: what the LLM (as art/asset director, not as 3D modeller)
//! writes. Both quality levels — exploration and hero — start from the same
//! spec; only the resolution of the pipeline changes.

use crate::materials::MaterialLibrary;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

/// Component groups of the hierarchy (ship -> group -> component).
pub const GROUPS: [&str; 5] = [
    "primary_structure",
    "propulsion",
    "landing",
    "external_systems",
    "surface_details",
];

/// How a component's geometry is produced.
pub const SOURCES: [&str; 3] = ["hunyuan", "blender_procedural", "texture_detail"];

/// More parts than this and a hero run turns into a pile of mismatched pieces.
pub const MAX_COMPONENTS: usize = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dimensions {
    pub length: f64,
    pub width: f64,
    pub height: f64,
    #[serde(default = "default_unit")]
    pub unit: String,
}

fn default_unit() -> String {
    "m".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignLanguage {
    #[serde(default)]
    pub primary_form: String,
    #[serde(default)]
    pub structural: Vec<String>,
    #[serde(default)]
    pub material_language: Vec<String>,
    #[serde(default)]
    pub detail_level: String,
    #[serde(default)]
    pub constraints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Component {
    pub id: String,
    pub group: String,
    #[serde(default)]
    pub description: String,
    pub material: String,
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Generate once, mirror in Blender (left engine -> right engine).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror_of: Option<String>,
    /// Placement in asset space, metres, +X right, +Y forward, +Z up.
    #[serde(default)]
    pub anchor: [f64; 3],
    /// Bounding box the generated mesh is normalised into, metres.
    #[serde(default = "default_size")]
    pub size: [f64; 3],
    /// Semantic material info (roughness/metallic numbers override the
    /// library; wear/color/heatDiscoloration are prompt hints for paint).
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub overrides: Map<String, Value>,
}

fn default_source() -> String {
    "hunyuan".into()
}

fn default_size() -> [f64; 3] {
    [1.0, 1.0, 1.0]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignSpec {
    pub id: String,
    pub asset_class: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub design_language: DesignLanguage,
    pub dimensions: Dimensions,
    #[serde(default = "default_symmetry")]
    pub symmetry: String,
    pub components: Vec<Component>,
}

fn default_symmetry() -> String {
    "mirror_x".into()
}

impl DesignSpec {
    pub fn component(&self, id: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.id == id)
    }
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub ok: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

fn is_slug(s: &str) -> bool {
    !s.is_empty()
        && s
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

pub fn validate(spec: &DesignSpec, lib: &MaterialLibrary) -> Report {
    let mut r = Report::default();
    let mut err = |m: String| r.errors.push(m);

    if !is_slug(&spec.id) {
        err(format!("id \"{}\" must be a lowercase slug", spec.id));
    }
    let d = &spec.dimensions;
    if !(d.length > 0.0 && d.width > 0.0 && d.height > 0.0) {
        err("dimensions must all be > 0".into());
    }
    if !["none", "mirror_x"].contains(&spec.symmetry.as_str()) {
        err(format!("symmetry \"{}\" must be none or mirror_x", spec.symmetry));
    }
    if spec.components.is_empty() {
        err("spec has no components".into());
    }

    let mut ids = BTreeSet::new();
    for c in &spec.components {
        if !is_slug(&c.id) {
            err(format!("component id \"{}\" must be a lowercase slug", c.id));
        }
        if !ids.insert(c.id.as_str()) {
            err(format!("duplicate component id \"{}\"", c.id));
        }
    }
    for c in &spec.components {
        let at = format!("component {}", c.id);
        if !GROUPS.contains(&c.group.as_str()) {
            err(format!("{at}: unknown group \"{}\" (one of {GROUPS:?})", c.group));
        }
        if !SOURCES.contains(&c.source.as_str()) {
            err(format!("{at}: unknown source \"{}\" (one of {SOURCES:?})", c.source));
        }
        if lib.get(&c.material).is_none() {
            err(format!("{at}: material \"{}\" is not in the library", c.material));
        }
        if c.size.iter().any(|v| *v <= 0.0) {
            err(format!("{at}: size must be > 0 on every axis"));
        }
        for key in ["roughness", "metallic"] {
            if let Some(v) = c.overrides.get(key) {
                match v.as_f64() {
                    Some(x) if (0.0..=1.0).contains(&x) => {}
                    _ => err(format!("{at}: override {key} must be a number in [0,1]")),
                }
            }
        }
        if let Some(p) = &c.parent {
            if !ids.contains(p.as_str()) || p == &c.id {
                err(format!("{at}: parent \"{p}\" does not exist"));
            }
        }
        if let Some(m) = &c.mirror_of {
            match spec.component(m) {
                None => err(format!("{at}: mirrorOf \"{m}\" does not exist")),
                Some(src) if src.mirror_of.is_some() || src.id == c.id => {
                    err(format!("{at}: mirrorOf \"{m}\" must be a non-mirrored component"))
                }
                Some(src) if src.material != c.material => err(format!(
                    "{at}: mirror must share material with \"{m}\" ({} vs {})",
                    c.material, src.material
                )),
                _ => {}
            }
        }
    }
    if !spec.components.iter().any(|c| c.group == "primary_structure") {
        err("spec needs at least one primary_structure component".into());
    }

    if spec.components.len() > MAX_COMPONENTS {
        r.warnings.push(format!(
            "{} components (> {MAX_COMPONENTS}); merge small parts into their parent",
            spec.components.len()
        ));
    }
    for c in &spec.components {
        if c.group == "surface_details" && c.source == "hunyuan" {
            r.warnings.push(format!(
                "component {}: surface details rarely survive Hunyuan; prefer texture_detail or blender_procedural",
                c.id
            ));
        }
    }
    let distinct: BTreeSet<&str> = spec.components.iter().map(|c| c.material.as_str()).collect();
    if distinct.len() > 6 {
        r.warnings.push(format!(
            "{} distinct materials; a coherent asset rarely needs more than 6",
            distinct.len()
        ));
    }
    if spec.symmetry == "mirror_x" {
        for c in &spec.components {
            if c.mirror_of.is_none() && c.anchor[0].abs() > 1e-6 {
                let mirrored = spec.components.iter().any(|o| o.mirror_of.as_deref() == Some(&c.id));
                if !mirrored {
                    r.warnings.push(format!(
                        "component {} sits off-centre (x={}) but has no mirror partner",
                        c.id, c.anchor[0]
                    ));
                }
            }
        }
    }

    r.ok = r.errors.is_empty();
    r
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub fn sample() -> DesignSpec {
        serde_json::from_value(json!({
            "id": "probe",
            "assetClass": "spacecraft",
            "dimensions": { "length": 20, "width": 12, "height": 5 },
            "components": [
                { "id": "main_hull", "group": "primary_structure", "material": "M01", "size": [4, 14, 4] },
                { "id": "engine_l", "group": "propulsion", "material": "M05", "anchor": [-4, -6, 0], "size": [2, 5, 2],
                  "overrides": { "roughness": 0.31, "heatDiscoloration": "subtle" } },
                { "id": "engine_r", "group": "propulsion", "material": "M05", "anchor": [4, -6, 0], "size": [2, 5, 2],
                  "mirrorOf": "engine_l", "overrides": { "roughness": 0.31, "heatDiscoloration": "subtle" } },
                { "id": "panels", "group": "surface_details", "material": "M02", "source": "texture_detail", "parent": "main_hull" }
            ]
        }))
        .unwrap()
    }

    #[test]
    fn sample_spec_is_valid() {
        let r = validate(&sample(), &MaterialLibrary::default_library());
        assert!(r.ok, "{:?}", r.errors);
    }

    #[test]
    fn unknown_material_and_bad_mirror_are_errors() {
        let mut s = sample();
        s.components[1].material = "M99".into();
        s.components[2].mirror_of = Some("nope".into());
        let r = validate(&s, &MaterialLibrary::default_library());
        assert!(!r.ok);
        assert!(r.errors.iter().any(|e| e.contains("M99")));
        assert!(r.errors.iter().any(|e| e.contains("nope")));
    }

    #[test]
    fn mirror_with_other_material_is_an_error() {
        let mut s = sample();
        s.components[2].material = "M04".into();
        let r = validate(&s, &MaterialLibrary::default_library());
        assert!(r.errors.iter().any(|e| e.contains("share material")));
    }

    #[test]
    fn off_centre_without_partner_warns() {
        let mut s = sample();
        s.components.remove(2);
        let r = validate(&s, &MaterialLibrary::default_library());
        assert!(r.ok);
        assert!(r.warnings.iter().any(|w| w.contains("engine_l")));
    }
}
