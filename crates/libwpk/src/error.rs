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

    #[error("'{0}' cannot be a bucket name")]
    BucketName(String),

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

    #[error("{}: {message}", path.display())]
    Shortcut { path: PathBuf, message: String },

    #[error("package '{0}' not found")]
    PackageNotFound(String),

    #[error("{package}: invalid manifest: {message}")]
    ManifestParse { package: String, message: String },

    #[error("{package}: cannot install: {reason}")]
    Unsupported {
        package: String,
        reason: crate::manifest::Unsupported,
    },

    #[error("{package}: '{name}' is already provided by '{owner}'")]
    Conflict {
        package: String,
        name: String,
        owner: String,
    },

    #[error("dependency cycle through '{0}'")]
    DependencyCycle(String),

    #[error("{package}: {message}")]
    Install { package: String, message: String },

    #[error("wpk cannot uninstall itself")]
    UninstallSelf,

    #[error("failed: {}", .0.join(", "))]
    Failed(Vec<String>),
}
