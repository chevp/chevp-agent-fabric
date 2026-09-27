use crate::model::{Event, Job, Org, Vision};
use nexus_tools::{ToolContext, ToolError};
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn io_err(path: &Path, err: std::io::Error) -> ToolError {
    ToolError::Other(format!("I/O error at {}: {err}", path.display()))
}

/// The `studio/` directory of one project.
pub struct Studio {
    dir: PathBuf,
}

/// Exclusive claim on one job file; the lock file is removed on drop.
pub struct JobLock {
    path: PathBuf,
}

impl Drop for JobLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn job_number(id: &str) -> Option<u64> {
    id.strip_prefix("JOB-")?.parse().ok()
}

impl Studio {
    pub fn open(ctx: &ToolContext<'_>, project_id: &str) -> Result<Self, ToolError> {
        let project = ctx.domain.projects.require_project(project_id)?;
        Ok(Self {
            dir: project.path.join("studio"),
        })
    }

    fn jobs_dir(&self) -> PathBuf {
        self.dir.join("jobs")
    }

    fn events_path(&self) -> PathBuf {
        self.dir.join("events.jsonl")
    }

    pub fn org(&self) -> Result<Org, ToolError> {
        let path = self.dir.join("org.yaml");
        let text = fs::read_to_string(&path).map_err(|err| match err.kind() {
            ErrorKind::NotFound => ToolError::InvalidInput(format!(
                "project has no studio org ({}); start from the template in nexus-tool-game-studio/templates/org.yaml",
                path.display()
            )),
            _ => io_err(&path, err),
        })?;
        serde_yaml::from_str(&text)
            .map_err(|err| ToolError::Other(format!("invalid {}: {err}", path.display())))
    }

    pub fn vision(&self) -> Result<Option<Vision>, ToolError> {
        let path = self.dir.join("vision.yaml");
        match fs::read_to_string(&path) {
            Ok(text) => serde_yaml::from_str(&text)
                .map(Some)
                .map_err(|err| ToolError::Other(format!("invalid {}: {err}", path.display()))),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(io_err(&path, err)),
        }
    }

    fn job_path(&self, id: &str) -> Result<PathBuf, ToolError> {
        if job_number(id).is_none() {
            return Err(ToolError::InvalidInput(format!(
                "\"{id}\" is not a job id (expected JOB-<number>)"
            )));
        }
        Ok(self.jobs_dir().join(format!("{id}.json")))
    }

    pub fn job(&self, id: &str) -> Result<Job, ToolError> {
        let path = self.job_path(id)?;
        let text = fs::read_to_string(&path).map_err(|err| match err.kind() {
            ErrorKind::NotFound => ToolError::InvalidInput(format!("job \"{id}\" not found")),
            _ => io_err(&path, err),
        })?;
        serde_json::from_str(&text)
            .map_err(|err| ToolError::Other(format!("invalid {}: {err}", path.display())))
    }

    /// All jobs, ordered by id.
    pub fn jobs(&self) -> Result<Vec<Job>, ToolError> {
        let dir = self.jobs_dir();
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(io_err(&dir, err)),
        };
        let mut ids: Vec<(u64, String)> = Vec::new();
        for entry in entries {
            let name = entry.map_err(|err| io_err(&dir, err))?.file_name();
            let name = name.to_string_lossy();
            if let Some(id) = name.strip_suffix(".json") {
                if let Some(n) = job_number(id) {
                    ids.push((n, id.to_string()));
                }
            }
        }
        ids.sort();
        ids.iter().map(|(_, id)| self.job(id)).collect()
    }

    /// Assigns the next free `JOB-nnnnnn` id and writes the job.
    pub fn create_job(&self, mut job: Job) -> Result<Job, ToolError> {
        let dir = self.jobs_dir();
        fs::create_dir_all(&dir).map_err(|err| io_err(&dir, err))?;
        let mut n = self
            .jobs()?
            .iter()
            .filter_map(|j| job_number(&j.id))
            .max()
            .unwrap_or(0)
            + 1;
        loop {
            job.id = format!("JOB-{n:06}");
            let path = dir.join(format!("{}.json", job.id));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    let text = serde_json::to_string_pretty(&job)
                        .map_err(|err| ToolError::Other(err.to_string()))?;
                    file.write_all(text.as_bytes())
                        .map_err(|err| io_err(&path, err))?;
                    return Ok(job);
                }
                Err(err) if err.kind() == ErrorKind::AlreadyExists => n += 1,
                Err(err) => return Err(io_err(&path, err)),
            }
        }
    }

    pub fn save_job(&self, job: &Job) -> Result<(), ToolError> {
        let path = self.job_path(&job.id)?;
        let tmp = path.with_extension("json.tmp");
        let text =
            serde_json::to_string_pretty(job).map_err(|err| ToolError::Other(err.to_string()))?;
        fs::write(&tmp, text).map_err(|err| io_err(&tmp, err))?;
        fs::rename(&tmp, &path).map_err(|err| io_err(&path, err))
    }

    /// `None` if another process holds the job right now.
    pub fn try_lock(&self, id: &str) -> Result<Option<JobLock>, ToolError> {
        let path = self.job_path(id)?.with_extension("lock");
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => Ok(Some(JobLock { path })),
            Err(err) if err.kind() == ErrorKind::AlreadyExists => Ok(None),
            Err(err) => Err(io_err(&path, err)),
        }
    }

    pub fn events(&self) -> Result<Vec<Event>, ToolError> {
        let path = self.events_path();
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(io_err(&path, err)),
        };
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .enumerate()
            .map(|(i, line)| {
                serde_json::from_str(line).map_err(|err| {
                    ToolError::Other(format!("invalid {} line {}: {err}", path.display(), i + 1))
                })
            })
            .collect()
    }

    /// Stamps `seq`/`ts` and appends. Returns the stored events.
    pub fn append_events(&self, mut events: Vec<Event>) -> Result<Vec<Event>, ToolError> {
        if events.is_empty() {
            return Ok(events);
        }
        let path = self.events_path();
        fs::create_dir_all(&self.dir).map_err(|err| io_err(&self.dir, err))?;
        let mut seq = self.events()?.last().map_or(0, |e| e.seq);
        let ts = now();
        let mut buf = String::new();
        for event in &mut events {
            seq += 1;
            event.seq = seq;
            event.ts = ts;
            buf.push_str(
                &serde_json::to_string(event).map_err(|err| ToolError::Other(err.to_string()))?,
            );
            buf.push('\n');
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|err| io_err(&path, err))?;
        file.write_all(buf.as_bytes())
            .map_err(|err| io_err(&path, err))?;
        Ok(events)
    }
}
