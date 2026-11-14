//! `opt/wpk/config.toml`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,

    /// Days between garbage collections of old versions.
    #[serde(default = "default_gc_interval_days")]
    pub gc_interval_days: u32,

    /// Buckets in priority order, highest first.
    #[serde(default, rename = "bucket")]
    pub buckets: Vec<BucketEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct BucketEntry {
    pub name: String,
    pub url: String,
}

/// Buckets `wpk init` adds.
pub const DEFAULT_BUCKETS: [(&str, &str); 3] = [
    ("wpk", "https://github.com/OrkWard/wpk-bucket"),
    ("main", "https://github.com/ScoopInstaller/Main"),
    ("extras", "https://github.com/ScoopInstaller/Extras"),
];

fn default_gc_interval_days() -> u32 {
    7
}

impl Default for Config {
    fn default() -> Config {
        Config {
            proxy: None,
            gc_interval_days: default_gc_interval_days(),
            buckets: DEFAULT_BUCKETS
                .iter()
                .map(|&(name, url)| BucketEntry {
                    name: name.to_owned(),
                    url: url.to_owned(),
                })
                .collect(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        let text = std::fs::read_to_string(path).map_err(|source| Error::Io {
            path: path.to_owned(),
            source,
        })?;
        toml::from_str(&text).map_err(|source| Error::ConfigParse {
            path: path.to_owned(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_round_trips() {
        let config = Config::default();
        let text = toml::to_string(&config).unwrap();
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), config);
    }

    #[test]
    fn empty_file_uses_defaults_without_buckets() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.gc_interval_days, 7);
        assert!(config.buckets.is_empty());
    }
}
