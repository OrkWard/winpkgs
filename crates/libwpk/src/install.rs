//! Installing, upgrading and uninstalling packages.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::archive::Tools;
use crate::installed::{self, Installed, Receipt};
use crate::manifest::Plan;
use crate::query::{Candidate, split_query};
use crate::shim::{ShimExes, ShimFile};
use crate::{Error, Layout, Result, Wpk, archive, download, gc, persist, shim, shortcut};

/// What a sync reports to the frontend as it goes.
#[derive(Debug)]
pub enum Event<'a> {
    /// Will be installed, or upgraded from `from`.
    Planned {
        name: &'a str,
        version: &'a str,
        from: Option<&'a str>,
    },
    /// Nothing to do, and why.
    Skipped {
        name: &'a str,
        reason: &'a str,
    },
    Progress {
        name: &'a str,
        file: &'a str,
        done: u64,
        total: Option<u64>,
    },
    Downloaded {
        name: &'a str,
        file: &'a str,
    },
    Installed {
        name: &'a str,
        version: &'a str,
    },
    Uninstalled {
        name: &'a str,
    },
    /// The manifest's `notes`.
    Notes {
        name: &'a str,
        lines: &'a [String],
    },
    /// `env_set` entries the user has to set up.
    EnvSet {
        name: &'a str,
        vars: &'a BTreeMap<String, String>,
    },
    /// Optional companions from `suggest`.
    Suggest {
        name: &'a str,
        packages: Vec<&'a str>,
    },
    Warning(String),
}

pub trait Reporter: Sync {
    fn event(&self, event: Event<'_>);
}

impl<F: Fn(Event<'_>) + Sync> Reporter for F {
    fn event(&self, event: Event<'_>) {
        self(event)
    }
}

/// A package to deploy.
struct Job {
    candidate: Candidate,
    plan: Plan,
    explicit: bool,
    previous: Option<Installed>,
}

impl Wpk {
    /// Install packages not yet installed, with their dependencies.
    pub fn install(&self, queries: &[String], reporter: &dyn Reporter) -> Result<()> {
        let _lock = self.lock()?;
        let mut jobs = Vec::new();
        let mut seen = HashSet::new();
        for query in queries {
            let candidate = self.find(query)?;
            if let Some(installed) = installed::get(&self.layout, &candidate.name)? {
                let reason = format!("{} is already installed", installed.receipt.version);
                reporter.event(Event::Skipped {
                    name: &candidate.name,
                    reason: &reason,
                });
                continue;
            }
            self.add_job(candidate, true, &mut jobs, &mut seen, &mut Vec::new())?;
        }
        self.run_jobs(jobs, reporter)
    }

    /// Upgrade the named packages, or every package that is not held.
    pub fn upgrade(&self, names: &[String], reporter: &dyn Reporter) -> Result<()> {
        let _lock = self.lock()?;
        let targets: Vec<Installed> = if names.is_empty() {
            installed::list(&self.layout, |name, e| {
                reporter.event(Event::Warning(format!("{name}: {e}")));
            })?
        } else {
            names
                .iter()
                .map(|name| {
                    installed::get(&self.layout, name)?
                        .ok_or_else(|| Error::NotInstalled(name.clone()))
                })
                .collect::<Result<_>>()?
        };

        let mut jobs = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for target in targets {
            let name = target.name.as_str();
            if target.held {
                reporter.event(Event::Skipped {
                    name,
                    reason: "held",
                });
                continue;
            }
            let query = format!("{}/{name}", target.receipt.bucket);
            let candidate = match self.find(&query) {
                Ok(candidate) => candidate,
                Err(e) => {
                    reporter.event(Event::Warning(format!("{name}: {e}")));
                    continue;
                }
            };
            // Versions differing only in case share a directory.
            if candidate
                .manifest
                .version
                .eq_ignore_ascii_case(&target.receipt.version)
            {
                continue;
            }
            // A running old version would save its persist_copy files after
            // they were copied, into a dir no longer current.
            let copies = candidate.plan().is_ok_and(|plan| !plan.persist_copy.is_empty());
            if copies
                && let Some(busy) =
                    gc::find_busy_image(&self.layout.version(name, &target.receipt.version))
            {
                let reason = format!("{} is running; close it and upgrade again", busy.display());
                reporter.event(Event::Skipped { name, reason: &reason });
                continue;
            }
            self.add_job(
                candidate,
                target.receipt.explicit,
                &mut jobs,
                &mut seen,
                &mut Vec::new(),
            )?;
        }
        self.run_jobs(jobs, reporter)
    }

