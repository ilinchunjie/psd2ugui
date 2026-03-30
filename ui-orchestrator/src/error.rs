use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum OrchestratorError {
    #[error("bundle directory not found: {0}")]
    MissingBundle(PathBuf),
    #[error("missing required file: {0}")]
    MissingFile(PathBuf),
    #[error("failed to read {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write {path}: {source}")]
    WriteFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse json from {path}: {source}")]
    ParseJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("plan validation failed:\n{0}")]
    PlanValidation(String),
}

pub type Result<T> = std::result::Result<T, OrchestratorError>;
