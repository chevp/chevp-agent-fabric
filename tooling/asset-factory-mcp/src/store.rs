//! The factory root (`ASSET_FACTORY_ROOT`, usually an icc-frost-lib lab):
//!
//! ```text
//! <root>/
//! ├── materials.json               # optional override of the embedded library
//! ├── specs/<id>.json              # design specs
//! └── runs/<id>/<level>/
//!     ├── plan.json                # written by factory_plan
//!     ├── reviews.jsonl            # appended by factory_review
//!     └── refs/ meshes/ textures/ validation/ asset.glb card.png   (drivers)
//! ```

use crate::materials::MaterialLibrary;
use crate::spec::DesignSpec;
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct Store {
    pub root: PathBuf,
}

fn io(path: &Path, e: std::io::Error) -> String {
    format!("{}: {e}", path.display())
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn library(&self) -> Result<MaterialLibrary, String> {
        let path = self.root.join("materials.json");
        match fs::read_to_string(&path) {
            Ok(text) => MaterialLibrary::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(MaterialLibrary::default_library())
            }
            Err(e) => Err(io(&path, e)),
        }
    }

    pub fn spec_ids(&self) -> Result<Vec<String>, String> {
        let dir = self.root.join("specs");
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io(&dir, e)),
        };
        let mut ids: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                name.strip_suffix(".json").map(str::to_string)
            })
            .collect();
        ids.sort();
        Ok(ids)
    }

    pub fn load_spec(&self, id: &str) -> Result<DesignSpec, String> {
        if id.contains(['/', '\\']) || id.contains("..") {
            return Err(format!("invalid spec id \"{id}\""));
        }
        let path = self.root.join("specs").join(format!("{id}.json"));
        let text = fs::read_to_string(&path).map_err(|e| io(&path, e))?;
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn run_dir(&self, spec_id: &str, level: &str) -> PathBuf {
        self.root.join("runs").join(spec_id).join(level)
    }

    pub fn write_json(&self, path: &Path, value: &Value) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
        }
        let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
        fs::write(path, text + "\n").map_err(|e| io(path, e))
    }

    pub fn append_jsonl(&self, path: &Path, value: &Value) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
        }
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| io(path, e))?;
        writeln!(f, "{value}").map_err(|e| io(path, e))
    }

    pub fn read_jsonl(&self, path: &Path) -> Result<Vec<Value>, String> {
        match fs::read_to_string(path) {
            Ok(text) => Ok(text
                .lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(io(path, e)),
        }
    }
}