    /// Queue `candidate` after its missing dependencies.
    fn add_job(
        &self,
        candidate: Candidate,
        explicit: bool,
        jobs: &mut Vec<Job>,
        seen: &mut HashSet<String>,
        path: &mut Vec<String>,
    ) -> Result<()> {
        if path.contains(&candidate.name) {
            return Err(Error::DependencyCycle(candidate.name));
        }
        if !seen.insert(candidate.name.clone()) {
            return Ok(());
        }
        let version = &candidate.manifest.version;
        if !crate::layout::is_safe_name(&candidate.name) || !crate::layout::is_safe_name(version) {
            return Err(Error::Unsupported {
                package: candidate.ident(),
                reason: crate::manifest::Unsupported::Malformed(format!(
                    "name or version `{version}`"
                )),
            });
        }
        let plan = candidate.plan().map_err(|reason| Error::Unsupported {
            package: candidate.ident(),
            reason,
        })?;
        path.push(candidate.name.clone());
        for dep in &candidate.manifest.depends.0 {
            let (_, dep_name) = split_query(dep);
            if seen.contains(&dep_name.to_ascii_lowercase())
                || installed::current(&self.layout, &dep_name.to_ascii_lowercase())?.is_some()
            {
                continue;
            }
            let dep = self.find(dep)?;
            self.add_job(dep, false, jobs, seen, path)?;
        }
        path.pop();
        let previous = installed::get(&self.layout, &candidate.name)?;
        jobs.push(Job {
            candidate,
            plan,
            explicit,
            previous,
        });
        Ok(())
    }

    fn run_jobs(&self, jobs: Vec<Job>, reporter: &dyn Reporter) -> Result<()> {
        if jobs.is_empty() {
            return Ok(());
        }
        self.check_conflicts(&jobs)?;
        for job in &jobs {
            reporter.event(Event::Planned {
                name: &job.candidate.name,
                version: &job.candidate.manifest.version,
                from: job.previous.as_ref().map(|p| p.receipt.version.as_str()),
            });
        }

        let files = self.download_all(&jobs, reporter)?;
        for (job, files) in jobs.iter().zip(files) {
            let tools = self.tools();
            self.deploy(job, &files, &tools, &self.shims)?;
            let c = &job.candidate;
            reporter.event(Event::Installed {
                name: &c.name,
                version: &c.manifest.version,
            });
            if !c.manifest.notes.0.is_empty() {
                let vars = Vars::of(&self.layout, &c.name, &c.manifest.version);
                let lines: Vec<String> = c.manifest.notes.0.iter().map(|l| vars.expand(l)).collect();
                reporter.event(Event::Notes {
                    name: &c.name,
                    lines: &lines,
                });
            }
            if !job.plan.env_set.is_empty() {
                let vars = Vars::of(&self.layout, &c.name, &c.manifest.version);
                let expanded: BTreeMap<String, String> = job
                    .plan
                    .env_set
                    .iter()
                    .map(|(k, v)| (k.clone(), vars.expand(v)))
                    .collect();
                reporter.event(Event::EnvSet {
                    name: &c.name,
                    vars: &expanded,
                });
            }
            let suggested: Vec<&str> = c
                .manifest
                .suggest
                .values()
                .flat_map(|list| list.0.iter().map(String::as_str))
                .filter(|s| {
                    installed::current(&self.layout, split_query(s).1)
                        .ok()
                        .flatten()
                        .is_none()
                })
                .collect();
            if !suggested.is_empty() {
                reporter.event(Event::Suggest {
                    name: &c.name,
                    packages: suggested,
                });
            }
        }
        if let Err(e) = gc::run_if_due(self) {
            reporter.event(Event::Warning(format!("gc: {e}")));
        }
        Ok(())
    }

