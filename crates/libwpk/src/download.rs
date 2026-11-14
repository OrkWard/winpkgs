//! Downloading package files into the cache.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use md5::Md5;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};

use crate::fs::io_error;
use crate::manifest::{Download, Hash, HashAlgo};
use crate::{Error, Layout, Result};

/// Servers that special-case Scoop see the same request from wpk.
const USER_AGENT: &str = concat!(
    "Scoop/1.0 (+http://scoop.sh/) wpk/",
    env!("CARGO_PKG_VERSION")
);

/// Cache file name: `<pkg>#<version>#<url>` with unsafe characters replaced,
/// as Scoop names them.
pub fn cache_name(package: &str, version: &str, download: &Download) -> String {
    let url = format!("{}#/{}", download.url, download.file_name);
    let safe: String = url
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{package}#{version}#{safe}")
}

/// Path of the verified download in the cache, fetching it when missing or
/// when the cached copy does not match its hash. `progress` gets bytes done
/// and the total when known.
pub fn fetch(
    layout: &Layout,
    proxy: Option<&str>,
    package: &str,
    version: &str,
    download: &Download,
    progress: &(dyn Fn(u64, Option<u64>) + Sync),
) -> Result<PathBuf> {
    let cache = layout.cache();
    crate::fs::create_dir_all(cache)?;
    let path = cache.join(cache_name(package, version, download));
    if path.exists() {
        match &download.hash {
            None => return Ok(path),
            Some(hash) if hash_file(&path, hash.algo)? == hash.hex => return Ok(path),
            Some(_) => crate::fs::remove_file(&path)?,
        }
    }

    let mut partial = path.clone().into_os_string();
    partial.push(".download");
    let partial = PathBuf::from(partial);
    let result = get(
        &download.url,
        proxy,
        &partial,
        download.hash.as_ref().map(|h| h.algo),
        progress,
    );
    let actual = match result {
        Ok(actual) => actual,
        Err(e) => {
            let _ = std::fs::remove_file(&partial);
            return Err(e);
        }
    };
    if let (Some(hash), Some(actual)) = (&download.hash, actual)
        && actual != hash.hex
    {
        let _ = std::fs::remove_file(&partial);
        return Err(Error::HashMismatch {
            url: download.url.clone(),
            expected: hash.to_string(),
            actual,
        });
    }
    crate::fs::rename(&partial, &path)?;
    Ok(path)
}

/// Download `url` to `dest`, returning the hex digest when `algo` is given.
fn get(
    url: &str,
    proxy: Option<&str>,
    dest: &Path,
    algo: Option<HashAlgo>,
    progress: &(dyn Fn(u64, Option<u64>) + Sync),
) -> Result<Option<String>> {
    let net = |e: curl::Error| Error::Download {
        url: url.to_owned(),
        message: e.to_string(),
    };
    let mut file = File::create(dest).map_err(io_error(dest))?;
    let mut hasher = algo.map(Hasher::new);
    let mut write_error = None;

    let mut easy = curl::easy::Easy::new();
    easy.url(url).map_err(net)?;
    easy.follow_location(true).map_err(net)?;
    easy.max_redirections(20).map_err(net)?;
    easy.useragent(USER_AGENT).map_err(net)?;
    easy.fail_on_error(true).map_err(net)?;
    easy.progress(true).map_err(net)?;
    if let Some(proxy) = proxy {
        easy.proxy(proxy).map_err(net)?;
    }
    {
        let mut transfer = easy.transfer();
        transfer
            .write_function(|data| {
                if let Err(e) = file.write_all(data) {
                    write_error = Some(e);
                    return Ok(0);
                }
                if let Some(h) = hasher.as_mut() {
                    h.update(data);
                }
                Ok(data.len())
            })
            .map_err(net)?;
        transfer
            .progress_function(|total, done, _, _| {
                let total = (total > 0.0).then_some(total as u64);
                progress(done as u64, total);
                true
            })
            .map_err(net)?;
        let performed = transfer.perform();
        drop(transfer);
        if let Some(e) = write_error {
            return Err(io_error(dest)(e));
        }
        performed.map_err(net)?;
    }
    file.flush().map_err(io_error(dest))?;
    Ok(hasher.map(Hasher::finish))
}

pub fn hash_file(path: &Path, algo: HashAlgo) -> Result<String> {
    let mut file = File::open(path).map_err(io_error(path))?;
    let mut hasher = Hasher::new(algo);
    let mut buf = vec![0; 1 << 16];
    loop {
        let n = std::io::Read::read(&mut file, &mut buf).map_err(io_error(path))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finish())
}

enum Hasher {
    Md5(Md5),
    Sha1(Sha1),
    Sha256(Sha256),
    Sha512(Sha512),
}

impl Hasher {
    fn new(algo: HashAlgo) -> Hasher {
        match algo {
            HashAlgo::Md5 => Hasher::Md5(Md5::new()),
            HashAlgo::Sha1 => Hasher::Sha1(Sha1::new()),
            HashAlgo::Sha256 => Hasher::Sha256(Sha256::new()),
            HashAlgo::Sha512 => Hasher::Sha512(Sha512::new()),
        }
    }

    fn update(&mut self, data: &[u8]) {
        match self {
            Hasher::Md5(h) => h.update(data),
            Hasher::Sha1(h) => h.update(data),
            Hasher::Sha256(h) => h.update(data),
            Hasher::Sha512(h) => h.update(data),
        }
    }

    fn finish(self) -> String {
        let bytes = match self {
            Hasher::Md5(h) => h.finalize().to_vec(),
            Hasher::Sha1(h) => h.finalize().to_vec(),
            Hasher::Sha256(h) => h.finalize().to_vec(),
            Hasher::Sha512(h) => h.finalize().to_vec(),
        };
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Whether `hash` matches the file, for callers holding a [`Hash`].
pub fn verify(path: &Path, hash: &Hash) -> Result<bool> {
    Ok(hash_file(path, hash.algo)? == hash.hex)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Extract;

    #[test]
    fn digests() {
        let dir = crate::testutil::TempDir::new("hash");
        let file = dir.join("abc");
        std::fs::write(&file, "abc").unwrap();
        assert_eq!(
            hash_file(&file, HashAlgo::Md5).unwrap(),
            "900150983cd24fb0d6963f7d28e17f72"
        );
        assert_eq!(
            hash_file(&file, HashAlgo::Sha256).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn cache_names_are_safe() {
        let d = Download {
            url: "https://h/a b.zip?x=1".into(),
            file_name: "a b.zip".into(),
            hash: None,
            extract: Extract::Zip,
            extract_dir: None,
            extract_to: None,
        };
        assert_eq!(
            cache_name("fd", "1.0", &d),
            "fd#1.0#https___h_a_b.zip_x_1__a_b.zip"
        );
    }
}
