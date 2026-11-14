//! Buckets: bare, shallow git clones read straight from their object store.
//!
//! No worktree is checked out. An update only fetches and moves the branch,
//! so it cannot conflict with local files, and history rewritten upstream is
//! followed like any other change.

use std::num::NonZeroU32;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use crate::{Error, Layout, Result};

static NEVER_INTERRUPT: AtomicBool = AtomicBool::new(false);

pub struct Bucket {
    name: String,
    repo: gix::Repository,
}

impl Bucket {
    pub fn open(layout: &Layout, name: &str) -> Result<Bucket> {
        let path = layout.buckets().join(name);
        if !path.exists() {
            return Err(Error::BucketNotFound(name.to_owned()));
        }
        let repo = open_repo(&path, None).map_err(git_error(name))?;
        Ok(Bucket {
            name: name.to_owned(),
            repo,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Id of the commit the bucket is at.
    pub fn commit(&self) -> Result<String> {
        let commit = self.repo.head_commit().map_err(git_error(&self.name))?;
        Ok(commit.id.to_string())
    }

    /// The tree holding the manifests: `bucket/` when present, else the root.
    fn manifest_tree(&self) -> Result<gix::Tree<'_>> {
        let err = git_error(&self.name);
        let root = self
            .repo
            .head_commit()
            .map_err(&err)?
            .tree()
            .map_err(&err)?;
        let sub = root.lookup_entry_by_path("bucket").map_err(&err)?;
        match sub {
            Some(entry) if entry.mode().is_tree() => Ok(entry.object().map_err(&err)?.into_tree()),
            _ => Ok(root),
        }
    }

    /// Raw JSON of a package's manifest.
    pub fn manifest(&self, package: &str) -> Result<Option<Vec<u8>>> {
        let err = git_error(&self.name);
        let tree = self.manifest_tree()?;
        let file = format!("{package}.json");
        let Some(entry) = tree.find_entry(file.as_str()) else {
            return Ok(None);
        };
        let blob = entry.object().map_err(&err)?;
        Ok(Some(blob.detach().data))
    }

    /// Package names and raw JSON of every manifest.
    pub fn manifests(&self) -> Result<Vec<(String, Vec<u8>)>> {
        let err = git_error(&self.name);
        let tree = self.manifest_tree()?;
        let mut out = Vec::new();
        for entry in tree.iter() {
            let entry = entry.map_err(&err)?;
            if !entry.mode().is_blob() {
                continue;
            }
            let file = entry.filename().to_string();
            let Some(package) = file.strip_suffix(".json") else {
                continue;
            };
            let data = self
                .repo
                .find_object(entry.oid())
                .map_err(&err)?
                .detach()
                .data;
            out.push((package.to_owned(), data));
        }
        Ok(out)
    }
}

/// Names of the bucket directories, unordered.
pub fn installed(layout: &Layout) -> Result<Vec<String>> {
    let dir = layout.buckets();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(Error::Io { path: dir, source }),
    };
    let mut names = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_dir() && !name.ends_with(".tmp") {
            names.push(name);
        }
    }
    Ok(names)
}

/// Clone `url` as bucket `name`. The bucket appears only once complete.
pub fn add(layout: &Layout, name: &str, url: &str, proxy: Option<&str>) -> Result<()> {
    let dir = layout.buckets();
    let path = dir.join(name);
    if path.exists() {
        return Err(Error::BucketExists(name.to_owned()));
    }
    let tmp = dir.join(format!("{name}.tmp"));
    crate::fs::remove_dir_all(&tmp)?;
    crate::fs::create_dir_all(&dir)?;

    let err = git_error(name);
    let mut prepare = gix::clone::PrepareFetch::new(
        url,
        &tmp,
        gix::create::Kind::Bare,
        gix::create::Options::default(),
        open_options(proxy),
    )
    .map_err(&err)?
    .with_shallow(depth_one());
    prepare
        .fetch_only(gix::progress::Discard, &NEVER_INTERRUPT)
        .map_err(&err)?;
    drop(prepare);
    crate::fs::rename(&tmp, &path)
}

/// Fetch the latest commit of bucket `name`'s branch.
pub fn update(layout: &Layout, name: &str, proxy: Option<&str>) -> Result<()> {
    let path = layout.buckets().join(name);
    let err = git_error(name);
    let repo = open_repo(&path, proxy).map_err(&err)?;
    // Shallow clones store no refspec. Map the remote's default branch onto
    // our HEAD branch, whatever either is named.
    let head = repo.head_name().map_err(&err)?.ok_or_else(|| Error::Git {
        bucket: name.to_owned(),
        message: "HEAD is detached".into(),
    })?;
    let refspec = format!("+HEAD:{}", head.as_bstr());
    let mut remote = repo
        .find_default_remote(gix::remote::Direction::Fetch)
        .ok_or_else(|| Error::Git {
            bucket: name.to_owned(),
            message: "no remote".into(),
        })?
        .map_err(&err)?
        .with_fetch_tags(gix::remote::fetch::Tags::None);
    remote
        .replace_refspecs([refspec.as_str()], gix::remote::Direction::Fetch)
        .map_err(&err)?;
    remote
        .connect(gix::remote::Direction::Fetch)
        .map_err(&err)?
        .prepare_fetch(gix::progress::Discard, Default::default())
        .map_err(&err)?
        .with_shallow(depth_one())
        .receive(gix::progress::Discard, &NEVER_INTERRUPT)
        .map_err(&err)?;
    Ok(())
}

pub fn remove(layout: &Layout, name: &str) -> Result<()> {
    let path = layout.buckets().join(name);
    if !path.exists() {
        return Err(Error::BucketNotFound(name.to_owned()));
    }
    crate::fs::remove_dir_all(&path)
}

/// The fetch url of bucket `name`.
pub fn url(layout: &Layout, name: &str) -> Result<Option<String>> {
    let path = layout.buckets().join(name);
    let repo = open_repo(&path, None).map_err(git_error(name))?;
    let url = repo
        .find_default_remote(gix::remote::Direction::Fetch)
        .and_then(|remote| remote.ok())
        .and_then(|remote| {
            remote
                .url(gix::remote::Direction::Fetch)
                .map(|url| url.to_bstring().to_string())
        });
    Ok(url)
}

fn depth_one() -> gix::remote::fetch::Shallow {
    gix::remote::fetch::Shallow::DepthAtRemote(NonZeroU32::MIN)
}

/// Ignore the user's git configuration, so `url.insteadOf` and the like
/// cannot redirect buckets, and apply wpk's proxy.
fn open_options(proxy: Option<&str>) -> gix::open::Options {
    let overrides: Vec<String> = proxy
        .map(|p| format!("http.proxy={p}"))
        .into_iter()
        .collect();
    gix::open::Options::isolated().config_overrides(overrides)
}

fn open_repo(path: &Path, proxy: Option<&str>) -> gix::Result<gix::Repository> {
    gix::open_opts(path, open_options(proxy))
}

fn git_error<E: std::fmt::Display>(bucket: &str) -> impl Fn(E) -> Error + '_ {
    move |e| Error::Git {
        bucket: bucket.to_owned(),
        message: e.to_string(),
    }
}
