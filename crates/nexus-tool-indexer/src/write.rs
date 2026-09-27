//! Idempotent, ownership-respecting file writes.
//!
//! Every file this crate writes carries [`MARKER`]. On a re-run: a file that
//! has the marker is refreshed in place (or left alone if the content is
//! already identical); a file at the same path *without* the marker is
//! someone's hand-authored file and is never touched.

use serde::Serialize;
use std::fs;
use std::io;
use std::path::Path;

pub const MARKER: &str = "generated-by: nexus-tool-indexer";

#[derive(Clone, Copy)]
pub enum Status {
    Created,
    Updated,
    Unchanged,
    Skipped,
    WouldWrite,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Created => "created",
            Status::Updated => "updated",
            Status::Unchanged => "unchanged",
            Status::Skipped => "skipped",
            Status::WouldWrite => "would_write",
        }
    }
}

#[derive(Serialize)]
pub struct FileOutcome {
    pub path: String,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn outcome(path: &Path, status: Status, note: Option<&str>) -> FileOutcome {
    FileOutcome {
        path: path.display().to_string(),
        status: status.as_str(),
        note: note.map(str::to_string),
    }
}

/// Writes `content` to `path`, refusing to clobber a file that doesn't carry
/// [`MARKER`]. `dry_run` reports what would happen without touching disk.
pub fn write_generated(path: &Path, content: &str, dry_run: bool) -> io::Result<FileOutcome> {
    if let Ok(existing) = fs::read_to_string(path) {
        if !existing.contains(MARKER) {
            return Ok(outcome(
                path,
                Status::Skipped,
                Some("existing file has no generator marker; left untouched"),
            ));
        }
        if existing == content {
            return Ok(outcome(path, Status::Unchanged, None));
        }
        if dry_run {
            return Ok(outcome(path, Status::WouldWrite, Some("would update")));
        }
        fs::write(path, content)?;
        return Ok(outcome(path, Status::Updated, None));
    }

    if dry_run {
        return Ok(outcome(path, Status::WouldWrite, Some("would create")));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(outcome(path, Status::Created, None))
}
