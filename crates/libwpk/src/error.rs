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

    #[error("{url}: {message}")]
    Download { url: String, message: String },

    #[error("{url}: hash mismatch, expected {expected}, got {actual}")]
    HashMismatch {
        url: String,
        expected: String,
        actual: String,
    },

    #[error("{file}: {message}")]
    Extract { file: String, message: String },

    #[error("extracting needs the '{0}' package; install it with `wpk install {0}`")]
    MissingTool(&'static str),

    #[error("{}: {message}", path.display())]
    Corrupt { path: PathBuf, message: String },

    #[error("'{0}' is not installed")]
    NotInstalled(String),
}
