//! [`Wpk`], the entry point for frontends.

use crate::config::BucketEntry;
use crate::shim::ShimExes;
use crate::{Config, Error, Layout, Result, bucket};

pub struct Wpk {
    pub layout: Layout,
    pub config: Config,
    /// Where shims are copied from.
    pub shims: ShimExes,
}

impl Wpk {
    /// Load the config of an initialized wpk root.
    pub fn open(layout: Layout) -> Result<Wpk> {
        let path = layout.config();
        if !path.exists() {
            return Err(Error::NotInitialized(layout.root().to_owned()));
        }
        let config = Config::load(&path)?;
        Ok(Wpk {
            layout,
            config,
            shims: ShimExes::beside_current_exe()?,
        })
    }

    pub fn save_config(&self) -> Result<()> {
        self.config.save(&self.layout.config())
    }

    fn proxy(&self) -> Option<&str> {
        self.config.proxy.as_deref()
    }

    /// Clone a bucket and append it to the config, lowest priority.
    pub fn bucket_add(&mut self, name: &str, url: &str) -> Result<()> {
        if self.config.bucket(name).is_some() {
            return Err(Error::BucketExists(name.to_owned()));
        }
        bucket::add(&self.layout, name, url, self.proxy())?;
        self.config.buckets.push(BucketEntry {
            name: name.to_owned(),
            url: url.to_owned(),
        });
        self.save_config()
    }

    pub fn bucket_remove(&mut self, name: &str) -> Result<()> {
        let before = self.config.buckets.len();
        self.config.buckets.retain(|b| b.name != name);
        if self.config.buckets.len() == before {
            return Err(Error::BucketNotFound(name.to_owned()));
        }
        bucket::remove(&self.layout, name)?;
        self.save_config()
    }

    /// Fetch every configured bucket in parallel, cloning missing ones.
    /// `report` gets each bucket's result as it finishes.
    pub fn update(&self, report: impl Fn(&str, Result<()>) + Sync) {
        std::thread::scope(|scope| {
            for entry in &self.config.buckets {
                let report = &report;
                scope.spawn(move || {
                    let result = if self.layout.buckets().join(&entry.name).exists() {
                        bucket::update(&self.layout, &entry.name, self.proxy())
                    } else {
                        bucket::add(&self.layout, &entry.name, &entry.url, self.proxy())
                    };
                    report(&entry.name, result);
                });
            }
        });
    }

    /// Open the configured buckets in priority order. Missing ones are
    /// skipped; `wpk update` clones them.
    pub fn buckets(&self) -> Vec<bucket::Bucket> {
        self.config
            .buckets
            .iter()
            .filter_map(|entry| bucket::Bucket::open(&self.layout, &entry.name).ok())
            .collect()
    }
}