    /// Fail when a package would take a command or shortcut another owns.
    fn check_conflicts(&self, jobs: &[Job]) -> Result<()> {
        let mut owners: HashMap<String, String> = HashMap::new();
        for other in installed::list(&self.layout, |_, _| {})? {
            for bin in &other.receipt.bins {
                owners.insert(
                    format!("bin:{}", bin.to_ascii_lowercase()),
                    other.name.clone(),
                );
            }
            for lnk in &other.receipt.shortcuts {
                owners.insert(
                    format!("lnk:{}", lnk.to_ascii_lowercase()),
                    other.name.clone(),
                );
            }
        }
        for job in jobs {
            let name = &job.candidate.name;
            let keys = job
                .plan
                .bins
                .iter()
                .map(|b| (format!("bin:{}", b.name.to_ascii_lowercase()), &b.name))
                .chain(job.plan.shortcuts.iter().map(|s| {
                    (
                        format!("lnk:{}", shortcut_file(&s.name).to_ascii_lowercase()),
                        &s.name,
                    )
                }));
            for (key, shown) in keys {
                match owners.get(&key) {
                    Some(owner) if owner != name => {
                        return Err(Error::Conflict {
                            package: name.clone(),
                            name: shown.clone(),
                            owner: owner.clone(),
                        });
                    }
                    _ => {
                        owners.insert(key, name.clone());
                    }
                }
            }
        }
        Ok(())
    }

    fn download_all(&self, jobs: &[Job], reporter: &dyn Reporter) -> Result<Vec<Vec<PathBuf>>> {
        let results: Mutex<HashMap<(usize, usize), Result<PathBuf>>> = Mutex::new(HashMap::new());
        std::thread::scope(|scope| {
            for (j, job) in jobs.iter().enumerate() {
                for (d, dl) in job.plan.downloads.iter().enumerate() {
                    let results = &results;
                    scope.spawn(move || {
                        let name = job.candidate.name.as_str();
                        let file = dl.file_name.as_str();
                        let progress = |done, total| {
                            reporter.event(Event::Progress {
                                name,
                                file,
                                done,
                                total,
                            });
                        };
                        let result = download::fetch(
                            &self.layout,
                            self.config.proxy.as_deref(),
                            name,
                            &job.candidate.manifest.version,
                            dl,
                            &progress,
                        );
                        if result.is_ok() {
                            reporter.event(Event::Downloaded { name, file });
                        }
                        results.lock().unwrap().insert((j, d), result);
                    });
                }
            }
        });
        let mut results = results.into_inner().unwrap();
        jobs.iter()
            .enumerate()
            .map(|(j, job)| {
                (0..job.plan.downloads.len())
                    .map(|d| results.remove(&(j, d)).expect("every download ran"))
                    .collect()
            })
            .collect()
    }

    fn tools(&self) -> Tools {
        let find = |package: &str, exe: &str| {
            let version = installed::current(&self.layout, package).ok().flatten()?;
            let path = self.layout.version(package, &version).join(exe);
            path.exists().then_some(path)
        };
        Tools {
            seven_zip: find("7zip", "7z.exe"),
            innounp: find("innounp", "innounp.exe"),
        }
    }

