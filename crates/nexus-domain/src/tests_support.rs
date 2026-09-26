#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// An isolated temp directory with `projects/` and `global/` roots for tests.
pub struct TmpRepo {
    _dir: TempDir,
    #[allow(dead_code)]
    pub root: PathBuf,
    pub projects_root: PathBuf,
    pub global_root: PathBuf,
}

impl TmpRepo {
    pub fn new() -> Self {
        let dir = TempDir::new().expect("create tempdir");
        let root = dir.path().to_path_buf();
        let projects_root = root.join("projects");
        let global_root = root.join("global");
        fs::create_dir_all(&projects_root).unwrap();
        fs::create_dir_all(&global_root).unwrap();
        Self {
            _dir: dir,
            root,
            projects_root,
            global_root,
        }
    }
}

pub fn write_file(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}
