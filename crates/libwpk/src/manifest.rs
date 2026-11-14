//! Scoop manifests, and the install plan wpk derives from one.
//!
//! Parsing accepts any well-formed Scoop manifest so search and info work on
//! every package. [`Manifest::plan`] is where unsupported packages are
//! rejected.

use std::collections::BTreeMap;
use std::fmt;

use serde::de::Deserializer;
use serde::{Deserialize, Serialize};

/// Fields wpk refuses to install, at the top level or under `architecture`.
pub const REJECTED_FIELDS: [&str; 8] = [
    "pre_install",
    "post_install",
    "installer",
    "uninstaller",
    "pre_uninstall",
    "post_uninstall",
    "psmodule",
    "env_add_path",
];

/// Extensions a `bin` or shortcut target may have.
pub const RUNNABLE_EXTENSIONS: [&str; 3] = ["exe", "bat", "cmd"];

#[derive(Clone, Debug, Deserialize)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub license: Option<License>,
    #[serde(default)]
    pub depends: OneOrMany<String>,
    #[serde(default)]
    pub suggest: BTreeMap<String, OneOrMany<String>>,
    #[serde(default)]
    pub notes: OneOrMany<String>,
    #[serde(default)]
    pub innosetup: bool,
    #[serde(default)]
    pub extract_to: OneOrMany<String>,
    #[serde(default)]
    pub persist: OneOrMany<OneOrMany<String>>,
    /// wpk extension: AppUserModelID per shortcut name.
    #[serde(default)]
    pub shortcut_aumid: BTreeMap<String, String>,
    #[serde(default)]
    pub architecture: BTreeMap<String, ArchFields>,
    #[serde(flatten)]
    pub common: ArchFields,
}

/// Fields that `architecture.<arch>` can override.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ArchFields {
    #[serde(default)]
    pub url: Option<OneOrMany<String>>,
    #[serde(default)]
    pub hash: Option<OneOrMany<String>>,
    #[serde(default)]
    pub extract_dir: Option<OneOrMany<String>>,
    #[serde(default)]
    pub bin: Option<OneOrMany<OneOrMany<String>>>,
    #[serde(default)]
    pub shortcuts: Option<Vec<Vec<String>>>,
    #[serde(default)]
    pub env_set: Option<BTreeMap<String, String>>,
    /// Everything else, kept to find rejected fields.
    #[serde(flatten)]
    pub rest: BTreeMap<String, serde_json::Value>,
}

/// A string or an array of them in JSON.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct OneOrMany<T>(pub Vec<T>);

