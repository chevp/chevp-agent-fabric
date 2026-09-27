//! Thin wrapper over the `git` CLI. Every function returns `None`/empty when
//! git is missing or the path is not in a repository; ingestion then simply
//! has no history.

use serde::Serialize;
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn available(dir: &Path) -> bool {
    git(dir, &["rev-parse", "--is-inside-work-tree"]).is_some_and(|s| s.trim() == "true")
}

pub fn head(dir: &Path) -> Option<String> {
    git(dir, &["rev-parse", "HEAD"]).map(|s| s.trim().to_string())
}

/// Last commit that touched `path` (relative to `dir`).
pub fn last_commit_for(dir: &Path, path: &str) -> Option<String> {
    git(dir, &["log", "-1", "--format=%H", "--", path])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Content of `path` (relative to `dir`) at `rev`.
pub fn show(dir: &Path, rev: &str, path: &str) -> Option<String> {
    git(dir, &["show", &format!("{rev}:./{path}")])
}

#[derive(Debug, Clone, Serialize)]
pub struct CommitFile {
    pub status: String,
    /// Relative to the directory that was queried.
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommitInfo {
    pub sha: String,
    pub author: String,
    pub date: String,
    pub subject: String,
    pub files: Vec<CommitFile>,
}

/// Most recent commits touching `dir`, newest first, with file paths made
/// relative to `dir`.
pub fn log(dir: &Path, max: usize) -> Vec<CommitInfo> {
    let prefix = git(dir, &["rev-parse", "--show-prefix"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let Some(raw) = git(
        dir,
        &[
            "log",
            &format!("--max-count={max}"),
            "--name-status",
            "--format=%x1e%H%x1f%an%x1f%aI%x1f%s",
            "--",
            ".",
        ],
    ) else {
        return Vec::new();
    };
    raw.split('\u{1e}')
        .filter(|r| !r.trim().is_empty())
        .filter_map(|record| {
            let mut lines = record.lines();
            let header: Vec<&str> = lines.next()?.split('\u{1f}').collect();
            if header.len() < 4 {
                return None;
            }
            let files = lines
                .filter_map(|l| {
                    let mut parts = l.split('\t');
                    let status = parts.next()?.trim().to_string();
                    let path = parts.last()?.trim();
                    let rel = path.strip_prefix(prefix.as_str())?;
                    (!status.is_empty()).then(|| CommitFile {
                        status: status.chars().next().unwrap_or('M').to_string(),
                        path: rel.to_string(),
                    })
                })
                .collect();
            Some(CommitInfo {
                sha: header[0].to_string(),
                author: header[1].to_string(),
                date: header[2].to_string(),
                subject: header[3].to_string(),
                files,
            })
        })
        .collect()
}
