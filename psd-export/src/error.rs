use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("failed to read or write {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to serialize manifest: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid CLI usage: {0}")]
    Cli(String),
    #[error("unsupported PSD: {0}")]
    UnsupportedPsd(String),
    #[error("failed to parse PSD: {0}")]
    PsdParse(String),
    #[error("strict mode aborted export because {warning_count} warning(s) were raised")]
    StrictWarnings { warning_count: usize },
    #[error("manifest generation failed: {0}")]
    Manifest(String),
    #[error("photoshop raster export failed: {0}")]
    Photoshop(String),
}

impl AppError {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Cli(_) => 2,
            Self::UnsupportedPsd(_) => 3,
            Self::PsdParse(_) => 4,
            Self::StrictWarnings { .. } => 5,
            Self::Manifest(_) => 6,
            Self::Photoshop(_) => 7,
            Self::Io { .. } | Self::Json(_) => 1,
        }
    }

    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