impl<T> Default for OneOrMany<T> {
    fn default() -> Self {
        OneOrMany(Vec::new())
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for OneOrMany<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Either<T> {
            Many(Vec<T>),
            One(T),
        }
        Ok(match Either::deserialize(deserializer)? {
            Either::Many(items) => OneOrMany(items),
            Either::One(item) => OneOrMany(vec![item]),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct License {
    pub identifier: String,
    pub url: Option<String>,
}

impl<'de> Deserialize<'de> for License {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Id(String),
            Full {
                identifier: String,
                url: Option<String>,
            },
        }
        Ok(match Raw::deserialize(deserializer)? {
            Raw::Id(identifier) => License {
                identifier,
                url: None,
            },
            Raw::Full { identifier, url } => License { identifier, url },
        })
    }
}

impl fmt::Display for License {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.url {
            Some(url) => write!(f, "{} ({url})", self.identifier),
            None => f.write_str(&self.identifier),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    X64,
    Arm64,
}

impl Arch {
    pub fn host() -> Arch {
        if cfg!(target_arch = "aarch64") {
            Arch::Arm64
        } else {
            Arch::X64
        }
    }

    /// `architecture` keys to try, in order. arm64 Windows runs x64 code.
    fn manifest_keys(self) -> &'static [&'static str] {
        match self {
            Arch::X64 => &["64bit"],
            Arch::Arm64 => &["arm64", "64bit"],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HashAlgo {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hash {
    pub algo: HashAlgo,
    /// Lowercase hex.
    pub hex: String,
}

impl Hash {
    /// `sha256:<hex>`, `md5:<hex>` etc., or bare hex whose length gives the
    /// algorithm (bare sha1 and sha512 included).
    pub fn parse(s: &str) -> Option<Hash> {
        let (algo, hex) = match s.split_once(':') {
            Some(("md5", hex)) => (HashAlgo::Md5, hex),
            Some(("sha1", hex)) => (HashAlgo::Sha1, hex),
            Some(("sha256", hex)) => (HashAlgo::Sha256, hex),
            Some(("sha512", hex)) => (HashAlgo::Sha512, hex),
            Some(_) => return None,
            None => {
                let algo = match s.len() {
                    32 => HashAlgo::Md5,
                    40 => HashAlgo::Sha1,
                    64 => HashAlgo::Sha256,
                    128 => HashAlgo::Sha512,
                    _ => return None,
                };
                (algo, s)
            }
        };
        let expected = match algo {
            HashAlgo::Md5 => 32,
            HashAlgo::Sha1 => 40,
            HashAlgo::Sha256 => 64,
            HashAlgo::Sha512 => 128,
        };
        if hex.len() != expected || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        Some(Hash {
            algo,
            hex: hex.to_ascii_lowercase(),
        })
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let algo = match self.algo {
            HashAlgo::Md5 => "md5",
            HashAlgo::Sha1 => "sha1",
            HashAlgo::Sha256 => "sha256",
            HashAlgo::Sha512 => "sha512",
        };
        write!(f, "{algo}:{}", self.hex)
    }
}

/// Why a package cannot be installed by wpk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unsupported {
    /// Has a field wpk does not run, such as an install script.
    Field(String),
    /// No download for this architecture.
    Arch(Arch),
    /// A `bin` or shortcut target that is not exe, bat or cmd.
    Target(String),
    /// A hash wpk cannot read.
    Hash(String),
    /// More hashes than urls, or entries of the wrong shape.
    Malformed(String),
}

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Unsupported::Field(field) => write!(f, "uses `{field}`"),
            Unsupported::Arch(arch) => write!(f, "no download for {arch:?}"),
            Unsupported::Target(target) => write!(f, "runs `{target}`, not an exe, bat or cmd"),
            Unsupported::Hash(hash) => write!(f, "invalid hash `{hash}`"),
            Unsupported::Malformed(what) => write!(f, "malformed {what}"),
        }
    }
}

/// Everything needed to install a package for one architecture.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub arch: Arch,
    pub downloads: Vec<Download>,
    pub bins: Vec<Bin>,
    pub shortcuts: Vec<Shortcut>,
    pub persist: Vec<Persist>,
    pub env_set: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Download {
    /// The url without the `#/name` suffix.
    pub url: String,
    /// Name to save the file as, from `#/name` or the url path.
    pub file_name: String,
    pub hash: Option<Hash>,
    pub extract: Extract,
    /// Directory inside the archive whose contents to install.
    pub extract_dir: Option<String>,
    /// Subdirectory of the install dir to extract into.
    pub extract_to: Option<String>,
}

/// How a download becomes files in the install dir, chosen like Scoop does
/// from the file name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Extract {
    /// Copied as is.
    None,
    Zip,
    /// `msiexec /a`.
    Msi,
    /// innounp, for `.exe` files of manifests with `innosetup`.
    Inno,
    /// 7-Zip, for every other archive format.
    SevenZip,
}

impl Extract {
    pub fn of(file_name: &str, innosetup: bool) -> Extract {
        let name = file_name.to_ascii_lowercase();
        if name.ends_with(".zip") {
            Extract::Zip
        } else if name.ends_with(".msi") {
            Extract::Msi
        } else if name.ends_with(".exe") {
            if innosetup { Extract::Inno } else { Extract::None }
        } else if is_7z_format(&name) {
            Extract::SevenZip
        } else {
            Extract::None
        }
    }
}

/// Scoop's `Test-7zipRequirement`:
/// `\.(001|7z|bz(ip)?2?|gz|img|iso|lzma|lzh|nupkg|rar|tar|t[abgpx]z2?|t?zst|xz)(\.[^\d.]+)?$`
fn is_7z_format(name: &str) -> bool {
    const EXTS: [&str; 13] = [
        "001", "7z", "gz", "img", "iso", "lzma", "lzh", "nupkg", "rar", "tar", "zst", "tzst", "xz",
    ];
    let matches = |ext: &str| {
        EXTS.contains(&ext)
            || matches!(ext, "bz" | "bz2" | "bzip" | "bzip2")
            || (ext.len() >= 3
                && ext.starts_with('t')
                && matches!(ext.as_bytes()[1], b'a' | b'b' | b'g' | b'p' | b'x')
                && (&ext[2..] == "z" || &ext[2..] == "z2"))
    };
    let segs: Vec<&str> = name.split('.').collect();
    match segs.as_slice() {
        [.., ext] if segs.len() >= 2 && matches(ext) => true,
        // A trailing suffix without digits, like `.7z.download`.
        [.., ext, suffix] if segs.len() >= 3 => {
            matches(ext) && !suffix.is_empty() && !suffix.bytes().any(|b| b.is_ascii_digit())
        }
        _ => false,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bin {
    /// Relative to the install dir.
    pub target: String,
    /// Command name, the target's file stem unless given.
    pub name: String,
    pub args: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shortcut {
    /// Relative to the install dir.
    pub target: String,
    /// Start menu entry name; may contain `\` for a subfolder.
    pub name: String,
    pub args: Option<String>,
    /// Relative to the install dir.
    pub icon: Option<String>,
    pub aumid: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Persist {
    /// Relative to the install dir.
    pub source: String,
    /// Relative to the package's var dir.
    pub target: String,
}

impl Manifest {
    pub fn parse(bytes: &[u8]) -> serde_json::Result<Manifest> {
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        serde_json::from_slice(bytes)
    }

    fn arch_fields(&self, arch: Arch) -> Option<&ArchFields> {
        arch.manifest_keys()
            .iter()
            .find_map(|key| self.architecture.get(*key))
    }

    /// The install plan for `arch`, or why wpk cannot install the package.
    pub fn plan(&self, arch: Arch) -> Result<Plan, Unsupported> {
        let specific = self.arch_fields(arch);
        for fields in [Some(&self.common), specific].into_iter().flatten() {
            if let Some(field) = REJECTED_FIELDS.iter().find(|f| fields.rest.contains_key(**f)) {
                return Err(Unsupported::Field((*field).to_owned()));
            }
        }
        // An architecture's field replaces the top-level one as a whole.
        let pick = |get: fn(&ArchFields) -> bool| -> &ArchFields {
            match specific {
                Some(fields) if get(fields) => fields,
                _ => &self.common,
            }
        };

        let urls = &pick(|f| f.url.is_some()).url;
        let Some(OneOrMany(urls)) = urls.as_ref().filter(|u| !u.0.is_empty()) else {
            return Err(Unsupported::Arch(arch));
        };
        let hashes = pick(|f| f.hash.is_some())
            .hash
            .clone()
            .unwrap_or_default()
            .0;
        if hashes.len() > urls.len() {
            return Err(Unsupported::Malformed("hash".into()));
        }
        let extract_dirs = pick(|f| f.extract_dir.is_some())
            .extract_dir
            .clone()
            .unwrap_or_default()
            .0;

        let mut downloads = Vec::with_capacity(urls.len());
        // Like Scoop, extract_dir and extract_to pair with the archives only.
        let mut archives = 0;
        for (i, raw) in urls.iter().enumerate() {
            let hash = match hashes.get(i) {
                Some(h) => Some(Hash::parse(h).ok_or_else(|| Unsupported::Hash(h.clone()))?),
                None => None,
            };
            let (url, file_name) = split_url(raw);
            let extract = Extract::of(&file_name, self.innosetup);
            let (extract_dir, extract_to) = if extract == Extract::None {
                (None, None)
            } else {
                archives += 1;
                let at = |list: &[String]| list.get(archives - 1).cloned().filter(|d| !d.is_empty());
                (at(&extract_dirs), at(&self.extract_to.0))
            };
            downloads.push(Download {
                url,
                file_name,
                hash,
                extract,
                extract_dir,
                extract_to,
            });
        }

        let mut bins = Vec::new();
        for entry in pick(|f| f.bin.is_some()).bin.iter().flat_map(|b| &b.0) {
            let (target, name, args) = match entry.0.as_slice() {
                [target] => (target, None, None),
                [target, name] => (target, Some(name), None),
                [target, name, args, ..] => (target, Some(name), Some(args)),
                [] => return Err(Unsupported::Malformed("bin".into())),
            };
            check_runnable(target)?;
            bins.push(Bin {
                target: target.clone(),
                name: name
                    .filter(|n| !n.is_empty())
                    .cloned()
                    .unwrap_or_else(|| file_stem(target)),
                args: args.filter(|a| !a.is_empty()).cloned(),
            });
        }

        let mut shortcuts = Vec::new();
        for entry in pick(|f| f.shortcuts.is_some()).shortcuts.iter().flatten() {
            let [target, name, rest @ ..] = entry.as_slice() else {
                return Err(Unsupported::Malformed("shortcuts".into()));
            };
            check_runnable(target)?;
            let optional = |i: usize| rest.get(i).filter(|s| !s.is_empty()).cloned();
            shortcuts.push(Shortcut {
                target: target.clone(),
                name: name.clone(),
                args: optional(0),
                icon: optional(1),
                aumid: self.shortcut_aumid.get(name).cloned(),
            });
        }

        let mut persist = Vec::new();
        for entry in &self.persist.0 {
            let (source, target) = match entry.0.as_slice() {
                [source] => (source, source),
                [source, target, ..] => (source, target),
                [] => return Err(Unsupported::Malformed("persist".into())),
            };
            persist.push(Persist {
                source: trim_slashes(source),
                target: trim_slashes(target),
            });
        }

        Ok(Plan {
            arch,
            downloads,
            bins,
            shortcuts,
            persist,
            env_set: pick(|f| f.env_set.is_some())
                .env_set
                .clone()
                .unwrap_or_default(),
        })
    }
}

fn check_runnable(target: &str) -> Result<(), Unsupported> {
    let ext = target.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase());
    match ext {
        Some(ext) if RUNNABLE_EXTENSIONS.contains(&ext.as_str()) => Ok(()),
        _ => Err(Unsupported::Target(target.to_owned())),
    }
}

fn file_stem(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_owned(),
        _ => name.to_owned(),
    }
}

fn trim_slashes(path: &str) -> String {
    path.trim_matches(['/', '\\']).to_owned()
}

/// Split `https://host/a/b.zip?x=1#/c.7z` into the url and the name to save
/// the download as.
fn split_url(raw: &str) -> (String, String) {
    if let Some((url, name)) = raw.split_once("#/") {
        return (url.to_owned(), name.to_owned());
    }
    let url = raw.split_once('#').map_or(raw, |(url, _)| url);
    let path = url.split(['?']).next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or(path);
    (url.to_owned(), percent_decode(name))
}

fn percent_decode(s: &str) -> String {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(hi << 4 | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(json: &str) -> Manifest {
        Manifest::parse(json.as_bytes()).unwrap()
    }

    #[test]
    fn plan_uses_architecture_overrides() {
        let m = manifest(
            r#"{
                "version": "1.0",
                "license": {"identifier": "MIT", "url": "https://x"},
                "bin": [["fd.exe", "f", "--hidden"], "rg.exe"],
                "architecture": {
                    "64bit": {
                        "url": "https://h/fd-x64.zip",
                        "hash": "ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
                        "extract_dir": "fd-x64"
                    },
                    "arm64": {
                        "url": ["https://h/fd-arm.exe#/fd.exe", "https://h/x%20y.zip?d=1"],
                        "hash": ["md5:0123456789abcdef0123456789abcdef"]
                    }
                }
            }"#,
        );
        let x64 = m.plan(Arch::X64).unwrap();
        assert_eq!(x64.downloads.len(), 1);
        assert_eq!(x64.downloads[0].file_name, "fd-x64.zip");
        assert_eq!(x64.downloads[0].extract_dir.as_deref(), Some("fd-x64"));
        assert_eq!(x64.downloads[0].hash.as_ref().unwrap().algo, HashAlgo::Sha256);
        assert_eq!(
            x64.bins,
            vec![
                Bin {
                    target: "fd.exe".into(),
                    name: "f".into(),
                    args: Some("--hidden".into())
                },
                Bin {
                    target: "rg.exe".into(),
                    name: "rg".into(),
                    args: None
                },
            ]
        );

        let arm = m.plan(Arch::Arm64).unwrap();
        assert_eq!(arm.downloads[0].url, "https://h/fd-arm.exe");
        assert_eq!(arm.downloads[0].file_name, "fd.exe");
        assert_eq!(arm.downloads[1].file_name, "x y.zip");
        assert_eq!(arm.downloads[1].hash, None);
        assert_eq!(arm.downloads[0].extract_dir, None);
    }

    #[test]
    fn arm64_falls_back_to_x64() {
        let m = manifest(r#"{"version": "1", "architecture": {"64bit": {"url": "https://h/a.zip"}}}"#);
        assert_eq!(m.plan(Arch::Arm64).unwrap().downloads[0].file_name, "a.zip");
    }

    #[test]
    fn rejects_scripts_and_path_changes() {
        let m = manifest(r#"{"version": "1", "url": "https://h/a.zip", "post_install": "x"}"#);
        assert_eq!(m.plan(Arch::X64), Err(Unsupported::Field("post_install".into())));
        let m = manifest(
            r#"{"version": "1", "url": "https://h/a.zip",
                "architecture": {"64bit": {"env_add_path": "bin"}}}"#,
        );
        assert_eq!(m.plan(Arch::X64), Err(Unsupported::Field("env_add_path".into())));
    }

    #[test]
    fn rejects_other_targets_and_32bit_only() {
        let m = manifest(r#"{"version": "1", "url": "https://h/a.zip", "bin": "a.ps1"}"#);
        assert_eq!(m.plan(Arch::X64), Err(Unsupported::Target("a.ps1".into())));
        let m = manifest(r#"{"version": "1", "architecture": {"32bit": {"url": "https://h/a.zip"}}}"#);
        assert_eq!(m.plan(Arch::X64), Err(Unsupported::Arch(Arch::X64)));
    }

    #[test]
    fn shortcuts_and_persist() {
        let m = manifest(
            r#"{
                "version": "1", "url": "https://h/a.zip",
                "shortcuts": [["w.exe", "WezTerm", "", "w.ico"]],
                "shortcut_aumid": {"WezTerm": "org.wezfurlong.wezterm"},
                "persist": ["conf\\", ["data", "d"]]
            }"#,
        );
        let plan = m.plan(Arch::X64).unwrap();
        assert_eq!(plan.shortcuts[0].args, None);
        assert_eq!(plan.shortcuts[0].icon.as_deref(), Some("w.ico"));
        assert_eq!(plan.shortcuts[0].aumid.as_deref(), Some("org.wezfurlong.wezterm"));
        assert_eq!(plan.persist[0].target, "conf");
        assert_eq!(plan.persist[1].target, "d");
    }

    #[test]
    fn extract_kinds() {
        assert_eq!(Extract::of("a.ZIP", false), Extract::Zip);
        assert_eq!(Extract::of("a.exe", false), Extract::None);
        assert_eq!(Extract::of("a.exe", true), Extract::Inno);
        assert_eq!(Extract::of("a.msi", true), Extract::Msi);
        for name in ["a.tar.gz", "a.tgz", "a.7z", "a.tbz2", "a.bzip2", "a.tar.zst", "a.7z.download", "a.nupkg"] {
            assert_eq!(Extract::of(name, false), Extract::SevenZip, "{name}");
        }
        for name in ["a.jar", "7z", "a.7z.v2", "a.dll", "a.tz3"] {
            assert_eq!(Extract::of(name, false), Extract::None, "{name}");
        }
    }

    #[test]
    fn extract_dir_pairs_with_archives_only() {
        let m = manifest(
            r#"{"version": "1", "url": ["https://h/tool.exe", "https://h/a.zip"],
                "extract_dir": "inner", "extract_to": "sub"}"#,
        );
        let plan = m.plan(Arch::X64).unwrap();
        assert_eq!(plan.downloads[0].extract_dir, None);
        assert_eq!(plan.downloads[1].extract_dir.as_deref(), Some("inner"));
        assert_eq!(plan.downloads[1].extract_to.as_deref(), Some("sub"));
    }

    #[test]
    fn hash_forms() {
        assert_eq!(Hash::parse(&"a".repeat(40)).unwrap().algo, HashAlgo::Sha1);
        assert_eq!(Hash::parse(&format!("sha512:{}", "A".repeat(128))).unwrap().hex, "a".repeat(128));
        assert!(Hash::parse("sha256:abc").is_none());
        assert!(Hash::parse(&"g".repeat(64)).is_none());
    }

    /// Parses every manifest of an installed Scoop bucket and prints how
    /// many wpk can install. Run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn scoop_buckets() {
        let Some(home) = std::env::home_dir() else { return };
        for bucket in ["main", "extras"] {
            let dir = home.join("scoop").join("buckets").join(bucket).join("bucket");
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            let (mut total, mut ok, mut broken) = (0, 0, Vec::new());
            let mut reasons = BTreeMap::<String, usize>::new();
            for entry in entries.flatten() {
                total += 1;
                let bytes = std::fs::read(entry.path()).unwrap();
                match Manifest::parse(&bytes) {
                    Err(e) => broken.push(format!("{}: {e}", entry.path().display())),
                    Ok(m) => match m.plan(Arch::X64) {
                        Ok(_) => ok += 1,
                        Err(e) => {
                            let key = match e {
                                Unsupported::Target(t) => format!("target .{}", t.rsplit('.').next().unwrap_or("")),
                                Unsupported::Hash(_) => "hash".into(),
                                e => e.to_string(),
                            };
                            *reasons.entry(key).or_default() += 1;
                        }
                    },
                }
            }
            println!("{bucket}: {total} total, {ok} installable, {} unparsable", broken.len());
            for b in &broken {
                println!("  {b}");
            }
            for (reason, n) in &reasons {
                println!("  {n:5} {reason}");
            }
        }
    }
}
