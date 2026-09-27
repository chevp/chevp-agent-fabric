use nexus_domain::NexusError;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum SemanticError {
    #[error(transparent)]
    Domain(#[from] NexusError),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("could not parse {source_path}: {message}")]
    Parse {
        source_path: String,
        message: String,
    },

    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{0}")]
    Provider(String),
}

impl SemanticError {
    pub fn not_found(kind: &'static str, id: impl Into<String>) -> Self {
        SemanticError::Domain(NexusError::NotFound {
            kind,
            id: id.into(),
        })
    }

    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        SemanticError::Io {
            path: path.into(),
            source,
        }
    }
}

pub type SemanticResult<T> = Result<T, SemanticError>;
