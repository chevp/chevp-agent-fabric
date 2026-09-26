use crate::errors::{NexusError, NexusResult};
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

pub fn path_exists(path: &Path) -> bool {
    path.exists()
}

/// Immediate subdirectories of `dir`. Empty if `dir` doesn't exist.
pub fn list_subdirectories(dir: &Path) -> NexusResult<Vec<String>> {
    if !path_exists(dir) {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in read_dir(dir)? {
        let entry = entry.map_err(|source| io_err(dir, source))?;
        if entry
            .file_type()
            .map_err(|source| io_err(dir, source))?
            .is_dir()
        {
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

/// Recursively lists files under `dir` whose extension is in `extensions`
/// (each given without the leading dot, e.g. "yaml"). Empty if `dir`
/// doesn't exist.
pub fn list_files_recursive(dir: &Path, extensions: &[&str]) -> NexusResult<Vec<PathBuf>> {
    let mut results = Vec::new();
    if !path_exists(dir) {
        return Ok(results);
    }
    walk(dir, extensions, &mut results)?;
    results.sort();
    Ok(results)
}

fn walk(dir: &Path, extensions: &[&str], results: &mut Vec<PathBuf>) -> NexusResult<()> {
    for entry in read_dir(dir)? {
        let entry = entry.map_err(|source| io_err(dir, source))?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| io_err(dir, source))?;
        if file_type.is_dir() {
            walk(&path, extensions, results)?;
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if extensions.contains(&ext) {
                results.push(path);
            }
        }
    }
    Ok(())
}

fn read_dir(dir: &Path) -> NexusResult<std::fs::ReadDir> {
    std::fs::read_dir(dir).map_err(|source| io_err(dir, source))
}

fn io_err(path: &Path, source: std::io::Error) -> NexusError {
    NexusError::Io {
        path: path.to_path_buf(),
        source,
    }
}

pub fn read_file_to_string(path: &Path) -> NexusResult<String> {
    std::fs::read_to_string(path).map_err(|source| io_err(path, source))
}

/// Reads and deserializes a YAML file, mapping any parse/schema error to a
/// `Validation` error carrying the offending path.
pub fn read_yaml_file<T: DeserializeOwned>(path: &Path) -> NexusResult<T> {
    let raw = read_file_to_string(path)?;
    serde_yaml::from_str(&raw).map_err(|err| NexusError::Validation {
        message: format!("invalid YAML: {err}"),
        path: path.to_path_buf(),
    })
}