    /// Put one package version in place and switch to it.
    fn deploy(&self, job: &Job, files: &[PathBuf], tools: &Tools, exes: &ShimExes) -> Result<()> {
        let layout = &self.layout;
        let c = &job.candidate;
        let name = c.name.as_str();
        let version = c.manifest.version.as_str();
        let dir = layout.version(name, version);
        let install_err = |message: String| Error::Install {
            package: name.to_owned(),
            message,
        };
        if let Some(previous) = &job.previous
            && layout
                .version(name, &previous.receipt.version)
                .to_string_lossy()
                .to_lowercase()
                == dir.to_string_lossy().to_lowercase()
        {
            return Err(install_err(format!(
                "{version} would replace the installed {}",
                previous.receipt.version
            )));
        }

        // A leftover of this version that is not current, e.g. from a
        // reinstall or a downgrade.
        crate::fs::remove_dir_all(&dir)?;
        let mut stage = dir.clone().into_os_string();
        stage.push(".tmp");
        let stage = PathBuf::from(stage);
        crate::fs::remove_dir_all(&stage)?;
        crate::fs::create_dir_all(&stage)?;
        let staged = (|| {
            for (dl, cached) in job.plan.downloads.iter().zip(files) {
                let file = stage.join(&dl.file_name);
                // Files installed as is get their own copy, so changes to
                // them never reach the cache.
                if dl.extract == crate::manifest::Extract::None {
                    std::fs::copy(cached, &file).map_err(crate::fs::io_error(&file))?;
                } else {
                    link_or_copy(cached, &file)?;
                }
                archive::unpack(&file, dl, &stage, tools)?;
            }
            Ok(())
        })();
        if let Err(e) = staged {
            let _ = crate::fs::remove_dir_all(&stage);
            return Err(e);
        }
        crate::fs::rename(&stage, &dir)?;

        let prepared = (|| {
            persist::link(&dir, &layout.persist(name), &job.plan.persist)?;
            let installed_dir = job
                .previous
                .as_ref()
                .map(|p| layout.version(name, &p.receipt.version));
            persist::copy_forward(
                &dir,
                installed_dir.as_deref(),
                &layout.persist(name),
                &job.plan.persist_copy,
            )?;
            for bin in &job.plan.bins {
                if let Some(source) = &bin.link_of {
                    let link = dir.join(&bin.target);
                    if !link.exists() {
                        std::fs::hard_link(dir.join(source), &link).map_err(crate::fs::io_error(&link))?;
                    }
                }
            }
            for bin in &job.plan.bins {
                if !dir.join(&bin.target).is_file() {
                    return Err(install_err(format!(
                        "bin '{}' not found in the package",
                        bin.target
                    )));
                }
            }
            for s in &job.plan.shortcuts {
                if !dir.join(&s.target).is_file() {
                    return Err(install_err(format!(
                        "shortcut target '{}' not found",
                        s.target
                    )));
                }
            }
            let records = layout.records(name, version);
            crate::fs::create_dir_all(&records)?;
            crate::fs::write_atomic(&records.join("manifest.json"), &c.raw)
        })();
        if let Err(e) = prepared {
            let _ = crate::fs::remove_dir_all(&dir);
            return Err(e);
        }

        // From here on the new version is complete. If linking it fails,
        // the previous version's links come back and the new dir goes.
        let mut receipt = Receipt {
            version: version.to_owned(),
            bucket: c.bucket.clone(),
            commit: c.commit.clone(),
            arch: job.plan.arch,
            installed_at: jiff::Timestamp::now(),
            explicit: job.explicit,
            bins: Vec::new(),
            launchers: Vec::new(),
            shortcuts: Vec::new(),
        };
        if let Err(e) = link_version(layout, name, version, &job.plan, exes, &mut receipt) {
            let mut restored = job.previous.is_none();
            if let Some(previous) = &job.previous {
                remove_stale(layout, name, &receipt, &previous.receipt).ok();
                restored = relink_previous(layout, name, &previous.receipt, exes).is_ok();
            } else {
                let empty = Receipt {
                    bins: Vec::new(),
                    launchers: Vec::new(),
                    shortcuts: Vec::new(),
                    ..receipt.clone()
                };
                remove_stale(layout, name, &receipt, &empty).ok();
            }
            if restored {
                let _ = crate::fs::remove_dir_all(&dir);
            }
            return Err(e);
        }
        installed::write_receipt(layout, name, &receipt)?;
        installed::set_current(layout, name, version)?;

        if let Some(previous) = &job.previous {
            remove_stale(layout, name, &previous.receipt, &receipt)?;
        }
        Ok(())
    }

