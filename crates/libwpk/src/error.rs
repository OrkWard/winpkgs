use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("home directory not found")]
    NoHome,

    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("{path}: {source}")]
    ConfigParse {
        path: PathBuf,
        source: toml::de::Error,
    },

    #[error("bucket '{0}' not found")]
    BucketNotFound(String),

    #[error("bucket '{0}' already exists")]
    BucketExists(String),

    #[error("bucket '{bucket}': {message}")]
    Git { bucket: String, message: String },

    #[error("wpk is not initialized at {}; run `wpk init`", .0.display())]
    NotInitialized(PathBuf),
}
