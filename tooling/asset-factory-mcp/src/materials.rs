//! The shared material library (M01..M10). Every generated asset only
//! carries material IDs plus small semantic overrides, so ten different
//! AI-generated ships live in one material world instead of ten slightly
//! different "grey metals".

use crate::spec::{Component, DesignSpec};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub const DEFAULT_LIBRARY: &str = include_str!("../data/material_library.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Material {
    pub id: String,
    pub name: String,
    pub base: String,
    pub color: [f64; 3],
    pub roughness: f64,
    pub metallic: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transmission: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emission: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialLibrary {
    pub version: u32,
    pub materials: Vec<Material>,
}

impl MaterialLibrary {
    pub fn parse(text: &str) -> Result<Self, String> {
        let lib: MaterialLibrary =
            serde_json::from_str(text).map_err(|e| format!("material library: {e}"))?;
        let mut seen = std::collections::BTreeSet::new();
        for m in &lib.materials {
            if !seen.insert(m.id.as_str()) {
                return Err(format!("material library: duplicate id {}", m.id));
            }
        }
        Ok(lib)
    }

    pub fn default_library() -> Self {
        Self::parse(DEFAULT_LIBRARY).expect("embedded material library is valid")
    }

    pub fn get(&self, id: &str) -> Option<&Material> {
        self.materials.iter().find(|m| m.id == id)
    }
}

/// One PBR paint unit: every component sharing a material ID (and the same
/// overrides) is textured together, so neighbouring parts never show a
/// material seam between two separately generated versions of one surface.
#[derive(Debug, Serialize)]
pub struct MaterialGroup {
    pub key: String,
    pub material: Material,
    pub overrides: Map<String, Value>,
    pub components: Vec<String>,
}

/// Groups components by (material id, overrides). Mirrored components join
/// the group of their source automatically since they carry the same IDs.
/// Surface details (`texture_detail`) have no geometry of their own: they are
/// painted inside the group of the component they sit on, not as a group.
pub fn material_groups(spec: &DesignSpec, lib: &MaterialLibrary) -> Vec<MaterialGroup> {
    let mut groups: BTreeMap<String, MaterialGroup> = BTreeMap::new();
    for c in spec.components.iter().filter(|c| c.source != "texture_detail") {
        let Some(material) = lib.get(&c.material) else {
            continue;
        };
        let overrides = c.overrides.clone();
        let key = group_key(c);
        groups
            .entry(key.clone())
            .or_insert_with(|| MaterialGroup {
                key,
                material: resolved(material, &overrides),
                overrides,
                components: Vec::new(),
            })
            .components
            .push(c.id.clone());
    }
    groups.into_values().collect()
}

fn group_key(c: &Component) -> String {
    if c.overrides.is_empty() {
        c.material.clone()
    } else {
        format!("{}+{}", c.material, Value::Object(c.overrides.clone()))
    }
}

/// Library material with numeric overrides (roughness/metallic) applied.
pub fn resolved(m: &Material, overrides: &Map<String, Value>) -> Material {
    let mut out = m.clone();
    if let Some(r) = overrides.get("roughness").and_then(Value::as_f64) {
        out.roughness = r;
    }
    if let Some(x) = overrides.get("metallic").and_then(Value::as_f64) {
        out.metallic = x;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_library_has_ten_materials() {
        let lib = MaterialLibrary::default_library();
        assert_eq!(lib.materials.len(), 10);
        assert_eq!(lib.get("M05").unwrap().name, "titanium");
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let text = r#"{"version":1,"materials":[
            {"id":"M01","name":"a","base":"b","color":[0,0,0],"roughness":0.1,"metallic":0},
            {"id":"M01","name":"c","base":"d","color":[0,0,0],"roughness":0.1,"metallic":0}]}"#;
        assert!(MaterialLibrary::parse(text).is_err());
    }
}