    /// Remove packages; their data in `var` stays unless `purge`.
    pub fn uninstall(&self, names: &[String], purge: bool, reporter: &dyn Reporter) -> Result<()> {
        let _lock = self.lock()?;
        for name in names {
            let name = name.to_ascii_lowercase();
            if name == crate::layout::SELF_PACKAGE {
                return Err(Error::UninstallSelf);
            }
            let Some(installed) = installed::get(&self.layout, &name)? else {
                return Err(Error::NotInstalled(name));
            };
            let version_dir = self.layout.version(&name, &installed.receipt.version);
            let copies = saved_plan(&self.layout, &name, &installed.receipt)
                .map(|plan| plan.persist_copy)
                .unwrap_or_default();
            persist::copy_back(&version_dir, &self.layout.persist(&name), &copies)?;
            let empty = Receipt {
                bins: Vec::new(),
                launchers: Vec::new(),
                shortcuts: Vec::new(),
                ..installed.receipt.clone()
            };
            remove_stale(&self.layout, &name, &installed.receipt, &empty)?;
            let _ = std::fs::remove_dir(self.layout.launchers(&name));
            crate::fs::remove_file(&self.layout.current(&name))?;
            crate::fs::remove_file(&self.layout.held(&name))?;
            // Versions in use stay until a later GC.
            if let Err(e) = gc::collect_package(&self.layout, &name) {
                reporter.event(Event::Warning(format!("{name}: {e}")));
            }
            if purge {
                crate::fs::remove_dir_all(&self.layout.persist(&name))?;
            }
            reporter.event(Event::Uninstalled { name: &name });
        }
        Ok(())
    }

    /// Hold the global lock until the guard drops.
    pub fn lock(&self) -> Result<std::fs::File> {
        lock(&self.layout)
    }
}

/// Hold the global lock of `layout` until the guard drops. Not reentrant:
/// taking it twice in one process blocks.
pub fn lock(layout: &Layout) -> Result<std::fs::File> {
    let path = layout.lock();
    crate::fs::create_dir_all(path.parent().expect("lock has a parent"))?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(crate::fs::io_error(&path))?;
    file.lock().map_err(crate::fs::io_error(&path))?;
    Ok(file)
}

/// Point the shims, launchers and shortcuts of `plan` at `version`,
/// recording each in `links` as soon as it exists. Icons are prepared
/// first, so the common failures happen before any shim changes.
fn link_version(
    layout: &Layout,
    name: &str,
    version: &str,
    plan: &Plan,
    exes: &ShimExes,
    links: &mut Receipt,
) -> Result<()> {
    let dir = layout.version(name, version);
    let vars = Vars::of(layout, name, version);
    let launchers = layout.launchers(name);
    let mut icons = Vec::with_capacity(plan.shortcuts.len());
    for s in &plan.shortcuts {
        crate::fs::create_dir_all(&launchers)?;
        let ico = launchers.join(format!("{}.ico", launcher_name(&s.name)));
        let source = s
            .icon
            .as_ref()
            .map(|i| dir.join(i))
            .unwrap_or_else(|| dir.join(&s.target));
        let has_icon = if source
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("ico"))
        {
            std::fs::copy(&source, &ico).is_ok()
        } else {
            shortcut::extract_icon(&source, &ico)?
        };
        icons.push(has_icon.then_some(ico));
    }

    let bin_dir = layout.bin();
    for bin in &plan.bins {
        let shim = ShimFile {
            path: dir.join(&bin.target).display().to_string(),
            args: bin.args.as_deref().map(|a| vars.expand(a)),
            dir: None,
        };
        links.bins.push(bin.name.clone());
        shim::write(&bin_dir, &bin.name, &shim, exes)?;
    }
    for (s, icon) in plan.shortcuts.iter().zip(icons) {
        let launcher = launcher_name(&s.name);
        let shim = ShimFile {
            path: dir.join(&s.target).display().to_string(),
            args: s.args.as_deref().map(|a| vars.expand(a)),
            dir: Some(dir.display().to_string()),
        };
        links.launchers.push(launcher.clone());
        shim::write(&launchers, &launcher, &shim, exes)?;
        let file = shortcut_file(&s.name);
        let aumid = s
            .aumid
            .clone()
            .unwrap_or_else(|| shortcut::aumid(name, &s.name));
        links.shortcuts.push(file.clone());
        shortcut::create(
            &layout.start_menu().join(&file),
            &launchers.join(format!("{launcher}.exe")),
            icon.as_deref(),
            &aumid,
        )?;
    }
    Ok(())
}

