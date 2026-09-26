use std::path::PathBuf;

/// All domain-level failures. Kept small and concrete on purpose: this is
/// not a generic "backend error" abstraction, it enumerates exactly the
/// things that can go wrong when reading the Git-backed project data.
#[derive(Debug, thiserror::Error)]
pub enum NexusError {
    #[error("{kind} \"{id}\" was not found")]
    NotFound { kind: &'static str, id: String },

    #[error("{message} ({path})")]
    Validation { message: String, path: PathBuf },

    #[error("client \"{client}\" lacks permission \"{permission}\"")]
    PermissionDenied { client: String, permission: String },

    #[error("{0}")]
    VersionConflict(String),

    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub type NexusResult<T> = Result<T, NexusError>;
