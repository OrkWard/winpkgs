//! Finding packages in buckets.

use crate::bucket::Bucket;
use crate::manifest::{Arch, Plan, Unsupported};
use crate::{Error, Manifest, Result, Wpk};

/// A package as a bucket offers it.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub name: String,
    pub bucket: String,
    pub commit: Option<String>,
    pub manifest: Manifest,
    /// The manifest file as is, kept with the installed version.
    pub raw: Vec<u8>,
}

impl Candidate {
    pub fn plan(&self) -> std::result::Result<Plan, Unsupported> {
        self.manifest.plan(Arch::host())
    }

    /// `bucket/name`.
    pub fn ident(&self) -> String {
        format!("{}/{}", self.bucket, self.name)
    }
}

/// Split `bucket/name` into its parts.
pub fn split_query(query: &str) -> (Option<&str>, &str) {
    match query.split_once('/') {
        Some((bucket, name)) => (Some(bucket), name),
        None => (None, query),
    }
}

impl Wpk {
    /// The package `query` names, `name` or `bucket/name`, from the first
    /// bucket in priority order that has it.
    pub fn find(&self, query: &str) -> Result<Candidate> {
        let (bucket, name) = split_query(query);
        let name = name.to_ascii_lowercase();
        let buckets = match bucket {
            Some(bucket) => vec![Bucket::open(&self.layout, bucket)?],
            None => self.buckets(),
        };
        for bucket in buckets {
            if let Some(raw) = bucket.manifest(&name)? {
                return candidate(&bucket, name, raw);
            }
        }
        Err(Error::PackageNotFound(query.to_owned()))
    }

    /// Packages whose name contains `pattern`, or which provide a command
    /// containing it, case-insensitively. Unparsable manifests are skipped.
    pub fn search(&self, pattern: &str) -> Result<Vec<Candidate>> {
        let pattern = pattern.to_ascii_lowercase();
        let names: Vec<&str> = self.config.buckets.iter().map(|b| b.name.as_str()).collect();
        let results: Vec<Result<Vec<Candidate>>> = std::thread::scope(|scope| {
            let handles: Vec<_> = names
                .iter()
                .map(|name| {
                    let pattern = &pattern;
                    scope.spawn(move || -> Result<Vec<Candidate>> {
                        // Not cloned yet; `wpk update` clones it.
                        let Ok(bucket) = Bucket::open(&self.layout, name) else {
                            return Ok(Vec::new());
                        };
                        let mut out = Vec::new();
                        for (name, raw) in bucket.manifests()? {
                            let name_hit = name.to_ascii_lowercase().contains(pattern.as_str());
                            if !name_hit && !raw_mentions(&raw, pattern) {
                                continue;
                            }
                            let Ok(candidate) = candidate(&bucket, name, raw) else {
                                continue;
                            };
                            if name_hit || provides(&candidate, pattern) {
                                out.push(candidate);
                            }
                        }
                        Ok(out)
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().expect("search thread")).collect()
        });
        let mut all = Vec::new();
        for result in results {
            all.extend(result?);
        }
        Ok(all)
    }
}

fn candidate(bucket: &Bucket, name: String, raw: Vec<u8>) -> Result<Candidate> {
    let manifest = Manifest::parse(&raw).map_err(|e| Error::ManifestParse {
        package: format!("{}/{name}", bucket.name()),
        message: e.to_string(),
    })?;
    Ok(Candidate {
        name,
        bucket: bucket.name().to_owned(),
        commit: bucket.commit().ok(),
        manifest,
        raw,
    })
}

/// Cheap prefilter before parsing: the pattern appears in the JSON at all.
fn raw_mentions(raw: &[u8], pattern: &str) -> bool {
    String::from_utf8_lossy(raw).to_ascii_lowercase().contains(pattern)
}

fn provides(candidate: &Candidate, pattern: &str) -> bool {
    candidate.plan().is_ok_and(|plan| {
        plan.bins
            .iter()
            .any(|bin| bin.name.to_ascii_lowercase().contains(pattern))
    })
}