/// Link the previously installed version again from its saved manifest.
fn relink_previous(layout: &Layout, name: &str, previous: &Receipt, exes: &ShimExes) -> Result<()> {
    let plan = saved_plan(layout, name, previous)?;
    let mut links = Receipt {
        bins: Vec::new(),
        launchers: Vec::new(),
        shortcuts: Vec::new(),
        ..previous.clone()
    };
    link_version(layout, name, &previous.version, &plan, exes, &mut links)
}

/// The plan of an installed version, from the manifest saved with it.
fn saved_plan(layout: &Layout, name: &str, receipt: &Receipt) -> Result<Plan> {
    let path = layout
        .records(name, &receipt.version)
        .join("manifest.json");
    let manifest =
        crate::Manifest::parse(&crate::fs::read(&path)?).map_err(|e| Error::Corrupt {
            path: path.clone(),
            message: e.to_string(),
        })?;
    manifest
        .plan(receipt.arch)
        .map_err(|reason| Error::Unsupported {
            package: name.to_owned(),
            reason,
        })
}

/// Remove what `old` created and `new` does not.
fn remove_stale(layout: &Layout, name: &str, old: &Receipt, new: &Receipt) -> Result<()> {
    let keep = |list: &[String], item: &str| list.iter().any(|x| x.eq_ignore_ascii_case(item));
    for bin in &old.bins {
        if !keep(&new.bins, bin) {
            shim::remove(&layout.bin(), bin)?;
        }
    }
    let launchers = layout.launchers(name);
    for launcher in &old.launchers {
        if !keep(&new.launchers, launcher) {
            shim::remove(&launchers, launcher)?;
            crate::fs::remove_file(&launchers.join(format!("{launcher}.ico")))?;
        }
    }
    for lnk in &old.shortcuts {
        if !keep(&new.shortcuts, lnk) {
            let path = layout.start_menu().join(lnk);
            crate::fs::remove_file(&path)?;
            // Drop subfolders the shortcut leaves empty.
            let mut parent = path.parent();
            while let Some(dir) = parent {
                if dir == layout.start_menu() || std::fs::remove_dir(dir).is_err() {
                    break;
                }
                parent = dir.parent();
            }
        }
    }
    Ok(())
}

/// Shortcut path under the Start menu folder for a `shortcuts` name.
fn shortcut_file(name: &str) -> String {
    format!("{}.lnk", name.replace('/', "\\"))
}

/// File name of the launcher shim for a shortcut name.
fn launcher_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '-'
            } else {
                c
            }
        })
        .collect()
}

/// Scoop's variables in `bin` and shortcut arguments and `env_set`.
struct Vars<'a> {
    dir: PathBuf,
    persist: PathBuf,
    version: &'a str,
    app: &'a str,
}

impl<'a> Vars<'a> {
    fn of(layout: &Layout, app: &'a str, version: &'a str) -> Vars<'a> {
        Vars {
            dir: layout.version(app, version),
            persist: layout.persist(app),
            version,
            app,
        }
    }

    fn expand(&self, s: &str) -> String {
        let dir = self.dir.display().to_string();
        s.replace("$original_dir", &dir)
            .replace("$persist_dir", &self.persist.display().to_string())
            .replace("$dir", &dir)
            .replace("$version", self.version)
            .replace("$app", self.app)
    }
}

fn link_or_copy(from: &Path, to: &Path) -> Result<()> {
    if std::fs::hard_link(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)
        .map(|_| ())
        .map_err(crate::fs::io_error(to))
}
